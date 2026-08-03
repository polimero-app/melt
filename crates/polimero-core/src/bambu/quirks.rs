use serde::{Deserialize, Serialize};

use super::{CanonicalModel, FirmwareInventory, ModelIdentity};

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

pub fn applicable(
    identity: &ModelIdentity,
    firmware: &FirmwareInventory,
) -> Vec<&'static QuirkEntry> {
    REGISTRY
        .iter()
        .filter(|entry| {
            matches!(
                entry.qualification,
                Qualification::Qualified | Qualification::Confirmed
            ) && entry.model == identity.canonical
                && entry
                    .module
                    .is_none_or(|name| firmware.software(name).is_some())
                && entry.firmware_min.is_none()
                && entry.firmware_max.is_none()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
