use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

const CONFIG_FILE: &str = "melt.yaml";
/// Version 2 adds camera profiles and printer links. Files without camera
/// data are still written as version 1 so older binaries keep reading them.
const MAX_VERSION: u8 = 2;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unsupported config schema version: got {got}, want 1 or {MAX_VERSION}")]
    UnsupportedVersion { got: u8 },
    #[error("malformed config file: {0}")]
    Malformed(#[from] serde_saphyr::Error),
    #[error("invalid camera configuration: {0}")]
    InvalidCameras(&'static str),
    #[error("config file changed since it was read")]
    ConcurrentUpdate,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("profile already exists")]
    ProfileAlreadyExists,
    #[error("profile not found")]
    ProfileNotFound,
    #[error("camera already exists")]
    CameraAlreadyExists,
    #[error("camera not found")]
    CameraNotFound,
    #[error("camera is not linked to that printer")]
    CameraNotLinked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub driver: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub serial: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub model: String,
    pub timeout: String,
    pub insecure: bool,
    pub created: String,
    pub updated: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedProfile {
    pub name: String,
    pub profile: Profile,
}

/// An external camera. The name is its map key; `id` is immutable and keys
/// both printer links and the keychain password, so renames touch neither.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CameraProfile {
    pub id: String,
    pub driver: String,
    pub host: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtsp_port: Option<u16>,
    pub timeout: String,
    pub created: String,
    pub updated: String,
}

/// Cameras linked to one printer. `selected: None` is the built-in feed.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrinterCameras {
    #[serde(default)]
    pub cameras: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
}

pub fn is_camera_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    version: u8,
    #[serde(default)]
    profiles: BTreeMap<String, Profile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    cameras: BTreeMap<String, CameraProfile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    printer_cameras: BTreeMap<String, PrinterCameras>,
}

#[derive(Debug)]
pub struct Config {
    profiles: BTreeMap<String, Profile>,
    cameras: BTreeMap<String, CameraProfile>,
    printer_cameras: BTreeMap<String, PrinterCameras>,
    digest: Option<[u8; 32]>,
}

impl Config {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = dir.as_ref().join(CONFIG_FILE);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::empty());
            }
            Err(error) => return Err(ConfigError::Io(error)),
        };
        if bytes.is_empty() {
            return Ok(Self {
                digest: Some(digest(&bytes)),
                ..Self::empty()
            });
        }

        let file: ConfigFile = serde_saphyr::from_slice(&bytes).map_err(ConfigError::Malformed)?;
        if !(1..=MAX_VERSION).contains(&file.version) {
            return Err(ConfigError::UnsupportedVersion { got: file.version });
        }
        if file.version == 1 && (!file.cameras.is_empty() || !file.printer_cameras.is_empty()) {
            return Err(ConfigError::InvalidCameras(
                "camera data requires config version 2",
            ));
        }
        let config = Self {
            profiles: file.profiles,
            cameras: file.cameras,
            printer_cameras: file.printer_cameras,
            digest: Some(digest(&bytes)),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn load() -> Result<Self, ConfigError> {
        Self::open(config_dir()?)
    }

    pub fn get_profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }

    pub fn sorted_profiles(&self) -> Vec<NamedProfile> {
        self.profiles
            .iter()
            .map(|(name, profile)| NamedProfile {
                name: name.clone(),
                profile: profile.clone(),
            })
            .collect()
    }

    pub fn add_profile(
        &mut self,
        name: impl Into<String>,
        profile: Profile,
    ) -> Result<(), ConfigError> {
        let name = name.into();
        if self.profiles.contains_key(&name) {
            return Err(ConfigError::ProfileAlreadyExists);
        }
        self.profiles.insert(name, profile);
        Ok(())
    }

    pub fn set_profile(&mut self, name: &str, profile: Profile) -> Result<(), ConfigError> {
        let stored = self
            .profiles
            .get_mut(name)
            .ok_or(ConfigError::ProfileNotFound)?;
        *stored = profile;
        Ok(())
    }

    /// Removes a printer and its camera links. Camera profiles stay.
    pub fn remove_profile(&mut self, name: &str) -> Result<Profile, ConfigError> {
        let profile = self
            .profiles
            .remove(name)
            .ok_or(ConfigError::ProfileNotFound)?;
        self.printer_cameras.remove(name);
        Ok(profile)
    }

    /// Renames a printer, carrying its camera links along in the same write.
    pub fn rename_profile(
        &mut self,
        current: &str,
        name: &str,
        profile: Profile,
    ) -> Result<(), ConfigError> {
        if self.profiles.contains_key(name) {
            return Err(ConfigError::ProfileAlreadyExists);
        }
        self.profiles
            .remove(current)
            .ok_or(ConfigError::ProfileNotFound)?;
        self.profiles.insert(name.to_owned(), profile);
        if let Some(links) = self.printer_cameras.remove(current) {
            self.printer_cameras.insert(name.to_owned(), links);
        }
        Ok(())
    }

    pub fn get_camera(&self, name: &str) -> Option<&CameraProfile> {
        self.cameras.get(name)
    }

    pub fn cameras(&self) -> &BTreeMap<String, CameraProfile> {
        &self.cameras
    }

    pub fn add_camera(&mut self, name: &str, camera: CameraProfile) -> Result<(), ConfigError> {
        if self.cameras.contains_key(name) {
            return Err(ConfigError::CameraAlreadyExists);
        }
        self.cameras.insert(name.to_owned(), camera);
        Ok(())
    }

    /// Replaces a camera, renaming it when `name` differs from `current`.
    pub fn replace_camera(
        &mut self,
        current: &str,
        name: &str,
        camera: CameraProfile,
    ) -> Result<(), ConfigError> {
        if name != current && self.cameras.contains_key(name) {
            return Err(ConfigError::CameraAlreadyExists);
        }
        self.cameras
            .remove(current)
            .ok_or(ConfigError::CameraNotFound)?;
        self.cameras.insert(name.to_owned(), camera);
        Ok(())
    }

    /// Removes a camera and every link to it; printers that had it selected
    /// fall back to their built-in feed.
    pub fn remove_camera(&mut self, name: &str) -> Result<CameraProfile, ConfigError> {
        let camera = self
            .cameras
            .remove(name)
            .ok_or(ConfigError::CameraNotFound)?;
        for links in self.printer_cameras.values_mut() {
            links.cameras.retain(|id| *id != camera.id);
            if links.selected.as_deref() == Some(camera.id.as_str()) {
                links.selected = None;
            }
        }
        self.printer_cameras
            .retain(|_, links| !links.cameras.is_empty());
        Ok(camera)
    }

    pub fn link_camera(
        &mut self,
        printer: &str,
        camera: &str,
        select: bool,
    ) -> Result<(), ConfigError> {
        if !self.profiles.contains_key(printer) {
            return Err(ConfigError::ProfileNotFound);
        }
        let id = self
            .cameras
            .get(camera)
            .ok_or(ConfigError::CameraNotFound)?
            .id
            .clone();
        let links = self.printer_cameras.entry(printer.to_owned()).or_default();
        if !links.cameras.contains(&id) {
            links.cameras.push(id.clone());
        }
        if select {
            links.selected = Some(id);
        }
        Ok(())
    }

    pub fn unlink_camera(&mut self, printer: &str, camera: &str) -> Result<(), ConfigError> {
        let id = &self
            .cameras
            .get(camera)
            .ok_or(ConfigError::CameraNotFound)?
            .id;
        let links = self
            .printer_cameras
            .get_mut(printer)
            .filter(|links| links.cameras.contains(id))
            .ok_or(ConfigError::CameraNotLinked)?;
        links.cameras.retain(|linked| linked != id);
        if links.selected.as_ref() == Some(id) {
            links.selected = None;
        }
        if links.cameras.is_empty() {
            self.printer_cameras.remove(printer);
        }
        Ok(())
    }

    /// Selects a linked camera, or the built-in feed when `camera` is `None`.
    pub fn select_camera(
        &mut self,
        printer: &str,
        camera: Option<&str>,
    ) -> Result<(), ConfigError> {
        if !self.profiles.contains_key(printer) {
            return Err(ConfigError::ProfileNotFound);
        }
        let Some(camera) = camera else {
            if let Some(links) = self.printer_cameras.get_mut(printer) {
                links.selected = None;
            }
            return Ok(());
        };
        let id = self
            .cameras
            .get(camera)
            .ok_or(ConfigError::CameraNotFound)?
            .id
            .clone();
        let links = self
            .printer_cameras
            .get_mut(printer)
            .filter(|links| links.cameras.contains(&id))
            .ok_or(ConfigError::CameraNotLinked)?;
        links.selected = Some(id);
        Ok(())
    }

    pub fn printer_cameras(&self, printer: &str) -> Option<&PrinterCameras> {
        self.printer_cameras.get(printer)
    }

    /// Printers linked to camera `id`, and the subset that selected it.
    pub fn camera_printers(&self, id: &str) -> (Vec<String>, Vec<String>) {
        let mut linked = Vec::new();
        let mut selected = Vec::new();
        for (printer, links) in &self.printer_cameras {
            if links.cameras.iter().any(|camera| camera == id) {
                linked.push(printer.clone());
            }
            if links.selected.as_deref() == Some(id) {
                selected.push(printer.clone());
            }
        }
        (linked, selected)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let mut ids = BTreeSet::new();
        for camera in self.cameras.values() {
            if !is_camera_id(&camera.id) || !ids.insert(camera.id.as_str()) {
                return Err(ConfigError::InvalidCameras(
                    "camera IDs must be unique 32-digit lowercase hex values",
                ));
            }
        }
        for (printer, links) in &self.printer_cameras {
            if !self.profiles.contains_key(printer) {
                return Err(ConfigError::InvalidCameras(
                    "a camera link names an unknown printer",
                ));
            }
            let linked: BTreeSet<_> = links.cameras.iter().map(String::as_str).collect();
            if linked.len() != links.cameras.len() || !linked.is_subset(&ids) {
                return Err(ConfigError::InvalidCameras(
                    "a camera link names an unknown or repeated camera",
                ));
            }
            if links
                .selected
                .as_deref()
                .is_some_and(|id| !linked.contains(id))
            {
                return Err(ConfigError::InvalidCameras(
                    "a selected camera is not linked to its printer",
                ));
            }
        }
        Ok(())
    }

    pub fn save(&mut self, dir: impl AsRef<Path>) -> Result<(), ConfigError> {
        let dir = dir.as_ref();
        self.validate()?;
        self.check_unchanged(dir)?;
        fs::create_dir_all(dir)?;
        restrict_dir_permissions(dir)?;

        let has_cameras = !self.cameras.is_empty() || !self.printer_cameras.is_empty();
        let bytes = serde_saphyr::to_string(&ConfigFile {
            version: if has_cameras { MAX_VERSION } else { 1 },
            profiles: self.profiles.clone(),
            cameras: self.cameras.clone(),
            printer_cameras: self.printer_cameras.clone(),
        })
        .map_err(std::io::Error::other)?
        .into_bytes();

        let mut temp = NamedTempFile::new_in(dir)?;
        restrict_file_permissions(temp.as_file())?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(dir.join(CONFIG_FILE))
            .map_err(|error| ConfigError::Io(error.error))?;

        self.digest = Some(digest(&bytes));
        sync_directory(dir);
        Ok(())
    }

    fn empty() -> Self {
        Self {
            profiles: BTreeMap::new(),
            cameras: BTreeMap::new(),
            printer_cameras: BTreeMap::new(),
            digest: None,
        }
    }

    fn check_unchanged(&self, dir: &Path) -> Result<(), ConfigError> {
        let path = dir.join(CONFIG_FILE);
        let current = match fs::read(path) {
            Ok(bytes) => Some(digest(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(ConfigError::Io(error)),
        };
        if current != self.digest {
            return Err(ConfigError::ConcurrentUpdate);
        }
        Ok(())
    }
}

pub fn config_dir() -> Result<PathBuf, ConfigError> {
    if let Some(dir) = env::var_os("MELT_CONFIG_DIR").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(dir));
    }

    #[cfg(target_os = "windows")]
    let base = env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support"));
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));

    base.map(|path| path.join("melt")).ok_or_else(|| {
        ConfigError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "locating config directory",
        ))
    })
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(unix)]
fn restrict_dir_permissions(dir: &Path) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_dir_permissions(_: &Path) -> Result<(), ConfigError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(file: &File) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file_permissions(_: &File) -> Result<(), ConfigError> {
    Ok(())
}

fn sync_directory(dir: &Path) {
    if let Ok(file) = File::open(dir) {
        let _ = file.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"version: 1
profiles:
  attic-p1s:
    driver: bambu-lan
    host: 192.0.2.11
    serial: 01P00C450100XYZ
    timeout: 15s
    insecure: true
    created: 2026-06-13T11:00:00Z
    updated: 2026-06-13T11:00:00Z
"#;

    fn profile() -> Profile {
        Profile {
            driver: "moonraker".into(),
            host: "printer.local".into(),
            serial: String::new(),
            model: String::new(),
            timeout: "10s".into(),
            insecure: false,
            created: "2026-06-13T11:00:00Z".into(),
            updated: "2026-06-13T11:00:00Z".into(),
        }
    }

    #[test]
    fn opens_the_existing_yaml_schema() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), CONFIG).unwrap();

        let config = Config::open(dir.path()).unwrap();

        assert_eq!(config.sorted_profiles()[0].name, "attic-p1s");
        assert_eq!(
            config.get_profile("attic-p1s").unwrap().serial,
            "01P00C450100XYZ"
        );
        assert_eq!(config.get_profile("attic-p1s").unwrap().model, "");
    }

    #[test]
    fn persists_optional_printer_models() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        let mut profile = profile();
        profile.model = "H2D".into();
        config.add_profile("workshop", profile).unwrap();
        config.save(dir.path()).unwrap();

        assert_eq!(
            Config::open(dir.path())
                .unwrap()
                .get_profile("workshop")
                .unwrap()
                .model,
            "H2D"
        );
    }

    #[test]
    fn rejects_unknown_fields_so_secrets_cannot_enter_yaml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(CONFIG_FILE),
            "version: 1\nprofiles: {}\naccess_code: secret\n",
        )
        .unwrap();

        assert!(matches!(
            Config::open(dir.path()),
            Err(ConfigError::Malformed(_))
        ));
    }

    #[test]
    fn refuses_to_overwrite_an_external_change() {
        let dir = tempfile::tempdir().unwrap();
        let mut left = Config::open(dir.path()).unwrap();
        let mut right = Config::open(dir.path()).unwrap();

        left.add_profile("left", profile()).unwrap();
        left.save(dir.path()).unwrap();
        right.add_profile("right", profile()).unwrap();

        assert!(matches!(
            right.save(dir.path()),
            Err(ConfigError::ConcurrentUpdate)
        ));
    }

    #[test]
    fn saves_twice_with_permissions_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("one", profile()).unwrap();
        config.save(dir.path()).unwrap();
        config.add_profile("two", profile()).unwrap();
        config.save(dir.path()).unwrap();

        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_profile("two")
                .is_some()
        );
        assert_eq!(
            fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().starts_with(".melt-"))
                .count(),
            0
        );
    }

    const PORCH: &str = "0123456789abcdef0123456789abcdef";
    const GATE: &str = "fedcba9876543210fedcba9876543210";

    fn camera(id: &str) -> CameraProfile {
        CameraProfile {
            id: id.into(),
            driver: "tapo".into(),
            host: "192.0.2.20".into(),
            username: "melt".into(),
            rtsp_port: None,
            timeout: "10s".into(),
            created: "2026-10-08T12:00:00Z".into(),
            updated: "2026-10-08T12:00:00Z".into(),
        }
    }

    #[test]
    fn printer_only_configs_stay_readable_by_version_1_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.save(dir.path()).unwrap();

        let yaml = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(yaml.starts_with("version: 1\n"), "{yaml}");
        assert!(!yaml.contains("cameras"), "{yaml}");
    }

    #[test]
    fn camera_data_is_written_as_version_2_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        config.link_camera("garage", "porch", true).unwrap();
        config.save(dir.path()).unwrap();

        let yaml = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(yaml.starts_with("version: 2\n"), "{yaml}");
        let reopened = Config::open(dir.path()).unwrap();
        assert_eq!(reopened.get_camera("porch"), Some(&camera(PORCH)));
        assert_eq!(
            reopened.printer_cameras("garage"),
            Some(&PrinterCameras {
                cameras: vec![PORCH.into()],
                selected: Some(PORCH.into()),
            })
        );
        assert_eq!(
            reopened.camera_printers(PORCH),
            (vec!["garage".to_owned()], vec!["garage".to_owned()])
        );
    }

    #[test]
    fn rejects_invalid_camera_sections_instead_of_dropping_them() {
        let printer = "profiles:\n  garage: {driver: moonraker, host: printer.local, timeout: 10s, insecure: false, created: x, updated: x}\n";
        let porch = "cameras:\n  porch: {id: 0123456789abcdef0123456789abcdef, driver: tapo, host: 192.0.2.20, username: melt, timeout: 10s, created: x, updated: x}\n";
        let cases = [
            ("camera data in version 1", format!("version: 1\n{porch}")),
            (
                "link to unknown printer",
                format!("version: 2\n{porch}printer_cameras:\n  garage: {{cameras: [0123456789abcdef0123456789abcdef]}}\n"),
            ),
            (
                "link to unknown camera",
                format!("version: 2\n{printer}{porch}printer_cameras:\n  garage: {{cameras: [fedcba9876543210fedcba9876543210]}}\n"),
            ),
            (
                "selection not linked",
                format!("version: 2\n{printer}{porch}printer_cameras:\n  garage: {{cameras: [], selected: 0123456789abcdef0123456789abcdef}}\n"),
            ),
            (
                "malformed id",
                "version: 2\ncameras:\n  porch: {id: NOT-HEX, driver: tapo, host: 192.0.2.20, username: melt, timeout: 10s, created: x, updated: x}\n".to_owned(),
            ),
        ];
        for (name, yaml) in cases {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join(CONFIG_FILE), yaml).unwrap();
            assert!(
                matches!(
                    Config::open(dir.path()),
                    Err(ConfigError::InvalidCameras(_))
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn future_versions_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), "version: 3\n").unwrap();
        assert!(matches!(
            Config::open(dir.path()),
            Err(ConfigError::UnsupportedVersion { got: 3 })
        ));
    }

    #[test]
    fn printer_rename_moves_links_and_printer_removal_drops_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        config.link_camera("garage", "porch", true).unwrap();

        config
            .rename_profile("garage", "workshop", profile())
            .unwrap();
        assert!(config.printer_cameras("garage").is_none());
        assert_eq!(
            config
                .printer_cameras("workshop")
                .unwrap()
                .selected
                .as_deref(),
            Some(PORCH)
        );

        config.remove_profile("workshop").unwrap();
        assert!(config.printer_cameras("workshop").is_none());
        assert!(config.get_camera("porch").is_some());
    }

    #[test]
    fn camera_removal_and_unlinking_fall_back_to_the_builtin_feed() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_profile("attic", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        config.add_camera("gate", camera(GATE)).unwrap();
        config.link_camera("garage", "porch", true).unwrap();
        config.link_camera("garage", "gate", false).unwrap();
        config.link_camera("attic", "porch", false).unwrap();

        config.remove_camera("porch").unwrap();
        assert_eq!(
            config.printer_cameras("garage"),
            Some(&PrinterCameras {
                cameras: vec![GATE.into()],
                selected: None,
            })
        );
        assert!(config.printer_cameras("attic").is_none());

        config.select_camera("garage", Some("gate")).unwrap();
        config.unlink_camera("garage", "gate").unwrap();
        assert!(config.printer_cameras("garage").is_none());
    }

    #[test]
    fn link_operations_report_missing_targets() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();

        assert!(matches!(
            config.link_camera("nope", "porch", false),
            Err(ConfigError::ProfileNotFound)
        ));
        assert!(matches!(
            config.link_camera("garage", "nope", false),
            Err(ConfigError::CameraNotFound)
        ));
        assert!(matches!(
            config.select_camera("garage", Some("porch")),
            Err(ConfigError::CameraNotLinked)
        ));
        assert!(matches!(
            config.unlink_camera("garage", "porch"),
            Err(ConfigError::CameraNotLinked)
        ));
        assert!(matches!(
            config.add_camera("porch", camera(GATE)),
            Err(ConfigError::CameraAlreadyExists)
        ));
    }

    /// Bytes exactly as serde_yaml 0.9 wrote them, including scalars a YAML
    /// library could retype (`'0123'`, `'null'`, `'~'`, plain `yes`).
    const SERDE_YAML_V2: &str = r#"version: 2
profiles:
  '0123':
    driver: bambu-lan
    host: 192.0.2.11
    serial: '0123'
    model: 'true'
    timeout: '10'
    insecure: false
    created: 2026-06-13T11:00:00Z
    updated: 2026-06-13T11:00:00Z
  'attic: p1s':
    driver: bambu-lan
    host: 192.0.2.11
    serial: 'null'
    model: 'true'
    timeout: '10'
    insecure: false
    created: 2026-06-13T11:00:00Z
    updated: 2026-06-13T11:00:00Z
cameras:
  yes:
    id: 0123456789abcdef0123456789abcdef
    driver: tapo
    host: cam.local
    username: '~'
    rtsp_port: 554
    timeout: '1e3'
    created: 2026-06-13T11:00:00Z
    updated: 2026-06-13T11:00:00Z
printer_cameras:
  '0123':
    cameras:
    - 0123456789abcdef0123456789abcdef
    selected: 0123456789abcdef0123456789abcdef
  'attic: p1s':
    cameras: []
"#;

    #[test]
    fn reads_and_rewrites_files_written_by_serde_yaml() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), SERDE_YAML_V2).unwrap();

        let mut config = Config::open(dir.path()).unwrap();
        let tricky = config.get_profile("0123").unwrap();
        assert_eq!(
            (
                tricky.serial.as_str(),
                tricky.model.as_str(),
                tricky.timeout.as_str()
            ),
            ("0123", "true", "10")
        );
        assert_eq!(config.get_profile("attic: p1s").unwrap().serial, "null");
        let camera = &config.cameras["yes"];
        assert_eq!(
            (
                camera.username.as_str(),
                camera.timeout.as_str(),
                camera.rtsp_port
            ),
            ("~", "1e3", Some(554))
        );
        assert_eq!(
            config.printer_cameras["0123"].selected.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            config.printer_cameras["attic: p1s"],
            PrinterCameras::default()
        );

        let (profiles, cameras, links) = (
            config.profiles.clone(),
            config.cameras.clone(),
            config.printer_cameras.clone(),
        );
        config.save(dir.path()).unwrap();
        let written = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        // Ambiguous strings must stay quoted so older serde_yaml builds read them.
        for plain in [
            "serial: 0123\n",
            "serial: null\n",
            "model: true\n",
            "username: ~\n",
            "timeout: 1e3\n",
        ] {
            assert!(
                !written.contains(plain),
                "unquoted {plain:?} in:\n{written}"
            );
        }
        let reopened = Config::open(dir.path()).unwrap();
        assert_eq!(
            (
                reopened.profiles,
                reopened.cameras,
                reopened.printer_cameras
            ),
            (profiles, cameras, links)
        );
    }
}
