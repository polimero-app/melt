use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::{
    VERSION,
    bambu::{self, CameraOwnerStatus, CameraSelection, RuntimeCapabilities},
    config::Profile,
    monitor,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub version: &'static str,
    pub platform: String,
    pub configured_profiles: usize,
    pub drivers: BTreeMap<String, usize>,
    pub identifiers_redacted: bool,
    pub monitor_workers: u8,
    pub monitor_interval_seconds: u64,
    pub protocol_traces_included: bool,
    pub bambu: Vec<BambuCompatibilityReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BambuCompatibilityReport {
    pub printer: String,
    pub model_raw: String,
    pub canonical_model: bambu::CanonicalModel,
    pub model_family: bambu::ModelFamily,
    pub firmware_modules: Vec<FirmwareModuleReport>,
    pub capability_observations: Vec<CapabilityObservationReport>,
    pub authorization: bambu::AuthorizationResolution,
    pub camera: CameraReport,
    pub storage_transport: bambu::StorageTransport,
    pub quirks: Vec<QuirkReport>,
    pub tls_pinned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareModuleReport {
    pub name: String,
    pub software: String,
    pub hardware: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityObservationReport {
    pub field: String,
    pub source: bambu::ObservationSource,
    pub report_kind: bambu::ReportKind,
    pub age_seconds: u64,
    pub value_summary: &'static str,
    pub unknown: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraReport {
    pub preferred: bambu::CameraTransport,
    pub source: bambu::CameraSelectionSource,
    pub rejected_advertisement: Option<String>,
    pub owner: Option<CameraOwnerReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraOwnerReport {
    pub transport: bambu::CameraOwnerTransport,
    pub subscribers: usize,
    pub generation: u64,
    pub has_jpeg: bool,
    pub reconnect_attempts: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuirkReport {
    pub id: &'static str,
    pub effect: bambu::QuirkEffect,
    pub qualification: bambu::QuirkQualification,
    pub active: bool,
}

pub fn report(profiles: impl IntoIterator<Item = Profile>) -> Report {
    let mut configured_profiles = 0;
    let mut drivers = BTreeMap::new();
    for profile in profiles {
        configured_profiles += 1;
        *drivers.entry(profile.driver).or_insert(0) += 1;
    }
    Report {
        version: VERSION,
        platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        configured_profiles,
        drivers,
        identifiers_redacted: true,
        monitor_workers: monitor::DEFAULT_WORKERS,
        monitor_interval_seconds: monitor::DEFAULT_INTERVAL.as_secs(),
        protocol_traces_included: false,
        bambu: Vec::new(),
    }
}

pub fn with_bambu(mut report: Report, bambu: Vec<BambuCompatibilityReport>) -> Report {
    report.bambu = bambu;
    report
}

pub fn bambu_compatibility_report(
    printer_index: usize,
    capabilities: &RuntimeCapabilities,
    selection: &CameraSelection,
    tls_pinned: bool,
    owner: Option<&CameraOwnerStatus>,
) -> BambuCompatibilityReport {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut observations = Vec::new();
    for (unknown, values) in [
        (false, &capabilities.observations.values),
        (true, &capabilities.observations.unknown_support_fields),
    ] {
        observations.extend(
            values
                .iter()
                .map(|(field, observed)| CapabilityObservationReport {
                    field: field.clone(),
                    source: observed.source,
                    report_kind: observed.report_kind,
                    age_seconds: now.saturating_sub(observed.observed_at_unix_ms) / 1000,
                    value_summary: value_summary(&observed.value),
                    unknown,
                }),
        );
    }
    observations.sort_by(|left, right| left.field.cmp(&right.field));
    let active = bambu::applicable_quirks(&capabilities.identity, &capabilities.firmware);
    let quirks = bambu::matching_quirks(&capabilities.identity)
        .into_iter()
        .map(|entry| QuirkReport {
            id: entry.id,
            effect: entry.effect,
            qualification: entry.qualification,
            active: active.iter().any(|candidate| candidate.id == entry.id),
        })
        .collect();
    BambuCompatibilityReport {
        printer: format!("printer-{printer_index}"),
        model_raw: capabilities.identity.raw.clone(),
        canonical_model: capabilities.identity.canonical,
        model_family: capabilities.model_family,
        firmware_modules: capabilities
            .firmware
            .modules
            .iter()
            .map(|module| FirmwareModuleReport {
                name: module.name.clone(),
                software: module.software.raw.clone(),
                hardware: module.hardware.clone(),
            })
            .collect(),
        capability_observations: observations,
        authorization: capabilities.authorization_resolution.clone(),
        camera: CameraReport {
            preferred: selection.preferred,
            source: selection.source,
            rejected_advertisement: selection.rejected_advertisement.clone(),
            owner: owner.map(|owner| CameraOwnerReport {
                transport: owner.transport,
                subscribers: owner.subscribers,
                generation: owner.generation,
                has_jpeg: owner.has_jpeg,
                reconnect_attempts: owner.reconnect_attempts,
            }),
        },
        storage_transport: capabilities.storage_transport,
        quirks,
        tls_pinned,
    }
}

fn value_summary(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(true) => "true",
        serde_json::Value::Bool(false) => "false",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string-redacted",
        serde_json::Value::Array(_) => "array-redacted",
        serde_json::Value::Object(_) => "object-redacted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_counts_drivers_without_retaining_printer_identifiers() {
        let profiles = vec![
            Profile {
                driver: "moonraker".into(),
                host: "private-printer.local".into(),
                serial: "SECRET-SERIAL".into(),
                model: String::new(),
                timeout: "10s".into(),
                insecure: false,
                created: String::new(),
                updated: String::new(),
            },
            Profile {
                driver: "moonraker".into(),
                host: "192.0.2.10".into(),
                serial: String::new(),
                model: String::new(),
                timeout: "10s".into(),
                insecure: false,
                created: String::new(),
                updated: String::new(),
            },
        ];

        let report = report(profiles);

        assert_eq!(report.configured_profiles, 2);
        assert_eq!(report.drivers["moonraker"], 2);
        assert!(report.identifiers_redacted);
        assert!(
            !serde_json::to_string(&report)
                .unwrap()
                .contains("private-printer.local")
        );
    }

    #[test]
    fn compatibility_report_redacts_nested_identifiers_and_endpoints() {
        let mut capabilities = RuntimeCapabilities::for_model("P1S");
        capabilities.firmware.modules.push(bambu::FirmwareModule {
            name: "ota".into(),
            software: bambu::FirmwareVersion::parse("01.02.03.04"),
            hardware: Some("AP05".into()),
            serial: Some("SECRET-MODULE-SERIAL".into()),
            project: None,
            unknown: BTreeMap::from([(
                "endpoint".into(),
                serde_json::json!("rtsps://192.0.2.99/private"),
            )]),
        });
        capabilities.observations.values.insert(
            "ipcam".into(),
            bambu::Observed {
                value: serde_json::json!({"rtsp_url":"rtsps://192.0.2.99/private"}),
                source: bambu::ObservationSource::MqttStatus,
                report_kind: bambu::ReportKind::Full,
                observed_at_unix_ms: 0,
            },
        );
        let selection = CameraSelection {
            preferred: bambu::CameraTransport::Unknown,
            source: bambu::CameraSelectionSource::Unknown,
            rejected_advertisement: Some(
                "advertised camera host differs from the configured printer".into(),
            ),
            quirk_ids: Vec::new(),
        };
        let report = bambu_compatibility_report(1, &capabilities, &selection, true, None);
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("SECRET-MODULE-SERIAL"));
        assert!(!json.contains("192.0.2.99"));
        assert!(!json.contains("/private"));
        assert!(json.contains("string-redacted") || json.contains("object-redacted"));
    }
}
