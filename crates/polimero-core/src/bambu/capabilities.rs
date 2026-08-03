use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ObservationSource {
    MqttStatus,
    VersionReport,
    Discovery,
    SafeProbe,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReportKind {
    Full,
    Delta,
    Explicit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observed<T> {
    pub value: T,
    pub source: ObservationSource,
    pub report_kind: ReportKind,
    pub observed_at_unix_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityObservations {
    pub values: BTreeMap<String, Observed<Value>>,
    pub unknown_support_fields: BTreeMap<String, Observed<Value>>,
}

impl CapabilityObservations {
    pub fn merge_status(&mut self, report: &Value, full: bool) {
        let Some(print) = report.get("print").and_then(Value::as_object) else {
            return;
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let kind = if full {
            ReportKind::Full
        } else {
            ReportKind::Delta
        };
        for (key, value) in print {
            if key.starts_with("support_") || key == "ipcam" {
                let observation = Observed {
                    value: value.clone(),
                    source: ObservationSource::MqttStatus,
                    report_kind: kind,
                    observed_at_unix_ms: timestamp,
                };
                if known_field(key) {
                    self.values.insert(key.clone(), observation);
                } else {
                    self.unknown_support_fields.insert(key.clone(), observation);
                }
            }
        }
    }
}

fn known_field(field: &str) -> bool {
    matches!(
        field,
        "support_mqtt_alive"
            | "support_send_to_sd"
            | "support_flow_calibration"
            | "support_timelapse"
            | "support_ams_humidity"
            | "support_bed_leveling"
            | "ipcam"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_unknown_support_fields_with_provenance() {
        let mut observations = CapabilityObservations::default();
        observations.merge_status(
            &json!({"print":{"support_future_mode":true,"support_timelapse":false}}),
            true,
        );
        assert_eq!(observations.values["support_timelapse"].value, false);
        assert_eq!(
            observations.unknown_support_fields["support_future_mode"].value,
            true
        );
        assert_eq!(
            observations.values["support_timelapse"].report_kind,
            ReportKind::Full
        );
    }
}
