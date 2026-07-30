use std::path::Path;

use serde::Serialize;
use thiserror::Error;

use crate::{
    config::{Config, ConfigError},
    keychain::{SERVICE, SecretError, SecretStore, account},
};

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile name is required")]
    MissingName,
    #[error("invalid profile name")]
    InvalidName,
    #[error("printer profile {0:?} not found")]
    NotFound(String),
    #[error("keychain operation failed")]
    Secret(#[source] SecretError),
    #[error("cannot save config")]
    Config(#[source] ConfigError),
    #[error("cannot save config and could not restore stored secrets")]
    RollbackFailed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveResult {
    pub name: String,
    pub access_code_removed: bool,
    pub tls_fingerprint_removed: bool,
    pub warnings: Vec<RemoveWarning>,
}

#[derive(Debug, Serialize)]
pub struct RemoveWarning {
    pub code: &'static str,
    pub message: &'static str,
}

pub fn remove(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
) -> Result<RemoveResult, ProfileError> {
    let name = normalize_name(name)?;
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(ProfileError::Config)?;
    let profile = config
        .get_profile(&name)
        .cloned()
        .ok_or_else(|| ProfileError::NotFound(name.clone()))?;
    let mut secrets = [
        StoredSecret::load(store, account(&profile.driver, &name, "access-code"))?,
        StoredSecret::load(store, account(&profile.driver, &name, "tls-fingerprint"))?,
    ];
    let mut warnings = Vec::new();
    if !secrets[0].present() {
        warnings.push(RemoveWarning {
            code: "access-code-not-found",
            message: "profile was removed, but no stored access code was found",
        });
    }
    if !profile.insecure && profile.driver == "bambu-lan" && !secrets[1].present() {
        warnings.push(RemoveWarning {
            code: "tls-fingerprint-not-found",
            message: "profile was removed, but no stored TLS fingerprint was found",
        });
    }

    for secret in &mut secrets {
        if let Err(error) = secret.delete(store) {
            restore(store, &secrets)?;
            return Err(ProfileError::Secret(error));
        }
    }
    config.remove_profile(&name).map_err(ProfileError::Config)?;
    if let Err(error) = config.save(dir) {
        restore(store, &secrets)?;
        return Err(ProfileError::Config(error));
    }

    Ok(RemoveResult {
        name,
        access_code_removed: secrets[0].deleted,
        tls_fingerprint_removed: secrets[1].deleted,
        warnings,
    })
}

fn normalize_name(name: &str) -> Result<String, ProfileError> {
    let name = name.to_ascii_lowercase();
    if name.is_empty() {
        return Err(ProfileError::MissingName);
    }
    if name.len() > 64
        || !name.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'_' | b'-'))
        })
    {
        return Err(ProfileError::InvalidName);
    }
    Ok(name)
}

struct StoredSecret {
    account: String,
    value: Option<String>,
    deleted: bool,
}

impl StoredSecret {
    fn load(store: &dyn SecretStore, account: String) -> Result<Self, ProfileError> {
        match store.get(SERVICE, &account) {
            Ok(value) => Ok(Self {
                account,
                value: Some(value),
                deleted: false,
            }),
            Err(SecretError::NotFound) => Ok(Self {
                account,
                value: None,
                deleted: false,
            }),
            Err(error) => Err(ProfileError::Secret(error)),
        }
    }

    fn present(&self) -> bool {
        self.value.is_some()
    }

    fn delete(&mut self, store: &dyn SecretStore) -> Result<(), SecretError> {
        if self.present() {
            store.delete(SERVICE, &self.account)?;
            self.deleted = true;
        }
        Ok(())
    }
}

fn restore(store: &dyn SecretStore, secrets: &[StoredSecret]) -> Result<(), ProfileError> {
    for secret in secrets {
        if secret.deleted {
            store
                .set(
                    SERVICE,
                    &secret.account,
                    secret.value.as_deref().expect("deleted secret has value"),
                )
                .map_err(ProfileError::Secret)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use crate::config::Profile;

    use super::*;

    #[derive(Default)]
    struct MemoryStore(Mutex<BTreeMap<String, String>>);

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

    #[test]
    fn removes_config_and_its_stored_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config
            .add_profile(
                "garage",
                Profile {
                    driver: "bambu-lan".into(),
                    host: "printer.local".into(),
                    serial: String::new(),
                    timeout: "10s".into(),
                    insecure: false,
                    created: String::new(),
                    updated: String::new(),
                },
            )
            .unwrap();
        config.save(dir.path()).unwrap();
        let store = MemoryStore::default();
        store
            .set(SERVICE, "bambu-lan:garage:access-code", "secret")
            .unwrap();
        store
            .set(SERVICE, "bambu-lan:garage:tls-fingerprint", "fingerprint")
            .unwrap();

        let result = remove(dir.path(), &store, "GARAGE").unwrap();

        assert!(result.access_code_removed && result.tls_fingerprint_removed);
        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_profile("garage")
                .is_none()
        );
        assert_eq!(
            store.get(SERVICE, "bambu-lan:garage:access-code"),
            Err(SecretError::NotFound)
        );
    }
}
