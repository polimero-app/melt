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
/// Version 2 adds camera profiles and printer links; version 3 adds smart
/// plugs. The writer picks the minimum version that fits the data, so older
/// binaries keep reading files that do not use newer features.
const MAX_VERSION: u8 = 3;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unsupported config schema version: got {got}, want 1 to {MAX_VERSION}")]
    UnsupportedVersion { got: u8 },
    #[error("malformed config file: {0}")]
    Malformed(#[from] serde_saphyr::Error),
    #[error("invalid camera configuration: {0}")]
    InvalidCameras(&'static str),
    #[error("invalid plug configuration: {0}")]
    InvalidPlugs(&'static str),
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
    #[error("plug already exists")]
    PlugAlreadyExists,
    #[error("plug not found")]
    PlugNotFound,
    #[error("plug is not linked to that target")]
    PlugNotLinked,
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

/// A smart plug. `id` is immutable; `printers` are printer profile names and
/// `cameras` are camera IDs this plug powers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlugProfile {
    pub id: String,
    /// Verified physical device identity. Older profiles remain read-only
    /// until an explicit verification enrolls their device.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    pub driver: String,
    pub host: String,
    pub username: String,
    pub timeout: String,
    #[serde(default)]
    pub printers: Vec<String>,
    #[serde(default)]
    pub cameras: Vec<String>,
    pub created: String,
    pub updated: String,
}

/// A device a plug powers. `Camera` holds the camera's name, resolved to its
/// immutable ID inside [`Config`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlugTarget {
    Printer(String),
    Camera(String),
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

pub fn is_profile_id(id: &str) -> bool {
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    plugs: BTreeMap<String, PlugProfile>,
}

#[derive(Debug)]
pub struct Config {
    profiles: BTreeMap<String, Profile>,
    cameras: BTreeMap<String, CameraProfile>,
    printer_cameras: BTreeMap<String, PrinterCameras>,
    plugs: BTreeMap<String, PlugProfile>,
    digest: Option<[u8; 32]>,
    write_lock: Option<File>,
}

impl Config {
    /// Holds the config's write lock until dropped; save back to the same directory.
    pub(crate) fn open_locked(dir: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let lock = lock_config(dir.as_ref())?;
        let mut config = Self::open(dir)?;
        config.write_lock = Some(lock);
        Ok(config)
    }

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
        if file.version < 3 && !file.plugs.is_empty() {
            return Err(ConfigError::InvalidPlugs(
                "plug data requires config version 3",
            ));
        }
        let config = Self {
            profiles: file.profiles,
            cameras: file.cameras,
            printer_cameras: file.printer_cameras,
            plugs: file.plugs,
            digest: Some(digest(&bytes)),
            write_lock: None,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn load() -> Result<Self, ConfigError> {
        Self::open(config_dir()?)
    }

    pub fn plugs(&self) -> &BTreeMap<String, PlugProfile> {
        &self.plugs
    }

    pub fn get_plug(&self, name: &str) -> Option<&PlugProfile> {
        self.plugs.get(name)
    }

    pub fn plug_by_id(&self, id: &str) -> Option<(&str, &PlugProfile)> {
        self.plugs
            .iter()
            .find(|(_, plug)| plug.id == id)
            .map(|(name, plug)| (name.as_str(), plug))
    }

    pub fn camera_name(&self, id: &str) -> Option<&str> {
        self.cameras
            .iter()
            .find(|(_, camera)| camera.id == id)
            .map(|(name, _)| name.as_str())
    }

    pub fn add_plug(&mut self, name: &str, plug: PlugProfile) -> Result<(), ConfigError> {
        if self.plugs.contains_key(name) {
            return Err(ConfigError::PlugAlreadyExists);
        }
        self.plugs.insert(name.to_owned(), plug);
        Ok(())
    }

    /// Replaces a plug, renaming it when `name` differs from `current`. The
    /// caller passes the links it wants kept; the ID and keychain account
    /// follow the profile it supplies.
    pub fn replace_plug(
        &mut self,
        current: &str,
        name: &str,
        plug: PlugProfile,
    ) -> Result<(), ConfigError> {
        if name != current && self.plugs.contains_key(name) {
            return Err(ConfigError::PlugAlreadyExists);
        }
        self.plugs
            .remove(current)
            .ok_or(ConfigError::PlugNotFound)?;
        self.plugs.insert(name.to_owned(), plug);
        Ok(())
    }

    /// Removes a plug and its links. Printer and camera profiles stay, and no
    /// relay is touched.
    pub fn remove_plug(&mut self, name: &str) -> Result<PlugProfile, ConfigError> {
        self.plugs.remove(name).ok_or(ConfigError::PlugNotFound)
    }

    /// Resolves a target to the string a plug stores (printer name or camera ID).
    fn plug_link_key(&self, target: &PlugTarget) -> Result<String, ConfigError> {
        match target {
            PlugTarget::Printer(name) if self.profiles.contains_key(name) => Ok(name.clone()),
            PlugTarget::Printer(_) => Err(ConfigError::ProfileNotFound),
            PlugTarget::Camera(name) => self
                .cameras
                .get(name)
                .map(|camera| camera.id.clone())
                .ok_or(ConfigError::CameraNotFound),
        }
    }

    /// Links a plug to a printer or camera. Idempotent.
    pub fn link_plug(&mut self, plug: &str, target: &PlugTarget) -> Result<(), ConfigError> {
        let key = self.plug_link_key(target)?;
        let stored = self.plugs.get_mut(plug).ok_or(ConfigError::PlugNotFound)?;
        let links = match target {
            PlugTarget::Printer(_) => &mut stored.printers,
            PlugTarget::Camera(_) => &mut stored.cameras,
        };
        if !links.contains(&key) {
            links.push(key);
        }
        Ok(())
    }

    pub fn unlink_plug(&mut self, plug: &str, target: &PlugTarget) -> Result<(), ConfigError> {
        let key = self.plug_link_key(target)?;
        let stored = self.plugs.get_mut(plug).ok_or(ConfigError::PlugNotFound)?;
        let links = match target {
            PlugTarget::Printer(_) => &mut stored.printers,
            PlugTarget::Camera(_) => &mut stored.cameras,
        };
        let before = links.len();
        links.retain(|linked| *linked != key);
        if links.len() == before {
            return Err(ConfigError::PlugNotLinked);
        }
        Ok(())
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

    /// Removes a printer, its camera links and its plug links. Camera and plug
    /// profiles stay.
    pub fn remove_profile(&mut self, name: &str) -> Result<Profile, ConfigError> {
        let profile = self
            .profiles
            .remove(name)
            .ok_or(ConfigError::ProfileNotFound)?;
        self.printer_cameras.remove(name);
        for plug in self.plugs.values_mut() {
            plug.printers.retain(|printer| printer != name);
        }
        Ok(profile)
    }

    /// Renames a printer, carrying its camera and plug links along in the same
    /// write.
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
        for printer in self.plugs.values_mut().flat_map(|p| &mut p.printers) {
            if printer == current {
                name.clone_into(printer);
            }
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

    /// Removes a camera and every link to it, including plug links; printers
    /// that had it selected fall back to their built-in feed.
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
        for plug in self.plugs.values_mut() {
            plug.cameras.retain(|id| *id != camera.id);
        }
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
            if !is_profile_id(&camera.id) || !ids.insert(camera.id.as_str()) {
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
        let mut plug_ids = BTreeSet::new();
        for plug in self.plugs.values() {
            let mut printers = BTreeSet::new();
            let mut cameras = BTreeSet::new();
            if !is_profile_id(&plug.id) || !plug_ids.insert(plug.id.as_str()) {
                return Err(ConfigError::InvalidPlugs(
                    "plug IDs must be unique 32-digit lowercase hex values",
                ));
            }
            if plug
                .device_id
                .as_ref()
                .is_some_and(|id| id.is_empty() || id.chars().any(char::is_control))
            {
                return Err(ConfigError::InvalidPlugs(
                    "a plug has an invalid device identity",
                ));
            }
            for printer in &plug.printers {
                if !self.profiles.contains_key(printer) {
                    return Err(ConfigError::InvalidPlugs("a plug names an unknown printer"));
                }
                if !printers.insert(printer.as_str()) {
                    return Err(ConfigError::InvalidPlugs("a plug repeats a printer"));
                }
            }
            for camera in &plug.cameras {
                if !ids.contains(camera.as_str()) {
                    return Err(ConfigError::InvalidPlugs("a plug names an unknown camera"));
                }
                if !cameras.insert(camera.as_str()) {
                    return Err(ConfigError::InvalidPlugs("a plug repeats a camera"));
                }
            }
        }
        Ok(())
    }

    pub fn save(&mut self, dir: impl AsRef<Path>) -> Result<(), ConfigError> {
        let dir = dir.as_ref();
        self.validate()?;
        let _lock = if self.write_lock.is_none() {
            Some(lock_config(dir)?)
        } else {
            None
        };
        self.check_unchanged(dir)?;

        let has_cameras = !self.cameras.is_empty() || !self.printer_cameras.is_empty();
        let bytes = serde_saphyr::to_string(&ConfigFile {
            version: if !self.plugs.is_empty() {
                3
            } else if has_cameras {
                2
            } else {
                1
            },
            profiles: self.profiles.clone(),
            cameras: self.cameras.clone(),
            printer_cameras: self.printer_cameras.clone(),
            plugs: self.plugs.clone(),
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
            plugs: BTreeMap::new(),
            digest: None,
            write_lock: None,
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

fn lock_config(dir: &Path) -> Result<File, ConfigError> {
    fs::create_dir_all(dir)?;
    restrict_dir_permissions(dir)?;
    // The config is atomically replaced. Keep a separate, permanent lock inode.
    let lock = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("melt.yaml.lock"))?;
    restrict_file_permissions(&lock)?;
    lock.lock()?;
    Ok(lock)
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
    fn ordinary_saves_wait_for_a_locked_transaction_and_then_check_for_changes() {
        use std::{sync::mpsc, time::Duration};

        let dir = tempfile::tempdir().unwrap();
        let mut transaction = Config::open_locked(dir.path()).unwrap();
        let mut other = Config::open(dir.path()).unwrap();
        other.add_profile("other", profile()).unwrap();
        let (started_tx, started) = mpsc::channel();
        let (finished_tx, finished) = mpsc::channel();
        std::thread::scope(|scope| {
            let path = dir.path();
            let writer = scope.spawn(move || {
                started_tx.send(()).unwrap();
                let result = other.save(path);
                finished_tx.send(()).unwrap();
                result
            });
            started.recv().unwrap();
            let overlapped = finished.recv_timeout(Duration::from_millis(100)).is_ok();
            transaction.add_profile("winner", profile()).unwrap();
            transaction.save(dir.path()).unwrap();
            drop(transaction);
            let result = writer.join().unwrap();
            assert!(
                !overlapped,
                "ordinary saves must respect the transaction lock"
            );
            assert!(matches!(result, Err(ConfigError::ConcurrentUpdate)));
        });
        assert!(
            Config::open(dir.path())
                .unwrap()
                .get_profile("winner")
                .is_some()
        );
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
        fs::write(dir.path().join(CONFIG_FILE), "version: 4\n").unwrap();
        assert!(matches!(
            Config::open(dir.path()),
            Err(ConfigError::UnsupportedVersion { got: 4 })
        ));
    }

    const SOCKET: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const LAMP: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn plug(id: &str) -> PlugProfile {
        PlugProfile {
            id: id.into(),
            device_id: None,
            driver: "tapo".into(),
            host: "192.0.2.30".into(),
            username: "melt".into(),
            timeout: "10s".into(),
            printers: vec![],
            cameras: vec![],
            created: "2026-10-18T12:00:00Z".into(),
            updated: "2026-10-18T12:00:00Z".into(),
        }
    }

    #[test]
    fn legacy_plug_profiles_stay_unbound_and_verified_identity_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_plug("bench", plug(SOCKET)).unwrap();
        config.save(dir.path()).unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        assert!(config.get_plug("bench").unwrap().device_id.is_none());
        let mut bound = config.get_plug("bench").unwrap().clone();
        bound.device_id = Some("test-p110".into());
        config.replace_plug("bench", "bench", bound).unwrap();
        config.save(dir.path()).unwrap();

        let mut config = Config::open(dir.path()).unwrap();
        assert_eq!(
            config.get_plug("bench").unwrap().device_id.as_deref(),
            Some("test-p110")
        );
        let mut invalid = config.get_plug("bench").unwrap().clone();
        invalid.device_id = Some(String::new());
        config.replace_plug("bench", "bench", invalid).unwrap();
        assert!(matches!(
            config.save(dir.path()),
            Err(ConfigError::InvalidPlugs(_))
        ));
    }

    fn saved_version(config: &mut Config) -> String {
        let dir = tempfile::tempdir().unwrap();
        config.digest = None;
        config.save(dir.path()).unwrap();
        let yaml = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        yaml.lines().next().unwrap().to_owned()
    }

    #[test]
    fn plug_data_round_trips_as_version_3() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        let mut socket = plug(SOCKET);
        socket.printers = vec!["garage".into()];
        socket.cameras = vec![PORCH.into()];
        config.plugs.insert("socket".into(), socket.clone());
        config.save(dir.path()).unwrap();

        let yaml = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(yaml.starts_with("version: 3\n"), "{yaml}");
        let reopened = Config::open(dir.path()).unwrap();
        assert_eq!(reopened.plugs().get("socket"), Some(&socket));
    }

    #[test]
    fn writer_picks_the_minimum_version() {
        let mut config = Config::empty();
        assert_eq!(saved_version(&mut config), "version: 1");
        config.add_camera("porch", camera(PORCH)).unwrap();
        assert_eq!(saved_version(&mut config), "version: 2");
        config.plugs.insert("socket".into(), plug(SOCKET));
        assert_eq!(saved_version(&mut config), "version: 3");
        config.cameras.clear();
        assert_eq!(saved_version(&mut config), "version: 3");
    }

    #[test]
    fn plug_data_in_older_schemas_is_rejected() {
        let plugs = format!(
            "plugs:\n  socket: {{id: {SOCKET}, driver: tapo, host: h, username: u, timeout: 10s, created: x, updated: x}}\n"
        );
        for version in [1, 2] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(
                dir.path().join(CONFIG_FILE),
                format!("version: {version}\n{plugs}"),
            )
            .unwrap();
            assert!(
                matches!(
                    Config::open(dir.path()),
                    Err(ConfigError::InvalidPlugs(
                        "plug data requires config version 3"
                    ))
                ),
                "version {version}"
            );
        }
    }

    #[test]
    fn invalid_plug_data_is_rejected() {
        type Mutate = fn(&mut BTreeMap<String, PlugProfile>);
        let cases: [(&str, Mutate); 6] = [
            ("malformed id", |p| {
                p.get_mut("a").unwrap().id = "NOPE".into()
            }),
            ("duplicate id", |p| {
                p.get_mut("b").unwrap().id = SOCKET.into()
            }),
            ("repeated printer", |p| {
                p.get_mut("a").unwrap().printers = vec!["garage".into(), "garage".into()]
            }),
            ("repeated camera", |p| {
                p.get_mut("a").unwrap().cameras = vec![PORCH.into(), PORCH.into()]
            }),
            ("unknown printer", |p| {
                p.get_mut("a").unwrap().printers = vec!["nope".into()]
            }),
            ("unknown camera", |p| {
                p.get_mut("a").unwrap().cameras = vec![GATE.into()]
            }),
        ];
        for (name, mutate) in cases {
            let mut config = Config::empty();
            config.add_profile("garage", profile()).unwrap();
            config.add_camera("porch", camera(PORCH)).unwrap();
            config.plugs.insert("a".into(), plug(SOCKET));
            config.plugs.insert("b".into(), plug(LAMP));
            mutate(&mut config.plugs);
            assert!(
                matches!(config.validate(), Err(ConfigError::InvalidPlugs(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn several_plugs_may_power_the_same_printer_and_camera() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        for (name, id) in [("a", SOCKET), ("b", LAMP)] {
            let mut shared = plug(id);
            shared.printers = vec!["garage".into()];
            shared.cameras = vec![PORCH.into()];
            config.plugs.insert(name.into(), shared);
        }
        config.save(dir.path()).unwrap();
        assert_eq!(Config::open(dir.path()).unwrap().plugs().len(), 2);
    }

    #[test]
    fn version_2_files_open_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), SERDE_YAML_V2).unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        assert!(config.plugs().is_empty());
        config.save(dir.path()).unwrap();
        let written = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        // The serializer's quoting style differs from serde_yaml's, so compare
        // the schema, not the bytes.
        assert!(written.starts_with("version: 2\n"), "{written}");
        assert!(!written.contains("plugs"), "{written}");
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

    fn plug_fixture(dir: &Path) -> Config {
        let mut config = Config::open(dir).unwrap();
        config.add_profile("old", profile()).unwrap();
        config.add_profile("other", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        config.add_plug("socket", plug(SOCKET)).unwrap();
        config
            .link_plug("socket", &PlugTarget::Printer("old".into()))
            .unwrap();
        config
            .link_plug("socket", &PlugTarget::Printer("other".into()))
            .unwrap();
        config
            .link_plug("socket", &PlugTarget::Camera("porch".into()))
            .unwrap();
        config
    }

    #[test]
    fn printer_rename_rewrites_plug_links() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = plug_fixture(dir.path());
        config.rename_profile("old", "new", profile()).unwrap();
        config.save(dir.path()).unwrap();
        let reopened = Config::open(dir.path()).unwrap();
        assert_eq!(
            reopened.get_plug("socket").unwrap().printers,
            ["new", "other"]
        );
    }

    #[test]
    fn printer_removal_drops_it_from_plugs_and_keeps_plugs() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = plug_fixture(dir.path());
        config.remove_profile("old").unwrap();
        config.save(dir.path()).unwrap();
        let reopened = Config::open(dir.path()).unwrap();
        let socket = reopened.get_plug("socket").unwrap();
        assert_eq!(socket.printers, ["other"]);
        assert_eq!(socket.cameras, [PORCH]);
    }

    #[test]
    fn camera_rename_keeps_plug_links_and_removal_drops_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = plug_fixture(dir.path());
        let renamed = config.get_camera("porch").unwrap().clone();
        config.replace_camera("porch", "deck", renamed).unwrap();
        assert_eq!(config.get_plug("socket").unwrap().cameras, [PORCH]);
        assert_eq!(config.camera_name(PORCH), Some("deck"));
        config.remove_camera("deck").unwrap();
        config.save(dir.path()).unwrap();
        let reopened = Config::open(dir.path()).unwrap();
        assert!(reopened.get_plug("socket").unwrap().cameras.is_empty());
        assert_eq!(reopened.camera_name(PORCH), None);
    }

    #[test]
    fn plug_links_are_idempotent_and_mixed() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = plug_fixture(dir.path());
        for _ in 0..2 {
            config
                .link_plug("socket", &PlugTarget::Printer("old".into()))
                .unwrap();
            config
                .link_plug("socket", &PlugTarget::Camera("porch".into()))
                .unwrap();
        }
        let socket = config.get_plug("socket").unwrap();
        assert_eq!(socket.printers, ["old", "other"]);
        assert_eq!(socket.cameras, [PORCH]);
        assert_eq!(
            config.plug_by_id(SOCKET).map(|(name, _)| name),
            Some("socket")
        );
        config
            .unlink_plug("socket", &PlugTarget::Printer("old".into()))
            .unwrap();
        config
            .unlink_plug("socket", &PlugTarget::Camera("porch".into()))
            .unwrap();
        let socket = config.get_plug("socket").unwrap();
        assert_eq!(socket.printers, ["other"]);
        assert!(socket.cameras.is_empty());
    }

    #[test]
    fn unlinking_an_absent_target_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = plug_fixture(dir.path());
        config
            .unlink_plug("socket", &PlugTarget::Printer("old".into()))
            .unwrap();
        config
            .unlink_plug("socket", &PlugTarget::Camera("porch".into()))
            .unwrap();
        for target in [
            PlugTarget::Printer("old".into()),
            PlugTarget::Camera("porch".into()),
        ] {
            assert!(matches!(
                config.unlink_plug("socket", &target),
                Err(ConfigError::PlugNotLinked)
            ));
        }
    }

    #[test]
    fn linking_unknown_targets_fails_without_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = plug_fixture(dir.path());
        let before = config.get_plug("socket").unwrap().clone();
        assert!(matches!(
            config.link_plug("socket", &PlugTarget::Printer("nope".into())),
            Err(ConfigError::ProfileNotFound)
        ));
        assert!(matches!(
            config.link_plug("socket", &PlugTarget::Camera("nope".into())),
            Err(ConfigError::CameraNotFound)
        ));
        assert!(matches!(
            config.link_plug("nope", &PlugTarget::Printer("old".into())),
            Err(ConfigError::PlugNotFound)
        ));
        assert_eq!(config.get_plug("socket"), Some(&before));
    }

    #[test]
    fn plug_crud_rejects_duplicates_and_missing_names() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = plug_fixture(dir.path());
        assert!(matches!(
            config.add_plug("socket", plug(LAMP)),
            Err(ConfigError::PlugAlreadyExists)
        ));
        config.add_plug("lamp", plug(LAMP)).unwrap();
        assert!(matches!(
            config.replace_plug("socket", "lamp", plug(SOCKET)),
            Err(ConfigError::PlugAlreadyExists)
        ));
        assert!(matches!(
            config.replace_plug("nope", "x", plug(SOCKET)),
            Err(ConfigError::PlugNotFound)
        ));
        let mut kept = config.get_plug("socket").unwrap().clone();
        kept.host = "192.0.2.31".into();
        config
            .replace_plug("socket", "bench", kept.clone())
            .unwrap();
        assert_eq!(config.get_plug("socket"), None);
        assert_eq!(config.get_plug("bench"), Some(&kept));
        assert_eq!(config.remove_plug("bench").unwrap(), kept);
        assert!(matches!(
            config.remove_plug("bench"),
            Err(ConfigError::PlugNotFound)
        ));
        assert!(config.get_profile("old").is_some());
    }

    #[test]
    fn renamed_then_removed_printer_leaves_a_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = plug_fixture(dir.path());
        config.save(dir.path()).unwrap();
        config.rename_profile("old", "new", profile()).unwrap();
        config.remove_profile("new").unwrap();
        config.save(dir.path()).unwrap();
        let reopened = Config::open(dir.path()).unwrap();
        assert_eq!(reopened.get_plug("socket").unwrap().printers, ["other"]);
    }
}
