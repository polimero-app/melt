//! Driver-neutral, read-only printer update availability.

use serde::Serialize;

pub const MAX_UPDATE_COMPONENTS: usize = 64;
pub const MAX_UPDATE_ISSUES: usize = 16;
pub const MAX_COMPONENT_ID_BYTES: usize = 64;
pub const MAX_COMPONENT_LABEL_BYTES: usize = 128;
pub const MAX_VERSION_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirmwareUpdateAvailability {
    Available,
    Current,
    Unknown,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirmwareUpdateComponentKind {
    PrinterFirmware,
    AccessoryFirmware,
    PrinterSoftware,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirmwareUpdateSource {
    BambuMqtt,
    MoonrakerUpdateManager,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirmwareEvidenceSource {
    BambuLanInventory,
    BambuLanAdvertisement,
    BambuLanHistory,
    BambuPublicCatalogue,
    MoonrakerUpdateManager,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirmwareEvidenceRole {
    Installed,
    PrinterAdvertised,
    DeviceCatalogue,
    PublicStable,
    UpstreamCurrent,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareVersionEvidence {
    pub source: FirmwareEvidenceSource,
    pub role: FirmwareEvidenceRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FirmwareUpdateAssessment {
    Required,
    ConfirmedAvailable,
    DeviceCatalogueNewer,
    PublicReleaseNewer,
    Current,
    Conflict,
    Unknown,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareUpdateComponent {
    pub id: String,
    pub label: String,
    pub kind: FirmwareUpdateComponentKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_version: Option<String>,
    pub availability: FirmwareUpdateAvailability,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<FirmwareVersionEvidence>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareUpdateIssue {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareUpdateReport {
    pub availability: FirmwareUpdateAvailability,
    pub assessment: FirmwareUpdateAssessment,
    pub source: FirmwareUpdateSource,
    pub components: Vec<FirmwareUpdateComponent>,
    pub issues: Vec<FirmwareUpdateIssue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_sources: Vec<FirmwareEvidenceSource>,
}

impl FirmwareUpdateReport {
    pub fn from_components(
        source: FirmwareUpdateSource,
        mut components: Vec<FirmwareUpdateComponent>,
        mut issues: Vec<FirmwareUpdateIssue>,
    ) -> Self {
        components.sort_by(|left, right| {
            left.kind.cmp(&right.kind).then_with(|| {
                left.id
                    .to_ascii_lowercase()
                    .cmp(&right.id.to_ascii_lowercase())
            })
        });
        let mut truncated = false;
        if components.len() > MAX_UPDATE_COMPONENTS {
            components.truncate(MAX_UPDATE_COMPONENTS);
            truncated = true;
        }
        if issues.len() > MAX_UPDATE_ISSUES {
            issues.truncate(MAX_UPDATE_ISSUES);
            truncated = true;
        }
        if truncated && issues.len() < MAX_UPDATE_ISSUES {
            issues.push(provider_data_truncated());
        }
        let availability = if components
            .iter()
            .any(|component| component.availability == FirmwareUpdateAvailability::Available)
        {
            FirmwareUpdateAvailability::Available
        } else if !components.is_empty()
            && components
                .iter()
                .all(|component| component.availability == FirmwareUpdateAvailability::Current)
        {
            FirmwareUpdateAvailability::Current
        } else {
            FirmwareUpdateAvailability::Unknown
        };
        let assessment = assessment_for(&components, availability, &issues);
        let mut evidence_sources = components
            .iter()
            .flat_map(|component| component.evidence.iter().map(|evidence| evidence.source))
            .collect::<Vec<_>>();
        evidence_sources.sort_unstable();
        evidence_sources.dedup();
        Self {
            availability,
            assessment,
            source,
            components,
            issues,
            evidence_sources,
        }
    }

    pub fn unsupported(
        source: FirmwareUpdateSource,
        code: &'static str,
        message: &'static str,
    ) -> Self {
        Self {
            availability: FirmwareUpdateAvailability::Unsupported,
            assessment: FirmwareUpdateAssessment::Unsupported,
            source,
            components: Vec::new(),
            issues: vec![FirmwareUpdateIssue { code, message }],
            evidence_sources: Vec::new(),
        }
    }
}

fn assessment_for(
    components: &[FirmwareUpdateComponent],
    availability: FirmwareUpdateAvailability,
    issues: &[FirmwareUpdateIssue],
) -> FirmwareUpdateAssessment {
    if components.iter().any(|component| component.required) {
        return FirmwareUpdateAssessment::Required;
    }
    if availability == FirmwareUpdateAvailability::Available {
        return FirmwareUpdateAssessment::ConfirmedAvailable;
    }
    if issues.iter().any(|issue| {
        matches!(
            issue.code,
            "targetNotNewer"
                | "historyTargetNotNewer"
                | "uncomparableVersion"
                | "uncomparableHistoryVersion"
                | "uncomparablePublicVersion"
                | "publicTargetNotNewer"
        )
    }) || components.iter().any(conflicting_evidence)
    {
        return FirmwareUpdateAssessment::Conflict;
    }
    if components.iter().any(|component| {
        evidence_newer(component, FirmwareEvidenceRole::DeviceCatalogue)
    }) {
        return FirmwareUpdateAssessment::DeviceCatalogueNewer;
    }
    if components.iter().any(|component| {
        evidence_newer(component, FirmwareEvidenceRole::PublicStable)
    }) {
        return FirmwareUpdateAssessment::PublicReleaseNewer;
    }
    match availability {
        FirmwareUpdateAvailability::Available => FirmwareUpdateAssessment::ConfirmedAvailable,
        FirmwareUpdateAvailability::Current => FirmwareUpdateAssessment::Current,
        FirmwareUpdateAvailability::Unsupported => FirmwareUpdateAssessment::Unsupported,
        FirmwareUpdateAvailability::Unknown => FirmwareUpdateAssessment::Unknown,
    }
}

fn numeric_version(value: &str) -> Option<Vec<u64>> {
    let numbers = value
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .next()
        .unwrap_or_default();
    let parsed = numbers
        .split('.')
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<Vec<u64>, _>>()
        .ok()?;
    (!parsed.is_empty()).then_some(parsed)
}

fn evidence_newer(component: &FirmwareUpdateComponent, role: FirmwareEvidenceRole) -> bool {
    let Some(current) = component.current_version.as_deref().and_then(numeric_version) else {
        return false;
    };
    component.evidence.iter().any(|evidence| {
        evidence.role == role
            && evidence.version.as_deref().and_then(numeric_version).is_some_and(|target| {
                compare_numeric_versions(&target, &current) == std::cmp::Ordering::Greater
            })
    })
}

fn compare_numeric_versions(left: &[u64], right: &[u64]) -> std::cmp::Ordering {
    let length = left.len().max(right.len());
    (0..length)
        .map(|index| {
            left.get(index)
                .copied()
                .unwrap_or(0)
                .cmp(&right.get(index).copied().unwrap_or(0))
        })
        .find(|ordering| *ordering != std::cmp::Ordering::Equal)
        .unwrap_or(std::cmp::Ordering::Equal)
}

fn conflicting_evidence(component: &FirmwareUpdateComponent) -> bool {
    let versions = component
        .evidence
        .iter()
        .filter(|evidence| evidence.role != FirmwareEvidenceRole::Installed)
        .filter_map(|evidence| evidence.version.as_deref())
        .collect::<std::collections::BTreeSet<_>>();
    versions.len() > 1 && component.availability != FirmwareUpdateAvailability::Available
}

pub(crate) fn bounded_provider_text(value: &str, maximum_bytes: usize) -> (String, bool) {
    let value = value.trim();
    if value.len() <= maximum_bytes {
        return (value.to_owned(), false);
    }
    let mut end = maximum_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_owned(), true)
}

pub(crate) fn provider_data_truncated() -> FirmwareUpdateIssue {
    FirmwareUpdateIssue {
        code: "providerDataTruncated",
        message: "The printer returned more update data than Melt can safely retain.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(
        id: &str,
        kind: FirmwareUpdateComponentKind,
        availability: FirmwareUpdateAvailability,
    ) -> FirmwareUpdateComponent {
        FirmwareUpdateComponent {
            id: id.into(),
            label: id.into(),
            kind,
            current_version: None,
            available_version: None,
            availability,
            required: false,
            evidence: Vec::new(),
        }
    }

    #[test]
    fn available_component_wins_and_components_sort_stably() {
        let report = FirmwareUpdateReport::from_components(
            FirmwareUpdateSource::BambuMqtt,
            vec![
                component(
                    "z-accessory",
                    FirmwareUpdateComponentKind::AccessoryFirmware,
                    FirmwareUpdateAvailability::Unknown,
                ),
                component(
                    "ota",
                    FirmwareUpdateComponentKind::PrinterFirmware,
                    FirmwareUpdateAvailability::Available,
                ),
                component(
                    "a-accessory",
                    FirmwareUpdateComponentKind::AccessoryFirmware,
                    FirmwareUpdateAvailability::Current,
                ),
            ],
            Vec::new(),
        );

        assert_eq!(report.availability, FirmwareUpdateAvailability::Available);
        assert_eq!(
            report
                .components
                .iter()
                .map(|component| component.id.as_str())
                .collect::<Vec<_>>(),
            ["ota", "a-accessory", "z-accessory"]
        );
    }

    #[test]
    fn current_requires_at_least_one_authoritative_current_component() {
        let empty = FirmwareUpdateReport::from_components(
            FirmwareUpdateSource::MoonrakerUpdateManager,
            Vec::new(),
            Vec::new(),
        );
        assert_eq!(empty.availability, FirmwareUpdateAvailability::Unknown);

        let current = FirmwareUpdateReport::from_components(
            FirmwareUpdateSource::MoonrakerUpdateManager,
            vec![component(
                "klipper",
                FirmwareUpdateComponentKind::PrinterSoftware,
                FirmwareUpdateAvailability::Current,
            )],
            Vec::new(),
        );
        assert_eq!(current.availability, FirmwareUpdateAvailability::Current);
    }

    #[test]
    fn provider_text_bounds_preserve_utf8_boundaries() {
        let (bounded, truncated) = bounded_provider_text("abcdé", 5);
        assert_eq!(bounded, "abcd");
        assert!(truncated);
    }

    #[test]
    fn json_contract_uses_camel_case_enum_values_and_fields() {
        let report = FirmwareUpdateReport::unsupported(
            FirmwareUpdateSource::MoonrakerUpdateManager,
            "missing",
            "Missing",
        );
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["availability"], "unsupported");
        assert_eq!(value["source"], "moonrakerUpdateManager");
        assert_eq!(value["issues"][0]["code"], "missing");
    }

    #[test]
    fn report_collects_evidence_sources_and_assessment() {
        let mut component = component(
            "ota",
            FirmwareUpdateComponentKind::PrinterFirmware,
            FirmwareUpdateAvailability::Available,
        );
        component.current_version = Some("01.08.00.00".into());
        component.available_version = Some("01.09.00.00".into());
        component.evidence.push(FirmwareVersionEvidence {
            source: FirmwareEvidenceSource::BambuPublicCatalogue,
            role: FirmwareEvidenceRole::PublicStable,
            version: component.available_version.clone(),
            required: false,
        });
        let report = FirmwareUpdateReport::from_components(
            FirmwareUpdateSource::BambuMqtt,
            vec![component],
            Vec::new(),
        );
        assert_eq!(report.assessment, FirmwareUpdateAssessment::PublicReleaseNewer);
        assert_eq!(report.evidence_sources, [FirmwareEvidenceSource::BambuPublicCatalogue]);
    }
}
