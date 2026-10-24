use keyring::Entry;
use thiserror::Error;

pub const SERVICE: &str = "melt";

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SecretError {
    #[error("secret not found")]
    NotFound,
    #[error("keychain operation failed: {0}")]
    Unavailable(String),
}

pub trait SecretStore {
    fn get(&self, service: &str, account: &str) -> Result<String, SecretError>;
    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), SecretError>;
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError>;
}

#[derive(Debug, Default)]
pub struct SystemKeychain;

impl SecretStore for SystemKeychain {
    fn get(&self, service: &str, account: &str) -> Result<String, SecretError> {
        self.entry(service, account)?
            .get_password()
            .map_err(map_keyring_error)
    }

    fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), SecretError> {
        self.entry(service, account)?
            .set_password(secret)
            .map_err(map_keyring_error)
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        self.entry(service, account)?
            .delete_credential()
            .map_err(map_keyring_error)
    }
}

impl SystemKeychain {
    fn entry(&self, service: &str, account: &str) -> Result<Entry, SecretError> {
        Entry::new(service, account).map_err(map_keyring_error)
    }
}

pub fn account(driver: &str, profile: &str, key: &str) -> String {
    format!("{driver}:{profile}:{key}")
}

fn map_keyring_error(error: keyring::Error) -> SecretError {
    match error {
        keyring::Error::NoEntry => SecretError::NotFound,
        other => SecretError::Unavailable(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_name_matches_the_existing_keychain_contract() {
        assert_eq!(
            account("bambu-lan", "garage-x1c", "access-code"),
            "bambu-lan:garage-x1c:access-code"
        );
    }

    // Keyring 4's v1 feature selects Keychain, Credential Manager and Secret
    // Service rather than requiring us to initialize a credential store.
    #[test]
    fn keyring_enables_a_real_platform_backend_on_every_target() {
        let manifest = include_str!("../../../Cargo.toml");
        let entry = manifest
            .lines()
            .find(|line| line.starts_with("keyring "))
            .expect("workspace manifest pins keyring");
        assert!(
            entry.contains("\"v1\""),
            "keyring must enable v1 to select persistent platform stores: {entry}"
        );
    }

    #[test]
    fn missing_secret_has_a_distinct_recovery_error() {
        assert_eq!(
            map_keyring_error(keyring::Error::NoEntry),
            SecretError::NotFound
        );
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::{collections::BTreeMap, sync::Mutex};

    use super::{SecretError, SecretStore};

    #[derive(Default)]
    pub(crate) struct MemoryStore(pub(crate) Mutex<BTreeMap<String, String>>);

    impl SecretStore for MemoryStore {
        fn get(&self, _: &str, account: &str) -> Result<String, SecretError> {
            self.0
                .lock()
                .unwrap()
                .get(account)
                .cloned()
                .ok_or(SecretError::NotFound)
        }
        fn set(&self, _: &str, account: &str, secret: &str) -> Result<(), SecretError> {
            self.0.lock().unwrap().insert(account.into(), secret.into());
            Ok(())
        }
        fn delete(&self, _: &str, account: &str) -> Result<(), SecretError> {
            self.0
                .lock()
                .unwrap()
                .remove(account)
                .map(|_| ())
                .ok_or(SecretError::NotFound)
        }
    }
}
