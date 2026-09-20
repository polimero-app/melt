use std::{cmp::Ordering, collections::BTreeMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::firmware_updates::{
    FirmwareEvidenceRole, FirmwareEvidenceSource, FirmwareUpdateAvailability,
    FirmwareUpdateComponent, FirmwareUpdateComponentKind, FirmwareUpdateIssue, FirmwareUpdateReport,
    FirmwareUpdateSource, FirmwareVersionEvidence, MAX_COMPONENT_ID_BYTES,
    MAX_COMPONENT_LABEL_BYTES, MAX_VERSION_BYTES, bounded_provider_text, provider_data_truncated,
};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareVersion {
    pub raw: String,
    pub numeric: Vec<u64>,
    pub suffix: Option<String>,
}

impl FirmwareVersion {
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let trimmed = raw.trim();
        let split = trimmed.find(|character: char| character != '.' && !character.is_ascii_digit());
        let (numbers, suffix) = split.map_or((trimmed, None), |index| {
            (&trimmed[..index], Some(trimmed[index..].to_owned()))
        });
        let numeric = numbers
            .split('.')
            .filter(|part| !part.is_empty())
            .map(str::parse)
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        Self {
            raw,
            numeric,
            suffix,
        }
    }

    pub fn numeric_cmp(&self, other: &Self) -> Ordering {
        let length = self.numeric.len().max(other.numeric.len());
        (0..length)
            .map(|index| {
                self.numeric
                    .get(index)
                    .copied()
                    .unwrap_or(0)
                    .cmp(&other.numeric.get(index).copied().unwrap_or(0))
            })
            .find(|ordering| *ordering != Ordering::Equal)
            .unwrap_or(Ordering::Equal)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareModule {
    pub name: String,
    pub software: FirmwareVersion,
    pub hardware: Option<String>,
    pub serial: Option<String>,
    pub project: Option<String>,
    pub unknown: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareInventory {
    pub modules: Vec<FirmwareModule>,
}

impl FirmwareInventory {
    pub fn from_version_info(info: &Value) -> Self {
        let modules = info
            .get("module")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .filter_map(|module| {
                let name = module.get("name")?.as_str()?.to_owned();
                let software = FirmwareVersion::parse(
                    module
                        .get("sw_ver")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                let known = ["name", "sw_ver", "hw_ver", "sn", "project_name"];
                let unknown = module
                    .iter()
                    .filter(|(key, _)| !known.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                Some(FirmwareModule {
                    name,
                    software,
                    hardware: module
                        .get("hw_ver")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    serial: module.get("sn").and_then(Value::as_str).map(str::to_owned),
                    project: module
                        .get("project_name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    unknown,
                })
            })
            .collect();
        Self { modules }
    }

    pub fn software(&self, name: &str) -> Option<&FirmwareVersion> {
        self.modules
            .iter()
            .find(|module| module.name == name)
            .map(|module| &module.software)
    }
}

/// Builds a conservative update report from an accumulated Bambu status
/// snapshot and the separately queried per-module inventory. An absent target
/// is deliberately unknown: it is not proof that the printer refreshed its
/// cloud-backed firmware catalogue.
pub fn update_report(status: &Value, inventory: &FirmwareInventory) -> FirmwareUpdateReport {
    let mut components = BTreeMap::new();
    let mut issues = Vec::new();
    let mut truncated = false;

    for module in &inventory.modules {
        let (id, id_truncated) = bounded_provider_text(&module.name, MAX_COMPONENT_ID_BYTES);
        if id.is_empty() {
            continue;
        }
        let label_source = module.project.as_deref().unwrap_or(&module.name);
        let (label, label_truncated) =
            bounded_provider_text(label_source, MAX_COMPONENT_LABEL_BYTES);
        let (version, version_truncated) =
            bounded_provider_text(&module.software.raw, MAX_VERSION_BYTES);
        truncated |= id_truncated || label_truncated || version_truncated;
        components.insert(
            id.to_ascii_lowercase(),
            FirmwareUpdateComponent {
                kind: component_kind(&id),
                id,
                label,
                current_version: (!version.is_empty()).then_some(version.clone()),
                available_version: None,
                availability: FirmwareUpdateAvailability::Unknown,
                required: false,
                evidence: (!version.is_empty()).then(|| vec![FirmwareVersionEvidence {
                    source: FirmwareEvidenceSource::BambuLanInventory,
                    role: FirmwareEvidenceRole::Installed,
                    version: Some(version.clone()),
                    required: false,
                }]).unwrap_or_default(),
            },
        );
    }

    let Some(print) = status.get("print").and_then(Value::as_object) else {
        return finish_report(components, issues, truncated);
    };
    let force_upgrade = print
        .get("force_upgrade")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(upgrade) = print.get("upgrade_state").and_then(Value::as_object) else {
        if force_upgrade {
            merge_target(
                &mut components,
                "ota",
                "Printer firmware",
                None,
                true,
                &mut issues,
                &mut truncated,
            );
        }
        return finish_report(components, issues, truncated);
    };
    let required = force_upgrade
        || upgrade
            .get("force_upgrade")
            .and_then(Value::as_bool)
            .unwrap_or(false);

    if let Some(entries) = upgrade.get("new_ver_list").and_then(Value::as_array) {
        for entry in entries.iter().filter_map(Value::as_object) {
            let target = ["sw_new_ver", "new_version", "version"]
                .into_iter()
                .find_map(|key| entry.get(key).and_then(Value::as_str))
                .filter(|value| meaningful_version(value));
            let Some(target) = target else {
                continue;
            };
            let id = ["name", "module", "dev_model_name"]
                .into_iter()
                .find_map(|key| entry.get(key).and_then(Value::as_str))
                .filter(|value| !value.trim().is_empty());
            let Some(id) = id else {
                push_issue(
                    &mut issues,
                    FirmwareUpdateIssue {
                        code: "updateTargetMissingModule",
                        message: "The printer advertised an update without a module identifier.",
                    },
                );
                continue;
            };
            let label = entry
                .get("product_name")
                .and_then(Value::as_str)
                .unwrap_or(id);
            merge_target(
                &mut components,
                id,
                label,
                Some(target),
                required,
                &mut issues,
                &mut truncated,
            );
        }
    }

    for (field, id, label) in [
        ("ota_new_version_number", "ota", "Printer firmware"),
        ("ams_new_version_number", "ams", "AMS firmware"),
        ("ahb_new_version_number", "ahb", "AHB firmware"),
        ("ext_new_version_number", "ext", "Extension firmware"),
    ] {
        let target = upgrade
            .get(field)
            .and_then(Value::as_str)
            .filter(|value| meaningful_version(value));
        if let Some(target) = target {
            merge_target(
                &mut components,
                id,
                label,
                Some(target),
                required,
                &mut issues,
                &mut truncated,
            );
        }
    }

    if required
        && !components
            .values()
            .any(|component| component.available_version.is_some())
    {
        merge_target(
            &mut components,
            "ota",
            "Printer firmware",
            None,
            true,
            &mut issues,
            &mut truncated,
        );
    }

    finish_report(components, issues, truncated)
}

/// Adds the read-only `upgrade.get_history` catalogue to an existing LAN
/// report. History is treated as a device catalogue, not as proof that the
/// printer has accepted or staged an update.
pub fn merge_history_report(mut report: FirmwareUpdateReport, history: &Value) -> FirmwareUpdateReport {
    let mut components = report
        .components
        .drain(..)
        .map(|component| (component.id.to_ascii_lowercase(), component))
        .collect::<BTreeMap<_, _>>();
    let mut issues = report.issues;
    let mut truncated = false;

    let Some(upgrade) = history.get("upgrade").and_then(Value::as_object) else {
        return FirmwareUpdateReport::from_components(report.source, components.into_values().collect(), issues);
    };
    let firmware_optional = upgrade
        .get("firmware_optional")
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for entry in firmware_optional {
        let Some(firmware) = entry.get("firmware").and_then(Value::as_object) else {
            continue;
        };
        let Some(version) = firmware.get("version").and_then(Value::as_str) else {
            continue;
        };
        merge_catalogue_target(
            &mut components,
            "ota",
            "Printer firmware",
            version,
            firmware.get("force_update").and_then(Value::as_bool).unwrap_or(false),
            &mut issues,
            &mut truncated,
        );
        for ams in entry.get("ams").and_then(Value::as_array).into_iter().flatten() {
            let Some(ams_object) = ams.as_object() else {
                continue;
            };
            let id = ["device_id", "dev_model_name", "address"]
                .into_iter()
                .find_map(|key| ams_object.get(key).and_then(Value::as_str))
                .filter(|value| !value.trim().is_empty());
            let Some(id) = id else { continue };
            let label = ams_object
                .get("dev_model_name")
                .and_then(Value::as_str)
                .unwrap_or(id);
            for firmware in ams_object
                .get("firmware")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(version) = firmware.get("version").and_then(Value::as_str) else {
                    continue;
                };
                merge_catalogue_target(
                    &mut components,
                    id,
                    label,
                    version,
                    firmware.get("force_update").and_then(Value::as_bool).unwrap_or(false),
                    &mut issues,
                    &mut truncated,
                );
            }
        }
    }
    if truncated {
        push_issue(&mut issues, provider_data_truncated());
    }
    FirmwareUpdateReport::from_components(report.source, components.into_values().collect(), issues)
}

/// Adds a public stable release candidate without replacing a printer-owned
/// advertisement. Public catalogue evidence is informational when the two
/// sources disagree.
pub fn merge_public_catalogue_report(
    mut report: FirmwareUpdateReport,
    version: &str,
) -> FirmwareUpdateReport {
    let mut components = report
        .components
        .drain(..)
        .map(|component| (component.id.to_ascii_lowercase(), component))
        .collect::<BTreeMap<_, _>>();
    let mut issues = report.issues;
    let mut truncated = false;
    merge_public_target(
        &mut components,
        "ota",
        "Printer firmware",
        version,
        &mut issues,
        &mut truncated,
    );
    if truncated {
        push_issue(&mut issues, provider_data_truncated());
    }
    FirmwareUpdateReport::from_components(report.source, components.into_values().collect(), issues)
}

fn merge_public_target(
    components: &mut BTreeMap<String, FirmwareUpdateComponent>,
    raw_id: &str,
    raw_label: &str,
    raw_target: &str,
    issues: &mut Vec<FirmwareUpdateIssue>,
    truncated: &mut bool,
) {
    let (id, id_truncated) = bounded_provider_text(raw_id, MAX_COMPONENT_ID_BYTES);
    let (label, label_truncated) = bounded_provider_text(raw_label, MAX_COMPONENT_LABEL_BYTES);
    let (target, version_truncated) = bounded_provider_text(raw_target, MAX_VERSION_BYTES);
    *truncated |= id_truncated || label_truncated || version_truncated;
    if target.is_empty() || !meaningful_version(&target) {
        return;
    }
    let key = id.to_ascii_lowercase();
    let component = components.entry(key).or_insert_with(|| FirmwareUpdateComponent {
        kind: component_kind(&id),
        id,
        label,
        current_version: None,
        available_version: None,
        availability: FirmwareUpdateAvailability::Unknown,
        required: false,
        evidence: Vec::new(),
    });
    component.evidence.push(FirmwareVersionEvidence {
        source: FirmwareEvidenceSource::BambuPublicCatalogue,
        role: FirmwareEvidenceRole::PublicStable,
        version: Some(target.clone()),
        required: false,
    });
    let Some(current) = component.current_version.as_deref() else {
        if component.available_version.is_none() {
            component.available_version = Some(target);
            component.availability = FirmwareUpdateAvailability::Available;
        }
        return;
    };
    let current = FirmwareVersion::parse(current);
    let target_version = FirmwareVersion::parse(&target);
    if current.numeric.is_empty() || target_version.numeric.is_empty() {
        push_issue(
            issues,
            FirmwareUpdateIssue {
                code: "uncomparablePublicVersion",
                message: "A public catalogue version could not be compared safely.",
            },
        );
        return;
    }
    match target_version.numeric_cmp(&current) {
        Ordering::Greater => {
            if component.available_version.is_none() {
                component.available_version = Some(target);
                component.availability = FirmwareUpdateAvailability::Available;
            }
        }
        Ordering::Equal if component.availability != FirmwareUpdateAvailability::Available => {
            component.availability = FirmwareUpdateAvailability::Current;
        }
        Ordering::Less => push_issue(
            issues,
            FirmwareUpdateIssue {
                code: "publicTargetNotNewer",
                message: "The public catalogue reported a version older than the installed version.",
            },
        ),
        Ordering::Equal => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn merge_catalogue_target(
    components: &mut BTreeMap<String, FirmwareUpdateComponent>,
    raw_id: &str,
    raw_label: &str,
    raw_target: &str,
    required: bool,
    issues: &mut Vec<FirmwareUpdateIssue>,
    truncated: &mut bool,
) {
    let (id, id_truncated) = bounded_provider_text(raw_id, MAX_COMPONENT_ID_BYTES);
    let (label, label_truncated) = bounded_provider_text(raw_label, MAX_COMPONENT_LABEL_BYTES);
    let (target, version_truncated) = bounded_provider_text(raw_target, MAX_VERSION_BYTES);
    *truncated |= id_truncated || label_truncated || version_truncated;
    if target.is_empty() || !meaningful_version(&target) {
        return;
    }
    let key = id.to_ascii_lowercase();
    let component = components.entry(key).or_insert_with(|| FirmwareUpdateComponent {
        kind: component_kind(&id),
        id,
        label,
        current_version: None,
        available_version: None,
        availability: FirmwareUpdateAvailability::Unknown,
        required: false,
        evidence: Vec::new(),
    });
    component.required |= required;
    component.evidence.push(FirmwareVersionEvidence {
        source: FirmwareEvidenceSource::BambuLanHistory,
        role: FirmwareEvidenceRole::DeviceCatalogue,
        version: Some(target.clone()),
        required,
    });
    let comparison = component.current_version.as_deref().map(|current| {
        let current = FirmwareVersion::parse(current);
        let target_version = FirmwareVersion::parse(&target);
        if current.numeric.is_empty() || target_version.numeric.is_empty() {
            None
        } else {
            Some(target_version.numeric_cmp(&current))
        }
    });
    match (component.current_version.is_none(), comparison.flatten()) {
        (true, _) => {
            component.available_version = Some(target);
            component.availability = FirmwareUpdateAvailability::Available;
        }
        (false, Some(Ordering::Greater)) => {
            component.available_version = Some(target);
            component.availability = FirmwareUpdateAvailability::Available;
        }
        (false, Some(Ordering::Equal)) => {
            if component.availability != FirmwareUpdateAvailability::Available {
                component.availability = FirmwareUpdateAvailability::Current;
            }
        }
        (false, Some(Ordering::Less)) => push_issue(
            issues,
            FirmwareUpdateIssue {
                code: "historyTargetNotNewer",
                message: "The device catalogue reported a version older than the installed version.",
            },
        ),
        (false, None) => push_issue(
            issues,
            FirmwareUpdateIssue {
                code: "uncomparableHistoryVersion",
                message: "A device catalogue version could not be compared safely.",
            },
        ),
    }
    if component.required {
        component.availability = FirmwareUpdateAvailability::Available;
    }
}

fn component_kind(id: &str) -> FirmwareUpdateComponentKind {
    if id.eq_ignore_ascii_case("ota") {
        FirmwareUpdateComponentKind::PrinterFirmware
    } else {
        FirmwareUpdateComponentKind::AccessoryFirmware
    }
}

fn meaningful_version(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed != "0"
        && trimmed
            .split('.')
            .any(|part| part.parse::<u64>().is_ok_and(|number| number != 0))
}

#[allow(clippy::too_many_arguments)]
fn merge_target(
    components: &mut BTreeMap<String, FirmwareUpdateComponent>,
    raw_id: &str,
    raw_label: &str,
    raw_target: Option<&str>,
    required: bool,
    issues: &mut Vec<FirmwareUpdateIssue>,
    truncated: &mut bool,
) {
    let (id, id_truncated) = bounded_provider_text(raw_id, MAX_COMPONENT_ID_BYTES);
    let (label, label_truncated) = bounded_provider_text(raw_label, MAX_COMPONENT_LABEL_BYTES);
    let (target, version_truncated) = raw_target.map_or((None, false), |target| {
        let (target, truncated) = bounded_provider_text(target, MAX_VERSION_BYTES);
        (Some(target), truncated)
    });
    *truncated |= id_truncated || label_truncated || version_truncated;
    let key = id.to_ascii_lowercase();
    let component = components
        .entry(key)
        .or_insert_with(|| FirmwareUpdateComponent {
            kind: component_kind(&id),
            id,
            label,
            current_version: None,
            available_version: None,
            availability: FirmwareUpdateAvailability::Unknown,
            required: false,
            evidence: Vec::new(),
        });
    component.required |= required;
    if let Some(target) = target {
        component.available_version = Some(target.clone());
        component.evidence.push(FirmwareVersionEvidence {
            source: FirmwareEvidenceSource::BambuLanAdvertisement,
            role: FirmwareEvidenceRole::PrinterAdvertised,
            version: Some(target.clone()),
            required,
        });
        component.availability = match component.current_version.as_deref() {
            None => FirmwareUpdateAvailability::Available,
            Some(current) => {
                let current = FirmwareVersion::parse(current);
                let target = FirmwareVersion::parse(target);
                if current.numeric.is_empty() || target.numeric.is_empty() {
                    push_issue(
                        issues,
                        FirmwareUpdateIssue {
                            code: "uncomparableVersion",
                            message: "An advertised update version could not be compared safely.",
                        },
                    );
                    FirmwareUpdateAvailability::Unknown
                } else {
                    match target.numeric_cmp(&current) {
                        Ordering::Greater => FirmwareUpdateAvailability::Available,
                        Ordering::Equal => FirmwareUpdateAvailability::Unknown,
                        Ordering::Less => {
                            push_issue(
                                issues,
                                FirmwareUpdateIssue {
                                    code: "targetNotNewer",
                                    message: "The advertised target is older than the installed version.",
                                },
                            );
                            FirmwareUpdateAvailability::Unknown
                        }
                    }
                }
            }
        };
    }
    if component.required {
        component.availability = FirmwareUpdateAvailability::Available;
    }
}

fn push_issue(issues: &mut Vec<FirmwareUpdateIssue>, issue: FirmwareUpdateIssue) {
    if !issues.iter().any(|candidate| candidate.code == issue.code) {
        issues.push(issue);
    }
}

fn finish_report(
    components: BTreeMap<String, FirmwareUpdateComponent>,
    mut issues: Vec<FirmwareUpdateIssue>,
    truncated: bool,
) -> FirmwareUpdateReport {
    if truncated {
        push_issue(&mut issues, provider_data_truncated());
    }
    FirmwareUpdateReport::from_components(
        FirmwareUpdateSource::BambuMqtt,
        components.into_values().collect(),
        issues,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_all_modules_and_unknown_fields() {
        let inventory = FirmwareInventory::from_version_info(&json!({"module": [
            {"name":"ota","sw_ver":"01.08.00.00-beta1","hw_ver":"AP05","future":7},
            {"name":"n3f/0","sw_ver":"00.00.06.40"}
        ]}));
        assert_eq!(inventory.modules.len(), 2);
        assert_eq!(inventory.software("ota").unwrap().numeric, [1, 8, 0, 0]);
        assert_eq!(
            inventory.software("ota").unwrap().suffix.as_deref(),
            Some("-beta1")
        );
        assert_eq!(inventory.modules[0].unknown["future"], 7);
    }

    #[test]
    fn update_report_combines_installed_and_advertised_modules() {
        let inventory = FirmwareInventory::from_version_info(&json!({"module": [
            {"name":"ota","sw_ver":"01.08.00.00","project_name":"X1 firmware"},
            {"name":"n3f/0","sw_ver":"00.00.06.40"}
        ]}));
        let report = update_report(
            &json!({"print":{"upgrade_state":{
                "new_ver_list":[
                    {"name":"ota","sw_new_ver":"01.09.00.00","product_name":"X1 firmware"},
                    {"name":"n3f/0","sw_new_ver":"00.00.06.41"}
                ]
            }}}),
            &inventory,
        );

        assert_eq!(report.availability, FirmwareUpdateAvailability::Available);
        assert_eq!(report.components.len(), 2);
        assert_eq!(
            report.components[0].available_version.as_deref(),
            Some("01.09.00.00")
        );
        assert_eq!(
            report.components[1].current_version.as_deref(),
            Some("00.00.06.40")
        );
    }

    #[test]
    fn missing_bambu_advertisement_is_unknown_not_current() {
        let inventory = FirmwareInventory::from_version_info(
            &json!({"module":[{"name":"ota","sw_ver":"01.08.00.00"}]}),
        );
        let report = update_report(
            &json!({"print":{"upgrade_state":{"new_version_state":2,"new_ver_list":[]}}}),
            &inventory,
        );

        assert_eq!(report.availability, FirmwareUpdateAvailability::Unknown);
        assert_eq!(
            report.components[0].availability,
            FirmwareUpdateAvailability::Unknown
        );
    }

    #[test]
    fn forced_update_is_available_without_a_target_version() {
        let report = update_report(
            &json!({"print":{"force_upgrade":true,"upgrade_state":{"force_upgrade":true}}}),
            &FirmwareInventory::default(),
        );

        assert_eq!(report.availability, FirmwareUpdateAvailability::Available);
        assert!(report.components[0].required);
        assert!(report.components[0].available_version.is_none());
    }

    #[test]
    fn lower_or_uncomparable_targets_do_not_create_false_updates() {
        let inventory = FirmwareInventory::from_version_info(
            &json!({"module":[{"name":"ota","sw_ver":"02.00.00.00"}]}),
        );
        let lower = update_report(
            &json!({"print":{"upgrade_state":{"ota_new_version_number":"01.00.00.00"}}}),
            &inventory,
        );
        assert_eq!(lower.availability, FirmwareUpdateAvailability::Unknown);
        assert_eq!(lower.issues[0].code, "targetNotNewer");

        let malformed_inventory = FirmwareInventory::from_version_info(
            &json!({"module":[{"name":"ota","sw_ver":"development"}]}),
        );
        let malformed = update_report(
            &json!({"print":{"upgrade_state":{"ota_new_version_number":"future"}}}),
            &malformed_inventory,
        );
        assert_eq!(malformed.availability, FirmwareUpdateAvailability::Unknown);
    }

    #[test]
    fn history_adds_device_catalogue_evidence_without_update_mutations() {
        let inventory = FirmwareInventory::from_version_info(
            &json!({"module":[{"name":"ota","sw_ver":"01.08.00.00"}]}),
        );
        let report = update_report(&json!({"print":{}}), &inventory);
        let merged = merge_history_report(
            report,
            &json!({"upgrade":{"firmware_optional":[{
                "firmware":{"version":"01.09.00.00","force_update":false},
                "ams":[{"device_id":"ams-0","dev_model_name":"AMS","firmware":[{"version":"00.01.00.00"}]}]
            }]}}),
        );
        assert_eq!(merged.availability, FirmwareUpdateAvailability::Available);
        let ota = merged.components.iter().find(|component| component.id == "ota").unwrap();
        assert_eq!(ota.available_version.as_deref(), Some("01.09.00.00"));
        assert!(ota.evidence.iter().any(|evidence| {
            evidence.source == FirmwareEvidenceSource::BambuLanHistory
                && evidence.role == FirmwareEvidenceRole::DeviceCatalogue
        }));
        assert!(merged.components.iter().any(|component| component.id == "ams-0"));
    }

    #[test]
    fn public_catalogue_adds_distinct_evidence() {
        let inventory = FirmwareInventory::from_version_info(
            &json!({"module":[{"name":"ota","sw_ver":"01.08.00.00"}]}),
        );
        let merged = merge_public_catalogue_report(update_report(&json!({"print":{}}), &inventory), "01.09.00.00");
        let ota = merged.components.iter().find(|component| component.id == "ota").unwrap();
        assert_eq!(merged.assessment, FirmwareUpdateAssessment::PublicReleaseNewer);
        assert!(ota.evidence.iter().any(|evidence| {
            evidence.source == FirmwareEvidenceSource::BambuPublicCatalogue
                && evidence.role == FirmwareEvidenceRole::PublicStable
        }));
    }
}
