use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    VERSION,
    bambu::{self, CameraOwnerStatus, CameraSelection, RuntimeCapabilities},
    cameras::CameraSummary,
    config::Profile,
    firmware_updates::FirmwareUpdateReport,
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
    pub cameras: Vec<ExternalCameraReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BambuCompatibilityReport {
    pub printer: String,
    pub model_raw: String,
    pub canonical_model: bambu::CanonicalModel,
    pub model_family: bambu::ModelFamily,
    pub model_source: bambu::ModelSource,
    pub model_conflicts: Vec<bambu::ModelConflict>,
    pub firmware_modules: Vec<FirmwareModuleReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub firmware_updates: Option<FirmwareUpdateReport>,
    pub capability_observations: Vec<CapabilityObservationReport>,
    pub upgrade_observations: Vec<CapabilityObservationReport>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback: Option<PlaybackReport>,
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

impl From<&CameraOwnerStatus> for CameraOwnerReport {
    fn from(owner: &CameraOwnerStatus) -> Self {
        Self {
            transport: owner.transport,
            subscribers: owner.subscribers,
            generation: owner.generation,
            has_jpeg: owner.has_jpeg,
            reconnect_attempts: owner.reconnect_attempts,
        }
    }
}

/// Playback is negotiated in the webview, so it arrives with the
/// diagnostics request instead of being observed here.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackTransport {
    Webrtc,
    Webcodecs,
    Mjpeg,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackReport {
    /// Absent when playback could not be established.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<PlaybackTransport>,
    /// Negotiation failures. Free text from the webview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Playback keyed by printer and by camera profile name.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackRequest {
    #[serde(default)]
    pub printers: BTreeMap<String, PlaybackReport>,
    #[serde(default)]
    pub cameras: BTreeMap<String, PlaybackReport>,
}

pub const MAX_PLAYBACK_DETAIL_CHARS: usize = 500;

/// Bounds the webview's free text and masks every known identifier in it.
pub fn redact_playback(playback: &PlaybackReport, identifiers: &[&str]) -> PlaybackReport {
    let detail = playback.detail.as_ref().map(|detail| {
        let mut redacted = String::new();
        let mut redacted_until = 0;
        let mut output_chars = 0;
        // Match the original text, including overlaps, never replacement markers.
        for (offset, character) in detail.char_indices() {
            if output_chars >= MAX_PLAYBACK_DETAIL_CHARS {
                break;
            }
            let match_end = identifiers
                .iter()
                .filter(|id| !id.is_empty() && detail[offset..].starts_with(**id))
                .map(|id| offset + id.len())
                .max()
                .unwrap_or(0);
            if offset >= redacted_until {
                if match_end > offset {
                    redacted.push_str("[redacted]");
                    output_chars += "[redacted]".len();
                } else {
                    redacted.push(character);
                    output_chars += 1;
                }
            }
            redacted_until = redacted_until.max(match_end);
        }
        redacted.chars().take(MAX_PLAYBACK_DETAIL_CHARS).collect()
    });
    PlaybackReport {
        transport: playback.transport,
        detail,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalCameraReport {
    pub camera: String,
    pub driver: String,
    pub linked_printers: usize,
    pub streams: Vec<ExternalStreamReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback: Option<PlaybackReport>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalStreamReport {
    pub quality: String,
    pub owner: CameraOwnerReport,
}

/// Identifies the camera only as report-local `camera-N`; its owners are
/// keyed `camera:{id}:{quality}` by the camera manager.
pub fn external_camera_report(
    camera_index: usize,
    camera: &CameraSummary,
    owners: &[CameraOwnerStatus],
    playback: Option<&PlaybackReport>,
) -> ExternalCameraReport {
    let prefix = format!("camera:{}:", camera.id);
    let identifiers = [
        camera.name.as_str(),
        camera.id.as_str(),
        camera.host.as_str(),
        camera.username.as_str(),
    ];
    ExternalCameraReport {
        camera: format!("camera-{camera_index}"),
        driver: camera.driver.clone(),
        linked_printers: camera.printers.len(),
        streams: owners
            .iter()
            .filter_map(|owner| {
                let quality = owner.printer_key.strip_prefix(&prefix)?;
                Some(ExternalStreamReport {
                    quality: quality.to_owned(),
                    owner: owner.into(),
                })
            })
            .collect(),
        playback: playback.map(|playback| redact_playback(playback, &identifiers)),
    }
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
        cameras: Vec::new(),
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
    bambu_compatibility_report_with_updates(
        printer_index,
        capabilities,
        selection,
        tls_pinned,
        owner,
        None,
    )
}

pub fn bambu_compatibility_report_with_updates(
    printer_index: usize,
    capabilities: &RuntimeCapabilities,
    selection: &CameraSelection,
    tls_pinned: bool,
    owner: Option<&CameraOwnerStatus>,
    firmware_updates: Option<&FirmwareUpdateReport>,
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
    let mut upgrade_observations = capabilities
        .observations
        .upgrade_fields
        .iter()
        .map(|(field, observed)| CapabilityObservationReport {
            field: field.clone(),
            source: observed.source,
            report_kind: observed.report_kind,
            age_seconds: now.saturating_sub(observed.observed_at_unix_ms) / 1000,
            value_summary: value_summary(&observed.value),
            unknown: !known_upgrade_field(field),
        })
        .collect::<Vec<_>>();
    upgrade_observations.sort_by(|left, right| left.field.cmp(&right.field));
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
        model_source: capabilities.model_source,
        model_conflicts: capabilities.model_conflicts.clone(),
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
        firmware_updates: firmware_updates.cloned(),
        capability_observations: observations,
        upgrade_observations,
        authorization: capabilities.authorization_resolution.clone(),
        camera: CameraReport {
            preferred: selection.preferred,
            source: selection.source,
            rejected_advertisement: selection.rejected_advertisement.clone(),
            owner: owner.map(CameraOwnerReport::from),
            playback: None,
        },
        storage_transport: capabilities.storage_transport,
        quirks,
        tls_pinned,
    }
}

fn known_upgrade_field(field: &str) -> bool {
    matches!(
        field,
        "new_ver_list"
            | "ota_new_version_number"
            | "ams_new_version_number"
            | "ahb_new_version_number"
            | "ext_new_version_number"
            | "force_upgrade"
            | "new_version_state"
    )
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
            product: Some("Bambu Lab P1S".into()),
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
        capabilities.observations.merge_status(
            &serde_json::json!({"print":{"upgrade_state":{
                "new_ver_list":[{"sw_new_ver":"2.0","url":"https://private.invalid/package"}],
                "future_provider_field":{"device":"SECRET-SERIAL"}
            }}}),
            true,
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
        assert!(!json.contains("private.invalid"));
        assert!(!json.contains("SECRET-SERIAL"));
        assert_eq!(report.upgrade_observations.len(), 2);
        assert!(
            report
                .upgrade_observations
                .iter()
                .any(|observation| observation.field == "future_provider_field"
                    && observation.unknown
                    && observation.value_summary == "object-redacted")
        );
        assert!(json.contains("string-redacted") || json.contains("object-redacted"));
    }

    fn owner(key: &str, subscribers: usize) -> CameraOwnerStatus {
        CameraOwnerStatus {
            printer_key: key.into(),
            transport: bambu::CameraOwnerTransport::RtspH264,
            subscribers,
            generation: 1,
            has_jpeg: false,
            reconnect_attempts: 2,
            selection_source: bambu::CameraSelectionSource::Unknown,
            rejected_advertisement: None,
        }
    }

    #[test]
    fn external_camera_report_keeps_only_its_own_streams_and_redacts_identifiers() {
        let camera = CameraSummary {
            name: "garage".into(),
            id: "cam-abc".into(),
            driver: "tapo".into(),
            host: "192.0.2.50".into(),
            username: "camuser".into(),
            rtsp_port: 554,
            timeout: "10s".into(),
            printers: vec!["p1s".into(), "x1c".into()],
            selected_by: vec!["p1s".into()],
        };
        let owners = [
            owner("camera:cam-abc:low", 1),
            owner("camera:cam-other:high", 3),
            owner("bambu:SN001", 4),
        ];
        let playback = PlaybackReport {
            transport: Some(PlaybackTransport::Mjpeg),
            detail: Some("garage at 192.0.2.50 as camuser failed".into()),
        };

        let report = external_camera_report(2, &camera, &owners, Some(&playback));
        let json = serde_json::to_string(&report).unwrap();

        assert_eq!(report.camera, "camera-2");
        assert_eq!(report.driver, "tapo");
        assert_eq!(report.linked_printers, 2);
        assert_eq!(report.streams.len(), 1);
        assert_eq!(report.streams[0].quality, "low");
        assert_eq!(report.streams[0].owner.subscribers, 1);
        assert_eq!(
            report.playback.as_ref().unwrap().transport,
            Some(PlaybackTransport::Mjpeg)
        );
        for secret in ["garage", "192.0.2.50", "camuser", "cam-abc", "p1s"] {
            assert!(!json.contains(secret), "{secret} leaked: {json}");
        }
    }

    #[test]
    fn playback_detail_is_redacted_and_bounded() {
        let cases = [
            ("no identifiers", vec![], "no identifiers"),
            (
                "p1s at host.local",
                vec!["p1s", "host.local"],
                "[redacted] at [redacted]",
            ),
            ("Kept", vec![""], "Kept"),
            (
                "garage.local garage-admin",
                vec!["garage", "garage.local", "garage-admin"],
                "[redacted] [redacted]",
            ),
            ("ababa", vec!["aba", "bab"], "[redacted]"),
            ("aaaa", vec!["aaa"], "[redacted]"),
            (
                "camera user",
                vec!["camera", "a", "user"],
                "[redacted] [redacted]",
            ),
            ("ééé", vec!["éé"], "[redacted]"),
        ];
        for (detail, identifiers, want) in cases {
            let playback = PlaybackReport {
                transport: Some(PlaybackTransport::Webcodecs),
                detail: Some(detail.into()),
            };
            assert_eq!(
                redact_playback(&playback, &identifiers).detail.as_deref(),
                Some(want)
            );
        }

        let long = PlaybackReport {
            transport: Some(PlaybackTransport::Webrtc),
            detail: Some("é".repeat(MAX_PLAYBACK_DETAIL_CHARS + 10)),
        };
        let detail = redact_playback(&long, &[]).detail.unwrap();
        assert_eq!(detail.chars().count(), MAX_PLAYBACK_DETAIL_CHARS);

        let boundary = PlaybackReport {
            transport: None,
            detail: Some(format!("{}192.0.2.50", "x".repeat(494))),
        };
        let detail = redact_playback(&boundary, &["192.0.2.50"]).detail.unwrap();
        assert!(!detail.contains("192.0."));
        assert_eq!(detail.chars().count(), MAX_PLAYBACK_DETAIL_CHARS);

        let expanding = PlaybackReport {
            transport: None,
            detail: Some("x".repeat(MAX_PLAYBACK_DETAIL_CHARS)),
        };
        let detail = redact_playback(&expanding, &["x"]).detail.unwrap();
        assert_eq!(detail.chars().count(), MAX_PLAYBACK_DETAIL_CHARS);
    }

    #[test]
    fn playback_transports_deserialize_from_the_webview_names() {
        let request: PlaybackRequest = serde_json::from_str(
            r#"{"printers":{"P1S":{"transport":"webcodecs","detail":"x"}},"cameras":{"garage":{"transport":"mjpeg"},"offline":{"detail":"connection failed"}}}"#,
        )
        .unwrap();
        assert_eq!(
            request.printers["P1S"].transport,
            Some(PlaybackTransport::Webcodecs)
        );
        assert_eq!(request.cameras["garage"].detail, None);
        assert_eq!(request.cameras["offline"].transport, None);
        assert_eq!(
            serde_json::to_value(&request.cameras["offline"]).unwrap(),
            serde_json::json!({ "detail": "connection failed" })
        );
        assert!(
            serde_json::from_str::<PlaybackRequest>(r#"{"cameras":{"a":{"transport":"hls"}}}"#)
                .is_err()
        );
    }

    /// The detection source names a channel, never the evidence that
    /// channel read. A serial prefix is three characters of a serial, so
    /// reporting it at all would leak part of a TLS-bound identifier.
    #[test]
    fn the_detection_source_is_reported_without_the_serial_behind_it() {
        let detection = bambu::detect("01P00SECRETSERIAL", "X1 Carbon", &Default::default());
        let mut capabilities = RuntimeCapabilities::for_identity(detection.identity);
        capabilities.model_source = detection.source;
        capabilities.model_conflicts = detection.conflicts;

        let report = bambu_compatibility_report(
            1,
            &capabilities,
            &CameraSelection {
                preferred: bambu::CameraTransport::Unknown,
                source: bambu::CameraSelectionSource::Unknown,
                rejected_advertisement: None,
                quirk_ids: Vec::new(),
            },
            true,
            None,
        );
        let json = serde_json::to_string(&report).unwrap();

        assert_eq!(report.model_source, bambu::ModelSource::SerialPrefix);
        assert_eq!(report.canonical_model, bambu::CanonicalModel::P1S);
        assert_eq!(
            report.model_conflicts,
            [bambu::ModelConflict {
                source: bambu::ModelSource::Configured,
                canonical: bambu::CanonicalModel::X1Carbon,
            }]
        );
        assert!(json.contains("\"modelSource\":\"serialPrefix\""));
        assert!(!json.contains("SECRETSERIAL"));
        assert!(!json.contains("01P"));
    }
}
