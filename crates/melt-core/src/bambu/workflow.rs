//! Print preflight composition and stable stages shared by CLI and desktop.

use std::path::Path;

use serde::Serialize;

use crate::moonraker::AmsData;

use super::{
    FilamentMapping, JobNames, MappingStatus, PackageError, PackageIssue, PackageIssueSeverity,
    PlateManifest, PrintPackage, derive_job_names, inspect_print_package, reconcile_filaments,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PrintStage {
    Inspect,
    ResolveCapabilities,
    ResolveDestination,
    ReconcileMaterials,
    CheckStorage,
    Upload,
    Verify,
    SendCommand,
    AwaitAcceptance,
    AwaitPrintStart,
    Finished,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintStageEvent {
    pub stage: PrintStage,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_transferred: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintPreflight {
    pub package: PrintPackage,
    pub selected_plate: PlateManifest,
    pub names: JobNames,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filament_mapping: Option<FilamentMapping>,
    pub issues: Vec<PackageIssue>,
    pub ready: bool,
}

pub fn preflight_print_package(
    source: &Path,
    display_name: Option<&str>,
    plate_index: Option<u32>,
    inventory: Option<&AmsData>,
) -> Result<PrintPreflight, PackageError> {
    let package = inspect_print_package(source)?;
    let mut issues = package.issues.clone();
    let selected_plate = plate_index
        .and_then(|index| package.plate(index))
        .or_else(|| {
            plate_index
                .is_none()
                .then(|| {
                    package
                        .plates
                        .iter()
                        .find(|plate| !plate.gcode_path.is_empty())
                })
                .flatten()
        })
        .cloned()
        .unwrap_or_else(|| {
            issues.push(PackageIssue {
                severity: PackageIssueSeverity::Error,
                code: "plate_not_found",
                message: plate_index.map_or_else(
                    || "the package has no printable plate".into(),
                    |index| format!("plate {index} is not present or is not printable"),
                ),
            });
            PlateManifest {
                index: plate_index.unwrap_or(1),
                ..PlateManifest::default()
            }
        });
    let names = derive_job_names(source, display_name, selected_plate.name.as_deref());
    let filament_mapping = inventory.map(|inventory| {
        let mapping = reconcile_filaments(&selected_plate, inventory);
        if mapping.status != MappingStatus::Exact {
            issues.push(PackageIssue {
                severity: PackageIssueSeverity::Error,
                code: "filament_mapping_unresolved",
                message: "the sliced filaments could not be mapped uniquely to loaded trays".into(),
            });
        }
        mapping
    });
    if !selected_plate.filaments.is_empty() && inventory.is_none() {
        issues.push(PackageIssue {
            severity: PackageIssueSeverity::Warning,
            code: "filament_inventory_unavailable",
            message: "filament requirements were found but no live AMS inventory was supplied"
                .into(),
        });
    }
    let ready = package.sliced
        && !selected_plate.gcode_path.is_empty()
        && !issues
            .iter()
            .any(|issue| issue.severity == PackageIssueSeverity::Error);
    Ok(PrintPreflight {
        package,
        selected_plate,
        names,
        filament_mapping,
        issues,
        ready,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use crate::moonraker::{AmsTray, AmsUnit};

    use super::*;

    fn package() -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        {
            let mut writer = ZipWriter::new(file.as_file_mut());
            writer
                .start_file("Metadata/plate_2.gcode", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"G28").unwrap();
            writer
                .start_file("Metadata/plate_2.json", SimpleFileOptions::default())
                .unwrap();
            writer
                .write_all(br##"{"plate_name":"Production","filament":[{"id":0,"tray_info_idx":"GFA00","color":"FFFFFFFF"}]}"##)
                .unwrap();
            writer.finish().unwrap();
        }
        file
    }

    #[test]
    fn composes_package_names_plate_and_exact_mapping() {
        let source = package();
        let inventory = AmsData {
            units: vec![AmsUnit {
                trays: vec![AmsTray {
                    tray_index: Some(3),
                    tray_info_id: Some("GFA00".into()),
                    color: Some("FFFFFFFF".into()),
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let preflight = preflight_print_package(
            source.path(),
            Some("Customer part"),
            Some(2),
            Some(&inventory),
        )
        .unwrap();
        assert!(preflight.ready);
        assert_eq!(preflight.names.display_name, "Customer part");
        assert_eq!(preflight.selected_plate.name.as_deref(), Some("Production"));
        assert_eq!(preflight.filament_mapping.unwrap().ams_mapping, vec![3]);
    }

    #[test]
    fn missing_plate_is_a_blocking_preflight_issue() {
        let source = package();
        let preflight = preflight_print_package(source.path(), None, Some(1), None).unwrap();
        assert!(!preflight.ready);
        assert!(
            preflight
                .issues
                .iter()
                .any(|issue| issue.code == "plate_not_found")
        );
    }
}
