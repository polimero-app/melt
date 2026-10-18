//! Plug profile workflows: validated, keychain-backed CRUD and links.
//! Verification is async, so create and update split into `prepare_*` and
//! `commit_*`; the caller verifies between them and commits only on success.

use std::{path::Path, time::Duration};

use serde::{Deserialize, Serialize};

use super::{DRIVERS, PlugError, ResolvedPlug};
use crate::{
    config::{Config, ConfigError, PlugProfile, PlugTarget},
    drivers,
    keychain::{SERVICE, SecretError, SecretStore, account},
    profiles::{self, ProfileError, RemoveWarning, StoredSecret, new_id, now},
};

/// Add or edit input. Deliberately not `Debug`: it holds a password. An
/// empty `password` on update keeps the stored one.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlugRequest {
    pub name: String,
    pub driver: String,
    pub host: String,
    pub username: String,
    pub password: String,
    pub timeout: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlugSummary {
    pub name: String,
    pub id: String,
    pub driver: String,
    pub host: String,
    pub username: String,
    pub timeout: String,
    pub printers: Vec<String>,
    /// Camera names.
    pub cameras: Vec<String>,
    /// The profile revision.
    pub updated: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlugRemoveResult {
    pub name: String,
    pub id: String,
    pub password_removed: bool,
    pub unlinked_printers: Vec<String>,
    pub unlinked_cameras: Vec<String>,
    pub warnings: Vec<RemoveWarning>,
}

/// A validated edit awaiting verification. Not `Debug`: it holds a secret.
pub struct PlugUpdate {
    /// Carries the existing links; empty password unless `needs_verify`.
    pub plug: ResolvedPlug,
    current: String,
    password_changed: bool,
    /// Driver, host, username or password changed: verify before committing.
    pub needs_verify: bool,
}

/// Validates, checks the name is free and mints the ID. Saves nothing.
pub fn prepare_create(
    dir: impl AsRef<Path>,
    request: PlugRequest,
) -> Result<ResolvedPlug, PlugError> {
    let name = plug_name(&request.name)?;
    let timeout = validate(&request)?;
    if request.password.is_empty() {
        return Err(PlugError::InvalidPassword);
    }
    let config = Config::open(dir).map_err(PlugError::Config)?;
    if config.get_plug(&name).is_some() {
        return Err(PlugError::AlreadyExists(name));
    }
    let now = now();
    Ok(ResolvedPlug {
        name,
        profile: PlugProfile {
            id: new_id(),
            driver: request.driver,
            host: request.host,
            username: request.username,
            timeout: request.timeout,
            printers: Vec::new(),
            cameras: Vec::new(),
            created: now.clone(),
            updated: now,
        },
        password: request.password,
        timeout,
    })
}

/// Re-checks the name, stores the secret, saves, and rolls the secret back
/// when the config write fails.
pub fn commit_create(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    plug: ResolvedPlug,
) -> Result<PlugSummary, PlugError> {
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(PlugError::Config)?;
    if config.get_plug(&plug.name).is_some() {
        return Err(PlugError::AlreadyExists(plug.name));
    }
    let mut secrets = vec![
        StoredSecret::replace(store, password_account(&plug.profile.id), &plug.password)
            .map_err(secret_error)?,
    ];
    if let Err(error) = config
        .add_plug(&plug.name, plug.profile.clone())
        .and_then(|()| config.save(dir))
    {
        profiles::restore(store, &mut secrets).map_err(|_| PlugError::RollbackFailed)?;
        return Err(config_error(error, &plug.name));
    }
    Ok(summary(&config, &plug.name, &plug.profile))
}

/// Loads the stored password only when the edit needs verification, so a
/// rename or timeout change works even if the keychain entry is gone.
pub fn prepare_update(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    current: &str,
    request: PlugRequest,
) -> Result<PlugUpdate, PlugError> {
    let current = plug_name(current)?;
    let name = plug_name(&request.name)?;
    let timeout = validate(&request)?;
    let config = Config::open(dir).map_err(PlugError::Config)?;
    let previous = config
        .get_plug(&current)
        .cloned()
        .ok_or_else(|| PlugError::NotFound(current.clone()))?;
    if name != current && config.get_plug(&name).is_some() {
        return Err(PlugError::AlreadyExists(name));
    }

    let password_changed = !request.password.is_empty();
    let needs_verify = password_changed
        || request.driver != previous.driver
        || request.host != previous.host
        || request.username != previous.username;
    let password = if password_changed {
        request.password
    } else if needs_verify {
        StoredSecret::load(store, password_account(&previous.id))
            .map_err(secret_error)?
            .value
            .ok_or(PlugError::MissingSecret)?
    } else {
        String::new()
    };
    Ok(PlugUpdate {
        plug: ResolvedPlug {
            name,
            profile: PlugProfile {
                id: previous.id,
                driver: request.driver,
                host: request.host,
                username: request.username,
                timeout: request.timeout,
                printers: previous.printers,
                cameras: previous.cameras,
                created: previous.created,
                updated: now(),
            },
            password,
            timeout,
        },
        current,
        password_changed,
        needs_verify,
    })
}

pub fn commit_update(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    update: PlugUpdate,
) -> Result<PlugSummary, PlugError> {
    let dir = dir.as_ref();
    let PlugUpdate {
        mut plug,
        current,
        password_changed,
        ..
    } = update;
    let mut config = Config::open(dir).map_err(PlugError::Config)?;
    // Links may have changed while the caller verified; keep the latest.
    let latest = config
        .get_plug(&current)
        .ok_or_else(|| PlugError::NotFound(current.clone()))?;
    plug.profile.printers = latest.printers.clone();
    plug.profile.cameras = latest.cameras.clone();

    let mut secrets = Vec::new();
    if password_changed {
        secrets.push(
            StoredSecret::replace(store, password_account(&plug.profile.id), &plug.password)
                .map_err(secret_error)?,
        );
    }
    if let Err(error) = config
        .replace_plug(&current, &plug.name, plug.profile.clone())
        .and_then(|()| config.save(dir))
    {
        profiles::restore(store, &mut secrets).map_err(|_| PlugError::RollbackFailed)?;
        return Err(config_error(error, &plug.name));
    }
    Ok(summary(&config, &plug.name, &plug.profile))
}

/// Removes the profile and its links first, then the keychain password; a
/// failed password delete is reported as a warning, not a failure.
pub fn remove(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
) -> Result<PlugRemoveResult, PlugError> {
    let name = plug_name(name)?;
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(PlugError::Config)?;
    let plug = config
        .remove_plug(&name)
        .map_err(|error| config_error(error, &name))?;
    config.save(dir).map_err(PlugError::Config)?;

    let mut warnings = Vec::new();
    let password_removed = match store.delete(SERVICE, &password_account(&plug.id)) {
        Ok(()) => true,
        Err(SecretError::NotFound) => false,
        Err(_) => {
            warnings.push(RemoveWarning {
                code: "password_delete_failed",
                message: "plug was removed, but its stored password could not be deleted from keychain",
            });
            false
        }
    };
    Ok(PlugRemoveResult {
        unlinked_cameras: camera_names(&config, &plug.cameras),
        unlinked_printers: plug.printers,
        name,
        id: plug.id,
        password_removed,
        warnings,
    })
}

pub fn list(dir: impl AsRef<Path>) -> Result<Vec<PlugSummary>, PlugError> {
    let config = Config::open(dir).map_err(PlugError::Config)?;
    Ok(config
        .plugs()
        .iter()
        .map(|(name, plug)| summary(&config, name, plug))
        .collect())
}

/// Plugs attached to one printer or camera (by name).
pub fn list_for(dir: impl AsRef<Path>, target: &PlugTarget) -> Result<Vec<PlugSummary>, PlugError> {
    let target = normalize_target(target)?;
    let config = Config::open(dir).map_err(PlugError::Config)?;
    let attached: Box<dyn Fn(&PlugProfile) -> bool> = match &target {
        PlugTarget::Printer(name) => {
            if config.get_profile(name).is_none() {
                return Err(PlugError::PrinterNotFound(name.clone()));
            }
            Box::new(|plug| plug.printers.contains(name))
        }
        PlugTarget::Camera(name) => {
            let id = config
                .get_camera(name)
                .map(|camera| camera.id.clone())
                .ok_or_else(|| PlugError::CameraNotFound(name.clone()))?;
            Box::new(move |plug| plug.cameras.contains(&id))
        }
    };
    Ok(config
        .plugs()
        .iter()
        .filter(|(_, plug)| attached(plug))
        .map(|(name, plug)| summary(&config, name, plug))
        .collect())
}

pub fn resolve(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
) -> Result<ResolvedPlug, PlugError> {
    let name = plug_name(name)?;
    let profile = Config::open(dir)
        .map_err(PlugError::Config)?
        .get_plug(&name)
        .cloned()
        .ok_or_else(|| PlugError::NotFound(name.clone()))?;
    resolved(store, name, profile)
}

pub fn resolve_by_id(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    id: &str,
) -> Result<ResolvedPlug, PlugError> {
    let config = Config::open(dir).map_err(PlugError::Config)?;
    let (name, profile) = config
        .plug_by_id(id)
        .ok_or_else(|| PlugError::NotFound(id.to_owned()))?;
    resolved(store, name.to_owned(), profile.clone())
}

/// The profile revision, read from config only; `None` once removed.
pub fn revision(dir: impl AsRef<Path>, id: &str) -> Result<Option<String>, PlugError> {
    let config = Config::open(dir).map_err(PlugError::Config)?;
    Ok(config.plug_by_id(id).map(|(_, plug)| plug.updated.clone()))
}

pub fn link(
    dir: impl AsRef<Path>,
    plug: &str,
    target: &PlugTarget,
) -> Result<PlugSummary, PlugError> {
    change_links(dir.as_ref(), plug, target, Config::link_plug)
}

pub fn unlink(
    dir: impl AsRef<Path>,
    plug: &str,
    target: &PlugTarget,
) -> Result<PlugSummary, PlugError> {
    change_links(dir.as_ref(), plug, target, Config::unlink_plug)
}

fn change_links(
    dir: &Path,
    plug: &str,
    target: &PlugTarget,
    change: impl FnOnce(&mut Config, &str, &PlugTarget) -> Result<(), ConfigError>,
) -> Result<PlugSummary, PlugError> {
    let plug = plug_name(plug)?;
    let target = normalize_target(target)?;
    let mut config = Config::open(dir).map_err(PlugError::Config)?;
    change(&mut config, &plug, &target).map_err(|error| link_error(error, &plug, &target))?;
    config.save(dir).map_err(PlugError::Config)?;
    let profile = config
        .get_plug(&plug)
        .cloned()
        .ok_or_else(|| PlugError::NotFound(plug.clone()))?;
    Ok(summary(&config, &plug, &profile))
}

fn link_error(error: ConfigError, plug: &str, target: &PlugTarget) -> PlugError {
    let name = match target {
        PlugTarget::Printer(name) | PlugTarget::Camera(name) => name.clone(),
    };
    match error {
        ConfigError::PlugNotFound => PlugError::NotFound(plug.to_owned()),
        ConfigError::ProfileNotFound => PlugError::PrinterNotFound(name),
        ConfigError::CameraNotFound => PlugError::CameraNotFound(name),
        ConfigError::PlugNotLinked => PlugError::NotLinked {
            plug: plug.to_owned(),
            target: name,
        },
        other => PlugError::Config(other),
    }
}

fn config_error(error: ConfigError, name: &str) -> PlugError {
    match error {
        ConfigError::PlugAlreadyExists => PlugError::AlreadyExists(name.to_owned()),
        ConfigError::PlugNotFound => PlugError::NotFound(name.to_owned()),
        other => PlugError::Config(other),
    }
}

fn resolved(
    store: &dyn SecretStore,
    name: String,
    profile: PlugProfile,
) -> Result<ResolvedPlug, PlugError> {
    let password = match store.get(SERVICE, &password_account(&profile.id)) {
        Ok(password) => password,
        Err(SecretError::NotFound) => return Err(PlugError::MissingSecret),
        Err(error) => return Err(PlugError::Secret(error)),
    };
    Ok(ResolvedPlug {
        name,
        timeout: parse_timeout(&profile.timeout)?,
        profile,
        password,
    })
}

fn camera_names(config: &Config, ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter_map(|id| config.camera_name(id))
        .map(str::to_owned)
        .collect()
}

fn summary(config: &Config, name: &str, plug: &PlugProfile) -> PlugSummary {
    PlugSummary {
        name: name.to_owned(),
        id: plug.id.clone(),
        driver: plug.driver.clone(),
        host: plug.host.clone(),
        username: plug.username.clone(),
        timeout: plug.timeout.clone(),
        printers: plug.printers.clone(),
        cameras: camera_names(config, &plug.cameras),
        updated: plug.updated.clone(),
    }
}

/// Plug, printer and camera names share the printer profile name rules.
fn plug_name(name: &str) -> Result<String, PlugError> {
    profiles::normalize_name(name).map_err(|error| match error {
        ProfileError::MissingName => PlugError::MissingName,
        _ => PlugError::InvalidName,
    })
}

fn normalize_target(target: &PlugTarget) -> Result<PlugTarget, PlugError> {
    Ok(match target {
        PlugTarget::Printer(name) => PlugTarget::Printer(plug_name(name)?),
        PlugTarget::Camera(name) => PlugTarget::Camera(plug_name(name)?),
    })
}

/// Validates every field and returns the parsed timeout.
fn validate(request: &PlugRequest) -> Result<Duration, PlugError> {
    if !DRIVERS.contains(&request.driver.as_str()) {
        return Err(PlugError::UnknownDriver(request.driver.clone()));
    }
    profiles::validate_endpoint_host(&request.host).map_err(|_| PlugError::InvalidHost)?;
    let username = &request.username;
    if username.is_empty()
        || username.chars().count() > 254
        || username.chars().any(char::is_control)
    {
        return Err(PlugError::InvalidUsername);
    }
    if request.password.chars().any(char::is_control) {
        return Err(PlugError::InvalidPassword);
    }
    parse_timeout(&request.timeout)
}

fn parse_timeout(value: &str) -> Result<Duration, PlugError> {
    drivers::parse_timeout(value).map_err(|_| PlugError::InvalidTimeout)
}

fn password_account(id: &str) -> String {
    account("plug", id, "password")
}

fn secret_error(error: ProfileError) -> PlugError {
    match error {
        ProfileError::Secret(error) => PlugError::Secret(error),
        _ => PlugError::RollbackFailed,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        config::{CameraProfile, Profile},
        keychain::test_support::MemoryStore,
    };

    fn request(name: &str, password: &str) -> PlugRequest {
        PlugRequest {
            name: name.into(),
            driver: "tapo".into(),
            host: "192.0.2.30".into(),
            username: "operator@example.invalid".into(),
            password: password.into(),
            timeout: "10s".into(),
        }
    }

    fn create(dir: &Path, store: &dyn SecretStore, name: &str) -> PlugSummary {
        let plug = prepare_create(dir, request(name, "s3cret")).unwrap();
        commit_create(dir, store, plug).unwrap()
    }

    fn add_devices(dir: &Path) -> String {
        let mut config = Config::open(dir).unwrap();
        config
            .add_profile(
                "workshop",
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
        let id = "0123456789abcdef0123456789abcdef".to_owned();
        config
            .add_camera(
                "porch",
                CameraProfile {
                    id: id.clone(),
                    driver: "tapo".into(),
                    host: "192.0.2.20".into(),
                    username: "melt".into(),
                    rtsp_port: None,
                    timeout: "10s".into(),
                    created: "2026-10-08T12:00:00Z".into(),
                    updated: "2026-10-08T12:00:00Z".into(),
                },
            )
            .unwrap();
        config.save(dir).unwrap();
        id
    }

    fn printer() -> PlugTarget {
        PlugTarget::Printer("workshop".into())
    }

    fn camera() -> PlugTarget {
        PlugTarget::Camera("porch".into())
    }

    #[test]
    fn create_saves_profile_and_secret_after_commit() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();

        let plug = prepare_create(dir.path(), request("Bench", "s3cret")).unwrap();
        assert!(list(dir.path()).unwrap().is_empty());
        assert!(store.0.lock().unwrap().is_empty());
        let summary = commit_create(dir.path(), &store, plug).unwrap();

        assert_eq!(summary.name, "bench");
        assert!(crate::config::is_profile_id(&summary.id));
        assert!(summary.updated.ends_with('Z'));
        let yaml = fs::read_to_string(dir.path().join("melt.yaml")).unwrap();
        assert!(!yaml.contains("s3cret"));
        assert_eq!(
            store
                .get(SERVICE, &format!("plug:{}:password", summary.id))
                .unwrap(),
            "s3cret"
        );
        let again = prepare_create(dir.path(), request("bench", "x"));
        assert!(matches!(again, Err(PlugError::AlreadyExists(_))));
    }

    #[test]
    fn failed_verification_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();

        // Verification fails, so the caller never commits.
        let _prepared = prepare_create(dir.path(), request("bench", "wrong")).unwrap();

        assert!(list(dir.path()).unwrap().is_empty());
        assert!(store.0.lock().unwrap().is_empty());
        assert!(!dir.path().join("melt.yaml").exists());
    }

    #[test]
    fn config_failure_rolls_back_the_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let plug = prepare_create(dir.path(), request("bench", "s3cret")).unwrap();
        // An invalid ID passes the secret store but fails config validation on save.
        let mut plug = plug;
        plug.profile.id = "bad".into();

        let result = commit_create(dir.path(), &store, plug);

        assert!(
            matches!(result, Err(PlugError::Config(_))),
            "{:?}",
            result.err()
        );
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[test]
    fn edit_metadata_works_without_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        create(dir.path(), &store, "bench");
        store.0.lock().unwrap().clear();

        let mut edit = request("rack", "");
        edit.timeout = "20s".into();
        let update = prepare_update(dir.path(), &store, "bench", edit).unwrap();
        assert!(!update.needs_verify);
        let summary = commit_update(dir.path(), &store, update).unwrap();

        assert_eq!(summary.name, "rack");
        assert_eq!(summary.timeout, "20s");
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[test]
    fn edit_connection_needs_verify_and_stored_secret() {
        type Edit = fn(&mut PlugRequest);
        let cases: [(&str, Edit); 3] = [
            ("host", |request| request.host = "192.0.2.31".into()),
            ("username", |request| request.username = "other".into()),
            ("driver", |request| request.driver = "tapo".into()),
        ];
        for (name, edit) in cases {
            let dir = tempfile::tempdir().unwrap();
            let store = MemoryStore::default();
            create(dir.path(), &store, "bench");
            let mut changed = request("bench", "");
            edit(&mut changed);

            let update = prepare_update(dir.path(), &store, "bench", changed);
            // An unchanged driver is not a connection change.
            if name == "driver" {
                assert!(!update.unwrap().needs_verify, "{name}");
                continue;
            }
            let update = update.unwrap();
            assert!(update.needs_verify, "{name}");
            assert_eq!(update.plug.password, "s3cret", "{name}");

            store.0.lock().unwrap().clear();
            let mut changed = request("bench", "");
            edit(&mut changed);
            let error = prepare_update(dir.path(), &store, "bench", changed).err();
            assert!(matches!(error, Some(PlugError::MissingSecret)), "{name}");
        }

        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        create(dir.path(), &store, "bench");
        let update = prepare_update(dir.path(), &store, "bench", request("bench", "new")).unwrap();
        assert!(update.needs_verify);
        assert_eq!(
            resolve(dir.path(), &store, "bench").unwrap().password,
            "s3cret",
            "prepare must not store"
        );
        commit_update(dir.path(), &store, update).unwrap();
        assert_eq!(
            resolve(dir.path(), &store, "bench").unwrap().password,
            "new"
        );
    }

    #[test]
    fn rename_keeps_id_links_and_keychain_account() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        add_devices(dir.path());
        let created = create(dir.path(), &store, "bench");
        link(dir.path(), "bench", &printer()).unwrap();
        link(dir.path(), "bench", &camera()).unwrap();

        let update = prepare_update(dir.path(), &store, "bench", request("rack", "")).unwrap();
        let renamed = commit_update(dir.path(), &store, update).unwrap();

        assert_eq!(renamed.id, created.id);
        assert_eq!(renamed.printers, ["workshop"]);
        assert_eq!(renamed.cameras, ["porch"]);
        assert!(matches!(
            resolve(dir.path(), &store, "bench"),
            Err(PlugError::NotFound(_))
        ));
        assert_eq!(
            resolve(dir.path(), &store, "rack").unwrap().password,
            "s3cret"
        );
        assert_eq!(
            resolve_by_id(dir.path(), &store, &created.id).unwrap().name,
            "rack"
        );
        assert_ne!(
            revision(dir.path(), &created.id).unwrap(),
            None,
            "revision follows the id"
        );
    }

    #[test]
    fn username_case_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let mut mixed = request("bench", "s3cret");
        mixed.username = "Operator@Example.invalid".into();
        let plug = prepare_create(dir.path(), mixed).unwrap();
        commit_create(dir.path(), &store, plug).unwrap();

        let resolved = resolve(dir.path(), &store, "bench").unwrap();
        assert_eq!(resolved.profile.username, "Operator@Example.invalid");
        assert_eq!(
            list(dir.path()).unwrap()[0].username,
            "Operator@Example.invalid"
        );
    }

    #[test]
    fn invalid_requests_are_rejected() {
        type Edit = fn(&mut PlugRequest);
        type Check = fn(&PlugError) -> bool;
        let cases: [(&str, Edit, Check); 12] = [
            (
                "empty name",
                |r| r.name.clear(),
                |e| matches!(e, PlugError::MissingName),
            ),
            (
                "slash name",
                |r| r.name = "a/b".into(),
                |e| matches!(e, PlugError::InvalidName),
            ),
            (
                "driver",
                |r| r.driver = "kasa".into(),
                |e| matches!(e, PlugError::UnknownDriver(_)),
            ),
            (
                "scheme host",
                |r| r.host = "http://h".into(),
                |e| matches!(e, PlugError::InvalidHost),
            ),
            (
                "port host",
                |r| r.host = "h:80".into(),
                |e| matches!(e, PlugError::InvalidHost),
            ),
            (
                "user host",
                |r| r.host = "u@h".into(),
                |e| matches!(e, PlugError::InvalidHost),
            ),
            (
                "empty username",
                |r| r.username.clear(),
                |e| matches!(e, PlugError::InvalidUsername),
            ),
            (
                "long username",
                |r| r.username = "u".repeat(255),
                |e| matches!(e, PlugError::InvalidUsername),
            ),
            (
                "control username",
                |r| r.username = "a\u{7}b".into(),
                |e| matches!(e, PlugError::InvalidUsername),
            ),
            (
                "control password",
                |r| r.password = "a\u{7}b".into(),
                |e| matches!(e, PlugError::InvalidPassword),
            ),
            (
                "empty password",
                |r| r.password.clear(),
                |e| matches!(e, PlugError::InvalidPassword),
            ),
            (
                "timeout",
                |r| r.timeout = "soon".into(),
                |e| matches!(e, PlugError::InvalidTimeout),
            ),
        ];
        for (name, edit, matches) in cases {
            let dir = tempfile::tempdir().unwrap();
            let mut invalid = request("bench", "s3cret");
            edit(&mut invalid);
            let error = prepare_create(dir.path(), invalid).err();
            assert!(error.as_ref().is_some_and(matches), "{name}: {error:?}");
        }
        let long_ok = {
            let mut r = request("bench", "s3cret");
            r.username = "u".repeat(254);
            r
        };
        assert!(prepare_create(tempfile::tempdir().unwrap().path(), long_ok).is_ok());
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
    fn remove_reports_unlinked_targets_and_keychain_warning() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        add_devices(dir.path());
        create(dir.path(), &store, "bench");
        link(dir.path(), "bench", &printer()).unwrap();
        link(dir.path(), "bench", &camera()).unwrap();

        let removed = remove(dir.path(), &store, "bench").unwrap();
        assert!(removed.password_removed);
        assert_eq!(removed.unlinked_printers, ["workshop"]);
        assert_eq!(removed.unlinked_cameras, ["porch"]);
        assert!(store.0.lock().unwrap().is_empty());
        assert!(matches!(
            remove(dir.path(), &store, "bench"),
            Err(PlugError::NotFound(_))
        ));

        let failing = DeleteFails(MemoryStore::default());
        create(dir.path(), &failing, "bench");
        let removed = remove(dir.path(), &failing, "bench").unwrap();
        assert!(!removed.password_removed);
        assert_eq!(removed.warnings.len(), 1);
        assert_eq!(removed.warnings[0].code, "password_delete_failed");
        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_plug("bench")
                .is_none()
        );
    }

    #[test]
    fn list_for_filters_by_printer_and_camera_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        add_devices(dir.path());
        create(dir.path(), &store, "bench");
        create(dir.path(), &store, "rack");
        link(dir.path(), "bench", &printer()).unwrap();
        link(dir.path(), "rack", &camera()).unwrap();
        link(dir.path(), "rack", &camera()).unwrap();

        let names = |target| {
            list_for(dir.path(), &target)
                .unwrap()
                .into_iter()
                .map(|plug| plug.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(names(PlugTarget::Printer("Workshop".into())), ["bench"]);
        assert_eq!(names(camera()), ["rack"]);
        assert_eq!(list(dir.path()).unwrap()[1].cameras, ["porch"]);
        assert!(matches!(
            list_for(dir.path(), &PlugTarget::Printer("attic".into())),
            Err(PlugError::PrinterNotFound(_))
        ));
        assert!(matches!(
            list_for(dir.path(), &PlugTarget::Camera("attic".into())),
            Err(PlugError::CameraNotFound(_))
        ));

        unlink(dir.path(), "rack", &camera()).unwrap();
        assert!(names(camera()).is_empty());
        assert!(matches!(
            unlink(dir.path(), "rack", &camera()),
            Err(PlugError::NotLinked { .. })
        ));
    }

    #[test]
    fn unknown_targets_fail_before_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        add_devices(dir.path());
        create(dir.path(), &store, "bench");
        let before = fs::read_to_string(dir.path().join("melt.yaml")).unwrap();

        assert!(matches!(
            link(dir.path(), "bench", &PlugTarget::Printer("attic".into())),
            Err(PlugError::PrinterNotFound(_))
        ));
        assert!(matches!(
            link(dir.path(), "bench", &PlugTarget::Camera("attic".into())),
            Err(PlugError::CameraNotFound(_))
        ));
        assert!(matches!(
            link(dir.path(), "ghost", &printer()),
            Err(PlugError::NotFound(_))
        ));

        assert_eq!(
            fs::read_to_string(dir.path().join("melt.yaml")).unwrap(),
            before
        );
    }
}
