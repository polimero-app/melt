use std::collections::BTreeMap;

use serde::Serialize;

use crate::{VERSION, config::Profile, monitor};

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
                timeout: "10s".into(),
                insecure: false,
                created: String::new(),
                updated: String::new(),
            },
            Profile {
                driver: "moonraker".into(),
                host: "192.0.2.10".into(),
                serial: String::new(),
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
}
