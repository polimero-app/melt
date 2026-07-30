//! Bambu Lab LAN profile validation and MQTT protocol primitives.
//!
//! This module intentionally implements neither MQTT/TLS transport nor printer
//! commands. Transport code can use these values when one is available.

use std::{net::IpAddr, str::FromStr};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MQTT_PORT: u16 = 8883;
pub const MQTT_USERNAME: &str = "bblp";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile {
    host: String,
    serial: String,
    insecure: bool,
}

impl Profile {
    pub fn new(
        host: impl Into<String>,
        serial: impl Into<String>,
        insecure: bool,
    ) -> Result<Self, ProfileError> {
        let host = host.into();
        let serial = serial.into();
        validate_profile(&host, &serial)?;
        Ok(Self {
            host,
            serial,
            insecure,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    pub fn insecure(&self) -> bool {
        self.insecure
    }

    pub fn mqtt_endpoint(&self) -> String {
        mqtt_endpoint(&self.host)
    }

    pub fn mqtt_topics(&self) -> MqttTopics {
        MqttTopics::for_serial(&self.serial)
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ProfileError {
    #[error("Bambu LAN host is required")]
    MissingHost,
    #[error("invalid Bambu LAN host: expected a bare hostname or IP address")]
    InvalidHost,
    #[error("Bambu LAN serial is required")]
    MissingSerial,
    #[error("Bambu LAN serial is too long (maximum 64 bytes)")]
    SerialTooLong,
    #[error("Bambu LAN serial must be printable ASCII without whitespace")]
    InvalidSerial,
}

pub fn validate_profile(host: &str, serial: &str) -> Result<(), ProfileError> {
    validate_host(host)?;
    validate_serial(serial)
}

pub fn validate_host(host: &str) -> Result<(), ProfileError> {
    if host.is_empty() {
        return Err(ProfileError::MissingHost);
    }
    if host.trim() != host
        || host.chars().any(char::is_whitespace)
        || host.contains("://")
        || host.contains(['/', '@', '[', ']'])
    {
        return Err(ProfileError::InvalidHost);
    }
    if IpAddr::from_str(host).is_ok() {
        return Ok(());
    }
    if host.contains(':') || host.len() > 253 || !valid_hostname(host) {
        return Err(ProfileError::InvalidHost);
    }
    Ok(())
}

fn valid_hostname(host: &str) -> bool {
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

pub fn validate_serial(serial: &str) -> Result<(), ProfileError> {
    if serial.is_empty() {
        return Err(ProfileError::MissingSerial);
    }
    if serial.len() > 64 {
        return Err(ProfileError::SerialTooLong);
    }
    if !serial.bytes().all(|byte| (b'!'..=b'~').contains(&byte)) {
        return Err(ProfileError::InvalidSerial);
    }
    Ok(())
}

/// Formats the fixed Bambu MQTT endpoint, including brackets for IPv6 literals.
pub fn mqtt_endpoint(host: &str) -> String {
    if host
        .parse::<IpAddr>()
        .is_ok_and(|address| address.is_ipv6())
    {
        format!("[{host}]:{MQTT_PORT}")
    } else {
        format!("{host}:{MQTT_PORT}")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MqttTopics {
    pub request: String,
    pub report: String,
}

impl MqttTopics {
    pub fn for_serial(serial: &str) -> Self {
        Self {
            request: format!("device/{serial}/request"),
            report: format!("device/{serial}/report"),
        }
    }
}

/// Builds Bambu's full-status request. The caller owns sequence allocation.
pub fn pushall_payload(sequence_id: u64) -> String {
    serde_json::to_string(&Pushall {
        pushing: Pushing {
            sequence_id: sequence_id.to_string(),
            command: "pushall",
            version: 1,
            push_target: 1,
        },
    })
    .expect("serializing a fixed Bambu pushall payload cannot fail")
}

/// Extracts the sequence ID from a known Bambu command envelope.
///
/// Invalid or unrelated JSON is deliberately ignored, matching the Go driver:
/// it must not be mistaken for a response to the active request.
pub fn payload_sequence_id(payload: &[u8]) -> Option<String> {
    let envelope: Envelope = serde_json::from_slice(payload).ok()?;
    envelope
        .print
        .or(envelope.system)
        .or(envelope.pushing)
        .and_then(|command| command.sequence_id)
}

pub fn is_pushall_payload(payload: &[u8]) -> bool {
    serde_json::from_slice::<Envelope>(payload)
        .ok()
        .and_then(|envelope| envelope.pushing)
        .is_some_and(|pushing| pushing.command.as_deref() == Some("pushall"))
}

const TLS_FINGERPRINT_PREFIX: &str = "sha256:";

/// Returns whether a fingerprint follows Polimero's pinned-certificate format.
pub fn is_valid_tls_fingerprint(fingerprint: &str) -> bool {
    fingerprint.len() == TLS_FINGERPRINT_PREFIX.len() + 64
        && fingerprint.starts_with(TLS_FINGERPRINT_PREFIX)
        && fingerprint[TLS_FINGERPRINT_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub fn tls_fingerprint(certificate_der: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(certificate_der))
}

/// Verifies a presented certificate after a transport completes its handshake.
///
/// Bambu LAN certificates are normally self-signed, so a TLS transport must
/// disable platform CA verification and call this pin check instead.
pub fn verify_tls_fingerprint(
    insecure: bool,
    expected: &str,
    certificate_der: &[u8],
) -> Result<(), TlsPinError> {
    if insecure {
        return Ok(());
    }
    if !is_valid_tls_fingerprint(expected) {
        return Err(TlsPinError::MissingOrInvalid);
    }
    if tls_fingerprint(certificate_der) != expected {
        return Err(TlsPinError::Mismatch);
    }
    Ok(())
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TlsPinError {
    #[error("TLS fingerprint is missing or invalid")]
    MissingOrInvalid,
    #[error("TLS fingerprint does not match the printer certificate")]
    Mismatch,
}

#[derive(Deserialize)]
struct Envelope {
    print: Option<CommandEnvelope>,
    system: Option<CommandEnvelope>,
    pushing: Option<CommandEnvelope>,
}

#[derive(Deserialize)]
struct CommandEnvelope {
    sequence_id: Option<String>,
    command: Option<String>,
}

#[derive(Serialize)]
struct Pushall {
    pushing: Pushing,
}

#[derive(Serialize)]
struct Pushing {
    sequence_id: String,
    command: &'static str,
    version: u8,
    push_target: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_bare_hosts_and_printable_serials() {
        for host in ["192.168.1.5", "printer.local", "printer", "2001:db8::1"] {
            assert!(validate_profile(host, "01S09C450100XXX").is_ok(), "{host}");
        }
    }

    #[test]
    fn rejects_bambu_hosts_that_would_change_the_fixed_endpoint() {
        for host in [
            "",
            "https://printer.local",
            "printer.local:8883",
            "printer.local/api",
            "[::1]",
            "printer local",
            "-printer.local",
        ] {
            assert!(validate_host(host).is_err(), "{host}");
        }
    }

    #[test]
    fn rejects_missing_or_unsafe_serials() {
        assert_eq!(validate_serial(""), Err(ProfileError::MissingSerial));
        assert_eq!(
            validate_serial("serial with whitespace"),
            Err(ProfileError::InvalidSerial)
        );
        assert_eq!(
            validate_serial(&"a".repeat(65)),
            Err(ProfileError::SerialTooLong)
        );
    }

    #[test]
    fn formats_fixed_endpoint_and_topics() {
        let profile = Profile::new("2001:db8::1", "SN001", false).unwrap();
        assert_eq!(profile.mqtt_endpoint(), "[2001:db8::1]:8883");
        assert_eq!(
            profile.mqtt_topics(),
            MqttTopics {
                request: "device/SN001/request".into(),
                report: "device/SN001/report".into(),
            }
        );
    }

    #[test]
    fn builds_and_recognizes_pushall_payloads() {
        let payload = pushall_payload(42);
        assert_eq!(
            payload,
            r#"{"pushing":{"sequence_id":"42","command":"pushall","version":1,"push_target":1}}"#
        );
        assert_eq!(
            payload_sequence_id(payload.as_bytes()).as_deref(),
            Some("42")
        );
        assert!(is_pushall_payload(payload.as_bytes()));
    }

    #[test]
    fn only_extracts_sequence_ids_from_known_string_envelopes() {
        assert_eq!(
            payload_sequence_id(br#"{"system":{"sequence_id":"13","command":"ledctrl"}}"#)
                .as_deref(),
            Some("13")
        );
        assert_eq!(
            payload_sequence_id(br#"{"print":{"sequence_id":13}}"#),
            None
        );
        assert_eq!(payload_sequence_id(b"not json"), None);
    }

    #[test]
    fn validates_and_verifies_tls_pins() {
        let certificate = b"test certificate";
        let fingerprint = tls_fingerprint(certificate);
        assert!(is_valid_tls_fingerprint(&fingerprint));
        assert_eq!(
            verify_tls_fingerprint(false, &fingerprint, certificate),
            Ok(())
        );
        assert_eq!(
            verify_tls_fingerprint(false, &fingerprint, b"other certificate"),
            Err(TlsPinError::Mismatch)
        );
        assert_eq!(
            verify_tls_fingerprint(false, "", certificate),
            Err(TlsPinError::MissingOrInvalid)
        );
        assert_eq!(verify_tls_fingerprint(true, "", certificate), Ok(()));
    }
}
