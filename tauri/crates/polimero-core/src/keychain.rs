use keyring::Entry;
use thiserror::Error;

pub const SERVICE: &str = "polimero";

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

    #[test]
    fn missing_secret_has_a_distinct_recovery_error() {
        assert_eq!(
            map_keyring_error(keyring::Error::NoEntry),
            SecretError::NotFound
        );
    }
}
