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
    pub source: FirmwareUpdateSource,
    pub components: Vec<FirmwareUpdateComponent>,
    pub issues: Vec<FirmwareUpdateIssue>,
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
        Self {
            availability,
            source,
            components,
            issues,
        }
    }

    pub fn unsupported(
        source: FirmwareUpdateSource,
        code: &'static str,
        message: &'static str,
    ) -> Self {
        Self {
            availability: FirmwareUpdateAvailability::Unsupported,
            source,
            components: Vec::new(),
            issues: vec![FirmwareUpdateIssue { code, message }],
        }
    }
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
}
