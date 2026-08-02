use std::{net::IpAddr, path::Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    bambu::is_valid_tls_fingerprint,
    config::{Config, ConfigError},
    drivers::{self, DriverError},
    keychain::{SERVICE, SecretError, SecretStore, account},
};

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("profile name is required")]
    MissingName,
    #[error("invalid profile name")]
    InvalidName,
    #[error("invalid printer host")]
    InvalidHost,
    #[error("access code must not contain control characters")]
    InvalidAccessCode,
    #[error("an access code is required for this printer driver")]
    MissingAccessCode,
    #[error("invalid TLS fingerprint")]
    InvalidTlsFingerprint,
    #[error("printer profile {0:?} not found")]
    NotFound(String),
    #[error("keychain operation failed")]
    Secret(#[source] SecretError),
    #[error("cannot save config")]
    Config(#[source] ConfigError),
    #[error("cannot save config and could not restore stored secrets")]
    RollbackFailed,
    #[error("{0}")]
    Driver(#[from] DriverError),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TlsRefreshResult {
    pub name: String,
    pub fingerprint: Option<String>,
    pub insecure: bool,
    pub warnings: Vec<RemoveWarning>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveResult {
    pub name: String,
    pub access_code_removed: bool,
    pub tls_fingerprint_removed: bool,
    pub warnings: Vec<RemoveWarning>,
}

pub fn refresh_tls(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
    insecure: bool,
    timeout: Option<String>,
) -> Result<TlsRefreshResult, ProfileError> {
    let name = normalize_name(name)?;
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(ProfileError::Config)?;
    let mut profile = config
        .get_profile(&name)
        .cloned()
        .ok_or_else(|| ProfileError::NotFound(name.clone()))?;
    if let Some(timeout) = timeout {
        profile.timeout = timeout;
    }
    let driver = drivers::profile(&profile)?;
    if !driver.driver().supports(drivers::Operation::TlsRefresh) {
        return Err(ProfileError::Driver(DriverError::UnsupportedOperation(
            driver.driver(),
            drivers::Operation::TlsRefresh,
        )));
    }
    let fingerprint_account = account(&profile.driver, &name, "tls-fingerprint");
    if insecure {
        profile.insecure = true;
        profile.updated = now();
        config
            .set_profile(&name, profile)
            .map_err(ProfileError::Config)?;
        config.save(dir).map_err(ProfileError::Config)?;
        let mut warnings = Vec::new();
        match store.delete(SERVICE, &fingerprint_account) {
            Ok(()) | Err(SecretError::NotFound) => {}
            Err(_) => warnings.push(RemoveWarning {
                code: "tls_fingerprint_delete_failed",
                message: "profile was switched to insecure, but the stored TLS fingerprint could not be deleted from keychain",
            }),
        }
        return Ok(TlsRefreshResult {
            name,
            fingerprint: None,
            insecure: true,
            warnings,
        });
    }

    let fingerprint = preview_tls(dir, &name, Some(profile.timeout.clone()))?;
    store_tls_fingerprint(dir, store, &name, &fingerprint)
}

pub fn preview_tls(
    dir: impl AsRef<Path>,
    name: &str,
    timeout: Option<String>,
) -> Result<String, ProfileError> {
    let name = normalize_name(name)?;
    let config = Config::open(dir).map_err(ProfileError::Config)?;
    let mut profile = config
        .get_profile(&name)
        .cloned()
        .ok_or_else(|| ProfileError::NotFound(name.clone()))?;
    if let Some(timeout) = timeout {
        profile.timeout = timeout;
    }
    let driver = drivers::profile(&profile)?;
    if !driver.driver().supports(drivers::Operation::TlsRefresh) {
        return Err(ProfileError::Driver(DriverError::UnsupportedOperation(
            driver.driver(),
            drivers::Operation::TlsRefresh,
        )));
    }
    drivers::capture_tls_fingerprint(&driver).map_err(ProfileError::Driver)
}

pub fn store_tls_fingerprint(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
    fingerprint: &str,
) -> Result<TlsRefreshResult, ProfileError> {
    if !is_valid_tls_fingerprint(fingerprint) {
        return Err(ProfileError::InvalidTlsFingerprint);
    }
    let name = normalize_name(name)?;
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(ProfileError::Config)?;
    let mut profile = config
        .get_profile(&name)
        .cloned()
        .ok_or_else(|| ProfileError::NotFound(name.clone()))?;
    let driver = drivers::profile(&profile)?;
    if !driver.driver().supports(drivers::Operation::TlsRefresh) {
        return Err(ProfileError::Driver(DriverError::UnsupportedOperation(
            driver.driver(),
            drivers::Operation::TlsRefresh,
        )));
    }
    let fingerprint_account = account(&profile.driver, &name, "tls-fingerprint");
    let mut stored = StoredSecret::replace(store, fingerprint_account, fingerprint)?;
    profile.insecure = false;
    profile.updated = now();
    if let Err(error) = config
        .set_profile(&name, profile)
        .and_then(|_| config.save(dir))
    {
        if stored.restore(store).is_err() {
            return Err(ProfileError::RollbackFailed);
        }
        return Err(ProfileError::Config(error));
    }
    Ok(TlsRefreshResult {
        name,
        fingerprint: Some(fingerprint.into()),
        insecure: false,
        warnings: Vec::new(),
    })
}

#[derive(Debug, Serialize)]
pub struct RemoveWarning {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRequest {
    pub name: String,
    pub driver: String,
    pub host: String,
    #[serde(default)]
    pub serial: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    #[serde(default)]
    pub insecure: bool,
    #[serde(default)]
    pub access_code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateResult {
    pub name: String,
    #[serde(flatten)]
    pub profile: crate::config::Profile,
}

pub fn create(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    request: CreateRequest,
) -> Result<CreateResult, ProfileError> {
    let name = normalize_name(&request.name)?;
    validate_host(&request.host)?;
    validate_access_code(&request.access_code)?;

    let now = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("formatting an RFC 3339 timestamp cannot fail");
    let profile = crate::config::Profile {
        driver: request.driver,
        host: request.host,
        serial: request.serial,
        model: request.model,
        timeout: request.timeout,
        insecure: request.insecure,
        created: now.clone(),
        updated: now,
    };
    let driver_profile = drivers::profile(&profile)?;
    if driver_profile.driver().requires_access_code() && request.access_code.is_empty() {
        return Err(ProfileError::MissingAccessCode);
    }
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(ProfileError::Config)?;
    if config.get_profile(&name).is_some() {
        return Err(ProfileError::Config(ConfigError::ProfileAlreadyExists));
    }

    let tls_fingerprint = drivers::verify(
        &driver_profile,
        (!request.access_code.is_empty()).then_some(&request.access_code),
    )?;

    let mut secrets = Vec::new();
    if !request.access_code.is_empty() {
        secrets.push(StoredSecret::replace(
            store,
            account(&profile.driver, &name, "access-code"),
            &request.access_code,
        )?);
    }
    if let Some(fingerprint) = tls_fingerprint {
        match StoredSecret::replace(
            store,
            account(&profile.driver, &name, "tls-fingerprint"),
            &fingerprint,
        ) {
            Ok(secret) => secrets.push(secret),
            Err(error) => {
                if restore(store, &mut secrets).is_err() {
                    return Err(ProfileError::RollbackFailed);
                }
                return Err(error);
            }
        }
    }
    if let Err(error) = config.add_profile(&name, profile.clone()) {
        if restore(store, &mut secrets).is_err() {
            return Err(ProfileError::RollbackFailed);
        }
        return Err(ProfileError::Config(error));
    }
    if let Err(error) = config.save(dir) {
        if restore(store, &mut secrets).is_err() {
            return Err(ProfileError::RollbackFailed);
        }
        return Err(ProfileError::Config(error));
    }

    Ok(CreateResult { name, profile })
}

fn default_timeout() -> String {
    "10s".into()
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("formatting an RFC 3339 timestamp cannot fail")
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
            code: "access_code_not_found",
            message: "profile was removed, but no stored access code was found",
        });
    }
    if !profile.insecure && profile.driver == "bambu-lan" && !secrets[1].present() {
        warnings.push(RemoveWarning {
            code: "tls_fingerprint_not_found",
            message: "profile was removed, but no stored TLS fingerprint was found",
        });
    }

    for index in 0..secrets.len() {
        if let Err(error) = secrets[index].delete(store) {
            restore(store, &mut secrets)?;
            return Err(ProfileError::Secret(error));
        }
    }
    config.remove_profile(&name).map_err(ProfileError::Config)?;
    if let Err(error) = config.save(dir) {
        restore(store, &mut secrets)?;
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

fn validate_access_code(access_code: &str) -> Result<(), ProfileError> {
    if access_code.chars().any(|character| character.is_control()) {
        return Err(ProfileError::InvalidAccessCode);
    }
    Ok(())
}

pub fn validate_host(host: &str) -> Result<(), ProfileError> {
    if host.is_empty() || host.trim() != host || host.chars().any(char::is_whitespace) {
        return Err(ProfileError::InvalidHost);
    }
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let authority = match host {
        host if host.starts_with("http://") => &host["http://".len()..],
        host if host.starts_with("https://") => &host["https://".len()..],
        host if host.contains("://") => return Err(ProfileError::InvalidHost),
        host => host,
    };
    if authority.contains('@') {
        return Err(ProfileError::InvalidHost);
    }
    let authority = authority.split('/').next().unwrap_or_default();
    let hostname = authority
        .strip_prefix('[')
        .and_then(|value| value.split(']').next())
        .unwrap_or_else(|| authority.split(':').next().unwrap_or_default());
    if hostname.is_empty()
        || hostname.len() > 253
        || hostname.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(ProfileError::InvalidHost);
    }
    Ok(())
}

struct StoredSecret {
    account: String,
    value: Option<String>,
    deleted: bool,
}

impl StoredSecret {
    fn replace(
        store: &dyn SecretStore,
        account: String,
        value: &str,
    ) -> Result<Self, ProfileError> {
        let mut secret = Self::load(store, account)?;
        store
            .set(SERVICE, &secret.account, value)
            .map_err(ProfileError::Secret)?;
        secret.deleted = true;
        Ok(secret)
    }

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

    fn restore(&mut self, store: &dyn SecretStore) -> Result<(), SecretError> {
        if !self.deleted {
            return Ok(());
        }
        match self.value.as_deref() {
            Some(value) => store.set(SERVICE, &self.account, value)?,
            None => match store.delete(SERVICE, &self.account) {
                Ok(()) | Err(SecretError::NotFound) => {}
                Err(error) => return Err(error),
            },
        }
        self.deleted = false;
        Ok(())
    }
}

fn restore(store: &dyn SecretStore, secrets: &mut [StoredSecret]) -> Result<(), ProfileError> {
    for secret in secrets {
        secret.restore(store).map_err(ProfileError::Secret)?;
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

    struct UnavailableStore;

    impl SecretStore for UnavailableStore {
        fn get(&self, _: &str, _: &str) -> Result<String, SecretError> {
            Err(SecretError::Unavailable("unavailable".into()))
        }
        fn set(&self, _: &str, _: &str, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Unavailable("unavailable".into()))
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Unavailable("unavailable".into()))
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
                    model: String::new(),
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

    #[test]
    fn switching_to_insecure_removes_only_the_tls_pin_after_saving_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config
            .add_profile(
                "garage",
                Profile {
                    driver: "bambu-lan".into(),
                    host: "printer.local".into(),
                    serial: "SN001".into(),
                    model: String::new(),
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
            .set(SERVICE, "bambu-lan:garage:access-code", "access-code")
            .unwrap();
        store
            .set(SERVICE, "bambu-lan:garage:tls-fingerprint", "old-pin")
            .unwrap();

        let result = refresh_tls(dir.path(), &store, "garage", true, None).unwrap();

        assert!(result.insecure);
        assert_eq!(result.fingerprint, None);
        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_profile("garage")
                .unwrap()
                .insecure
        );
        assert_eq!(
            store.get(SERVICE, "bambu-lan:garage:access-code").unwrap(),
            "access-code"
        );
        assert_eq!(
            store.get(SERVICE, "bambu-lan:garage:tls-fingerprint"),
            Err(SecretError::NotFound)
        );
    }

    #[test]
    fn accepts_supported_host_shapes_and_rejects_credentials() {
        for host in [
            "192.0.2.10",
            "printer.local",
            "https://printer.local:7125/api",
        ] {
            assert!(validate_host(host).is_ok(), "{host}");
        }
        for host in [
            "",
            " printer.local",
            "http://user:secret@printer.local",
            "bad_host",
        ] {
            assert!(validate_host(host).is_err(), "{host}");
        }
    }

    #[test]
    fn rejects_access_codes_with_control_characters_before_connecting() {
        assert!(matches!(
            validate_access_code("bad\ncode"),
            Err(ProfileError::InvalidAccessCode)
        ));
    }

    #[test]
    fn requires_a_bambu_access_code_before_attempting_profile_verification() {
        let dir = tempfile::tempdir().unwrap();

        let error = create(
            dir.path(),
            &MemoryStore::default(),
            CreateRequest {
                name: "garage".into(),
                driver: "bambu-lan".into(),
                host: "192.0.2.1".into(),
                serial: "SN001".into(),
                model: "P1S".into(),
                timeout: "10s".into(),
                insecure: false,
                access_code: String::new(),
            },
        )
        .unwrap_err();

        assert!(matches!(error, ProfileError::MissingAccessCode));
        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_profile("garage")
                .is_none()
        );
    }

    #[test]
    fn creates_a_verified_moonraker_profile_without_serializing_its_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let (host, server) = moonraker_server(None);

        let result = create(
            dir.path(),
            &store,
            CreateRequest {
                name: "Garage".into(),
                driver: "moonraker".into(),
                host,
                serial: String::new(),
                model: String::new(),
                timeout: "10s".into(),
                insecure: false,
                access_code: "key".into(),
            },
        )
        .unwrap();
        server.join().unwrap();

        assert_eq!(result.name, "garage");
        assert!(!result.profile.created.is_empty());
        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_profile("garage")
                .is_some()
        );
        assert_eq!(
            store.get(SERVICE, "moonraker:garage:access-code").unwrap(),
            "key"
        );
        assert!(
            !std::fs::read_to_string(dir.path().join("polimero.yaml"))
                .unwrap()
                .contains("key")
        );
    }

    #[test]
    fn permits_a_verified_keyless_profile_when_the_keychain_is_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let (host, server) = moonraker_server(None);

        create(
            dir.path(),
            &UnavailableStore,
            CreateRequest {
                name: "garage".into(),
                driver: "moonraker".into(),
                host,
                serial: String::new(),
                model: String::new(),
                timeout: "10s".into(),
                insecure: false,
                access_code: String::new(),
            },
        )
        .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn restores_the_previous_secret_when_config_changes_after_verification() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.save(dir.path()).unwrap();
        let store = MemoryStore::default();
        store
            .set(SERVICE, "moonraker:garage:access-code", "old-key")
            .unwrap();
        let (host, server) = moonraker_server(Some(dir.path().join("polimero.yaml")));

        let error = create(
            dir.path(),
            &store,
            CreateRequest {
                name: "garage".into(),
                driver: "moonraker".into(),
                host,
                serial: String::new(),
                model: String::new(),
                timeout: "10s".into(),
                insecure: false,
                access_code: "new-key".into(),
            },
        )
        .unwrap_err();
        server.join().unwrap();

        assert!(matches!(
            error,
            ProfileError::Config(ConfigError::ConcurrentUpdate)
        ));
        assert_eq!(
            store.get(SERVICE, "moonraker:garage:access-code").unwrap(),
            "old-key"
        );
        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_profile("garage")
                .is_none()
        );
    }

    fn moonraker_server(
        mutate_config: Option<std::path::PathBuf>,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..read]);
            }
            if let Some(path) = mutate_config {
                std::fs::write(path, "version: 1\nprofiles: {}\n# changed\n").unwrap();
            }
            let body = r#"{"result":{"status":{"print_stats":{"state":"ready"}}}}"#;
            write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
        });
        (host, server)
    }
}
