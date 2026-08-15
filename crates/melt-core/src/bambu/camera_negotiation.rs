use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use super::{CameraTransport, QuirkEffect, RuntimeCapabilities, applicable_quirks};

const ADVERTISEMENT_MAX_AGE: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CameraSelectionSource {
    Advertised,
    QualifiedQuirk,
    ModelDefault,
    FamilyHint,
    SafeProbe,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSelection {
    pub preferred: CameraTransport,
    pub source: CameraSelectionSource,
    pub rejected_advertisement: Option<String>,
    pub quirk_ids: Vec<String>,
}

pub fn select_camera_transport(host: &str, capabilities: &RuntimeCapabilities) -> CameraSelection {
    if let Some(ipcam) = capabilities.observations.values.get("ipcam")
        && observation_is_fresh(ipcam.observed_at_unix_ms)
        && let Some(selection) = advertised_selection(host, &ipcam.value)
    {
        return selection;
    }

    let quirks = applicable_quirks(&capabilities.identity, &capabilities.firmware);
    if let Some(entry) = quirks.iter().find(|entry| {
        matches!(
            entry.effect,
            QuirkEffect::PreferMjpegCamera | QuirkEffect::PreferRtspsCamera
        )
    }) {
        return CameraSelection {
            preferred: if entry.effect == QuirkEffect::PreferMjpegCamera {
                CameraTransport::MjpegTls
            } else {
                CameraTransport::RtspsH264
            },
            source: CameraSelectionSource::QualifiedQuirk,
            rejected_advertisement: None,
            quirk_ids: quirks.iter().map(|entry| entry.id.to_owned()).collect(),
        };
    }

    let source = if capabilities.camera == CameraTransport::Unknown {
        CameraSelectionSource::Unknown
    } else if capabilities.identity.canonical != super::CanonicalModel::Unknown {
        CameraSelectionSource::ModelDefault
    } else if capabilities.model_family != super::ModelFamily::Unknown {
        CameraSelectionSource::FamilyHint
    } else {
        CameraSelectionSource::Unknown
    };
    CameraSelection {
        preferred: capabilities.camera,
        source,
        rejected_advertisement: None,
        quirk_ids: Vec::new(),
    }
}

fn observation_is_fresh(observed_at_unix_ms: u64) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    now.saturating_sub(observed_at_unix_ms) <= ADVERTISEMENT_MAX_AGE.as_millis() as u64
}

fn advertised_selection(host: &str, ipcam: &Value) -> Option<CameraSelection> {
    let rtsp = ipcam
        .get("rtsp_url")
        .and_then(Value::as_str)
        .or_else(|| ipcam.get("rtspUrl").and_then(Value::as_str));
    if let Some(raw) = rtsp {
        let Ok(url) = Url::parse(raw) else {
            return Some(rejected("invalid advertised RTSPS URL"));
        };
        if url.scheme() != "rtsps" {
            return Some(rejected("advertised camera URL is not RTSPS"));
        }
        if url.host_str() != Some(host) {
            return Some(rejected(
                "advertised camera host differs from the configured printer",
            ));
        }
        return Some(selected(CameraTransport::RtspsH264));
    }
    let local = ipcam.get("liveview").and_then(|value| value.get("local"));
    match local {
        Some(Value::String(value))
            if matches!(
                value.to_ascii_lowercase().as_str(),
                "mjpeg" | "mjpeg_tls" | "local"
            ) =>
        {
            Some(selected(CameraTransport::MjpegTls))
        }
        Some(Value::Bool(true)) => Some(selected(CameraTransport::MjpegTls)),
        _ => None,
    }
}

fn selected(preferred: CameraTransport) -> CameraSelection {
    CameraSelection {
        preferred,
        source: CameraSelectionSource::Advertised,
        rejected_advertisement: None,
        quirk_ids: Vec::new(),
    }
}

fn rejected(reason: &str) -> CameraSelection {
    CameraSelection {
        preferred: CameraTransport::Unknown,
        source: CameraSelectionSource::Unknown,
        rejected_advertisement: Some(reason.to_owned()),
        quirk_ids: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bambu::{ObservationSource, Observed, ReportKind};

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[test]
    fn fresh_advertisement_outranks_model_default() {
        let mut capabilities = RuntimeCapabilities::for_model("A1");
        capabilities.observations.values.insert(
            "ipcam".into(),
            Observed {
                value: serde_json::json!({"rtsp_url":"rtsps://printer.local:322/streaming/live/1"}),
                source: ObservationSource::MqttStatus,
                report_kind: ReportKind::Full,
                observed_at_unix_ms: now(),
            },
        );
        let selection = select_camera_transport("printer.local", &capabilities);
        assert_eq!(selection.preferred, CameraTransport::RtspsH264);
        assert_eq!(selection.source, CameraSelectionSource::Advertised);
    }

    #[test]
    fn advertised_foreign_host_is_never_followed() {
        let mut capabilities = RuntimeCapabilities::for_model("P1S");
        capabilities.observations.values.insert(
            "ipcam".into(),
            Observed {
                value: serde_json::json!({"rtsp_url":"rtsps://attacker.invalid/live"}),
                source: ObservationSource::MqttStatus,
                report_kind: ReportKind::Full,
                observed_at_unix_ms: now(),
            },
        );
        let selection = select_camera_transport("printer.local", &capabilities);
        assert_eq!(selection.preferred, CameraTransport::Unknown);
        assert!(selection.rejected_advertisement.is_some());
    }

    #[test]
    fn stale_advertisement_does_not_override_defaults() {
        let mut capabilities = RuntimeCapabilities::for_model("A1");
        capabilities.observations.values.insert(
            "ipcam".into(),
            Observed {
                value: serde_json::json!({"rtsp_url":"rtsps://printer.local:322/live"}),
                source: ObservationSource::MqttStatus,
                report_kind: ReportKind::Full,
                observed_at_unix_ms: 1,
            },
        );
        let selection = select_camera_transport("printer.local", &capabilities);
        assert_eq!(selection.preferred, CameraTransport::MjpegTls);
        assert_eq!(selection.source, CameraSelectionSource::ModelDefault);
    }

    #[test]
    fn provisional_p1s_conflicts_cannot_select_a_transport() {
        let capabilities = RuntimeCapabilities::for_model("P1S");
        let selection = select_camera_transport("printer.local", &capabilities);
        assert_eq!(selection.source, CameraSelectionSource::Unknown);
        assert_eq!(selection.preferred, CameraTransport::Unknown);
        assert!(selection.quirk_ids.is_empty());
    }
}
