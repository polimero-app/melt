use std::{
    collections::BTreeMap,
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

const CONFIG_FILE: &str = "polimero.yaml";
const CURRENT_VERSION: u8 = 1;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unsupported config schema version: got {got}, want {CURRENT_VERSION}")]
    UnsupportedVersion { got: u8 },
    #[error("malformed config file: {0}")]
    Malformed(#[from] serde_yaml::Error),
    #[error("config file changed since it was read")]
    ConcurrentUpdate,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("profile already exists")]
    ProfileAlreadyExists,
    #[error("profile not found")]
    ProfileNotFound,
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    version: u8,
    #[serde(default)]
    profiles: BTreeMap<String, Profile>,
}

#[derive(Debug)]
pub struct Config {
    profiles: BTreeMap<String, Profile>,
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
                profiles: BTreeMap::new(),
                digest: Some(digest(&bytes)),
            });
        }

        let file: ConfigFile = serde_yaml::from_slice(&bytes).map_err(ConfigError::Malformed)?;
        if file.version != CURRENT_VERSION {
            return Err(ConfigError::UnsupportedVersion { got: file.version });
        }
        Ok(Self {
            profiles: file.profiles,
            digest: Some(digest(&bytes)),
        })
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

    pub fn remove_profile(&mut self, name: &str) -> Result<Profile, ConfigError> {
        self.profiles
            .remove(name)
            .ok_or(ConfigError::ProfileNotFound)
    }

    pub fn save(&mut self, dir: impl AsRef<Path>) -> Result<(), ConfigError> {
        let dir = dir.as_ref();
        self.check_unchanged(dir)?;
        fs::create_dir_all(dir)?;
        restrict_dir_permissions(dir)?;

        let bytes = serde_yaml::to_string(&ConfigFile {
            version: CURRENT_VERSION,
            profiles: self.profiles.clone(),
        })
        .map_err(ConfigError::Malformed)?
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
    if let Some(dir) = env::var_os("POLIMERO_CONFIG_DIR").filter(|value| !value.is_empty()) {
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

    base.map(|path| path.join("polimero")).ok_or_else(|| {
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
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".polimero-"))
                .count(),
            0
        );
    }
}
