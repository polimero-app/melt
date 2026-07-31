use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File},
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use native_tls::{HandshakeError, TlsConnector, TlsStream};
use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::moonraker::{
    FanResult, FileEntry, FileEntryType, FileList, FileRoot, Job, JobResult, LightResult,
    LightState, MotionResult, MotionState, PrinterState, Progress, SpeedResult, Status,
    StatusError, StatusWarning, Temperature, TemperatureResult, TemperatureTargets, Temperatures,
};

use super::{
    MQTT_USERNAME, MqttTopics, Profile, TlsPinError, is_pushall_payload, is_valid_tls_fingerprint,
    payload_sequence_id, pushall_payload, tls_fingerprint, verify_tls_fingerprint,
};

const FTP_PORT: u16 = 990;
const FILE_ROOT: &str = "sdcard";
const MAX_MQTT_PACKET_SIZE: usize = 8 << 20;
const MAX_FTP_REPLY_SIZE: usize = 64 << 10;
const MAX_FTP_LISTING_SIZE: u64 = 8 << 20;
const MAX_LIST_DEPTH: u8 = 32;
const PUSHALL_INTERVAL: Duration = Duration::from_secs(3);

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Error)]
pub enum Error {
    #[error("Bambu access code is required")]
    MissingAccessCode,
    #[error("invalid Bambu access code")]
    InvalidAccessCode,
    #[error("{0}")]
    Pin(#[from] TlsPinError),
    #[error("Bambu TLS certificate is unavailable")]
    MissingCertificate,
    #[error("Bambu connection failed")]
    Connection,
    #[error("Bambu TLS handshake failed")]
    Tls,
    #[error("Bambu MQTT authentication rejected")]
    Authentication,
    #[error("Bambu printer operation timed out")]
    Timeout,
    #[error("invalid response from Bambu printer")]
    InvalidResponse,
    #[error("Bambu printer rejected the command")]
    CommandRejected,
    #[error("Bambu printer rejected an unsigned command")]
    UnsignedCommand,
    #[error("invalid Bambu device path")]
    InvalidDevicePath,
    #[error("Bambu file already exists")]
    FileAlreadyExists,
    #[error("Bambu destination is a directory")]
    DirectoryDestination,
    #[error("Bambu file transfer failed")]
    FileTransfer,
    #[error("Bambu does not support {0}")]
    Unsupported(&'static str),
    #[error("invalid Bambu temperature target")]
    InvalidTemperatureTarget,
    #[error("invalid Bambu motion request")]
    InvalidMotion,
    #[error("invalid Bambu speed profile")]
    InvalidSpeedProfile,
    #[error("local file operation failed")]
    LocalIo,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct JobStartOptions {
    pub plate: Option<u32>,
    pub skip_leveling: bool,
}

/// Authenticated Bambu LAN operations for one validated profile.
#[derive(Clone, Debug)]
pub struct Client {
    profile: Profile,
}

impl Client {
    pub fn new(profile: Profile) -> Self {
        Self { profile }
    }

    /// Performs a TLS and MQTT authentication exchange before a profile is saved.
    ///
    /// A secure profile receives a leaf-certificate fingerprint for TOFU pinning;
    /// an explicitly insecure profile deliberately skips the network check.
    pub fn verify(&self, access_code: Option<&str>) -> Result<Option<String>, Error> {
        let access_code = valid_access_code(access_code)?;
        if self.profile.insecure() {
            return Ok(None);
        }

        let deadline = deadline_after(self.profile.timeout())?;
        let connector = tls_connector()?;
        let (stream, fingerprint) = open_tls(
            &connector,
            &self.profile,
            super::MQTT_PORT,
            None,
            false,
            deadline,
        )?;
        let mut mqtt = MqttConnection::new(stream, self.profile.mqtt_topics(), deadline);
        let result = mqtt.connect(access_code).map(|_| fingerprint);
        mqtt.disconnect();
        result.map(Some)
    }

    /// Captures the printer's leaf certificate without authenticating or sending
    /// a printer command. Callers must show and confirm the new pin before use.
    pub fn capture_fingerprint(&self) -> Result<String, Error> {
        let deadline = deadline_after(self.profile.timeout())?;
        let connector = tls_connector()?;
        let (_, fingerprint) = open_tls(
            &connector,
            &self.profile,
            super::MQTT_PORT,
            None,
            false,
            deadline,
        )?;
        Ok(fingerprint)
    }

    pub fn status(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<Status, Error> {
        let payload = pushall_payload(next_sequence());
        let report = self.exchange(access_code, fingerprint, payload, is_full_report)?;
        parse_status(&report)
    }

    pub fn job_start(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        device_path: &str,
        options: JobStartOptions,
    ) -> Result<JobResult, Error> {
        let path = normalize_device_path(device_path)?;
        if path == "/" {
            return Err(Error::InvalidDevicePath);
        }
        let payload = job_start_payload(&path, options)?;
        let report = self.exchange(access_code, fingerprint, payload, |report| {
            report_state_is(report, &[PrinterState::Printing])
        })?;
        Ok(JobResult {
            state: parse_status(&report)?.state,
        })
    }

    pub fn job_pause(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<JobResult, Error> {
        self.job_control(access_code, fingerprint, "pause", PrinterState::Paused)
    }

    pub fn job_resume(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<JobResult, Error> {
        self.job_control(access_code, fingerprint, "resume", PrinterState::Printing)
    }

    pub fn job_cancel(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<JobResult, Error> {
        self.job_control(access_code, fingerprint, "stop", PrinterState::Idle)
    }

    pub fn emergency_stop(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<(), Error> {
        self.publish_only(access_code, fingerprint, gcode_payload("M112"))
    }

    pub fn temperature_set(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        targets: TemperatureTargets,
    ) -> Result<TemperatureResult, Error> {
        validate_temperature_targets(&targets)?;
        if targets.nozzle_celsius.is_none()
            && targets.bed_celsius.is_none()
            && targets.chamber_celsius.is_none()
        {
            return Err(Error::InvalidTemperatureTarget);
        }

        let nozzle = targets.nozzle_celsius.map(f64::round);
        let bed = targets.bed_celsius.map(f64::round);
        let chamber = targets.chamber_celsius.map(f64::round);
        let mut lines = Vec::new();
        if let Some(value) = nozzle {
            lines.push(format!("M104 S{value:.0}"));
        }
        if let Some(value) = bed {
            lines.push(format!("M140 S{value:.0}"));
        }
        if let Some(value) = chamber {
            lines.push(format!("M141 S{value:.0}"));
        }
        let report = self.exchange(
            access_code,
            fingerprint,
            gcode_payload(&lines.join("\n")),
            |report| {
                let Ok(status) = parse_status_value(report) else {
                    return false;
                };
                target_matches(
                    status
                        .temperatures
                        .as_ref()
                        .and_then(|value| value.nozzle.as_ref()),
                    nozzle,
                ) && target_matches(
                    status
                        .temperatures
                        .as_ref()
                        .and_then(|value| value.bed.as_ref()),
                    bed,
                )
            },
        )?;
        let status = parse_status(&report)?;
        Ok(TemperatureResult {
            targets: TemperatureTargets {
                nozzle_celsius: nozzle.filter(|_| {
                    status
                        .temperatures
                        .as_ref()
                        .and_then(|value| value.nozzle.as_ref())
                        .and_then(|value| value.target_celsius)
                        .is_some()
                }),
                bed_celsius: bed.filter(|_| {
                    status
                        .temperatures
                        .as_ref()
                        .and_then(|value| value.bed.as_ref())
                        .and_then(|value| value.target_celsius)
                        .is_some()
                }),
                // Bambu does not report a portable chamber target in pushall.
                chamber_celsius: chamber,
            },
        })
    }

    pub fn motion_home(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        axes: &[crate::moonraker::Axis],
    ) -> Result<MotionResult, Error> {
        let mut command = String::from("G28");
        for axis in axes {
            command.push(' ');
            command.push(match axis {
                crate::moonraker::Axis::X => 'X',
                crate::moonraker::Axis::Y => 'Y',
                crate::moonraker::Axis::Z => 'Z',
            });
        }
        self.command_accepted(access_code, fingerprint, gcode_payload(&command))
    }

    pub fn motion_jog(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        delta: crate::moonraker::JogDelta,
    ) -> Result<MotionResult, Error> {
        validate_jog(delta)?;
        let mut movement = Vec::new();
        if let Some(value) = delta.x_millimeters {
            movement.push(format!("X{value:.3}"));
        }
        if let Some(value) = delta.y_millimeters {
            movement.push(format!("Y{value:.3}"));
        }
        if let Some(value) = delta.z_millimeters {
            movement.push(format!("Z{value:.3}"));
        }
        if movement.is_empty() {
            return Err(Error::InvalidMotion);
        }
        movement.push(format!("F{}", delta.feedrate_mm_per_min));
        self.command_accepted(
            access_code,
            fingerprint,
            gcode_payload(&format!("G91\nG1 {}\nG90", movement.join(" "))),
        )
    }

    pub fn fan_set(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        fan: &str,
        speed_percent: u8,
    ) -> Result<FanResult, Error> {
        if speed_percent > 100 {
            return Err(Error::Unsupported("fan speed above 100%"));
        }
        let prefix = match fan {
            "partCooling" => "M106",
            "auxiliary" => "M106 P2",
            "chamber" => "M106 P3",
            _ => return Err(Error::Unsupported("requested fan")),
        };
        let pwm = (u16::from(speed_percent) * 255 + 50) / 100;
        let report = self.exchange(
            access_code,
            fingerprint,
            gcode_payload(&format!("{prefix} S{pwm}")),
            |report| {
                parse_status_value(report)
                    .ok()
                    .and_then(|status| status.fans.get(fan).copied())
                    .is_some_and(|reported| reported.abs_diff(speed_percent) <= 4)
            },
        )?;
        let status = parse_status(&report)?;
        if !status.fans.contains_key(fan) && !status.fans.is_empty() {
            return Err(Error::Unsupported("requested fan on this printer model"));
        }
        Ok(FanResult {
            fan: fan.to_owned(),
            speed_percent,
        })
    }

    pub fn light_set(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        light: &str,
        state: LightState,
    ) -> Result<LightResult, Error> {
        if light != "chamber" {
            return Err(Error::Unsupported("requested light"));
        }
        let report = self.exchange(access_code, fingerprint, ledctrl_payload(state), |report| {
            light_state_is(report, "chamber_light", state)
                || light_unsupported_on_model(report, "chamber_light")
        })?;
        let report: Value = serde_json::from_slice(&report).map_err(|_| Error::InvalidResponse)?;
        if light_unsupported_on_model(&report, "chamber_light") {
            return Err(Error::Unsupported("requested light on this printer model"));
        }
        Ok(LightResult {
            light: light.to_owned(),
            state,
        })
    }

    pub fn speed_set(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        speed_profile: &str,
    ) -> Result<SpeedResult, Error> {
        let level = match speed_profile {
            "silent" => 1,
            "standard" => 2,
            "sport" => 3,
            "ludicrous" => 4,
            _ => return Err(Error::InvalidSpeedProfile),
        };
        let payload = json!({
            "print": {
                "sequence_id": next_sequence_id(),
                "command": "print_speed",
                "param": level.to_string(),
            }
        })
        .to_string();
        self.exchange(access_code, fingerprint, payload, |report| {
            report
                .get("print")
                .and_then(Value::as_object)
                .and_then(|print| integer(print.get("spd_lvl")))
                == Some(level)
        })?;
        Ok(SpeedResult {
            speed_profile: speed_profile.to_owned(),
        })
    }

    pub fn file_roots(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<Vec<FileRoot>, Error> {
        let mut ftp = FtpsConnection::open(&self.profile, access_code, fingerprint)?;
        let result = vec![FileRoot {
            name: FILE_ROOT,
            description: "Bambu SD card",
            writable: true,
            metadata: BTreeMap::new(),
        }];
        ftp.quit();
        Ok(result)
    }

    pub fn file_list(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        device_path: &str,
        recursive: bool,
    ) -> Result<FileList, Error> {
        let path = normalize_device_path(device_path)?;
        let mut ftp = FtpsConnection::open(&self.profile, access_code, fingerprint)?;
        let result = if recursive {
            ftp.list_recursive(&path)
        } else {
            ftp.list(&path)
        }
        .map(|entries| FileList { entries });
        ftp.quit();
        result
    }

    pub fn download_to(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        device_path: &str,
        destination: &mut dyn Write,
    ) -> Result<u64, Error> {
        let path = normalize_device_path(device_path)?;
        if path == "/" {
            return Err(Error::InvalidDevicePath);
        }
        let mut ftp = FtpsConnection::open(&self.profile, access_code, fingerprint)?;
        let result = ftp.download(&path, destination);
        ftp.quit();
        result
    }

    pub fn upload_file(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        source: &Path,
        device_path: &str,
        overwrite: bool,
    ) -> Result<u64, Error> {
        let metadata = fs::metadata(source).map_err(|_| Error::LocalIo)?;
        if !metadata.is_file() {
            return Err(Error::LocalIo);
        }
        let path = normalize_device_path(device_path)?;
        if path == "/" {
            return Err(Error::DirectoryDestination);
        }
        let mut ftp = FtpsConnection::open(&self.profile, access_code, fingerprint)?;
        let result = (|| {
            if !overwrite && ftp.file_exists(&path)? {
                return Err(Error::FileAlreadyExists);
            }
            let mut source = File::open(source).map_err(|_| Error::LocalIo)?;
            ftp.upload(&path, &mut source)
        })();
        ftp.quit();
        result
    }

    fn job_control(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        command: &str,
        state: PrinterState,
    ) -> Result<JobResult, Error> {
        let payload = json!({
            "print": {
                "sequence_id": next_sequence_id(),
                "command": command,
            }
        })
        .to_string();
        let report = self.exchange(access_code, fingerprint, payload, |report| {
            report_state_is(report, &[state])
        })?;
        Ok(JobResult {
            state: parse_status(&report)?.state,
        })
    }

    fn command_accepted(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        payload: String,
    ) -> Result<MotionResult, Error> {
        self.exchange(access_code, fingerprint, payload, is_full_report)?;
        Ok(MotionResult {
            state: MotionState::Accepted,
        })
    }

    fn publish_only(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        payload: String,
    ) -> Result<(), Error> {
        self.with_mqtt(access_code, fingerprint, |mqtt| mqtt.publish(&payload))
    }

    fn exchange(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        payload: String,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Vec<u8>, Error> {
        self.with_mqtt(access_code, fingerprint, |mqtt| {
            mqtt.exchange(payload, predicate)
        })
    }

    fn with_mqtt<T>(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        operation: impl FnOnce(&mut MqttConnection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let access_code = valid_access_code(access_code)?;
        validate_pin(&self.profile, fingerprint)?;
        let deadline = deadline_after(self.profile.timeout())?;
        let connector = tls_connector()?;
        let (stream, _) = open_tls(
            &connector,
            &self.profile,
            super::MQTT_PORT,
            fingerprint,
            true,
            deadline,
        )?;
        let mut mqtt = MqttConnection::new(stream, self.profile.mqtt_topics(), deadline);
        if let Err(error) = mqtt.connect(access_code).and_then(|_| mqtt.subscribe()) {
            mqtt.disconnect();
            return Err(error);
        }
        let result = operation(&mut mqtt);
        mqtt.disconnect();
        result
    }
}

fn validate_pin(profile: &Profile, fingerprint: Option<&str>) -> Result<(), Error> {
    if !profile.insecure() && !is_valid_tls_fingerprint(fingerprint.unwrap_or_default()) {
        return Err(Error::Pin(TlsPinError::MissingOrInvalid));
    }
    Ok(())
}

fn valid_access_code(access_code: Option<&str>) -> Result<&str, Error> {
    let access_code = access_code
        .filter(|value| !value.is_empty())
        .ok_or(Error::MissingAccessCode)?;
    if access_code.len() > u16::MAX as usize || access_code.chars().any(char::is_control) {
        return Err(Error::InvalidAccessCode);
    }
    Ok(access_code)
}

fn deadline_after(timeout: Duration) -> Result<Instant, Error> {
    Instant::now().checked_add(timeout).ok_or(Error::Timeout)
}

fn tls_connector() -> Result<TlsConnector, Error> {
    TlsConnector::builder()
        // Bambu LAN uses a self-signed leaf. The leaf is always pinned below
        // before credentials or data are sent unless the profile is explicit.
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|_| Error::Tls)
}

fn open_tls(
    connector: &TlsConnector,
    profile: &Profile,
    port: u16,
    expected_fingerprint: Option<&str>,
    verify_pin: bool,
    deadline: Instant,
) -> Result<(TlsStream<TcpStream>, String), Error> {
    let socket = resolve_and_connect(profile.host(), port, deadline)?;
    open_tls_socket(
        connector,
        profile,
        socket,
        expected_fingerprint,
        verify_pin,
        deadline,
    )
}

fn open_tls_socket(
    connector: &TlsConnector,
    profile: &Profile,
    socket: TcpStream,
    expected_fingerprint: Option<&str>,
    verify_pin: bool,
    deadline: Instant,
) -> Result<(TlsStream<TcpStream>, String), Error> {
    set_socket_timeout(&socket, deadline)?;
    let connection = connector
        .connect(profile.serial(), socket)
        .map_err(|error| match error {
            HandshakeError::Failure(_) => Error::Tls,
            HandshakeError::WouldBlock(_) => Error::Timeout,
        })?;
    let certificate = connection
        .peer_certificate()
        .map_err(|_| Error::Tls)?
        .ok_or(Error::MissingCertificate)?;
    let certificate = certificate.to_der().map_err(|_| Error::Tls)?;
    let actual_fingerprint = tls_fingerprint(&certificate);
    if verify_pin && !profile.insecure() {
        verify_tls_fingerprint(
            false,
            expected_fingerprint.unwrap_or_default(),
            &certificate,
        )?;
    }
    Ok((connection, actual_fingerprint))
}

fn resolve_and_connect(host: &str, port: u16, deadline: Instant) -> Result<TcpStream, Error> {
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(|_| Error::Connection)?;
    let mut last_error = None;
    for address in addresses {
        let remaining = remaining(deadline)?;
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => {
                set_socket_timeout(&stream, deadline)?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }
    match last_error {
        Some(error) if is_timeout(&error) => Err(Error::Timeout),
        Some(_) | None => Err(Error::Connection),
    }
}

fn connect_socket(address: SocketAddr, deadline: Instant) -> Result<TcpStream, Error> {
    let stream = TcpStream::connect_timeout(&address, remaining(deadline)?).map_err(|error| {
        if is_timeout(&error) {
            Error::Timeout
        } else {
            Error::Connection
        }
    })?;
    set_socket_timeout(&stream, deadline)?;
    Ok(stream)
}

fn set_socket_timeout(socket: &TcpStream, deadline: Instant) -> Result<(), Error> {
    let timeout = remaining(deadline)?;
    socket
        .set_read_timeout(Some(timeout))
        .and_then(|_| socket.set_write_timeout(Some(timeout)))
        .map_err(|_| Error::Connection)
}

fn remaining(deadline: Instant) -> Result<Duration, Error> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(Error::Timeout)
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

struct MqttConnection {
    stream: TlsStream<TcpStream>,
    topics: MqttTopics,
    deadline: Instant,
}

impl MqttConnection {
    fn new(stream: TlsStream<TcpStream>, topics: MqttTopics, deadline: Instant) -> Self {
        Self {
            stream,
            topics,
            deadline,
        }
    }

    fn connect(&mut self, access_code: &str) -> Result<(), Error> {
        let mut payload = Vec::new();
        mqtt_string(&mut payload, "MQTT")?;
        payload.extend_from_slice(&[4, 0b1100_0010]);
        payload.extend_from_slice(&60_u16.to_be_bytes());
        mqtt_string(&mut payload, &format!("polimero-{:016x}", next_sequence()))?;
        mqtt_string(&mut payload, MQTT_USERNAME)?;
        mqtt_string(&mut payload, access_code)?;
        self.write_packet(0x10, &payload)?;

        let packet = self
            .read_packet_until(self.deadline)?
            .ok_or(Error::Timeout)?;
        if packet.kind != 0x20 || packet.payload.len() != 2 {
            return Err(Error::InvalidResponse);
        }
        match packet.payload[1] {
            0 => Ok(()),
            4 | 5 => Err(Error::Authentication),
            _ => Err(Error::InvalidResponse),
        }
    }

    fn subscribe(&mut self) -> Result<(), Error> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1_u16.to_be_bytes());
        mqtt_string(&mut payload, &self.topics.report)?;
        payload.push(0);
        self.write_packet(0x82, &payload)?;
        let packet = self
            .read_packet_until(self.deadline)?
            .ok_or(Error::Timeout)?;
        if packet.kind != 0x90
            || packet.payload.len() < 3
            || packet.payload[..2] != 1_u16.to_be_bytes()
            || packet.payload[2] == 0x80
        {
            return Err(Error::InvalidResponse);
        }
        Ok(())
    }

    fn publish(&mut self, payload: &str) -> Result<(), Error> {
        let mut packet = Vec::new();
        mqtt_string(&mut packet, &self.topics.request)?;
        packet.extend_from_slice(payload.as_bytes());
        self.write_packet(0x30, &packet)
    }

    fn exchange(
        &mut self,
        command: String,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Vec<u8>, Error> {
        let command_sequence = payload_sequence_id(command.as_bytes());
        self.publish(&command)?;
        let refresh = if is_pushall_payload(command.as_bytes()) {
            command
        } else {
            let refresh = pushall_payload(next_sequence());
            self.publish(&refresh)?;
            refresh
        };
        let mut retry_at = Instant::now() + PUSHALL_INTERVAL;

        loop {
            let wait_until = self.deadline.min(retry_at);
            match self.read_packet_until(wait_until)? {
                Some(packet) if packet.kind >> 4 == 3 => {
                    let report = mqtt_publish_payload(packet.kind, &packet.payload)?;
                    if command_rejection(&report, command_sequence.as_deref())? {
                        return Err(Error::CommandRejected);
                    }
                    let value: Value =
                        serde_json::from_slice(&report).map_err(|_| Error::InvalidResponse)?;
                    if predicate(&value) {
                        return Ok(report);
                    }
                }
                Some(_) => {}
                None if Instant::now() >= self.deadline => return Err(Error::Timeout),
                None => {
                    self.publish(&refresh)?;
                    retry_at = Instant::now() + PUSHALL_INTERVAL;
                }
            }
        }
    }

    fn write_packet(&mut self, kind: u8, payload: &[u8]) -> Result<(), Error> {
        set_socket_timeout(self.stream.get_ref(), self.deadline)?;
        let mut packet = Vec::with_capacity(payload.len() + 5);
        packet.push(kind);
        mqtt_remaining_length(&mut packet, payload.len())?;
        packet.extend_from_slice(payload);
        self.stream.write_all(&packet).map_err(map_io)?;
        self.stream.flush().map_err(map_io)
    }

    fn read_packet_until(&mut self, until: Instant) -> Result<Option<MqttPacket>, Error> {
        let timeout = match until.checked_duration_since(Instant::now()) {
            Some(timeout) if !timeout.is_zero() => timeout,
            _ => return Ok(None),
        };
        self.stream
            .get_ref()
            .set_read_timeout(Some(timeout))
            .map_err(|_| Error::Connection)?;
        match read_mqtt_packet(&mut self.stream) {
            Ok(packet) => Ok(Some(packet)),
            Err(error) if is_timeout(&error) => Ok(None),
            Err(error) => Err(map_io(error)),
        }
    }

    fn disconnect(&mut self) {
        let _ = self.write_packet(0xe0, &[]);
    }
}

struct MqttPacket {
    kind: u8,
    payload: Vec<u8>,
}

fn mqtt_string(packet: &mut Vec<u8>, value: &str) -> Result<(), Error> {
    let bytes = value.as_bytes();
    let length = u16::try_from(bytes.len()).map_err(|_| Error::InvalidResponse)?;
    packet.extend_from_slice(&length.to_be_bytes());
    packet.extend_from_slice(bytes);
    Ok(())
}

fn mqtt_remaining_length(packet: &mut Vec<u8>, mut length: usize) -> Result<(), Error> {
    if length > MAX_MQTT_PACKET_SIZE {
        return Err(Error::InvalidResponse);
    }
    loop {
        let mut byte = u8::try_from(length % 128).map_err(|_| Error::InvalidResponse)?;
        length /= 128;
        if length > 0 {
            byte |= 0x80;
        }
        packet.push(byte);
        if length == 0 {
            return Ok(());
        }
    }
}

fn read_mqtt_packet(stream: &mut impl Read) -> io::Result<MqttPacket> {
    let mut first = [0];
    stream.read_exact(&mut first)?;
    let mut length = 0usize;
    let mut multiplier = 1usize;
    for _ in 0..4 {
        let mut encoded = [0];
        stream.read_exact(&mut encoded)?;
        length = length
            .checked_add(usize::from(encoded[0] & 0x7f).saturating_mul(multiplier))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid MQTT length"))?;
        if encoded[0] & 0x80 == 0 {
            if length > MAX_MQTT_PACKET_SIZE {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "MQTT packet too large",
                ));
            }
            let mut payload = vec![0; length];
            stream.read_exact(&mut payload)?;
            return Ok(MqttPacket {
                kind: first[0],
                payload,
            });
        }
        multiplier = multiplier
            .checked_mul(128)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid MQTT length"))?;
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid MQTT length",
    ))
}

fn mqtt_publish_payload(header: u8, payload: &[u8]) -> Result<Vec<u8>, Error> {
    if payload.len() < 2 {
        return Err(Error::InvalidResponse);
    }
    let topic_length = usize::from(u16::from_be_bytes([payload[0], payload[1]]));
    let mut offset = 2usize
        .checked_add(topic_length)
        .ok_or(Error::InvalidResponse)?;
    if offset > payload.len() {
        return Err(Error::InvalidResponse);
    }
    let qos = (header >> 1) & 0b11;
    if qos == 3 {
        return Err(Error::InvalidResponse);
    }
    if qos > 0 {
        offset = offset.checked_add(2).ok_or(Error::InvalidResponse)?;
    }
    payload
        .get(offset..)
        .map(ToOwned::to_owned)
        .ok_or(Error::InvalidResponse)
}

fn command_rejection(report: &[u8], sequence: Option<&str>) -> Result<bool, Error> {
    let Some(sequence) = sequence else {
        return Ok(false);
    };
    let report: Value = serde_json::from_slice(report).map_err(|_| Error::InvalidResponse)?;
    let command = ["print", "system"]
        .into_iter()
        .find_map(|key| report.get(key).and_then(Value::as_object));
    let Some(command) = command else {
        return Ok(false);
    };
    if string(command.get("sequence_id")).as_deref() != Some(sequence)
        || !string(command.get("result")).is_some_and(|result| result.eq_ignore_ascii_case("fail"))
    {
        return Ok(false);
    }
    let reason = string(command.get("reason")).unwrap_or_default();
    if integer(command.get("err_code")) == Some(84_033_543)
        || integer(command.get("errno")) == Some(84_033_543)
        || reason.to_ascii_lowercase().contains("verification failed")
    {
        return Err(Error::UnsignedCommand);
    }
    Ok(true)
}

fn next_sequence_id() -> String {
    next_sequence().to_string()
}

fn next_sequence() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX / 2)
        .saturating_add(u64::from(std::process::id()));
    let mut previous = SEQUENCE.load(Ordering::Relaxed);
    loop {
        let next = now.max(previous.saturating_add(1));
        match SEQUENCE.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(value) => previous = value,
        }
    }
}

fn gcode_payload(gcode: &str) -> String {
    json!({
        "print": {
            "sequence_id": next_sequence_id(),
            "command": "gcode_line",
            "param": gcode.trim_end_matches(['\r', '\n']),
        }
    })
    .to_string()
}

fn ledctrl_payload(state: LightState) -> String {
    json!({
        "system": {
            "sequence_id": next_sequence_id(),
            "command": "ledctrl",
            "led_node": "chamber_light",
            "led_mode": match state {
                LightState::On => "on",
                LightState::Off => "off",
            },
            "led_on_time": 500,
            "led_off_time": 500,
            "loop_times": 1,
            "interval_time": 1000,
        }
    })
    .to_string()
}

fn job_start_payload(path: &str, options: JobStartOptions) -> Result<String, Error> {
    let filename = path
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or(Error::InvalidDevicePath)?;
    let extension = filename.rsplit('.').next().unwrap_or_default();
    let command = if extension.eq_ignore_ascii_case("3mf") {
        "project_file"
    } else if extension.eq_ignore_ascii_case("gcode") {
        "gcode_file"
    } else {
        return Err(Error::InvalidDevicePath);
    };
    let mut print = Map::new();
    print.insert("sequence_id".into(), Value::String(next_sequence_id()));
    print.insert("command".into(), Value::String(command.into()));
    print.insert("bed_type".into(), Value::String("auto".into()));
    print.insert("bed_leveling".into(), Value::Bool(!options.skip_leveling));
    print.insert("flow_cali".into(), Value::Bool(false));
    print.insert("vibration_cali".into(), Value::Bool(false));
    print.insert("layer_inspect".into(), Value::Bool(false));
    print.insert("timelapse".into(), Value::Bool(false));
    print.insert("use_ams".into(), Value::Bool(false));
    if command == "project_file" {
        let plate = options.plate.unwrap_or(1).max(1);
        print.insert(
            "param".into(),
            Value::String(format!("Metadata/plate_{plate}.gcode")),
        );
        print.insert("subtask_name".into(), Value::String(filename.into()));
        for key in ["project_id", "profile_id", "task_id", "subtask_id", "cfg"] {
            print.insert(key.into(), Value::String("0".into()));
        }
        print.insert(
            "file".into(),
            Value::String(path.trim_start_matches('/').into()),
        );
        print.insert("url".into(), Value::String(format!("file://{path}")));
        print.insert("md5".into(), Value::String(String::new()));
        print.insert("ams_mapping".into(), Value::Array(Vec::new()));
        print.insert("ams_mapping2".into(), Value::Array(Vec::new()));
        for key in [
            "auto_bed_leveling",
            "nozzle_offset_cali",
            "extrude_cali_flag",
        ] {
            print.insert(key.into(), Value::from(0));
        }
    } else {
        print.insert("param".into(), Value::String(path.into()));
        print.insert("subtask_name".into(), Value::String(filename.into()));
        print.insert("plate_idx".into(), Value::from(options.plate.unwrap_or(0)));
    }
    Ok(Value::Object(Map::from_iter([("print".into(), Value::Object(print))])).to_string())
}

fn is_full_report(report: &Value) -> bool {
    report
        .get("print")
        .and_then(Value::as_object)
        .and_then(|print| string(print.get("gcode_state")))
        .is_some()
}

fn report_state_is(report: &Value, states: &[PrinterState]) -> bool {
    parse_status_value(report)
        .map(|status| states.contains(&status.state))
        .unwrap_or(false)
}

fn light_state_is(report: &Value, light: &str, state: LightState) -> bool {
    lights_report(report).is_some_and(|lights| {
        lights.iter().any(|entry| {
            entry.get("node").and_then(Value::as_str) == Some(light)
                && entry.get("mode").and_then(Value::as_str)
                    == Some(match state {
                        LightState::On => "on",
                        LightState::Off => "off",
                    })
        })
    })
}

fn light_unsupported_on_model(report: &Value, light: &str) -> bool {
    is_full_report(report)
        && lights_report(report).is_some_and(|lights| {
            !lights.is_empty()
                && !lights
                    .iter()
                    .any(|entry| entry.get("node").and_then(Value::as_str) == Some(light))
        })
}

fn lights_report(report: &Value) -> Option<&Vec<Value>> {
    report
        .get("lights_report")
        .or_else(|| {
            report
                .get("print")
                .and_then(|print| print.get("lights_report"))
        })
        .and_then(Value::as_array)
}

pub(crate) fn parse_status(report: &[u8]) -> Result<Status, Error> {
    let value = serde_json::from_slice(report).map_err(|_| Error::InvalidResponse)?;
    parse_status_value(&value)
}

fn parse_status_value(report: &Value) -> Result<Status, Error> {
    let print = report
        .get("print")
        .and_then(Value::as_object)
        .ok_or(Error::InvalidResponse)?;
    let state = match string(print.get("gcode_state")).as_deref() {
        Some("IDLE" | "FINISH") => PrinterState::Idle,
        Some("PRINTING" | "PREPARE" | "RUNNING" | "SLICING") => PrinterState::Printing,
        Some("PAUSED") => PrinterState::Paused,
        Some("FAILED") => PrinterState::Error,
        Some(_) => PrinterState::Unknown,
        None => return Err(Error::InvalidResponse),
    };
    let nozzle = heater(print, "nozzle_temper", "nozzle_target_temper");
    let bed = heater(print, "bed_temper", "bed_target_temper");
    let temperatures = (nozzle.is_some() || bed.is_some()).then_some(Temperatures { nozzle, bed });
    let mut warnings = Vec::new();
    if temperatures.is_none() {
        warnings.push(StatusWarning {
            code: "temperature-data-unavailable",
            message: "temperature data unavailable",
        });
    }
    let progress = integer(print.get("mc_percent")).map(|percent| Progress {
        percent: percent.clamp(0, 100).try_into().unwrap_or_default(),
        current_layer: integer(print.get("layer_num"))
            .or_else(|| integer(print.get("mc_layer_num")))
            .and_then(|value| value.try_into().ok()),
        total_layers: integer(print.get("total_layer_num")).and_then(|value| value.try_into().ok()),
    });
    if progress.is_none() {
        warnings.push(StatusWarning {
            code: "progress-unavailable",
            message: "progress unavailable",
        });
    }
    let job_name = string(print.get("subtask_name"))
        .filter(|value| !value.is_empty())
        .or_else(|| string(print.get("gcode_file")).filter(|value| !value.is_empty()));
    let job = matches!(state, PrinterState::Printing | PrinterState::Paused)
        .then(|| {
            job_name.map(|name| Job {
                id: synthetic_job_id(&name),
                name,
            })
        })
        .flatten();
    let errors = status_errors(print, state);
    Ok(Status {
        state,
        temperatures,
        job,
        progress: matches!(state, PrinterState::Printing | PrinterState::Paused)
            .then_some(progress)
            .flatten(),
        errors,
        warnings,
        fans: status_fans(print),
    })
}

fn heater(print: &Map<String, Value>, current: &str, target: &str) -> Option<Temperature> {
    let current_celsius = number(print.get(current));
    let target_celsius = number(print.get(target));
    (current_celsius.is_some() || target_celsius.is_some()).then_some(Temperature {
        current_celsius: current_celsius.unwrap_or_default(),
        target_celsius,
    })
}

fn status_errors(print: &Map<String, Value>, state: PrinterState) -> Vec<StatusError> {
    let mut errors = Vec::new();
    if integer(print.get("mc_print_error_code")).is_some_and(|value| value != 0) {
        errors.push(StatusError {
            code: "printer-error",
            message: "printer reported an error".into(),
        });
    }
    if print
        .get("hms")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.as_object().is_some_and(|hms| {
                    integer(hms.get("attr")).is_some_and(|value| value != 0)
                        || integer(hms.get("code")).is_some_and(|value| value != 0)
                })
            })
        })
    {
        errors.push(StatusError {
            code: "hardware-error",
            message: "printer reported a hardware error".into(),
        });
    }
    if errors.is_empty() && state == PrinterState::Error {
        errors.push(StatusError {
            code: "printer-error",
            message: "printer reported an error state".into(),
        });
    }
    errors
}

fn status_fans(print: &Map<String, Value>) -> BTreeMap<String, u8> {
    [
        ("cooling_fan_speed", "partCooling"),
        ("heatbreak_fan_speed", "heatbreak"),
        ("big_fan1_speed", "auxiliary"),
        ("big_fan2_speed", "chamber"),
    ]
    .into_iter()
    .filter_map(|(field, name)| {
        integer(print.get(field)).map(|value| {
            let percent = value.clamp(0, 15) * 100 / 15;
            (name.to_owned(), percent.try_into().unwrap_or(100))
        })
    })
    .collect()
}

fn string(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(str::to_owned)
}

fn number(value: Option<&Value>) -> Option<f64> {
    value?
        .as_f64()
        .or_else(|| value?.as_i64().map(|value| value as f64))
        .or_else(|| value?.as_str()?.trim().parse().ok())
        .filter(|value: &f64| value.is_finite())
}

fn integer(value: Option<&Value>) -> Option<i64> {
    value?
        .as_i64()
        .or_else(|| value?.as_u64().and_then(|value| value.try_into().ok()))
        .or_else(|| {
            value?
                .as_f64()
                .filter(|value| value.is_finite() && value.fract() == 0.0)
                .and_then(|value| {
                    (i64::MIN as f64..=i64::MAX as f64)
                        .contains(&value)
                        .then_some(value as i64)
                })
        })
        .or_else(|| value?.as_str()?.trim().parse().ok())
}

fn synthetic_job_id(name: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("lan-{hash:x}")
}

fn target_matches(heater: Option<&Temperature>, requested: Option<f64>) -> bool {
    let Some(requested) = requested else {
        return true;
    };
    heater
        .and_then(|heater| heater.target_celsius)
        .is_some_and(|reported| (reported - requested).abs() < 0.01)
}

fn validate_temperature_targets(targets: &TemperatureTargets) -> Result<(), Error> {
    validate_temperature(targets.nozzle_celsius, 300.0)?;
    validate_temperature(targets.bed_celsius, 120.0)?;
    validate_temperature(targets.chamber_celsius, 120.0)
}

fn validate_temperature(value: Option<f64>, maximum: f64) -> Result<(), Error> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=maximum).contains(&value)) {
        return Err(Error::InvalidTemperatureTarget);
    }
    Ok(())
}

fn validate_jog(delta: crate::moonraker::JogDelta) -> Result<(), Error> {
    if delta.feedrate_mm_per_min == 0 || delta.feedrate_mm_per_min > 24_000 {
        return Err(Error::InvalidMotion);
    }
    for distance in [
        delta.x_millimeters,
        delta.y_millimeters,
        delta.z_millimeters,
    ] {
        if distance
            .is_some_and(|distance| !distance.is_finite() || !(-10.0..=10.0).contains(&distance))
        {
            return Err(Error::InvalidMotion);
        }
    }
    Ok(())
}

fn normalize_device_path(value: &str) -> Result<String, Error> {
    if value.contains('\0') || value.contains('\\') || value.chars().any(char::is_control) {
        return Err(Error::InvalidDevicePath);
    }
    let trimmed = value.trim_matches('/');
    if trimmed.is_empty() {
        return Ok("/".into());
    }
    let segments = trimmed.split('/').collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || matches!(*segment, "." | ".."))
    {
        return Err(Error::InvalidDevicePath);
    }
    Ok(format!("/{}", segments.join("/")))
}

fn map_io(error: io::Error) -> Error {
    if is_timeout(&error) {
        Error::Timeout
    } else {
        Error::Connection
    }
}

struct FtpsConnection {
    control: TlsStream<TcpStream>,
    connector: TlsConnector,
    profile: Profile,
    fingerprint: Option<String>,
    deadline: Instant,
    pending: Vec<u8>,
    last_reply: String,
}

impl FtpsConnection {
    fn open(
        profile: &Profile,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<Self, Error> {
        let access_code = valid_access_code(access_code)?;
        validate_pin(profile, fingerprint)?;
        let deadline = deadline_after(profile.timeout())?;
        let connector = tls_connector()?;
        let (control, _) = open_tls(&connector, profile, FTP_PORT, fingerprint, true, deadline)?;
        let mut connection = Self {
            control,
            connector,
            profile: profile.clone(),
            fingerprint: fingerprint.map(str::to_owned),
            deadline,
            pending: Vec::new(),
            last_reply: String::new(),
        };
        connection.expect(&[220])?;
        let user = connection.command(&format!("USER {MQTT_USERNAME}"))?;
        if user.code == 331 {
            connection.expect_command(&format!("PASS {access_code}"), &[230])?;
        } else if user.code != 230 {
            return Err(if user.code == 530 {
                Error::Authentication
            } else {
                Error::FileTransfer
            });
        }
        connection.expect_command("TYPE I", &[200])?;
        Ok(connection)
    }

    fn list(&mut self, path: &str) -> Result<Vec<FileEntry>, Error> {
        let mut data = self.passive_data()?;
        let reply = self.command(&format!("LIST {path}"))?;
        if !matches!(reply.code, 125 | 150) {
            return Err(Error::FileTransfer);
        }
        let bytes = read_data(&mut data, self.deadline)?;
        drop(data);
        self.expect(&[226, 250])?;
        let mut entries = parse_ftp_listing(&bytes, path);
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    fn list_recursive(&mut self, path: &str) -> Result<Vec<FileEntry>, Error> {
        let mut queue = VecDeque::from([(path.to_owned(), 0_u8)]);
        let mut all = Vec::new();
        while let Some((directory, depth)) = queue.pop_front() {
            if depth > MAX_LIST_DEPTH {
                return Err(Error::FileTransfer);
            }
            let entries = self.list(&directory)?;
            for entry in entries {
                if entry.entry_type == FileEntryType::Directory {
                    queue.push_back((entry.path.clone(), depth.saturating_add(1)));
                }
                all.push(entry);
            }
        }
        all.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(all)
    }

    fn file_exists(&mut self, path: &str) -> Result<bool, Error> {
        let reply = self.command(&format!("SIZE {path}"))?;
        match reply.code {
            213 => Ok(true),
            550 => Ok(false),
            _ => Err(Error::FileTransfer),
        }
    }

    fn download(&mut self, path: &str, destination: &mut dyn Write) -> Result<u64, Error> {
        let mut data = self.passive_data()?;
        let reply = self.command(&format!("RETR {path}"))?;
        if !matches!(reply.code, 125 | 150) {
            return Err(Error::FileTransfer);
        }
        let result = copy_data(&mut data, destination, self.deadline);
        drop(data);
        self.expect(&[226, 250])?;
        result
    }

    fn upload(&mut self, path: &str, source: &mut dyn Read) -> Result<u64, Error> {
        let mut data = self.passive_data()?;
        let reply = self.command(&format!("STOR {path}"))?;
        if !matches!(reply.code, 125 | 150) {
            return Err(Error::FileTransfer);
        }
        let result = copy_data(source, &mut data, self.deadline);
        let _ = data.flush();
        let _ = data.shutdown();
        drop(data);
        self.expect(&[226, 250])?;
        result
    }

    fn passive_data(&mut self) -> Result<TlsStream<TcpStream>, Error> {
        let port = match self.command("EPSV")? {
            FtpReply { code: 229 } => parse_epsv_port(self.last_reply_line()?)?,
            FtpReply {
                code: 500 | 501 | 502 | 504,
            } => {
                let reply = self.command("PASV")?;
                if reply.code != 227 {
                    return Err(Error::FileTransfer);
                }
                parse_pasv_port(self.last_reply_line()?)?
            }
            _ => return Err(Error::FileTransfer),
        };
        let address = SocketAddr::new(
            self.control
                .get_ref()
                .peer_addr()
                .map_err(|_| Error::Connection)?
                .ip(),
            port,
        );
        let socket = connect_socket(address, self.deadline)?;
        open_tls_socket(
            &self.connector,
            &self.profile,
            socket,
            self.fingerprint.as_deref(),
            true,
            self.deadline,
        )
        .map(|(stream, _)| stream)
    }

    fn command(&mut self, command: &str) -> Result<FtpReply, Error> {
        if command.contains(['\r', '\n']) {
            return Err(Error::InvalidDevicePath);
        }
        set_socket_timeout(self.control.get_ref(), self.deadline)?;
        self.control
            .write_all(command.as_bytes())
            .and_then(|_| self.control.write_all(b"\r\n"))
            .and_then(|_| self.control.flush())
            .map_err(map_io)?;
        self.read_reply()
    }

    fn expect_command(&mut self, command: &str, accepted: &[u16]) -> Result<(), Error> {
        let reply = self.command(command)?;
        if accepted.contains(&reply.code) {
            Ok(())
        } else if reply.code == 530 {
            Err(Error::Authentication)
        } else {
            Err(Error::FileTransfer)
        }
    }

    fn expect(&mut self, accepted: &[u16]) -> Result<(), Error> {
        let reply = self.read_reply()?;
        if accepted.contains(&reply.code) {
            Ok(())
        } else if reply.code == 530 {
            Err(Error::Authentication)
        } else {
            Err(Error::FileTransfer)
        }
    }

    fn read_reply(&mut self) -> Result<FtpReply, Error> {
        let first = self.read_line()?;
        let (code, separator) = parse_ftp_reply(&first)?;
        if separator == b'-' {
            loop {
                let line = self.read_line()?;
                let (end_code, end_separator) = parse_ftp_reply(&line)?;
                if end_code == code && end_separator == b' ' {
                    break;
                }
            }
        }
        self.last_reply = first;
        Ok(FtpReply { code })
    }

    fn last_reply_line(&self) -> Result<&str, Error> {
        (!self.last_reply.is_empty())
            .then_some(self.last_reply.as_str())
            .ok_or(Error::InvalidResponse)
    }

    fn read_line(&mut self) -> Result<String, Error> {
        loop {
            if let Some(index) = self.pending.iter().position(|byte| *byte == b'\n') {
                let line = self.pending.drain(..=index).collect::<Vec<_>>();
                if line.len() > MAX_FTP_REPLY_SIZE {
                    return Err(Error::InvalidResponse);
                }
                return String::from_utf8(line)
                    .map(|line| line.trim_end_matches(['\r', '\n']).to_owned())
                    .map_err(|_| Error::InvalidResponse);
            }
            if self.pending.len() >= MAX_FTP_REPLY_SIZE {
                return Err(Error::InvalidResponse);
            }
            set_socket_timeout(self.control.get_ref(), self.deadline)?;
            let mut buffer = [0; 1024];
            let read = self.control.read(&mut buffer).map_err(map_io)?;
            if read == 0 {
                return Err(Error::Connection);
            }
            self.pending.extend_from_slice(&buffer[..read]);
        }
    }

    fn quit(&mut self) {
        let _ = self.command("QUIT");
    }
}

struct FtpReply {
    code: u16,
}

fn parse_ftp_reply(line: &str) -> Result<(u16, u8), Error> {
    let bytes = line.as_bytes();
    if bytes.len() < 4
        || !bytes[..3].iter().all(u8::is_ascii_digit)
        || !matches!(bytes[3], b' ' | b'-')
    {
        return Err(Error::InvalidResponse);
    }
    let code = line[..3].parse().map_err(|_| Error::InvalidResponse)?;
    Ok((code, bytes[3]))
}

fn parse_epsv_port(line: &str) -> Result<u16, Error> {
    let start = line.find('(').ok_or(Error::InvalidResponse)?;
    let end = line[start + 1..]
        .find(')')
        .map(|index| start + index + 1)
        .ok_or(Error::InvalidResponse)?;
    let value = &line[start + 1..end];
    let delimiter = value
        .as_bytes()
        .first()
        .copied()
        .ok_or(Error::InvalidResponse)?;
    let parts = value.split(char::from(delimiter)).collect::<Vec<_>>();
    if parts.len() != 5 || !parts[..3].iter().all(|part| part.is_empty()) {
        return Err(Error::InvalidResponse);
    }
    parts[3].parse().map_err(|_| Error::InvalidResponse)
}

fn parse_pasv_port(line: &str) -> Result<u16, Error> {
    let start = line.find('(').ok_or(Error::InvalidResponse)?;
    let end = line[start + 1..]
        .find(')')
        .map(|index| start + index + 1)
        .ok_or(Error::InvalidResponse)?;
    let numbers = line[start + 1..end]
        .split(',')
        .map(str::parse::<u8>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::InvalidResponse)?;
    if numbers.len() != 6 {
        return Err(Error::InvalidResponse);
    }
    Ok(u16::from(numbers[4]) << 8 | u16::from(numbers[5]))
}

fn read_data(data: &mut TlsStream<TcpStream>, deadline: Instant) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        set_socket_timeout(data.get_ref(), deadline)?;
        let read = data.read(&mut buffer).map_err(map_io)?;
        if read == 0 {
            return Ok(bytes);
        }
        if (bytes.len() + read) as u64 > MAX_FTP_LISTING_SIZE {
            return Err(Error::FileTransfer);
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
}

fn copy_data(
    source: &mut dyn Read,
    destination: &mut dyn Write,
    deadline: Instant,
) -> Result<u64, Error> {
    let mut buffer = [0; 8192];
    let mut copied = 0_u64;
    loop {
        let read = source.read(&mut buffer).map_err(|_| Error::FileTransfer)?;
        if read == 0 {
            return Ok(copied);
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout);
        }
        destination.write_all(&buffer[..read]).map_err(map_io)?;
        copied = copied.checked_add(read as u64).ok_or(Error::FileTransfer)?;
    }
}

fn parse_ftp_listing(bytes: &[u8], parent: &str) -> Vec<FileEntry> {
    let Ok(listing) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    listing
        .lines()
        .filter_map(parse_ftp_list_line)
        .filter(|entry| {
            entry.name != "."
                && entry.name != ".."
                && !entry.name.contains('/')
                && !entry.name.contains('\\')
                && !entry.name.chars().any(char::is_control)
        })
        .map(|entry| {
            let path = if parent == "/" {
                format!("/{}", entry.name)
            } else {
                format!("{}/{}", parent.trim_end_matches('/'), entry.name)
            };
            FileEntry {
                name: entry.name,
                root: FILE_ROOT,
                device_path: format!("{FILE_ROOT}:{path}"),
                path,
                entry_type: entry.entry_type,
                size_bytes: entry.size_bytes,
                modified_at: None,
                metadata: BTreeMap::new(),
            }
        })
        .collect()
}

struct ParsedFtpEntry {
    name: String,
    entry_type: FileEntryType,
    size_bytes: Option<i64>,
}

fn parse_ftp_list_line(line: &str) -> Option<ParsedFtpEntry> {
    let mut fields = line.split_whitespace();
    let mode = fields.next()?;
    let entry_type = match mode.as_bytes().first().copied()? {
        b'd' => FileEntryType::Directory,
        b'-' => FileEntryType::File,
        _ => return None,
    };
    fields.next()?; // links
    fields.next()?; // owner
    fields.next()?; // group
    let size_bytes = (entry_type == FileEntryType::File)
        .then(|| fields.next()?.parse().ok())
        .flatten();
    if entry_type == FileEntryType::Directory {
        fields.next()?; // directory size
    }
    fields.next()?; // month
    fields.next()?; // day
    fields.next()?; // time or year
    let name = fields.collect::<Vec<_>>().join(" ");
    (!name.is_empty()).then_some(ParsedFtpEntry {
        name,
        entry_type,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use std::{io::Write, net::TcpListener, sync::mpsc, thread};

    use native_tls::{Identity, TlsAcceptor};
    use openssl::{
        asn1::Asn1Time,
        bn::BigNum,
        hash::MessageDigest,
        pkcs12::Pkcs12,
        pkey::PKey,
        rsa::Rsa,
        x509::{X509, X509NameBuilder},
    };

    use super::*;

    #[test]
    fn encodes_mqtt_remaining_lengths_and_rejects_oversized_packets() {
        let mut encoded = Vec::new();
        mqtt_remaining_length(&mut encoded, 321).unwrap();
        assert_eq!(encoded, [0xc1, 0x02]);
        assert!(mqtt_remaining_length(&mut encoded, MAX_MQTT_PACKET_SIZE + 1).is_err());
    }

    #[test]
    fn maps_full_bambu_reports_to_portable_status() {
        let status = parse_status(
            br#"{"print":{"gcode_state":"PRINTING","nozzle_temper":"212.5","nozzle_target_temper":"220","bed_temper":59,"bed_target_temper":60,"mc_percent":61,"layer_num":3,"total_layer_num":100,"subtask_name":"cube.3mf","cooling_fan_speed":"9"}}"#,
        )
        .unwrap();

        assert_eq!(status.state, PrinterState::Printing);
        assert_eq!(status.job.as_ref().unwrap().name, "cube.3mf");
        assert_eq!(status.progress.as_ref().unwrap().percent, 61);
        assert_eq!(
            status.temperatures.unwrap().nozzle.unwrap().target_celsius,
            Some(220.0)
        );
        assert_eq!(status.fans["partCooling"], 60);
    }

    #[test]
    fn rejects_unsafe_paths_before_the_ftp_transport() {
        for path in ["/../secret", "/models/./cube.3mf", "/bad\nname"] {
            assert!(matches!(
                normalize_device_path(path),
                Err(Error::InvalidDevicePath)
            ));
        }
    }

    #[test]
    fn parses_ftp_entries_without_exposing_parent_paths() {
        let entries = parse_ftp_listing(
            b"drwxr-xr-x 1 root root 0 Jan 01 12:00 models\r\n-rw-r--r-- 1 root root 12 Jan 01 12:00 cube.3mf\r\n",
            "/",
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].device_path, "sdcard:/models");
        assert_eq!(entries[1].size_bytes, Some(12));
    }

    #[test]
    fn command_rejections_are_scoped_to_the_active_sequence() {
        let rejection =
            br#"{"print":{"sequence_id":"7","result":"fail","reason":"verification failed"}}"#;
        assert!(matches!(
            command_rejection(rejection, Some("7")),
            Err(Error::UnsignedCommand)
        ));
        assert!(!command_rejection(rejection, Some("other")).unwrap());
    }

    #[test]
    fn builds_chamber_ledctrl_payload_and_acknowledges_reported_state() {
        let payload: Value = serde_json::from_str(&ledctrl_payload(LightState::On)).unwrap();
        let command = &payload["system"];
        assert!(
            command["sequence_id"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert_eq!(command["command"], "ledctrl");
        assert_eq!(command["led_node"], "chamber_light");
        assert_eq!(command["led_mode"], "on");
        assert_eq!(command["led_on_time"], 500);
        assert_eq!(command["led_off_time"], 500);
        assert_eq!(command["loop_times"], 1);
        assert_eq!(command["interval_time"], 1000);

        let top_level = serde_json::json!({
            "lights_report": [{"node": "chamber_light", "mode": "on"}]
        });
        let nested = serde_json::json!({
            "print": {"lights_report": [{"node": "chamber_light", "mode": "off"}]}
        });
        assert!(light_state_is(&top_level, "chamber_light", LightState::On));
        assert!(light_state_is(&nested, "chamber_light", LightState::Off));
        assert!(!light_state_is(&nested, "chamber_light", LightState::On));

        let full_without_chamber = serde_json::json!({
            "print": {"gcode_state": "IDLE"},
            "lights_report": [{"node": "work_light", "mode": "on"}]
        });
        let delta_without_chamber = serde_json::json!({
            "lights_report": [{"node": "work_light", "mode": "on"}]
        });
        assert!(light_unsupported_on_model(
            &full_without_chamber,
            "chamber_light"
        ));
        assert!(!light_unsupported_on_model(
            &delta_without_chamber,
            "chamber_light"
        ));
    }

    #[test]
    fn rejects_lights_other_than_the_chamber_before_connecting() {
        let profile = Profile::new("printer.local", "SN001", false).unwrap();
        assert!(matches!(
            Client::new(profile).light_set(None, None, "work", LightState::On),
            Err(Error::Unsupported("requested light"))
        ));
    }

    #[test]
    fn verifies_mqtt_credentials_then_uses_the_pinned_transport_for_status() {
        let (host, port, credentials, server) = mqtt_server();
        let profile = Profile::with_timeout(host, "SN001", false, Duration::from_secs(2)).unwrap();
        let connector = tls_connector().unwrap();

        let deadline = deadline_after(profile.timeout()).unwrap();
        let (stream, fingerprint) =
            open_tls(&connector, &profile, port, None, false, deadline).unwrap();
        let mut mqtt = MqttConnection::new(stream, profile.mqtt_topics(), deadline);
        mqtt.connect("access-code").unwrap();
        mqtt.disconnect();

        let deadline = deadline_after(profile.timeout()).unwrap();
        let (stream, _) = open_tls(
            &connector,
            &profile,
            port,
            Some(&fingerprint),
            true,
            deadline,
        )
        .unwrap();
        let mut mqtt = MqttConnection::new(stream, profile.mqtt_topics(), deadline);
        mqtt.connect("access-code").unwrap();
        mqtt.subscribe().unwrap();
        let report = mqtt
            .exchange(pushall_payload(next_sequence()), is_full_report)
            .unwrap();
        mqtt.disconnect();
        let status = parse_status(&report).unwrap();

        server.join().unwrap();
        assert_eq!(credentials.recv().unwrap(), "access-code");
        assert_eq!(credentials.recv().unwrap(), "access-code");
        assert_eq!(status.state, PrinterState::Idle);
        assert!(is_valid_tls_fingerprint(&fingerprint));
    }

    fn mqtt_server() -> (String, u16, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = address.ip().to_string();
        let acceptor = TlsAcceptor::new(test_identity()).unwrap();
        let (sender, credentials) = mpsc::channel();
        let server = thread::spawn(move || {
            for status_request in [false, true] {
                let (socket, _) = listener.accept().unwrap();
                let mut stream = acceptor.accept(socket).unwrap();
                let connect = read_mqtt_packet(&mut stream).unwrap();
                assert_eq!(connect.kind, 0x10);
                sender.send(connect_password(&connect.payload)).unwrap();
                stream.write_all(&[0x20, 0x02, 0, 0]).unwrap();
                stream.flush().unwrap();

                if !status_request {
                    continue;
                }
                let subscribe = read_mqtt_packet(&mut stream).unwrap();
                assert_eq!(subscribe.kind, 0x82);
                let id = &subscribe.payload[..2];
                stream.write_all(&[0x90, 0x03, id[0], id[1], 0]).unwrap();
                stream.flush().unwrap();
                let publish = read_mqtt_packet(&mut stream).unwrap();
                assert_eq!(publish.kind, 0x30);
                let report = br#"{"print":{"gcode_state":"IDLE","mc_percent":0}}"#;
                let mut payload = Vec::new();
                mqtt_string(&mut payload, "device/SN001/report").unwrap();
                payload.extend_from_slice(report);
                let mut packet = vec![0x30];
                mqtt_remaining_length(&mut packet, payload.len()).unwrap();
                packet.extend_from_slice(&payload);
                stream.write_all(&packet).unwrap();
                stream.flush().unwrap();
            }
        });
        (host, address.port(), credentials, server)
    }

    fn connect_password(payload: &[u8]) -> String {
        let mut offset = 0;
        let protocol = mqtt_test_string(payload, &mut offset);
        assert_eq!(protocol, "MQTT");
        offset += 4; // protocol version, flags, and keepalive
        let _client_id = mqtt_test_string(payload, &mut offset);
        let username = mqtt_test_string(payload, &mut offset);
        assert_eq!(username, MQTT_USERNAME);
        mqtt_test_string(payload, &mut offset)
    }

    fn mqtt_test_string(payload: &[u8], offset: &mut usize) -> String {
        let length = usize::from(u16::from_be_bytes([payload[*offset], payload[*offset + 1]]));
        *offset += 2;
        let end = *offset + length;
        let value = std::str::from_utf8(&payload[*offset..end])
            .unwrap()
            .to_owned();
        *offset = end;
        value
    }

    fn test_identity() -> Identity {
        let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", "SN001").unwrap();
        let name = name.build();
        let serial = BigNum::from_u32(1).unwrap().to_asn1_integer().unwrap();
        let mut certificate = X509::builder().unwrap();
        certificate.set_version(2).unwrap();
        certificate.set_serial_number(&serial).unwrap();
        certificate.set_subject_name(&name).unwrap();
        certificate.set_issuer_name(&name).unwrap();
        certificate
            .set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        certificate
            .set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        certificate.set_pubkey(&key).unwrap();
        certificate.sign(&key, MessageDigest::sha256()).unwrap();
        let certificate = certificate.build();
        let identity = Pkcs12::builder()
            .name("polimero-test")
            .pkey(&key)
            .cert(&certificate)
            .build2("polimero-test")
            .unwrap();
        Identity::from_pkcs12(&identity.to_der().unwrap(), "polimero-test").unwrap()
    }
}
