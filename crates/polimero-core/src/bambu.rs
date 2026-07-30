//! Bambu Lab LAN profile validation and authenticated LAN transports.

use std::{
    io::{self, Read, Write},
    net::{IpAddr, TcpStream, ToSocketAddrs},
    str::FromStr,
    time::Duration,
};

use native_tls::{TlsConnector, TlsStream};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

mod transport;

pub use transport::{Client, Error as TransportError, JobStartOptions};

pub const MQTT_PORT: u16 = 8883;
pub const MQTT_USERNAME: &str = "bblp";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const CAMERA_PORT: u16 = 6000;
const CAMERA_AUTH_SIZE: usize = 80;
const CAMERA_FRAME_HEADER_SIZE: usize = 16;
const CAMERA_MAX_FRAME_SIZE: u32 = 1 << 20;
const MJPEG_BOUNDARY: &str = "frame";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile {
    host: String,
    serial: String,
    insecure: bool,
    timeout: Duration,
}

impl Profile {
    pub fn new(
        host: impl Into<String>,
        serial: impl Into<String>,
        insecure: bool,
    ) -> Result<Self, ProfileError> {
        Self::with_timeout(host, serial, insecure, DEFAULT_TIMEOUT)
    }

    pub fn with_timeout(
        host: impl Into<String>,
        serial: impl Into<String>,
        insecure: bool,
        timeout: Duration,
    ) -> Result<Self, ProfileError> {
        let host = host.into();
        let serial = serial.into();
        validate_profile(&host, &serial)?;
        if timeout.is_zero() {
            return Err(ProfileError::InvalidTimeout);
        }
        Ok(Self {
            host,
            serial,
            insecure,
            timeout,
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

    pub fn timeout(&self) -> Duration {
        self.timeout
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
    #[error("Bambu LAN timeout must be greater than zero")]
    InvalidTimeout,
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

#[derive(Debug, Error)]
pub enum CameraError {
    #[error("Bambu camera access code is missing")]
    MissingAccessCode,
    #[error("Bambu camera TLS certificate is unavailable")]
    MissingCertificate,
    #[error("Bambu camera TLS fingerprint verification failed")]
    Pin(#[from] TlsPinError),
    #[error("Bambu camera connection failed")]
    Connect(#[source] io::Error),
    #[error("Bambu camera TLS handshake failed")]
    Tls(#[source] native_tls::Error),
    #[error("Bambu camera authentication failed")]
    Authentication(#[source] io::Error),
    #[error("Bambu camera frame is invalid")]
    InvalidFrame,
    #[error("Bambu camera stream failed")]
    Stream(#[source] io::Error),
}

/// A real Bambu LAN MJPEG stream, converted from the printer's framed TLS
/// protocol into the standard multipart response used by browser image tags.
pub struct MjpegStream {
    connection: TlsStream<TcpStream>,
    pending: Vec<u8>,
}

impl Read for MjpegStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            let frame = read_camera_frame(&mut self.connection)?;
            self.pending = multipart_frame(&frame);
        }
        let count = buffer.len().min(self.pending.len());
        buffer[..count].copy_from_slice(&self.pending[..count]);
        self.pending.drain(..count);
        Ok(count)
    }
}

/// Connects to the Bambu LAN MJPEG camera endpoint and returns its live stream.
///
/// No stream is synthesized: a successful return means the printer accepted the
/// camera TLS connection and its authentication packet.
pub fn open_mjpeg_stream(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<MjpegStream, CameraError> {
    let access_code = access_code
        .filter(|value| !value.is_empty())
        .ok_or(CameraError::MissingAccessCode)?;
    let mut connection = open_camera_connection(profile, fingerprint, timeout)?;
    send_camera_auth(&mut connection, access_code).map_err(CameraError::Authentication)?;
    Ok(MjpegStream {
        connection,
        pending: Vec::new(),
    })
}

/// Captures one actual JPEG frame from the Bambu LAN MJPEG endpoint.
pub fn snapshot(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<Vec<u8>, CameraError> {
    let mut stream = open_mjpeg_stream(profile, access_code, fingerprint, timeout)?;
    read_camera_frame(&mut stream.connection).map_err(|error| match error.kind() {
        io::ErrorKind::InvalidData => CameraError::InvalidFrame,
        _ => CameraError::Stream(error),
    })
}

fn open_camera_connection(
    profile: &Profile,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<TlsStream<TcpStream>, CameraError> {
    let address = (profile.host(), CAMERA_PORT)
        .to_socket_addrs()
        .map_err(CameraError::Connect)?
        .next()
        .ok_or_else(|| CameraError::Connect(io::Error::other("camera host has no address")))?;
    let connection = TcpStream::connect_timeout(&address, timeout).map_err(CameraError::Connect)?;
    connection
        .set_read_timeout(Some(timeout))
        .map_err(CameraError::Connect)?;
    connection
        .set_write_timeout(Some(timeout))
        .map_err(CameraError::Connect)?;
    let connector = TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(CameraError::Tls)?;
    let connection =
        connector
            .connect(profile.serial(), connection)
            .map_err(|error| match error {
                native_tls::HandshakeError::Failure(error) => CameraError::Tls(error),
                native_tls::HandshakeError::WouldBlock(_) => CameraError::Connect(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "TLS handshake blocked",
                )),
            })?;
    if !profile.insecure() {
        let certificate = connection
            .peer_certificate()
            .map_err(CameraError::Tls)?
            .ok_or(CameraError::MissingCertificate)?;
        verify_tls_fingerprint(
            false,
            fingerprint.unwrap_or_default(),
            &certificate.to_der().map_err(CameraError::Tls)?,
        )?;
    }
    Ok(connection)
}

fn send_camera_auth(connection: &mut impl Write, access_code: &str) -> io::Result<()> {
    let mut packet = [0; CAMERA_AUTH_SIZE];
    packet[0] = 0x40;
    packet[5] = 0x30;
    packet[16..48].copy_from_slice(&padded_camera_credential(MQTT_USERNAME));
    packet[48..80].copy_from_slice(&padded_camera_credential(access_code));
    connection.write_all(&packet)
}

fn padded_camera_credential(value: &str) -> [u8; 32] {
    let mut bytes = [0; 32];
    let length = value.len().min(bytes.len());
    bytes[..length].copy_from_slice(&value.as_bytes()[..length]);
    bytes
}

fn read_camera_frame(connection: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut header = [0; CAMERA_FRAME_HEADER_SIZE];
    connection.read_exact(&mut header)?;
    let size = u32::from_le_bytes(header[..4].try_into().expect("fixed frame header"));
    if size == 0 || size > CAMERA_MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid camera frame size",
        ));
    }
    let mut frame = vec![0; size as usize];
    connection.read_exact(&mut frame)?;
    Ok(frame)
}

fn multipart_frame(frame: &[u8]) -> Vec<u8> {
    let mut part = format!(
        "--{MJPEG_BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        frame.len()
    )
    .into_bytes();
    part.extend_from_slice(frame);
    part.extend_from_slice(b"\r\n");
    part
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

    #[test]
    fn camera_auth_packet_uses_bambu_wire_layout() {
        let mut packet = Vec::new();
        send_camera_auth(&mut packet, "secret").unwrap();

        assert_eq!(packet.len(), CAMERA_AUTH_SIZE);
        assert_eq!(&packet[..8], &[0x40, 0, 0, 0, 0, 0x30, 0, 0]);
        assert_eq!(&packet[16..20], b"bblp");
        assert_eq!(&packet[48..54], b"secret");
        assert!(packet[54..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn camera_frames_are_bounded_and_reemitted_as_mjpeg() {
        let jpeg = b"jpeg";
        let mut wire = Vec::new();
        wire.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
        wire.extend_from_slice(&[0; CAMERA_FRAME_HEADER_SIZE - 4]);
        wire.extend_from_slice(jpeg);

        let frame = read_camera_frame(&mut io::Cursor::new(wire)).unwrap();
        assert_eq!(frame, jpeg);
        assert_eq!(
            multipart_frame(&frame),
            b"--frame\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\n\r\njpeg\r\n"
        );

        let err =
            read_camera_frame(&mut io::Cursor::new([0; CAMERA_FRAME_HEADER_SIZE])).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
