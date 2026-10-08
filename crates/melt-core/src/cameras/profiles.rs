//! Camera profile workflows: validated, verified, keychain-backed CRUD and
//! printer links. Mirrors the printer workflow's secret rollback.

use std::{net::IpAddr, path::Path};

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{CameraError, DRIVERS, ResolvedCamera, TAPO_RTSP_PORT};
use crate::{
    config::{CameraProfile, Config, ConfigError},
    drivers,
    keychain::{SERVICE, SecretError, SecretStore, account},
    profiles::{self, ProfileError, RemoveWarning, StoredSecret},
};

/// Add or edit input. Deliberately not `Debug`: it holds a password. An
/// empty `password` on update keeps the stored one.
pub struct CameraRequest {
    pub name: String,
    pub driver: String,
    pub host: String,
    pub username: String,
    pub password: String,
    pub rtsp_port: Option<u16>,
    pub timeout: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSummary {
    pub name: String,
    pub id: String,
    pub driver: String,
    pub host: String,
    pub username: String,
    pub rtsp_port: u16,
    pub timeout: String,
    pub printers: Vec<String>,
    pub selected_by: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraRemoveResult {
    pub name: String,
    pub id: String,
    pub password_removed: bool,
    pub unlinked_printers: Vec<String>,
    pub warnings: Vec<RemoveWarning>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedFeed {
    pub printer: String,
    /// `None` is the printer's built-in feed.
    pub camera: Option<String>,
}

pub fn create(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    request: CameraRequest,
    verify: impl FnOnce(&ResolvedCamera) -> Result<(), CameraError>,
) -> Result<CameraSummary, CameraError> {
    let name = camera_name(&request.name)?;
    let timeout = validate(&request)?;
    if request.password.is_empty() {
        return Err(CameraError::InvalidPassword);
    }
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    if config.get_camera(&name).is_some() {
        return Err(CameraError::AlreadyExists(name));
    }
    let now = now();
    let camera = ResolvedCamera {
        name: name.clone(),
        profile: CameraProfile {
            id: new_id(),
            driver: request.driver,
            host: request.host,
            username: request.username,
            rtsp_port: request.rtsp_port,
            timeout: request.timeout,
            created: now.clone(),
            updated: now,
        },
        password: request.password,
        timeout,
    };
    verify(&camera)?;

    let mut secrets = vec![
        StoredSecret::replace(
            store,
            password_account(&camera.profile.id),
            &camera.password,
        )
        .map_err(secret_error)?,
    ];
    if let Err(error) = config
        .add_camera(&name, camera.profile.clone())
        .and_then(|()| config.save(dir))
    {
        profiles::restore(store, &mut secrets).map_err(|_| CameraError::RollbackFailed)?;
        return Err(CameraError::Config(error));
    }
    Ok(summary(&config, &name, &camera.profile))
}

pub fn update(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    current: &str,
    request: CameraRequest,
    verify: impl FnOnce(&ResolvedCamera) -> Result<(), CameraError>,
) -> Result<CameraSummary, CameraError> {
    let current = camera_name(current)?;
    let name = camera_name(&request.name)?;
    let timeout = validate(&request)?;
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    let previous = config
        .get_camera(&current)
        .cloned()
        .ok_or_else(|| CameraError::NotFound(current.clone()))?;
    if name != current && config.get_camera(&name).is_some() {
        return Err(CameraError::AlreadyExists(name));
    }

    let password_changed = !request.password.is_empty();
    let account = password_account(&previous.id);
    let reaches_camera_differently = password_changed
        || request.driver != previous.driver
        || request.host != previous.host
        || request.username != previous.username
        || request.rtsp_port != previous.rtsp_port;
    // Only a verifying edit needs the stored password; a rename must work
    // even when the keychain entry is gone.
    let password = if password_changed {
        request.password
    } else if reaches_camera_differently {
        StoredSecret::load(store, account.clone())
            .map_err(secret_error)?
            .value
            .ok_or(CameraError::MissingSecret)?
    } else {
        String::new()
    };
    let camera = ResolvedCamera {
        name: name.clone(),
        profile: CameraProfile {
            id: previous.id.clone(),
            driver: request.driver,
            host: request.host,
            username: request.username,
            rtsp_port: request.rtsp_port,
            timeout: request.timeout,
            created: previous.created.clone(),
            updated: now(),
        },
        password,
        timeout,
    };
    if reaches_camera_differently {
        verify(&camera)?;
    }

    let mut secrets = Vec::new();
    if password_changed {
        secrets
            .push(StoredSecret::replace(store, account, &camera.password).map_err(secret_error)?);
    }
    if let Err(error) = config
        .replace_camera(&current, &name, camera.profile.clone())
        .and_then(|()| config.save(dir))
    {
        profiles::restore(store, &mut secrets).map_err(|_| CameraError::RollbackFailed)?;
        return Err(CameraError::Config(error));
    }
    Ok(summary(&config, &name, &camera.profile))
}

/// Removes the profile and its links first, then the keychain password; a
/// failed password delete is reported as a warning, not a failure.
pub fn remove(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
) -> Result<CameraRemoveResult, CameraError> {
    let name = camera_name(name)?;
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    let camera = config
        .get_camera(&name)
        .cloned()
        .ok_or_else(|| CameraError::NotFound(name.clone()))?;
    let (unlinked_printers, _) = config.camera_printers(&camera.id);
    config.remove_camera(&name).map_err(CameraError::Config)?;
    config.save(dir).map_err(CameraError::Config)?;

    let mut warnings = Vec::new();
    let password_removed = match store.delete(SERVICE, &password_account(&camera.id)) {
        Ok(()) => true,
        Err(SecretError::NotFound) => false,
        Err(_) => {
            warnings.push(RemoveWarning {
                code: "password_delete_failed",
                message: "camera was removed, but its stored password could not be deleted from keychain",
            });
            false
        }
    };
    Ok(CameraRemoveResult {
        name,
        id: camera.id,
        password_removed,
        unlinked_printers,
        warnings,
    })
}

pub fn list(dir: impl AsRef<Path>) -> Result<Vec<CameraSummary>, CameraError> {
    let config = Config::open(dir).map_err(CameraError::Config)?;
    Ok(config
        .cameras()
        .iter()
        .map(|(name, camera)| summary(&config, name, camera))
        .collect())
}

pub fn profile(dir: impl AsRef<Path>, name: &str) -> Result<CameraProfile, CameraError> {
    let name = camera_name(name)?;
    Config::open(dir)
        .map_err(CameraError::Config)?
        .get_camera(&name)
        .cloned()
        .ok_or(CameraError::NotFound(name))
}

pub fn resolve(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
) -> Result<ResolvedCamera, CameraError> {
    let profile = profile(dir, name)?;
    let password = match store.get(SERVICE, &password_account(&profile.id)) {
        Ok(password) => password,
        Err(SecretError::NotFound) => return Err(CameraError::MissingSecret),
        Err(error) => return Err(CameraError::Secret(error)),
    };
    Ok(ResolvedCamera {
        name: camera_name(name)?,
        timeout: parse_timeout(&profile.timeout)?,
        profile,
        password,
    })
}

pub fn link(
    dir: impl AsRef<Path>,
    printer: &str,
    camera: &str,
    select: bool,
) -> Result<CameraSummary, CameraError> {
    change_links(dir.as_ref(), printer, camera, |config, printer, camera| {
        config.link_camera(printer, camera, select)
    })
}

pub fn unlink(
    dir: impl AsRef<Path>,
    printer: &str,
    camera: &str,
) -> Result<CameraSummary, CameraError> {
    change_links(dir.as_ref(), printer, camera, Config::unlink_camera)
}

pub fn select(
    dir: impl AsRef<Path>,
    printer: &str,
    camera: Option<&str>,
) -> Result<SelectedFeed, CameraError> {
    let dir = dir.as_ref();
    let printer = camera_name(printer)?;
    let camera = camera.map(camera_name).transpose()?;
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    config
        .select_camera(&printer, camera.as_deref())
        .map_err(|error| link_error(error, &printer, camera.as_deref().unwrap_or_default()))?;
    config.save(dir).map_err(CameraError::Config)?;
    Ok(SelectedFeed { printer, camera })
}

fn change_links(
    dir: &Path,
    printer: &str,
    camera: &str,
    change: impl FnOnce(&mut Config, &str, &str) -> Result<(), ConfigError>,
) -> Result<CameraSummary, CameraError> {
    let printer = camera_name(printer)?;
    let camera = camera_name(camera)?;
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    change(&mut config, &printer, &camera).map_err(|error| link_error(error, &printer, &camera))?;
    config.save(dir).map_err(CameraError::Config)?;
    let profile = config
        .get_camera(&camera)
        .cloned()
        .ok_or_else(|| CameraError::NotFound(camera.clone()))?;
    Ok(summary(&config, &camera, &profile))
}

fn link_error(error: ConfigError, printer: &str, camera: &str) -> CameraError {
    match error {
        ConfigError::ProfileNotFound => CameraError::PrinterNotFound(printer.to_owned()),
        ConfigError::CameraNotFound => CameraError::NotFound(camera.to_owned()),
        ConfigError::CameraNotLinked => CameraError::NotLinked {
            printer: printer.to_owned(),
            camera: camera.to_owned(),
        },
        other => CameraError::Config(other),
    }
}

fn summary(config: &Config, name: &str, camera: &CameraProfile) -> CameraSummary {
    let (printers, selected_by) = config.camera_printers(&camera.id);
    CameraSummary {
        name: name.to_owned(),
        id: camera.id.clone(),
        driver: camera.driver.clone(),
        host: camera.host.clone(),
        username: camera.username.clone(),
        rtsp_port: camera.rtsp_port.unwrap_or(TAPO_RTSP_PORT),
        timeout: camera.timeout.clone(),
        printers,
        selected_by,
    }
}

/// Camera and printer names share the printer profile name rules.
fn camera_name(name: &str) -> Result<String, CameraError> {
    profiles::normalize_name(name).map_err(|error| match error {
        ProfileError::MissingName => CameraError::MissingName,
        _ => CameraError::InvalidName,
    })
}

/// Validates every field and returns the parsed timeout.
fn validate(request: &CameraRequest) -> Result<std::time::Duration, CameraError> {
    if !DRIVERS.contains(&request.driver.as_str()) {
        return Err(CameraError::UnknownDriver(request.driver.clone()));
    }
    validate_host(&request.host)?;
    let username = &request.username;
    if username.is_empty()
        || username.chars().count() > 64
        || username
            .chars()
            .any(|character| character.is_control() || matches!(character, '"' | '\\'))
    {
        return Err(CameraError::InvalidUsername);
    }
    if request.password.chars().any(char::is_control) {
        return Err(CameraError::InvalidPassword);
    }
    if request.rtsp_port == Some(0) {
        return Err(CameraError::InvalidPort);
    }
    parse_timeout(&request.timeout)
}

/// A bare IP address or hostname: no scheme, port, path, or user info.
fn validate_host(host: &str) -> Result<(), CameraError> {
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    if host.contains(['/', ':', '@']) {
        return Err(CameraError::InvalidHost);
    }
    profiles::validate_host(host).map_err(|_| CameraError::InvalidHost)
}

fn parse_timeout(value: &str) -> Result<std::time::Duration, CameraError> {
    drivers::parse_timeout(value).map_err(|_| CameraError::InvalidTimeout)
}

fn password_account(id: &str) -> String {
    account("camera", id, "password")
}

fn secret_error(error: ProfileError) -> CameraError {
    match error {
        ProfileError::Secret(error) => CameraError::Secret(error),
        _ => CameraError::RollbackFailed,
    }
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    openssl::rand::rand_bytes(&mut bytes).expect("OpenSSL random source is available");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("formatting an RFC 3339 timestamp cannot fail")
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs};

    use super::*;
    use crate::{config::Profile, keychain::test_support::MemoryStore};

    fn request(name: &str, password: &str) -> CameraRequest {
        CameraRequest {
            name: name.into(),
            driver: "tapo".into(),
            host: "192.0.2.20".into(),
            username: "melt".into(),
            password: password.into(),
            rtsp_port: None,
            timeout: "10s".into(),
        }
    }

    fn accept(_: &ResolvedCamera) -> Result<(), CameraError> {
        Ok(())
    }

    fn add_printer(dir: &Path, name: &str) {
        let mut config = Config::open(dir).unwrap();
        config
            .add_profile(
                name,
                Profile {
                    driver: "moonraker".into(),
                    host: "printer.local".into(),
                    serial: String::new(),
                    model: String::new(),
                    timeout: "10s".into(),
                    insecure: false,
                    created: "2026-10-08T12:00:00Z".into(),
                    updated: "2026-10-08T12:00:00Z".into(),
                },
            )
            .unwrap();
        config.save(dir).unwrap();
    }

    #[test]
    fn create_verifies_with_the_password_and_keeps_it_out_of_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let seen = Cell::new(false);

        let summary = create(dir.path(), &store, request("Porch", "s3cret"), |camera| {
            seen.set(camera.password == "s3cret" && camera.name == "porch");
            Ok(())
        })
        .unwrap();

        assert!(seen.get());
        assert_eq!(summary.name, "porch");
        assert_eq!(summary.rtsp_port, 554);
        assert!(crate::config::is_camera_id(&summary.id));
        let yaml = fs::read_to_string(dir.path().join("melt.yaml")).unwrap();
        assert!(!yaml.contains("s3cret"));
        assert_eq!(
            store
                .get(SERVICE, &format!("camera:{}:password", summary.id))
                .unwrap(),
            "s3cret"
        );
    }

    #[test]
    fn create_rejects_invalid_input_before_contacting_the_camera() {
        type Edit = fn(&mut CameraRequest);
        let cases: [(&str, Edit); 12] = [
            ("name", |request| request.name = "-porch".into()),
            ("driver", |request| request.driver = "onvif".into()),
            ("scheme host", |request| {
                request.host = "rtsp://192.0.2.20".into()
            }),
            ("host with port", |request| {
                request.host = "camera.local:554".into()
            }),
            ("quoted username", |request| {
                request.username = "me\"lt".into()
            }),
            ("backslash username", |request| {
                request.username = "me\\lt".into()
            }),
            ("control username", |request| {
                request.username = "me\u{7}lt".into()
            }),
            ("control password", |request| {
                request.password = "se\u{7}cret".into()
            }),
            ("long username", |request| request.username = "u".repeat(65)),
            ("empty password", |request| request.password = String::new()),
            ("port zero", |request| request.rtsp_port = Some(0)),
            ("timeout", |request| request.timeout = "soon".into()),
        ];
        for (name, edit) in cases {
            let dir = tempfile::tempdir().unwrap();
            let mut invalid = request("porch", "s3cret");
            edit(&mut invalid);
            let called = Cell::new(false);
            let result = create(dir.path(), &MemoryStore::default(), invalid, |_| {
                called.set(true);
                Ok(())
            });
            assert!(result.is_err(), "{name}");
            assert!(!called.get(), "{name}");
        }
    }

    #[test]
    fn failed_verification_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();

        let result = create(dir.path(), &store, request("porch", "wrong"), |_| {
            Err(CameraError::Authentication)
        });

        assert!(matches!(result, Err(CameraError::Authentication)));
        assert!(Config::open(dir.path()).unwrap().cameras().is_empty());
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[test]
    fn concurrent_config_change_rolls_back_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();

        let result = create(dir.path(), &store, request("porch", "s3cret"), |_| {
            fs::write(dir.path().join("melt.yaml"), "version: 1\nprofiles: {}\n").unwrap();
            Ok(())
        });

        assert!(matches!(
            result,
            Err(CameraError::Config(ConfigError::ConcurrentUpdate))
        ));
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[test]
    fn rename_keeps_password_and_skips_verification() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let created = create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();

        let renamed = update(dir.path(), &store, "porch", request("front", ""), |_| {
            panic!("an offline rename must not contact the camera")
        })
        .unwrap();

        assert_eq!(renamed.id, created.id);
        assert!(profile(dir.path(), "porch").is_err());
        assert_eq!(
            resolve(dir.path(), &store, "front").unwrap().password,
            "s3cret"
        );
    }

    #[test]
    fn rename_works_without_the_stored_password_but_reconnecting_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();
        store.0.lock().unwrap().clear();

        let mut moved = request("porch", "");
        moved.host = "192.0.2.99".into();
        let error = update(dir.path(), &store, "porch", moved, accept).unwrap_err();
        assert!(matches!(error, CameraError::MissingSecret), "{error:?}");

        update(dir.path(), &store, "porch", request("front", ""), |_| {
            panic!("an offline rename must not contact the camera")
        })
        .unwrap();
        assert!(profile(dir.path(), "front").is_ok());
    }

    #[test]
    fn new_password_is_verified_before_it_replaces_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        create(dir.path(), &store, request("porch", "old"), accept).unwrap();

        let result = update(
            dir.path(),
            &store,
            "porch",
            request("porch", "new"),
            |camera| {
                assert_eq!(camera.password, "new");
                Err(CameraError::Authentication)
            },
        );

        assert!(matches!(result, Err(CameraError::Authentication)));
        assert_eq!(
            resolve(dir.path(), &store, "porch").unwrap().password,
            "old"
        );
    }

    #[test]
    fn remove_unlinks_printers_and_deletes_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        add_printer(dir.path(), "garage");
        let created = create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();
        link(dir.path(), "garage", "porch", true).unwrap();

        let removed = remove(dir.path(), &store, "porch").unwrap();

        assert!(removed.password_removed);
        assert_eq!(removed.unlinked_printers, ["garage"]);
        assert!(store.0.lock().unwrap().is_empty());
        let config = Config::open(dir.path()).unwrap();
        assert!(config.printer_cameras("garage").is_none());
        assert!(config.camera_printers(&created.id).0.is_empty());
    }

    #[test]
    fn links_and_selection_report_names_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        add_printer(dir.path(), "garage");
        create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();

        let linked = link(dir.path(), "Garage", "porch", false).unwrap();
        assert_eq!(linked.printers, ["garage"]);
        assert!(linked.selected_by.is_empty());

        let selected = select(dir.path(), "garage", Some("porch")).unwrap();
        assert_eq!(selected.camera.as_deref(), Some("porch"));
        assert_eq!(list(dir.path()).unwrap()[0].selected_by, ["garage"]);

        assert!(select(dir.path(), "garage", None).unwrap().camera.is_none());
        assert!(matches!(
            link(dir.path(), "attic", "porch", false),
            Err(CameraError::PrinterNotFound(_))
        ));
        unlink(dir.path(), "garage", "porch").unwrap();
        assert!(matches!(
            unlink(dir.path(), "garage", "porch"),
            Err(CameraError::NotLinked { .. })
        ));
    }

    struct DeleteFails(MemoryStore);

    impl SecretStore for DeleteFails {
        fn get(&self, service: &str, account: &str) -> Result<String, SecretError> {
            self.0.get(service, account)
        }
        fn set(&self, service: &str, account: &str, secret: &str) -> Result<(), SecretError> {
            self.0.set(service, account, secret)
        }
        fn delete(&self, _: &str, _: &str) -> Result<(), SecretError> {
            Err(SecretError::Unavailable("locked".into()))
        }
    }

    #[test]
    fn remove_warns_when_the_password_cannot_be_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeleteFails(MemoryStore::default());
        create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();

        let removed = remove(dir.path(), &store, "porch").unwrap();

        assert!(!removed.password_removed);
        assert_eq!(removed.warnings.len(), 1);
        assert_eq!(removed.warnings[0].code, "password_delete_failed");
        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_camera("porch")
                .is_none()
        );
    }

    #[test]
    fn concurrent_change_during_update_restores_the_old_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let created = create(dir.path(), &store, request("porch", "old"), accept).unwrap();

        let result = update(dir.path(), &store, "porch", request("porch", "new"), |_| {
            fs::write(dir.path().join("melt.yaml"), "version: 1\nprofiles: {}\n").unwrap();
            Ok(())
        });

        assert!(matches!(
            result,
            Err(CameraError::Config(ConfigError::ConcurrentUpdate))
        ));
        assert_eq!(
            store
                .get(SERVICE, &format!("camera:{}:password", created.id))
                .unwrap(),
            "old"
        );
    }

    #[test]
    fn changing_the_connection_verifies_with_the_stored_password() {
        type Edit = fn(&mut CameraRequest);
        let cases: [(&str, Edit); 3] = [
            ("host", |request| request.host = "192.0.2.21".into()),
            ("username", |request| request.username = "other".into()),
            ("port", |request| request.rtsp_port = Some(8554)),
        ];
        for (name, edit) in cases {
            let dir = tempfile::tempdir().unwrap();
            let store = MemoryStore::default();
            create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();
            let mut changed = request("porch", "");
            edit(&mut changed);
            let (host, username, port) = (
                changed.host.clone(),
                changed.username.clone(),
                changed.rtsp_port,
            );
            let called = Cell::new(false);

            update(dir.path(), &store, "porch", changed, |camera| {
                called.set(
                    camera.password == "s3cret"
                        && camera.profile.host == host
                        && camera.profile.username == username
                        && camera.profile.rtsp_port == port,
                );
                Ok(())
            })
            .unwrap();

            assert!(called.get(), "{name}");
        }
    }
}
