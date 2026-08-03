//! Bambu Lab LAN profile validation and authenticated LAN transports.

use std::{
    io::{self, Read, Write},
    net::{IpAddr, TcpStream},
    str::FromStr,
    time::{Duration, Instant},
};

use openssl::ssl::SslStream;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::trace::{SharedTracer, next_tracer_generation};

mod discovery;
mod mapping;
mod names;
mod package;
mod rtsp;
mod transport;
mod tunnel;

pub use discovery::{DiscoveredPrinter, DiscoveryError, discover};
pub use mapping::{FilamentAssignment, FilamentMapping, MappingStatus, reconcile_filaments};
pub use names::{JobNames, NameError, derive_job_names, validate_remote_filename};
pub use package::{
    FilamentRequirement, PackageError, PackageIssue, PackageIssueSeverity, PlateManifest,
    PrintPackage, inspect_print_package,
};
pub use rtsp::H264Stream;
pub use transport::{Client, Error as TransportError, JobStartOptions};

pub const MQTT_PORT: u16 = 8883;
pub const MQTT_USERNAME: &str = "bblp";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const CAMERA_PORT: u16 = 6000;
const CAMERA_AUTH_SIZE: usize = 80;
const CAMERA_FRAME_HEADER_SIZE: usize = 16;
const CAMERA_MAX_FRAME_SIZE: u32 = 1 << 20;
const MJPEG_BOUNDARY: &str = "frame";

#[derive(Clone, Debug)]
pub struct Profile {
    host: String,
    serial: String,
    model: String,
    insecure: bool,
    timeout: Duration,
    tracer: Option<SharedTracer>,
    tracer_generation: u64,
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
            model: String::new(),
            insecure,
            timeout,
            tracer: None,
            tracer_generation: 0,
        })
    }

    pub fn with_tracer(mut self, tracer: SharedTracer) -> Self {
        self.tracer = Some(tracer);
        self.tracer_generation = next_tracer_generation();
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into().trim().to_owned();
        self
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn model_family(&self) -> ModelFamily {
        ModelFamily::from_model(&self.model)
    }

    pub fn default_capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities::for_model(&self.model)
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

    pub(crate) fn connection_identity(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> [u8; 32] {
        let mut identity = Sha256::new();
        for component in [
            self.host.as_str(),
            self.serial.as_str(),
            self.model.as_str(),
            if self.insecure { "insecure" } else { "pinned" },
            fingerprint.unwrap_or_default(),
            access_code.unwrap_or_default(),
        ] {
            identity.update(component.len().to_be_bytes());
            identity.update(component.as_bytes());
        }
        identity.update(self.timeout.as_nanos().to_be_bytes());
        identity.update(self.tracer_generation.to_be_bytes());
        identity.finalize().into()
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelFamily {
    A1,
    A2,
    P1,
    P2,
    X1,
    X2,
    H2,
    #[default]
    Unknown,
}

impl ModelFamily {
    pub fn from_model(model: &str) -> Self {
        let model = model
            .trim()
            .to_ascii_uppercase()
            .replace(['-', '_', ' '], "");
        if model.starts_with("A1") {
            Self::A1
        } else if model.starts_with("A2") {
            Self::A2
        } else if model.starts_with("P1") {
            Self::P1
        } else if model.starts_with("P2") {
            Self::P2
        } else if model.starts_with("X1") {
            Self::X1
        } else if model.starts_with("X2") {
            Self::X2
        } else if model.starts_with("H2") {
            Self::H2
        } else {
            Self::Unknown
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthorizationMode {
    #[default]
    Unknown,
    DeveloperMode,
    SigningRequired,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CameraTransport {
    #[default]
    Unknown,
    MjpegTls,
    RtspsH264,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageTransport {
    #[default]
    Unknown,
    Ftps,
    Tunnel6000,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageVolume {
    SdCard,
    Emmc,
    UsbDisk,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BedLevelingSupport {
    #[default]
    Unknown,
    Unsupported,
    Toggle,
    AutomaticOrToggle,
}

/// Model-derived hints. Live observations may refine these values, but an
/// unknown model deliberately remains unknown instead of over-advertising.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilities {
    pub model_family: ModelFamily,
    pub authorization: AuthorizationMode,
    pub camera: CameraTransport,
    pub storage_transport: StorageTransport,
    pub storage_volumes: Vec<StorageVolume>,
    pub extruder_count: Option<u8>,
    pub ams_supported: Option<bool>,
    pub mqtt_alive_supported: Option<bool>,
    pub send_to_storage_supported: Option<bool>,
    pub flow_calibration_supported: Option<bool>,
    pub timelapse_supported: Option<bool>,
    pub ams_humidity_supported: Option<bool>,
    pub bed_leveling: BedLevelingSupport,
    pub nozzle_temperature_range: Option<[i32; 2]>,
    pub bed_temperature_range: Option<[i32; 2]>,
    pub firmware_version: Option<String>,
}

impl RuntimeCapabilities {
    pub fn for_model(model: &str) -> Self {
        let normalized = model
            .trim()
            .to_ascii_uppercase()
            .replace(['-', '_', ' '], "");
        let model_family = ModelFamily::from_model(&normalized);
        let (camera, storage_transport, storage_volumes, extruder_count) = match model_family {
            ModelFamily::A1 | ModelFamily::A2 => (
                CameraTransport::MjpegTls,
                StorageTransport::Ftps,
                vec![StorageVolume::SdCard],
                Some(1),
            ),
            ModelFamily::P1 => (
                if normalized.starts_with("P1S") {
                    CameraTransport::RtspsH264
                } else {
                    CameraTransport::MjpegTls
                },
                StorageTransport::Ftps,
                vec![StorageVolume::SdCard],
                Some(1),
            ),
            ModelFamily::X1 => (
                CameraTransport::RtspsH264,
                StorageTransport::Ftps,
                vec![StorageVolume::SdCard],
                Some(1),
            ),
            ModelFamily::P2 | ModelFamily::X2 | ModelFamily::H2 => (
                CameraTransport::RtspsH264,
                StorageTransport::Tunnel6000,
                vec![StorageVolume::Emmc, StorageVolume::UsbDisk],
                if normalized.starts_with("H2D") {
                    Some(2)
                } else if normalized.starts_with("H2C") || normalized.starts_with("X2D") {
                    None
                } else {
                    Some(1)
                },
            ),
            ModelFamily::Unknown => (
                CameraTransport::Unknown,
                StorageTransport::Unknown,
                Vec::new(),
                None,
            ),
        };
        Self {
            model_family,
            authorization: AuthorizationMode::Unknown,
            camera,
            storage_transport,
            storage_volumes,
            extruder_count,
            ams_supported: (model_family != ModelFamily::Unknown).then_some(true),
            mqtt_alive_supported: None,
            send_to_storage_supported: None,
            flow_calibration_supported: None,
            timelapse_supported: None,
            ams_humidity_supported: None,
            bed_leveling: BedLevelingSupport::Unknown,
            nozzle_temperature_range: None,
            bed_temperature_range: None,
            firmware_version: None,
        }
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
    Tls,
    #[error("Bambu camera authentication failed")]
    Authentication(#[source] io::Error),
    #[error("Bambu camera frame is invalid")]
    InvalidFrame,
    #[error("Bambu camera stream failed")]
    Stream(#[source] io::Error),
}

/// A real Bambu LAN camera stream, converted from whichever protocol the
/// printer actually speaks into the standard MJPEG multipart response used
/// by browser image tags. H/X-series printers only serve their camera over
/// RTSPS/H.264 on port 322; A1/A1 mini serve the classic framed-TLS MJPEG
/// protocol on port 6000. `open_mjpeg_stream` probes for the former first
/// and falls back to the latter, so callers never need to know which one a
/// given printer uses.
pub struct MjpegStream {
    source: MjpegSource,
    pending: Vec<u8>,
}

enum MjpegSource {
    Classic(SslStream<TcpStream>),
    H264(Box<rtsp::H264Stream>),
}

impl MjpegStream {
    /// Returns a socket handle that can interrupt a blocked stream read.
    pub fn shutdown_handle(&self) -> io::Result<TcpStream> {
        match &self.source {
            MjpegSource::Classic(connection) => connection.get_ref().try_clone(),
            MjpegSource::H264(stream) => stream.shutdown_handle(),
        }
    }

    fn next_frame(&mut self) -> io::Result<Vec<u8>> {
        match &mut self.source {
            MjpegSource::Classic(connection) => read_camera_frame(connection),
            MjpegSource::H264(stream) => stream.next_jpeg_frame(),
        }
    }
}

impl Read for MjpegStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.pending.is_empty() {
            let frame = self.next_frame()?;
            self.pending = multipart_frame(&frame);
        }
        let count = buffer.len().min(self.pending.len());
        buffer[..count].copy_from_slice(&self.pending[..count]);
        self.pending.drain(..count);
        Ok(count)
    }
}

/// Connects to the Bambu LAN camera endpoint and returns its live stream.
///
/// No stream is synthesized: a successful return means the printer accepted
/// the camera connection and its authentication. RTSPS (port 322, H.264) is
/// tried first since that's what H/X-series printers exclusively serve; the
/// classic MJPEG protocol (port 6000) is only tried as a fallback, and only
/// when the RTSPS attempt failed for a reason other than a TLS fingerprint
/// mismatch (a pin failure is a security signal, not evidence the printer
/// doesn't support RTSPS, so it must not be silently swallowed by a retry).
pub fn open_mjpeg_stream(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<MjpegStream, CameraError> {
    let access_code = access_code
        .filter(|value| !value.is_empty())
        .ok_or(CameraError::MissingAccessCode)?;
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| CameraError::Connect(io::Error::from(io::ErrorKind::TimedOut)))?;

    // Discovery gives us a strong ordering hint. Unknown models still probe
    // RTSPS first, but both attempts share one caller-supplied deadline.
    if profile.default_capabilities().camera != CameraTransport::MjpegTls {
        match rtsp::open_decoded_h264_stream(
            profile,
            access_code,
            fingerprint,
            camera_time_remaining(deadline)?,
        ) {
            Ok(stream) => {
                return Ok(MjpegStream {
                    source: MjpegSource::H264(Box::new(stream)),
                    pending: Vec::new(),
                });
            }
            Err(CameraError::Pin(error)) => return Err(CameraError::Pin(error)),
            Err(_) => {}
        }
    }

    let mut connection =
        open_camera_connection(profile, fingerprint, camera_time_remaining(deadline)?)?;
    send_camera_auth(&mut connection, access_code).map_err(CameraError::Authentication)?;
    Ok(MjpegStream {
        source: MjpegSource::Classic(connection),
        pending: Vec::new(),
    })
}

fn camera_time_remaining(deadline: Instant) -> Result<Duration, CameraError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| CameraError::Connect(io::Error::from(io::ErrorKind::TimedOut)))
}

/// Opens the native H.264/RTP camera stream without decoding it.
pub fn open_h264_stream(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<H264Stream, CameraError> {
    let access_code = access_code
        .filter(|value| !value.is_empty())
        .ok_or(CameraError::MissingAccessCode)?;
    rtsp::open_h264_stream(profile, access_code, fingerprint, timeout)
}

/// Captures one actual JPEG frame from the Bambu LAN camera endpoint.
pub fn snapshot(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<Vec<u8>, CameraError> {
    let mut stream = open_mjpeg_stream(profile, access_code, fingerprint, timeout)?;
    stream.next_frame().map_err(|error| match error.kind() {
        io::ErrorKind::InvalidData => CameraError::InvalidFrame,
        _ => CameraError::Stream(error),
    })
}

fn open_camera_connection(
    profile: &Profile,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<SslStream<TcpStream>, CameraError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| CameraError::Connect(io::Error::from(io::ErrorKind::TimedOut)))?;
    let connector = transport::tls_connector().map_err(|_| CameraError::Tls)?;
    transport::open_tls(
        &connector,
        profile,
        CAMERA_PORT,
        fingerprint,
        true,
        deadline,
    )
    .map(|(stream, _)| stream)
    .map_err(|error| match error {
        TransportError::Pin(error) => CameraError::Pin(error),
        TransportError::MissingCertificate => CameraError::MissingCertificate,
        TransportError::Tls => CameraError::Tls,
        TransportError::Timeout => CameraError::Connect(io::Error::from(io::ErrorKind::TimedOut)),
        _ => CameraError::Connect(io::Error::from(io::ErrorKind::ConnectionRefused)),
    })
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
    fn classifies_supported_model_families_without_exact_model_matches() {
        for (model, family) in [
            ("A1 mini", ModelFamily::A1),
            ("A2L", ModelFamily::A2),
            ("P1S", ModelFamily::P1),
            ("P2S", ModelFamily::P2),
            ("X1 Carbon", ModelFamily::X1),
            ("X2C", ModelFamily::X2),
            ("H2D", ModelFamily::H2),
            ("future-printer", ModelFamily::Unknown),
        ] {
            assert_eq!(ModelFamily::from_model(model), family, "{model}");
        }
    }

    #[test]
    fn derives_conservative_transport_hints_from_the_model() {
        let p1p = RuntimeCapabilities::for_model("P1P");
        assert_eq!(p1p.camera, CameraTransport::MjpegTls);
        assert_eq!(p1p.storage_transport, StorageTransport::Ftps);

        let h2d = RuntimeCapabilities::for_model("H2D");
        assert_eq!(h2d.camera, CameraTransport::RtspsH264);
        assert_eq!(h2d.storage_transport, StorageTransport::Tunnel6000);
        assert_eq!(h2d.extruder_count, Some(2));

        assert_eq!(RuntimeCapabilities::for_model("H2C").extruder_count, None);
        assert_eq!(RuntimeCapabilities::for_model("X2D").extruder_count, None);

        let unknown = RuntimeCapabilities::for_model("");
        assert_eq!(unknown.model_family, ModelFamily::Unknown);
        assert_eq!(unknown.camera, CameraTransport::Unknown);
        assert!(unknown.storage_volumes.is_empty());
        assert_eq!(unknown.ams_supported, None);
    }

    #[test]
    fn model_changes_connection_identity() {
        let p1 = Profile::new("printer.local", "SN001", false)
            .unwrap()
            .with_model("P1S");
        let h2 = Profile::new("printer.local", "SN001", false)
            .unwrap()
            .with_model("H2D");

        assert_ne!(
            p1.connection_identity(Some("code"), None),
            h2.connection_identity(Some("code"), None)
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
