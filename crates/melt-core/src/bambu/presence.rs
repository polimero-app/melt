use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::DiscoveredPrinter;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrinterPresence {
    pub serial: String,
    pub host: String,
    pub suggested_host: Option<String>,
    pub model: String,
    pub firmware: Option<String>,
    pub schema_version: Option<String>,
    pub connect_mode: Option<String>,
    pub bind_state: Option<String>,
    pub security_mode: Option<String>,
    pub interface: Option<String>,
    pub last_seen_unix_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct PresenceCache {
    entries: BTreeMap<String, PrinterPresence>,
}

impl PresenceCache {
    pub fn observe(
        &mut self,
        printer: &DiscoveredPrinter,
        configured_host: Option<&str>,
        now_unix_ms: u64,
    ) -> Option<&PrinterPresence> {
        if printer.serial.is_empty() {
            return None;
        }
        let suggested_host = configured_host
            .filter(|host| *host != printer.host)
            .map(|_| printer.host.clone());
        let presence = PrinterPresence {
            serial: printer.serial.clone(),
            host: printer.host.clone(),
            suggested_host,
            model: printer.model.clone(),
            firmware: printer.firmware.clone(),
            schema_version: printer.schema_version.clone(),
            connect_mode: printer.connect_mode.clone(),
            bind_state: printer.bind_state.clone(),
            security_mode: printer.security_mode.clone(),
            interface: printer.interface.clone(),
            last_seen_unix_ms: now_unix_ms,
        };
        let key = normalize_serial(&printer.serial);
        self.entries.insert(key.clone(), presence);
        self.entries.get(&key)
    }

    pub fn get(&self, serial: &str) -> Option<&PrinterPresence> {
        self.entries.get(&normalize_serial(serial))
    }

    /// Records an address reported by an authenticated status connection.
    /// The configured host is only compared; profile changes remain an
    /// explicit user action through `suggested_host`.
    pub fn observe_status_address(
        &mut self,
        serial: &str,
        reported_host: &str,
        configured_host: Option<&str>,
        model: &str,
        firmware: Option<&str>,
        now_unix_ms: u64,
    ) -> Option<&PrinterPresence> {
        let serial = serial.trim();
        let reported_host = reported_host.trim();
        if serial.is_empty() || reported_host.is_empty() {
            return None;
        }
        let key = normalize_serial(serial);
        let suggested_host = configured_host
            .filter(|host| host.trim() != reported_host)
            .map(|_| reported_host.to_owned());
        let presence = self
            .entries
            .entry(key.clone())
            .or_insert_with(|| PrinterPresence {
                serial: serial.to_owned(),
                host: reported_host.to_owned(),
                suggested_host: suggested_host.clone(),
                model: model.to_owned(),
                firmware: firmware.map(str::to_owned),
                schema_version: None,
                connect_mode: None,
                bind_state: None,
                security_mode: None,
                interface: None,
                last_seen_unix_ms: now_unix_ms,
            });
        presence.host = reported_host.to_owned();
        presence.suggested_host = suggested_host;
        if !model.trim().is_empty() {
            presence.model = model.to_owned();
        }
        if let Some(firmware) = firmware.filter(|firmware| !firmware.trim().is_empty()) {
            presence.firmware = Some(firmware.to_owned());
        }
        presence.last_seen_unix_ms = now_unix_ms;
        self.entries.get(&key)
    }

    pub fn entries(&self) -> impl Iterator<Item = &PrinterPresence> {
        self.entries.values()
    }

    pub fn retain_seen_since(&mut self, minimum_unix_ms: u64) {
        self.entries
            .retain(|_, presence| presence.last_seen_unix_ms >= minimum_unix_ms);
    }
}

fn normalize_serial(serial: &str) -> String {
    serial.trim().to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_changes_are_suggestions_not_profile_rewrites() {
        let printer = DiscoveredPrinter {
            driver: "bambu-lan",
            host: "192.0.2.11".into(),
            serial: "SN001".into(),
            display_model: "Bambu Lab P1S".into(),
            model: "P1S".into(),
            name: "Printer".into(),
            firmware: None,
            schema_version: None,
            connect_mode: None,
            bind_state: None,
            security_mode: None,
            interface: None,
        };
        let mut cache = PresenceCache::default();
        let presence = cache.observe(&printer, Some("192.0.2.10"), 42).unwrap();
        assert_eq!(presence.suggested_host.as_deref(), Some("192.0.2.11"));
        assert_eq!(presence.last_seen_unix_ms, 42);
        assert_eq!(cache.get(" sn001 ").unwrap().host, "192.0.2.11");
    }

    #[test]
    fn stale_presence_can_be_expired_without_touching_profiles() {
        let printer = DiscoveredPrinter {
            driver: "bambu-lan",
            host: "192.0.2.11".into(),
            serial: "SN001".into(),
            display_model: "Bambu Lab P1S".into(),
            model: "P1S".into(),
            name: "Printer".into(),
            firmware: None,
            schema_version: None,
            connect_mode: None,
            bind_state: None,
            security_mode: None,
            interface: None,
        };
        let mut cache = PresenceCache::default();
        cache.observe(&printer, Some("192.0.2.10"), 42);
        cache.retain_seen_since(43);
        assert!(cache.get("SN001").is_none());
    }

    #[test]
    fn status_addresses_are_advisory_and_preserve_discovery_metadata() {
        let printer = DiscoveredPrinter {
            driver: "bambu-lan",
            host: "192.0.2.10".into(),
            serial: "SN001".into(),
            display_model: "Bambu Lab P1S".into(),
            model: "P1S".into(),
            name: "Printer".into(),
            firmware: Some("01.00.00.00".into()),
            schema_version: Some("1.2".into()),
            connect_mode: Some("lan".into()),
            bind_state: None,
            security_mode: None,
            interface: Some("wlan0".into()),
        };
        let mut cache = PresenceCache::default();
        cache.observe(&printer, Some("printer.local"), 41);

        let presence = cache
            .observe_status_address(
                "sn001",
                "192.0.2.11",
                Some("printer.local"),
                "P1S",
                Some("01.01.00.00"),
                42,
            )
            .unwrap();

        assert_eq!(presence.host, "192.0.2.11");
        assert_eq!(presence.suggested_host.as_deref(), Some("192.0.2.11"));
        assert_eq!(presence.firmware.as_deref(), Some("01.01.00.00"));
        assert_eq!(presence.schema_version.as_deref(), Some("1.2"));
        assert_eq!(presence.interface.as_deref(), Some("wlan0"));
        assert_eq!(presence.last_seen_unix_ms, 42);
    }
}
