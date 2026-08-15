use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use super::{CanonicalModel, FirmwareInventory, FirmwareVersion, ModelIdentity};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Qualification {
    Provisional,
    Qualified,
    Confirmed,
    Contradicted,
    Expired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum QuirkEffect {
    PreferMjpegCamera,
    PreferRtspsCamera,
    RepairRtpTimestampsFromArrivalTime,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuirkEntry {
    pub id: &'static str,
    pub model: CanonicalModel,
    pub module: Option<&'static str>,
    pub firmware_min: Option<&'static str>,
    pub firmware_max: Option<&'static str>,
    pub effect: QuirkEffect,
    pub qualification: Qualification,
    pub evidence: &'static str,
    pub owner: &'static str,
    pub removal_condition: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryIssue {
    pub id: &'static str,
    pub message: String,
}

const REGISTRY: &[QuirkEntry] = &[
    QuirkEntry {
        id: "p1s-camera-port-6000-community-report",
        model: CanonicalModel::P1S,
        module: None,
        firmware_min: None,
        firmware_max: None,
        effect: QuirkEffect::PreferMjpegCamera,
        qualification: Qualification::Provisional,
        evidence: "Bambuddy source at 8d3abf4; conflicts with Open Bambu Networking and dynamic Bambu Studio fields",
        owner: "bambu-camera",
        removal_condition: "replace with exact module-firmware physical captures",
    },
    QuirkEntry {
        id: "p1s-rtsps-community-report",
        model: CanonicalModel::P1S,
        module: None,
        firmware_min: None,
        firmware_max: None,
        effect: QuirkEffect::PreferRtspsCamera,
        qualification: Qualification::Provisional,
        evidence: "Open Bambu Networking at 7f8ba03; conflicts with Bambuddy",
        owner: "bambu-camera",
        removal_condition: "replace with exact module-firmware physical captures",
    },
    QuirkEntry {
        id: "p2s-nonadvancing-rtp-timestamps",
        model: CanonicalModel::P2S,
        module: None,
        firmware_min: None,
        firmware_max: None,
        effect: QuirkEffect::RepairRtpTimestampsFromArrivalTime,
        qualification: Qualification::Provisional,
        evidence: "Bambuddy FFmpeg wall-clock workaround at 8d3abf4",
        owner: "bambu-camera",
        removal_condition: "qualify affected camera module firmware and timestamp captures",
    },
];

pub fn registry() -> &'static [QuirkEntry] {
    REGISTRY
}

pub fn matching(identity: &ModelIdentity) -> Vec<&'static QuirkEntry> {
    REGISTRY
        .iter()
        .filter(|entry| entry.model == identity.canonical)
        .collect()
}

pub fn applicable(
    identity: &ModelIdentity,
    firmware: &FirmwareInventory,
) -> Vec<&'static QuirkEntry> {
    applicable_entries(REGISTRY, identity, firmware)
}

fn applicable_entries<'a>(
    entries: &'a [QuirkEntry],
    identity: &ModelIdentity,
    firmware: &FirmwareInventory,
) -> Vec<&'a QuirkEntry> {
    entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.qualification,
                Qualification::Qualified | Qualification::Confirmed
            ) && entry.model == identity.canonical
                && firmware_matches(entry, firmware)
        })
        .collect()
}

fn firmware_matches(entry: &QuirkEntry, firmware: &FirmwareInventory) -> bool {
    let Some(module_name) = entry.module else {
        return entry.firmware_min.is_none() && entry.firmware_max.is_none();
    };
    let Some(observed) = firmware.software(module_name) else {
        return false;
    };
    entry.firmware_min.is_none_or(|minimum| {
        observed.numeric_cmp(&FirmwareVersion::parse(minimum)) != Ordering::Less
    }) && entry.firmware_max.is_none_or(|maximum| {
        observed.numeric_cmp(&FirmwareVersion::parse(maximum)) != Ordering::Greater
    })
}

pub fn validate_registry() -> Vec<RegistryIssue> {
    validate_entries(REGISTRY)
}

fn validate_entries(entries: &[QuirkEntry]) -> Vec<RegistryIssue> {
    let mut issues = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        for (field, value) in [
            ("id", entry.id),
            ("evidence", entry.evidence),
            ("owner", entry.owner),
            ("removal condition", entry.removal_condition),
        ] {
            if value.trim().is_empty() {
                issues.push(issue(entry.id, format!("{field} must not be empty")));
            }
        }
        if entries[..index]
            .iter()
            .any(|candidate| candidate.id == entry.id)
        {
            issues.push(issue(entry.id, "registry IDs must be unique"));
        }
        if (entry.firmware_min.is_some() || entry.firmware_max.is_some()) && entry.module.is_none()
        {
            issues.push(issue(
                entry.id,
                "firmware bounds require an exact module name",
            ));
        }
        for (label, bound) in [
            ("minimum", entry.firmware_min),
            ("maximum", entry.firmware_max),
        ] {
            if bound.is_some_and(|value| FirmwareVersion::parse(value).numeric.is_empty()) {
                issues.push(issue(entry.id, format!("{label} firmware is not numeric")));
            }
        }
        if let (Some(minimum), Some(maximum)) = (entry.firmware_min, entry.firmware_max)
            && FirmwareVersion::parse(minimum).numeric_cmp(&FirmwareVersion::parse(maximum))
                == Ordering::Greater
        {
            issues.push(issue(entry.id, "minimum firmware exceeds maximum firmware"));
        }
    }
    for (index, left) in entries.iter().enumerate() {
        for right in &entries[index + 1..] {
            if active(left)
                && active(right)
                && left.model == right.model
                && module_scopes_overlap(left, right)
                && version_ranges_overlap(left, right)
                && effects_conflict(left.effect, right.effect)
            {
                issues.push(issue(
                    right.id,
                    format!("overlaps conflicting active quirk {:?}", left.id),
                ));
            }
        }
    }
    issues
}

fn issue(id: &'static str, message: impl Into<String>) -> RegistryIssue {
    RegistryIssue {
        id,
        message: message.into(),
    }
}

fn active(entry: &QuirkEntry) -> bool {
    matches!(
        entry.qualification,
        Qualification::Qualified | Qualification::Confirmed
    )
}

fn module_scopes_overlap(left: &QuirkEntry, right: &QuirkEntry) -> bool {
    left.module.is_none() || right.module.is_none() || left.module == right.module
}

fn version_ranges_overlap(left: &QuirkEntry, right: &QuirkEntry) -> bool {
    let left_min = left.firmware_min.map(FirmwareVersion::parse);
    let left_max = left.firmware_max.map(FirmwareVersion::parse);
    let right_min = right.firmware_min.map(FirmwareVersion::parse);
    let right_max = right.firmware_max.map(FirmwareVersion::parse);
    !matches!((&left_max, &right_min), (Some(maximum), Some(minimum)) if maximum.numeric_cmp(minimum) == Ordering::Less)
        && !matches!((&right_max, &left_min), (Some(maximum), Some(minimum)) if maximum.numeric_cmp(minimum) == Ordering::Less)
}

fn effects_conflict(left: QuirkEffect, right: QuirkEffect) -> bool {
    matches!(
        (left, right),
        (
            QuirkEffect::PreferMjpegCamera,
            QuirkEffect::PreferRtspsCamera
        ) | (
            QuirkEffect::PreferRtspsCamera,
            QuirkEffect::PreferMjpegCamera
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bambu::FirmwareModule;

    const QUALIFIED: QuirkEntry = QuirkEntry {
        id: "qualified-range",
        model: CanonicalModel::P2S,
        module: Some("ota"),
        firmware_min: Some("01.02.00.00"),
        firmware_max: Some("01.03.00.00"),
        effect: QuirkEffect::RepairRtpTimestampsFromArrivalTime,
        qualification: Qualification::Qualified,
        evidence: "fixtures/bambu/p2s/rtp.json",
        owner: "bambu-camera",
        removal_condition: "upstream timestamps advance",
    };

    fn inventory(version: &str) -> FirmwareInventory {
        FirmwareInventory {
            modules: vec![FirmwareModule {
                name: "ota".into(),
                software: FirmwareVersion::parse(version),
                ..FirmwareModule::default()
            }],
        }
    }

    #[test]
    fn provisional_conflicts_never_change_runtime_behavior() {
        let identity = ModelIdentity::parse("P1S");
        assert!(applicable(&identity, &FirmwareInventory::default()).is_empty());
        assert_eq!(
            registry()
                .iter()
                .filter(|entry| entry.model == CanonicalModel::P1S)
                .count(),
            2
        );
    }

    #[test]
    fn qualified_ranges_are_inclusive_and_module_scoped() {
        let identity = ModelIdentity::parse("P2S");
        assert_eq!(
            applicable_entries(&[QUALIFIED], &identity, &inventory("01.02.00.00")).len(),
            1
        );
        assert_eq!(
            applicable_entries(&[QUALIFIED], &identity, &inventory("01.03.00.00-beta")).len(),
            1
        );
        assert!(applicable_entries(&[QUALIFIED], &identity, &inventory("01.03.00.01")).is_empty());
        assert!(
            applicable_entries(&[QUALIFIED], &identity, &FirmwareInventory::default()).is_empty()
        );
    }

    #[test]
    fn checked_in_registry_is_valid() {
        assert_eq!(validate_registry(), []);
    }

    #[test]
    fn validation_rejects_invalid_and_conflicting_entries() {
        let mut invalid = QUALIFIED;
        invalid.firmware_min = Some("2.0");
        invalid.firmware_max = Some("1.0");
        let mut conflict = QUALIFIED;
        conflict.id = "conflict";
        conflict.effect = QuirkEffect::PreferMjpegCamera;
        let mut competing = conflict;
        competing.id = "competing";
        competing.effect = QuirkEffect::PreferRtspsCamera;
        assert!(
            validate_entries(&[invalid])
                .iter()
                .any(|issue| issue.message.contains("exceeds"))
        );
        assert!(
            validate_entries(&[conflict, competing])
                .iter()
                .any(|issue| issue.message.contains("overlaps"))
        );
    }
}
