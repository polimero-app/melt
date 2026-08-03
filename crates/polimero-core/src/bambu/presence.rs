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
        self.entries.insert(printer.serial.clone(), presence);
        self.entries.get(&printer.serial)
    }

    pub fn get(&self, serial: &str) -> Option<&PrinterPresence> {
        self.entries.get(serial)
    }
    pub fn entries(&self) -> impl Iterator<Item = &PrinterPresence> {
        self.entries.values()
    }
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
    }
}
