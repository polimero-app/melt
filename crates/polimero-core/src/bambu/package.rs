//! Safe, read-only inspection of sliced Bambu-compatible 3MF print packages.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;
use zip::ZipArchive;

const MAX_PACKAGE_SIZE: u64 = 1 << 30;
const MAX_ENTRIES: usize = 10_000;
const MAX_EXPANDED_SIZE: u64 = 4 << 30;
const MAX_METADATA_SIZE: u64 = 8 << 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PackageIssueSeverity {
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageIssue {
    pub severity: PackageIssueSeverity,
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilamentRequirement {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tray_info_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filament_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_grams: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlateManifest {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub gcode_path: String,
    pub thumbnail_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_grams: Option<f64>,
    pub filaments: Vec<FilamentRequirement>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintPackage {
    pub source: PathBuf,
    pub source_file_name: String,
    pub size_bytes: u64,
    pub sliced: bool,
    pub plates: Vec<PlateManifest>,
    pub metadata_entries: BTreeMap<String, u64>,
    pub issues: Vec<PackageIssue>,
}

impl PrintPackage {
    #[must_use]
    pub fn is_printable(&self) -> bool {
        self.sliced
            && !self
                .issues
                .iter()
                .any(|issue| issue.severity == PackageIssueSeverity::Error)
    }

    #[must_use]
    pub fn plate(&self, index: u32) -> Option<&PlateManifest> {
        self.plates.iter().find(|plate| plate.index == index)
    }
}

#[derive(Debug, Error)]
pub enum PackageError {
    #[error("print package is not a regular file")]
    NotAFile,
    #[error("print package exceeds the 1 GiB inspection limit")]
    TooLarge,
    #[error("print package could not be opened")]
    Io,
    #[error("print package is not a valid 3MF archive")]
    InvalidArchive,
    #[error("print package contains an unsafe archive path")]
    UnsafePath,
    #[error("print package contains duplicate archive paths")]
    DuplicatePath,
    #[error("print package exceeds archive safety limits")]
    ArchiveLimit,
    #[error("print package metadata is invalid")]
    InvalidMetadata,
}

/// Inspects a Bambu/Orca-style 3MF without extracting files to disk.
pub fn inspect_print_package(path: &Path) -> Result<PrintPackage, PackageError> {
    let metadata = path.metadata().map_err(|_| PackageError::Io)?;
    if !metadata.is_file() {
        return Err(PackageError::NotAFile);
    }
    if metadata.len() > MAX_PACKAGE_SIZE {
        return Err(PackageError::TooLarge);
    }
    let source_file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or(PackageError::NotAFile)?
        .to_owned();
    let file = File::open(path).map_err(|_| PackageError::Io)?;
    let mut archive = ZipArchive::new(file).map_err(|_| PackageError::InvalidArchive)?;
    if archive.len() > MAX_ENTRIES {
        return Err(PackageError::ArchiveLimit);
    }

    let mut paths = BTreeSet::new();
    let mut metadata_entries = BTreeMap::new();
    let mut expanded = 0_u64;
    let mut plate_paths: BTreeMap<u32, PlateManifest> = BTreeMap::new();
    let mut plate_json_paths = Vec::new();

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|_| PackageError::InvalidArchive)?;
        let enclosed = entry.enclosed_name().ok_or(PackageError::UnsafePath)?;
        let name = enclosed.to_string_lossy().replace('\\', "/");
        if !paths.insert(name.clone()) {
            return Err(PackageError::DuplicatePath);
        }
        expanded = expanded
            .checked_add(entry.size())
            .filter(|size| *size <= MAX_EXPANDED_SIZE)
            .ok_or(PackageError::ArchiveLimit)?;
        if name.starts_with("Metadata/") {
            metadata_entries.insert(name.clone(), entry.size());
        }
        if let Some((plate, suffix)) = plate_entry(&name) {
            let manifest = plate_paths.entry(plate).or_insert_with(|| PlateManifest {
                index: plate,
                ..PlateManifest::default()
            });
            match suffix {
                "gcode" => manifest.gcode_path = name,
                "json" => plate_json_paths.push((plate, name)),
                "png" => manifest.thumbnail_paths.push(name),
                _ => {}
            }
        }
    }

    for (plate, path) in plate_json_paths {
        let value = read_json_entry(&mut archive, &path)?;
        if let Some(manifest) = plate_paths.get_mut(&plate) {
            apply_plate_metadata(manifest, &value);
        }
    }

    let mut plates = plate_paths
        .into_values()
        .filter(|plate| !plate.gcode_path.is_empty() || !plate.thumbnail_paths.is_empty())
        .collect::<Vec<_>>();
    plates.sort_by_key(|plate| plate.index);
    let sliced = plates.iter().any(|plate| !plate.gcode_path.is_empty());
    let mut issues = Vec::new();
    if !sliced {
        issues.push(PackageIssue {
            severity: PackageIssueSeverity::Error,
            code: "package_not_sliced",
            message: "the 3MF contains no Metadata/plate_N.gcode entry".into(),
        });
    }
    for plate in &plates {
        if plate.gcode_path.is_empty() {
            issues.push(PackageIssue {
                severity: PackageIssueSeverity::Warning,
                code: "plate_not_sliced",
                message: format!("plate {} contains no embedded G-code", plate.index),
            });
        }
    }

    Ok(PrintPackage {
        source: path.to_owned(),
        source_file_name,
        size_bytes: metadata.len(),
        sliced,
        plates,
        metadata_entries,
        issues,
    })
}

fn plate_entry(name: &str) -> Option<(u32, &str)> {
    let rest = name.strip_prefix("Metadata/plate_")?;
    let (number, suffix) = rest.split_once('.')?;
    let plate = number.parse().ok()?;
    matches!(suffix, "gcode" | "json" | "png").then_some((plate, suffix))
}

fn read_json_entry(archive: &mut ZipArchive<File>, path: &str) -> Result<Value, PackageError> {
    let entry = archive
        .by_name(path)
        .map_err(|_| PackageError::InvalidMetadata)?;
    if entry.size() > MAX_METADATA_SIZE {
        return Err(PackageError::ArchiveLimit);
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry
        .take(MAX_METADATA_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PackageError::Io)?;
    serde_json::from_slice(&bytes).map_err(|_| PackageError::InvalidMetadata)
}

fn apply_plate_metadata(plate: &mut PlateManifest, value: &Value) {
    plate.name = first_string(value, &["plate_name", "name"]);
    plate.estimated_seconds = first_number(value, &["prediction", "estimated_time"])
        .and_then(|value| (value >= 0.0).then_some(value as u64));
    plate.estimated_grams = first_number(value, &["weight", "total_weight"])
        .filter(|value| value.is_finite() && *value >= 0.0);
    let Some(filaments) = value
        .get("filament")
        .or_else(|| value.get("filaments"))
        .and_then(Value::as_array)
    else {
        return;
    };
    plate.filaments = filaments
        .iter()
        .enumerate()
        .map(|(position, filament)| FilamentRequirement {
            index: filament
                .get("id")
                .and_then(value_u32)
                .unwrap_or(position as u32),
            tray_info_id: first_string(filament, &["tray_info_idx", "setting_id"]),
            filament_type: first_string(filament, &["type", "filament_type"]),
            color: first_string(filament, &["color", "filament_color"]),
            used_grams: first_number(filament, &["used_g", "weight"]),
        })
        .collect();
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(key)?.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn first_number(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| {
        let value = value.get(key)?;
        value
            .as_f64()
            .or_else(|| value.as_str()?.trim().parse().ok())
            .filter(|number| number.is_finite())
    })
}

fn value_u32(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|value| value.try_into().ok())
        .or_else(|| value.as_str()?.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    fn package(entries: &[(&str, &[u8])]) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        {
            let mut writer = ZipWriter::new(file.as_file_mut());
            for (name, contents) in entries {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(contents).unwrap();
            }
            writer.finish().unwrap();
        }
        file
    }

    #[test]
    fn inspects_sliced_multiplate_package() {
        let file = package(&[
            ("Metadata/plate_1.gcode", b"G28"),
            ("Metadata/plate_1.png", b"png"),
            (
                "Metadata/plate_1.json",
                br##"{"plate_name":"Bracket","prediction":"120","weight":3.5,"filament":[{"id":1,"tray_info_idx":"GFA00","type":"PLA","color":"#FF0000","used_g":3.5}]}"##,
            ),
            ("Metadata/plate_2.gcode", b"G28"),
        ]);
        let result = inspect_print_package(file.path()).unwrap();
        assert!(result.is_printable());
        assert_eq!(result.plates.len(), 2);
        let plate = result.plate(1).unwrap();
        assert_eq!(plate.name.as_deref(), Some("Bracket"));
        assert_eq!(plate.estimated_seconds, Some(120));
        assert_eq!(plate.filaments[0].tray_info_id.as_deref(), Some("GFA00"));
    }

    #[test]
    fn reports_unsliced_package_without_treating_it_as_an_archive_error() {
        let file = package(&[("3D/3dmodel.model", b"model")]);
        let result = inspect_print_package(file.path()).unwrap();
        assert!(!result.is_printable());
        assert_eq!(result.issues[0].code, "package_not_sliced");
    }

    #[test]
    fn rejects_invalid_plate_metadata() {
        let file = package(&[
            ("Metadata/plate_1.gcode", b"G28"),
            ("Metadata/plate_1.json", b"not json"),
        ]);
        assert!(matches!(
            inspect_print_package(file.path()),
            Err(PackageError::InvalidMetadata)
        ));
    }
}
