//! Deterministic reconciliation of sliced-project filaments with live AMS trays.

use std::collections::BTreeSet;

use serde::Serialize;

use crate::moonraker::{AmsData, AmsTray};

use super::PlateManifest;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MappingStatus {
    Exact,
    Ambiguous,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilamentAssignment {
    pub requirement_index: u32,
    pub status: MappingStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tray_index: Option<i32>,
    pub candidates: Vec<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilamentMapping {
    pub status: MappingStatus,
    pub assignments: Vec<FilamentAssignment>,
    pub ams_mapping: Vec<i32>,
    pub nozzle_mapping: Vec<i32>,
}

/// Produces an explainable mapping and refuses to allocate one tray twice.
pub fn reconcile_filaments(plate: &PlateManifest, inventory: &AmsData) -> FilamentMapping {
    let trays = inventory
        .units
        .iter()
        .flat_map(|unit| unit.trays.iter())
        .filter_map(|tray| tray.tray_index.map(|index| (index, tray)))
        .collect::<Vec<_>>();
    let mut claimed = BTreeSet::new();
    let mut assignments = Vec::with_capacity(plate.filaments.len());

    for requirement in &plate.filaments {
        let candidates = trays
            .iter()
            .filter(|(index, tray)| !claimed.contains(index) && tray_matches(requirement, tray))
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        let (status, tray_index) = match candidates.as_slice() {
            [index] => {
                claimed.insert(*index);
                (MappingStatus::Exact, Some(*index))
            }
            [] => (MappingStatus::Unavailable, None),
            _ => (MappingStatus::Ambiguous, None),
        };
        assignments.push(FilamentAssignment {
            requirement_index: requirement.index,
            status,
            tray_index,
            candidates,
        });
    }
    let status = if assignments
        .iter()
        .any(|assignment| assignment.status == MappingStatus::Unavailable)
    {
        MappingStatus::Unavailable
    } else if assignments
        .iter()
        .any(|assignment| assignment.status == MappingStatus::Ambiguous)
    {
        MappingStatus::Ambiguous
    } else {
        MappingStatus::Exact
    };
    let ams_mapping = assignments
        .iter()
        .map(|assignment| assignment.tray_index.unwrap_or(-1))
        .collect();
    let nozzle_mapping = plate
        .filaments
        .iter()
        .filter_map(|requirement| requirement.nozzle_index)
        .map(|index| i32::try_from(index).unwrap_or(i32::MAX))
        .collect::<Vec<_>>();
    let nozzle_mapping = if nozzle_mapping.len() == plate.filaments.len() {
        nozzle_mapping
    } else {
        Vec::new()
    };
    FilamentMapping {
        status,
        assignments,
        ams_mapping,
        nozzle_mapping,
    }
}

fn tray_matches(requirement: &super::FilamentRequirement, tray: &AmsTray) -> bool {
    if let Some(required) = normalized(requirement.tray_info_id.as_deref()) {
        let identity_matches = [tray.tray_info_id.as_deref(), tray.setting_id.as_deref()]
            .into_iter()
            .filter_map(normalized)
            .any(|candidate| candidate == required);
        if !identity_matches {
            return false;
        }
    } else if let Some(required) = normalized(requirement.filament_type.as_deref())
        && normalized(tray.filament_type.as_deref()).as_deref() != Some(required.as_str())
    {
        return false;
    }
    match normalized_color(requirement.color.as_deref()) {
        Some(required) => normalized_color(tray.color.as_deref()).as_deref() == Some(&required),
        None => true,
    }
}

fn normalized(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_uppercase)
}

fn normalized_color(value: Option<&str>) -> Option<String> {
    normalized(value).map(|value| value.trim_start_matches('#').to_owned())
}

#[cfg(test)]
mod tests {
    use crate::{
        bambu::{FilamentRequirement, PlateManifest},
        moonraker::{AmsData, AmsTray, AmsUnit},
    };

    use super::*;

    fn tray(index: i32, identity: &str, color: &str) -> AmsTray {
        AmsTray {
            tray_index: Some(index),
            tray_info_id: Some(identity.into()),
            color: Some(color.into()),
            ..Default::default()
        }
    }

    fn plate(requirements: &[(&str, &str)]) -> PlateManifest {
        PlateManifest {
            filaments: requirements
                .iter()
                .enumerate()
                .map(|(index, (identity, color))| FilamentRequirement {
                    index: index as u32,
                    tray_info_id: Some((*identity).into()),
                    color: Some((*color).into()),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn matches_same_material_by_color_without_reusing_trays() {
        let inventory = AmsData {
            units: vec![AmsUnit {
                trays: vec![tray(0, "GFA00", "FF0000FF"), tray(1, "GFA00", "00FF00FF")],
                ..Default::default()
            }],
        };
        let mapping = reconcile_filaments(
            &plate(&[("GFA00", "#00FF00FF"), ("GFA00", "FF0000FF")]),
            &inventory,
        );
        assert_eq!(mapping.status, MappingStatus::Exact);
        assert_eq!(mapping.ams_mapping, vec![1, 0]);
    }

    #[test]
    fn reports_ambiguous_and_missing_requirements() {
        let inventory = AmsData {
            units: vec![AmsUnit {
                trays: vec![tray(0, "GFA00", "FFFFFFFF"), tray(1, "GFA00", "FFFFFFFF")],
                ..Default::default()
            }],
        };
        let ambiguous = reconcile_filaments(&plate(&[("GFA00", "FFFFFFFF")]), &inventory);
        assert_eq!(ambiguous.status, MappingStatus::Ambiguous);
        assert_eq!(ambiguous.assignments[0].candidates, vec![0, 1]);

        let missing = reconcile_filaments(&plate(&[("GFB00", "000000FF")]), &inventory);
        assert_eq!(missing.status, MappingStatus::Unavailable);
        assert_eq!(missing.ams_mapping, vec![-1]);
    }

    #[test]
    fn preserves_complete_nozzle_assignments_only() {
        let mut selected = plate(&[("GFA00", "FFFFFFFF"), ("GFB00", "000000FF")]);
        selected.filaments[0].nozzle_index = Some(1);
        selected.filaments[1].nozzle_index = Some(0);
        let inventory = AmsData {
            units: vec![AmsUnit {
                trays: vec![tray(0, "GFA00", "FFFFFFFF"), tray(1, "GFB00", "000000FF")],
                ..Default::default()
            }],
        };
        assert_eq!(
            reconcile_filaments(&selected, &inventory).nozzle_mapping,
            vec![1, 0]
        );
        selected.filaments[1].nozzle_index = None;
        assert!(
            reconcile_filaments(&selected, &inventory)
                .nozzle_mapping
                .is_empty()
        );
    }
}
