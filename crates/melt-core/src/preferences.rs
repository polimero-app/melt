use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use thiserror::Error;

const PREFERENCES_FILE: &str = "melt-preferences.yaml";
const CURRENT_VERSION: u8 = 1;

#[derive(Debug, Error)]
pub enum PreferencesError {
    #[error("unsupported preferences schema version: got {got}")]
    UnsupportedVersion { got: u8 },
    #[error("malformed preferences file: {0}")]
    Malformed(#[from] serde_yaml::Error),
    #[error("preferences file changed since it was read")]
    ConcurrentUpdate,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("slicer name is required")]
    SlicerNameMissing,
    #[error("slicer path is required")]
    SlicerPathMissing,
    #[error("slicer already exists")]
    SlicerAlreadyExists,
    #[error("slicer not found")]
    SlicerNotFound,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPreferences {
    pub completion: bool,
    pub failure: bool,
    pub disconnection: bool,
}

impl Default for NotificationPreferences {
    fn default() -> Self {
        Self {
            completion: true,
            failure: true,
            disconnection: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Slicer {
    pub name: String,
    pub path: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesFile {
    #[serde(default)]
    version: u8,
    #[serde(default)]
    notifications: NotificationPreferences,
    #[serde(default)]
    slicers: Vec<Slicer>,
    #[serde(default)]
    library_path: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub notifications: NotificationPreferences,
    pub slicers: Vec<Slicer>,
    /// The folder the file library last browsed, so it reopens where the
    /// user left off; `None` until they've either browsed or chosen one.
    pub library_path: Option<String>,
    #[serde(skip)]
    digest: Option<[u8; 32]>,
}

impl Preferences {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, PreferencesError> {
        let bytes = match fs::read(dir.as_ref().join(PREFERENCES_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(PreferencesError::Io(error)),
        };
        if bytes.is_empty() {
            return Ok(Self {
                digest: Some(digest(&bytes)),
                ..Self::default()
            });
        }

        let file: PreferencesFile = serde_yaml::from_slice(&bytes)?;
        if file.version != CURRENT_VERSION {
            return Err(PreferencesError::UnsupportedVersion { got: file.version });
        }
        Ok(Self {
            notifications: file.notifications,
            slicers: file.slicers,
            library_path: file.library_path,
            digest: Some(digest(&bytes)),
        })
    }

    pub fn load() -> Result<Self, PreferencesError> {
        Self::open(preferences_dir()?)
    }

    pub fn set_slicer(&mut self, mut slicer: Slicer) -> Result<(), PreferencesError> {
        slicer.name = slicer.name.trim().to_owned();
        slicer.path = slicer.path.trim().to_owned();
        if slicer.name.is_empty() {
            return Err(PreferencesError::SlicerNameMissing);
        }
        if slicer.path.is_empty() {
            return Err(PreferencesError::SlicerPathMissing);
        }
        if let Some(existing) = self
            .slicers
            .iter_mut()
            .find(|item| item.name.eq_ignore_ascii_case(&slicer.name))
        {
            *existing = slicer;
        } else {
            self.slicers.push(slicer);
            self.slicers.sort_by_key(|item| item.name.to_lowercase());
        }
        Ok(())
    }

    pub fn remove_slicer(&mut self, name: &str) -> Result<(), PreferencesError> {
        let before = self.slicers.len();
        self.slicers
            .retain(|slicer| !slicer.name.eq_ignore_ascii_case(name.trim()));
        if self.slicers.len() == before {
            return Err(PreferencesError::SlicerNotFound);
        }
        Ok(())
    }

    pub fn save(&mut self, dir: impl AsRef<Path>) -> Result<(), PreferencesError> {
        let dir = dir.as_ref();
        let path = dir.join(PREFERENCES_FILE);
        let current = match fs::read(&path) {
            Ok(bytes) => Some(digest(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(PreferencesError::Io(error)),
        };
        if current != self.digest {
            return Err(PreferencesError::ConcurrentUpdate);
        }

        fs::create_dir_all(dir)?;
        restrict_dir_permissions(dir)?;
        let bytes = serde_yaml::to_string(&PreferencesFile {
            version: CURRENT_VERSION,
            notifications: self.notifications.clone(),
            slicers: self.slicers.clone(),
            library_path: self.library_path.clone(),
        })?
        .into_bytes();
        let mut temp = NamedTempFile::new_in(dir)?;
        restrict_file_permissions(temp.as_file())?;
        std::io::Write::write_all(&mut temp, &bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(&path)
            .map_err(|error| PreferencesError::Io(error.error))?;
        self.digest = Some(digest(&bytes));
        sync_directory(dir);
        Ok(())
    }
}

pub fn preferences_dir() -> Result<PathBuf, PreferencesError> {
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
        PreferencesError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "locating preferences directory",
        ))
    })
}

pub fn detect_slicers() -> Vec<Slicer> {
    #[cfg(target_os = "macos")]
    return detect_macos_slicers();
    #[cfg(target_os = "windows")]
    return detect_windows_slicers();
    #[cfg(all(unix, not(target_os = "macos")))]
    detect_path_slicers()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn detect_path_slicers() -> Vec<Slicer> {
    let candidates = [
        ("Orca Slicer", &["orca-slicer", "orcaslicer"][..]),
        ("PrusaSlicer", &["prusa-slicer", "prusaslicer"][..]),
        ("Bambu Studio", &["bambu-studio", "bambustudio"][..]),
        ("UltiMaker Cura", &["cura"][..]),
    ];
    candidates
        .iter()
        .filter_map(|(name, commands)| {
            resolve_command(commands).map(|path| Slicer {
                name: (*name).to_owned(),
                path,
                enabled: true,
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn detect_macos_slicers() -> Vec<Slicer> {
    let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    [
        ("Orca Slicer", "OrcaSlicer.app"),
        ("PrusaSlicer", "PrusaSlicer.app"),
        ("Bambu Studio", "BambuStudio.app"),
        ("UltiMaker Cura", "UltiMaker Cura.app"),
    ]
    .into_iter()
    .filter_map(|(name, bundle)| {
        first_existing(&[
            PathBuf::from("/Applications").join(bundle),
            home.join("Applications").join(bundle),
        ])
        .map(|path| Slicer {
            name: name.to_owned(),
            path: path.to_string_lossy().into_owned(),
            enabled: true,
        })
    })
    .collect()
}

#[cfg(target_os = "windows")]
fn detect_windows_slicers() -> Vec<Slicer> {
    let program_files = env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    let local_app_data = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_default();
    [
        (
            "Orca Slicer",
            vec![
                program_files.join("OrcaSlicer/OrcaSlicer.exe"),
                local_app_data.join("Programs/OrcaSlicer/OrcaSlicer.exe"),
            ],
        ),
        (
            "PrusaSlicer",
            vec![program_files.join("Prusa3D/PrusaSlicer/prusa-slicer.exe")],
        ),
        (
            "Bambu Studio",
            vec![
                program_files.join("Bambu Studio/bambu-studio.exe"),
                local_app_data.join("Programs/Bambu Studio/bambu-studio.exe"),
            ],
        ),
        (
            "UltiMaker Cura",
            vec![program_files.join("UltiMaker Cura/UltiMaker-Cura.exe")],
        ),
    ]
    .into_iter()
    .filter_map(|(name, paths)| {
        first_existing(&paths).map(|path| Slicer {
            name: name.to_owned(),
            path: path.to_string_lossy().into_owned(),
            enabled: true,
        })
    })
    .collect()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|path| path.exists()).cloned()
}

pub fn merge_slicers(saved: &[Slicer], detected: &[Slicer]) -> Vec<Slicer> {
    let mut merged = saved.to_vec();
    for slicer in detected {
        if !merged
            .iter()
            .any(|saved| saved.name.eq_ignore_ascii_case(&slicer.name))
        {
            merged.push(slicer.clone());
        }
    }
    merged
}

fn resolve_command(commands: &[&str]) -> Option<String> {
    let path = env::var_os("PATH")?;
    for directory in env::split_paths(&path) {
        for command in commands {
            let candidate = directory.join(command);
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[cfg(unix)]
fn restrict_dir_permissions(dir: &Path) -> Result<(), PreferencesError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_dir_permissions(_: &Path) -> Result<(), PreferencesError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(file: &std::fs::File) -> Result<(), PreferencesError> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_file_permissions(_: &std::fs::File) -> Result<(), PreferencesError> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(dir: &Path) {
    if let Ok(file) = std::fs::File::open(dir) {
        let _ = file.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_directory(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn defaults_notifications_on_and_round_trip() {
        let dir = tempdir().unwrap();
        let mut preferences = Preferences::open(dir.path()).unwrap();
        assert_eq!(
            preferences.notifications,
            NotificationPreferences::default()
        );
        preferences.notifications.failure = false;
        preferences.save(dir.path()).unwrap();
        assert!(!Preferences::open(dir.path()).unwrap().notifications.failure);
    }

    #[test]
    fn slicers_are_upserted_and_removed_case_insensitively() {
        let mut preferences = Preferences::default();
        preferences
            .set_slicer(Slicer {
                name: "Bambu Studio".into(),
                path: "/opt/bambu".into(),
                enabled: true,
            })
            .unwrap();
        preferences
            .set_slicer(Slicer {
                name: "bambu studio".into(),
                path: "/opt/new".into(),
                enabled: false,
            })
            .unwrap();
        assert_eq!(preferences.slicers.len(), 1);
        assert!(!preferences.slicers[0].enabled);
        preferences.remove_slicer("BAMBU STUDIO").unwrap();
        assert!(preferences.slicers.is_empty());
    }

    #[test]
    fn concurrent_writes_are_rejected() {
        let dir = tempdir().unwrap();
        let mut first = Preferences::open(dir.path()).unwrap();
        first.save(dir.path()).unwrap();
        let mut second = Preferences::open(dir.path()).unwrap();
        first.notifications.failure = false;
        first.save(dir.path()).unwrap();
        second.notifications.failure = false;
        assert!(matches!(
            second.save(dir.path()),
            Err(PreferencesError::ConcurrentUpdate)
        ));
    }
}
