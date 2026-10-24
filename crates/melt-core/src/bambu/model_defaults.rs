use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{CanonicalModel, ModelIdentity};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDefaults {
    pub model_id: String,
    pub profile_version: String,
    pub source_url: String,
    pub source_revision: String,
    pub values: BTreeMap<String, Value>,
}

impl ModelDefaults {
    pub(super) fn for_identity(identity: &ModelIdentity) -> Option<Self> {
        if identity.canonical != CanonicalModel::P2S {
            return None;
        }
        // N7 has one manufacturer baseline, applicable from 00.00.00.00.
        // Other models remain unseeded rather than inheriting a family guess.
        Some(Self {
            model_id: "N7".into(),
            profile_version: "00.00.00.00".into(),
            source_url: "https://github.com/bambulab/BambuStudio/blob/79df7ae309be398ed30c9f397c13c9911c2edc57/resources/printers/N7.json".into(),
            source_revision: "79df7ae309be398ed30c9f397c13c9911c2edc57".into(),
            values: json!({
                "nozzle_temp_range": [0, 300],
                "bed_temp_range": [0, 110],
                "support_aux_fan": true,
                "support_chamber_fan": true,
                "support_ams_humidity": true,
                "support_bed_leveling": 2,
                "support_send_to_sd": true,
                "support_flow_calibration": true,
                "support_timelapse": true
            })
            .as_object()
            .expect("fixed manufacturer defaults are an object")
            .clone()
            .into_iter()
            .collect(),
        })
    }
}
