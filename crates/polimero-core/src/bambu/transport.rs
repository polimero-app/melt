use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, File},
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::Path,
    sync::{
        Mutex, MutexGuard, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::moonraker::{
    AmsData, AmsTray, AmsUnit, BambuExtension, Extensions, FanResult, FileEntry, FileEntryType,
    FileList, FileRoot, GcodePosition, Job, JobResult, LightResult, LightState, MotionResult,
    MotionState, PrintMeta, PrinterState, Progress, SpeedResult, Status, StatusError,
    StatusWarning, Temperature, TemperatureResult, TemperatureTargets, Temperatures, TimeEstimates,
    Timelapse, Wifi,
};
use crate::trace::TraceEvent;
use openssl::ssl::{
    SslConnector, SslMethod, SslSession, SslSessionRef, SslStream, SslVerifyMode, SslVersion,
};
use serde_json::{Map, Value, json};
use thiserror::Error;
use time::{Date, Month, OffsetDateTime, Time, format_description::well_known::Rfc3339};

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
const MAX_LIST_ENTRIES: usize = 50_000;
const PUSHALL_INTERVAL: Duration = Duration::from_secs(3);
const MQTT_DRAIN_QUIET_WINDOW: Duration = Duration::from_millis(5);

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
pub struct Client {
    profile: Profile,
    mqtt: Mutex<Option<CachedConnection>>,
}

impl Client {
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            mqtt: Mutex::new(None),
        }
    }

    /// Performs a TLS and MQTT authentication exchange before a profile is saved.
    ///
    /// A secure profile receives a leaf-certificate fingerprint for TOFU pinning.
    /// An insecure profile stores no pin, but its credentials are still checked
    /// against the printer: insecure means unpinned, not unverified.
    pub fn verify(&self, access_code: Option<&str>) -> Result<Option<String>, Error> {
        let access_code = valid_access_code(access_code)?;
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
        let result = mqtt.connect(access_code);
        mqtt.disconnect();
        result?;
        Ok((!self.profile.insecure()).then_some(fingerprint))
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

    /// Like [`status`](Self::status), but reuses a live MQTT session across
    /// calls (kept in `cached`) instead of connecting fresh each time.
    ///
    /// A `cached` connection that fails is dropped and retried once with a
    /// fresh connect, so a printer that closes an idle session self-heals on
    /// its next poll.
    pub fn poll_status(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        timeout: Duration,
    ) -> Result<Status, Error> {
        let started = Instant::now();
        let deadline = deadline_after(timeout)?;
        let access_code = valid_access_code(access_code)?;
        validate_pin(&self.profile, fingerprint)?;
        let identity = self
            .profile
            .connection_identity(Some(access_code), fingerprint);
        let payload = pushall_payload(next_sequence());
        let label = format!("poll {}", self.profile.mqtt_topics().request);
        self.trace(TraceEvent::request("mqtt", label.as_str()).with_bytes(payload.len() as u64));
        let mut cached = self.lock_mqtt_until(deadline)?;

        if let Some(mqtt) = matching_cached_connection(&mut cached, identity) {
            mqtt.deadline = deadline;
            match mqtt.exchange(payload.clone(), is_full_report) {
                Ok(report) => {
                    self.trace(
                        TraceEvent::response("mqtt", label.as_str())
                            .with_outcome("ok")
                            .with_bytes(report.len() as u64)
                            .with_elapsed_ms(started.elapsed().as_millis() as u64),
                    );
                    return parse_status(&report);
                }
                Err(_) => *cached = None,
            }
        }

        remaining(deadline)?;
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
            self.trace(
                TraceEvent::response("mqtt", label.as_str())
                    .with_outcome(error.to_string())
                    .with_elapsed_ms(started.elapsed().as_millis() as u64),
            );
            return Err(error);
        }
        let report = match mqtt.exchange(payload, is_full_report) {
            Ok(report) => report,
            Err(error) => {
                self.trace(
                    TraceEvent::response("mqtt", label.as_str())
                        .with_outcome(error.to_string())
                        .with_elapsed_ms(started.elapsed().as_millis() as u64),
                );
                return Err(error);
            }
        };
        self.trace(
            TraceEvent::response("mqtt", label.as_str())
                .with_outcome("ok")
                .with_bytes(report.len() as u64)
                .with_elapsed_ms(started.elapsed().as_millis() as u64),
        );
        let status = parse_status(&report)?;
        *cached = Some(CachedConnection { identity, mqtt });
        Ok(status)
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

    /// Stops the printer and waits for the report that proves it was accepted.
    ///
    /// M112 travels as a gcode_line, which current firmware refuses when it is
    /// unsigned; a refused stop must never be reported as a successful one.
    pub fn emergency_stop(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
    ) -> Result<(), Error> {
        self.exchange(
            access_code,
            fingerprint,
            gcode_payload("M112"),
            is_full_report,
        )
        .map(|_| ())
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
                    // Reports use a 0-15 scale, so one step is about 7 points.
                    .is_some_and(|reported| reported.abs_diff(speed_percent) <= 7)
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
        let report = self.exchange(
            access_code,
            fingerprint,
            ledctrl_payload(light, state),
            |report| {
                light_state_is(report, light, state) || light_unsupported_on_model(report, light)
            },
        )?;
        let report: Value = serde_json::from_slice(&report).map_err(|_| Error::InvalidResponse)?;
        if light_unsupported_on_model(&report, light) {
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
            description: "SD card",
            writable: true,
            capacity_bytes: None,
            free_bytes: None,
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
        self.trace_ftps("LIST", &path, result.as_ref().err());
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
        self.trace_ftps("RETR", &path, result.as_ref().err());
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
        self.trace_ftps("STOR", &path, result.as_ref().err());
        result
    }

    pub fn delete_file(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        device_path: &str,
    ) -> Result<(), Error> {
        let path = normalize_device_path(device_path)?;
        if path == "/" {
            return Err(Error::InvalidDevicePath);
        }
        let mut ftp = FtpsConnection::open(&self.profile, access_code, fingerprint)?;
        let result = ftp.command(&format!("DELE {path}")).and_then(|reply| {
            if matches!(reply.code, 250 | 251) {
                Ok(())
            } else {
                Err(Error::FileTransfer)
            }
        });
        ftp.quit();
        self.trace_ftps("DELE", &path, result.as_ref().err());
        result
    }

    /// Records one summary event per FTPS operation; the underlying
    /// connection issues several control-channel commands per call, which is
    /// more detail than a diagnostics trace needs.
    fn trace_ftps(&self, command: &str, path: &str, error: Option<&Error>) {
        let label = format!("{command} {path}");
        let event = match error {
            None => TraceEvent::response("ftps", label).with_outcome("ok"),
            Some(error) => TraceEvent::response("ftps", label).with_outcome(error.to_string()),
        };
        self.trace(event);
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

    fn exchange(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        payload: String,
        predicate: impl Fn(&Value) -> bool,
    ) -> Result<Vec<u8>, Error> {
        let started = Instant::now();
        let label = format!("publish {}", self.profile.mqtt_topics().request);
        self.trace(TraceEvent::request("mqtt", label.as_str()).with_bytes(payload.len() as u64));
        let result = self.with_mqtt(access_code, fingerprint, |mqtt| {
            mqtt.exchange(payload, predicate)
        });
        match &result {
            Ok(report) => self.trace(
                TraceEvent::response("mqtt", label.as_str())
                    .with_outcome("ok")
                    .with_bytes(report.len() as u64)
                    .with_elapsed_ms(started.elapsed().as_millis() as u64),
            ),
            Err(error) => self.trace(
                TraceEvent::response("mqtt", label.as_str())
                    .with_outcome(error.to_string())
                    .with_elapsed_ms(started.elapsed().as_millis() as u64),
            ),
        }
        result
    }

    fn trace(&self, event: TraceEvent) {
        if let Some(tracer) = &self.profile.tracer {
            tracer.record(event);
        }
    }

    fn with_mqtt<T>(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        operation: impl FnOnce(&mut MqttConnection) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let access_code = valid_access_code(access_code)?;
        validate_pin(&self.profile, fingerprint)?;
        let identity = self
            .profile
            .connection_identity(Some(access_code), fingerprint);
        let deadline = deadline_after(self.profile.timeout())?;
        let mut cached = self.lock_mqtt_until(deadline)?;
        if let Some(mqtt) = matching_cached_connection(&mut cached, identity) {
            mqtt.deadline = deadline;
            return match operation(mqtt) {
                Ok(result) => Ok(result),
                Err(error) => {
                    *cached = None;
                    Err(error)
                }
            };
        }

        remaining(deadline)?;
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
        match result {
            Ok(result) => {
                *cached = Some(CachedConnection { identity, mqtt });
                Ok(result)
            }
            Err(error) => {
                mqtt.disconnect();
                Err(error)
            }
        }
    }

    fn lock_mqtt_until(
        &self,
        deadline: Instant,
    ) -> Result<MutexGuard<'_, Option<CachedConnection>>, Error> {
        loop {
            match self.mqtt.try_lock() {
                Ok(cached) => return Ok(cached),
                Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
                Err(TryLockError::WouldBlock) => {
                    std::thread::sleep(remaining(deadline)?.min(Duration::from_millis(1)));
                }
            }
        }
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

pub(super) fn tls_connector() -> Result<SslConnector, Error> {
    let mut builder = SslConnector::builder(SslMethod::tls_client()).map_err(|_| Error::Tls)?;
    // Bambu LAN uses a self-signed leaf on an old TLS stack. The leaf is always
    // pinned below before credentials or data are sent unless the profile is
    // explicit, so platform CA checks and OpenSSL's policy level are relaxed.
    builder.set_verify(SslVerifyMode::NONE);
    builder.set_security_level(0);
    // TLS 1.2 keeps the session id that the printer's FTP server requires to
    // accept a data connection; 1.3 tickets arrive too late to be reused.
    builder
        .set_max_proto_version(Some(SslVersion::TLS1_2))
        .map_err(|_| Error::Tls)?;
    Ok(builder.build())
}

pub(super) fn open_tls(
    connector: &SslConnector,
    profile: &Profile,
    port: u16,
    expected_fingerprint: Option<&str>,
    verify_pin: bool,
    deadline: Instant,
) -> Result<(SslStream<TcpStream>, String), Error> {
    let socket = resolve_and_connect(profile.host(), port, deadline)?;
    open_tls_socket(
        connector,
        profile,
        socket,
        expected_fingerprint,
        verify_pin,
        deadline,
        None,
    )
}

/// Completes a TLS handshake on a connected socket and returns the leaf pin.
///
/// `session` resumes an established TLS session, which the printer's FTP server
/// requires before it will accept a passive data connection.
fn open_tls_socket(
    connector: &SslConnector,
    profile: &Profile,
    socket: TcpStream,
    expected_fingerprint: Option<&str>,
    verify_pin: bool,
    deadline: Instant,
    session: Option<&SslSessionRef>,
) -> Result<(SslStream<TcpStream>, String), Error> {
    set_socket_timeout(&socket, deadline)?;
    let mut ssl = connector
        .configure()
        .and_then(|configuration| {
            configuration
                .verify_hostname(false)
                .into_ssl(profile.serial())
        })
        .map_err(|_| Error::Tls)?;
    if let Some(session) = session {
        // SAFETY: the session was produced by a live connection built from this
        // same connector, which is what OpenSSL requires to resume it.
        unsafe { ssl.set_session(session) }.map_err(|_| Error::Tls)?;
    }
    let mut connection = SslStream::new(ssl, socket).map_err(|_| Error::Tls)?;
    connection
        .connect()
        .map_err(|error| match error.io_error() {
            Some(error) if is_timeout(error) => Error::Timeout,
            _ => Error::Tls,
        })?;
    let certificate = connection
        .ssl()
        .peer_certificate()
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
    set_socket_idle_timeout(socket, remaining(deadline)?)
}

/// Bounds how long a single read or write may stall, rather than how long the
/// whole operation may take, so bulk transfers are not cut off mid-file.
fn set_socket_idle_timeout(socket: &TcpStream, timeout: Duration) -> Result<(), Error> {
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

/// An authenticated, subscribed MQTT session kept alive across polls by a
/// caller (see [`Client::poll_status`]) instead of reconnecting each time.
/// Opaque outside this module: callers only ever pass it back in unchanged.
struct CachedConnection {
    identity: [u8; 32],
    mqtt: MqttConnection,
}

fn matching_cached_connection(
    cached: &mut Option<CachedConnection>,
    identity: [u8; 32],
) -> Option<&mut MqttConnection> {
    if cached
        .as_ref()
        .is_some_and(|connection| connection.identity != identity)
    {
        *cached = None;
    }
    cached.as_mut().map(|connection| &mut connection.mqtt)
}

struct MqttConnection {
    stream: SslStream<TcpStream>,
    topics: MqttTopics,
    deadline: Instant,
    pending: Vec<u8>,
}

impl MqttConnection {
    fn new(stream: SslStream<TcpStream>, topics: MqttTopics, deadline: Instant) -> Self {
        Self {
            stream,
            topics,
            deadline,
            pending: Vec::new(),
        }
    }

    fn connect(&mut self, access_code: &str) -> Result<(), Error> {
        let mut payload = Vec::new();
        mqtt_string(&mut payload, "MQTT")?;
        payload.extend_from_slice(&[4, 0b1100_0010]);
        // Keepalive is disabled: a connection lives for one operation and this
        // client never sends PINGREQ, so a long timeout must not be dropped.
        payload.extend_from_slice(&0_u16.to_be_bytes());
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
        let is_status_poll = is_pushall_payload(command.as_bytes());
        if is_status_poll {
            self.drain_stale_packets()?;
        }
        let mut acknowledged = is_status_poll;
        self.publish(&command)?;
        let refresh = if is_status_poll {
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
                    // Unrelated or malformed reports are ignored rather than
                    // failing the exchange, matching payload_sequence_id.
                    let Ok(value) = serde_json::from_slice::<Value>(&report) else {
                        continue;
                    };
                    command_rejection(&value, command_sequence.as_deref())?;
                    acknowledged |= report_matches_sequence(&value, command_sequence.as_deref());
                    if acknowledged && predicate(&value) {
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

    /// Clears reports received before a new status request is published. Bambu
    /// pushall reports do not consistently echo their sequence id, so request
    /// ordering is the only reliable freshness boundary for status polls.
    fn drain_stale_packets(&mut self) -> Result<(), Error> {
        let until = self.deadline.min(Instant::now() + MQTT_DRAIN_QUIET_WINDOW);
        loop {
            match self.read_packet_until(until)? {
                Some(_) => {}
                None if self.pending.is_empty() => return Ok(()),
                // Reusing a stream with half of an old packet would allow that
                // report to complete after publish and cross the freshness
                // boundary. Force the caller to reconnect instead.
                None => return Err(Error::Connection),
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

    /// Reads the next packet, buffering partial ones.
    ///
    /// A read timeout may land in the middle of a packet, so consumed bytes are
    /// kept: resuming a fresh `read_exact` there would desynchronize the stream.
    fn read_packet_until(&mut self, until: Instant) -> Result<Option<MqttPacket>, Error> {
        loop {
            if let Some(packet) = take_mqtt_packet(&mut self.pending)? {
                return Ok(Some(packet));
            }
            let timeout = match until.checked_duration_since(Instant::now()) {
                Some(timeout) if !timeout.is_zero() => timeout,
                _ => return Ok(None),
            };
            set_socket_idle_timeout(self.stream.get_ref(), timeout)?;
            let mut buffer = [0; 8192];
            match self.stream.read(&mut buffer) {
                Ok(0) => return Err(Error::Connection),
                Ok(read) => self.pending.extend_from_slice(&buffer[..read]),
                Err(error) if is_timeout(&error) => return Ok(None),
                Err(error) => return Err(map_io(error)),
            }
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

/// Removes the next complete packet from `buffer`, leaving partial ones behind.
fn take_mqtt_packet(buffer: &mut Vec<u8>) -> Result<Option<MqttPacket>, Error> {
    let Some(kind) = buffer.first().copied() else {
        return Ok(None);
    };
    let mut length = 0usize;
    let mut multiplier = 1usize;
    for index in 0..4 {
        let Some(encoded) = buffer.get(1 + index).copied() else {
            return Ok(None);
        };
        length += usize::from(encoded & 0x7f) * multiplier;
        if encoded & 0x80 == 0 {
            if length > MAX_MQTT_PACKET_SIZE {
                return Err(Error::InvalidResponse);
            }
            let start = 2 + index;
            let end = start + length;
            if buffer.len() < end {
                return Ok(None);
            }
            let payload = buffer[start..end].to_vec();
            buffer.drain(..end);
            return Ok(Some(MqttPacket { kind, payload }));
        }
        multiplier *= 128;
    }
    Err(Error::InvalidResponse)
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

/// Fails the exchange when the printer refuses the command we just sent.
fn command_rejection(report: &Value, sequence: Option<&str>) -> Result<(), Error> {
    let Some(command) = matching_command_envelope(report, sequence) else {
        return Ok(());
    };
    if !string(command.get("result")).is_some_and(|result| result.eq_ignore_ascii_case("fail")) {
        return Ok(());
    }
    let reason = string(command.get("reason")).unwrap_or_default();
    if integer(command.get("err_code")) == Some(84_033_543)
        || integer(command.get("errno")) == Some(84_033_543)
        || reason.to_ascii_lowercase().contains("verification failed")
    {
        return Err(Error::UnsignedCommand);
    }
    Err(Error::CommandRejected)
}

fn report_matches_sequence(report: &Value, sequence: Option<&str>) -> bool {
    matching_command_envelope(report, sequence).is_some()
}

fn matching_command_envelope<'a>(
    report: &'a Value,
    sequence: Option<&str>,
) -> Option<&'a Map<String, Value>> {
    let sequence = sequence?;
    ["print", "system", "pushing"]
        .into_iter()
        .filter_map(|key| report.get(key).and_then(Value::as_object))
        .find(|command| command.get("sequence_id").and_then(Value::as_str) == Some(sequence))
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

fn ledctrl_payload(node: &str, state: LightState) -> String {
    json!({
        "system": {
            "sequence_id": next_sequence_id(),
            "command": "ledctrl",
            "led_node": node,
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
    // ponytail: prints from the external spool only. Add use_ams plus a real
    // ams_mapping to JobStartOptions when AMS slot selection is exposed.
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
        print.insert(
            "url".into(),
            Value::String(format!("file:///{FILE_ROOT}{path}")),
        );
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
        // Firmware reports PAUSE; PAUSED is accepted for third-party stacks.
        Some("PAUSE" | "PAUSED") => PrinterState::Paused,
        Some("FAILED") => PrinterState::Error,
        Some(_) => PrinterState::Unknown,
        None => return Err(Error::InvalidResponse),
    };
    let nozzle = heater(print, "nozzle_temper", "nozzle_target_temper");
    let bed = heater(print, "bed_temper", "bed_target_temper");
    let chamber = chamber_temperature(print);
    let temperatures =
        (nozzle.is_some() || bed.is_some() || chamber.is_some()).then_some(Temperatures {
            nozzle,
            bed,
            chamber,
        });
    let mut warnings = Vec::new();
    if temperatures.is_none() {
        warnings.push(StatusWarning {
            code: "temperature_data_unavailable",
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
            code: "progress_unavailable",
            message: "progress unavailable",
        });
    }
    let job_name = string(print.get("subtask_name"))
        .filter(|value| !value.is_empty())
        .or_else(|| string(print.get("gcode_file")).filter(|value| !value.is_empty()));
    let job = job_name.map(|name| Job {
        id: synthetic_job_id(&name),
        name,
    });
    let errors = status_errors(print, state);
    let extension = BambuExtension {
        ams: ams_data(print),
        sd_card_state: sd_card_state(print),
        emmc_storage: has_emmc(print),
        reported_ip: string(print.get("wifi_ip")).filter(|value| !value.is_empty()),
    };
    Ok(Status {
        state,
        temperatures,
        job,
        progress,
        errors,
        warnings,
        fans: status_fans(print),
        time_estimates: time_estimates(print),
        speed_level: speed_level(print),
        wifi: wifi(print),
        lights: lights(report),
        print_meta: print_meta(print),
        stage: stage(print),
        timelapse: timelapse(print),
        gcode_position: gcode_position(print),
        firmware_version: firmware_version(print),
        extensions: Extensions {
            bambu_lan: (!extension.is_empty()).then_some(extension),
        },
    })
}

/// Chamber readings move between firmware generations: X1-class printers report
/// a flat `chamber_temper`, while H2-class printers expose the chamber
/// temperature controller at `device.ctc.info.temp`.
// ponytail: two known layouts, not a recursive key search. Add another arm if a
// model reports the chamber somewhere else.
fn chamber_temperature(print: &Map<String, Value>) -> Option<Temperature> {
    let current_celsius = number(print.get("chamber_temper"))
        .or_else(|| number(print.get("device")?.get("ctc")?.get("info")?.get("temp")))?;
    Some(Temperature {
        current_celsius,
        target_celsius: None,
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
            code: "printer_error",
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
            code: "hardware_error",
            message: "printer reported a hardware error".into(),
        });
    }
    if errors.is_empty() && state == PrinterState::Error {
        errors.push(StatusError {
            code: "printer_error",
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
            let percent = (value.clamp(0, 15) * 100 + 7) / 15;
            (name.to_owned(), percent.try_into().unwrap_or(100))
        })
    })
    .collect()
}

/// Remaining time is reported in minutes; H2-class firmware renames the field.
fn time_estimates(print: &Map<String, Value>) -> Option<TimeEstimates> {
    let minutes = integer(print.get("mc_remaining_time"))
        .or_else(|| integer(print.get("remain_time")))?
        .max(0);
    Some(TimeEstimates {
        elapsed_seconds: 0,
        remaining_seconds: (minutes * 60).try_into().ok(),
        total_seconds: None,
    })
}

fn speed_level(print: &Map<String, Value>) -> Option<String> {
    let level = integer(print.get("spd_lvl"))?;
    Some(
        match level {
            1 => "silent",
            2 => "standard",
            3 => "sport",
            4 => "ludicrous",
            _ => return Some(level.to_string()),
        }
        .to_owned(),
    )
}

/// H2-class firmware sends the signal as `"-69dBm"` rather than a number.
fn wifi(print: &Map<String, Value>) -> Option<Wifi> {
    let raw = print.get("wifi_signal")?;
    let signal = integer(Some(raw)).or_else(|| {
        let text = raw.as_str()?.trim();
        text.strip_suffix("dBm")
            .or_else(|| text.strip_suffix("dbm"))
            .unwrap_or(text)
            .trim()
            .parse()
            .ok()
    })?;
    Some(Wifi {
        signal_dbm: signal.try_into().ok()?,
    })
}

fn lights(report: &Value) -> BTreeMap<String, String> {
    lights_report(report)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let node = string(entry.get("node")).filter(|value| !value.is_empty())?;
                    let mode = string(entry.get("mode")).filter(|value| !value.is_empty())?;
                    Some((node, mode))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn stage(print: &Map<String, Value>) -> Option<&'static str> {
    let stage = integer(print.get("stg_cur"))?;
    match stage {
        0 => Some("printing"),
        1 => Some("auto_bed_leveling"),
        2 => Some("heatbed_preheating"),
        3 => Some("sweeping_xy_mech_mode"),
        4 => Some("changing_filament"),
        5 => Some("m400_pause"),
        6 => Some("filament_runout_pause"),
        7 => Some("heating_hotend"),
        8 => Some("calibrating_extrusion"),
        9 => Some("scanning_bed_surface"),
        10 => Some("inspecting_first_layer"),
        11 => Some("identifying_build_plate_type"),
        14 => Some("cleaning_nozzle_tip"),
        15 => Some("checking_extruder_temperature"),
        16 => Some("paused_user_input"),
        17 => Some("paused_front_cover_falling"),
        18 => Some("calibrating_micro_lidar"),
        19 => Some("calibrating_extrusion_flow"),
        20 => Some("paused_nozzle_temperature_malfunction"),
        21 => Some("paused_heat_bed_temperature_malfunction"),
        _ => None,
    }
}

fn print_meta(print: &Map<String, Value>) -> Option<PrintMeta> {
    let file_name = string(print.get("gcode_file"))
        .filter(|value| !value.is_empty())
        .or_else(|| string(print.get("subtask_name")).filter(|value| !value.is_empty()))?;
    Some(PrintMeta {
        file_name,
        file_size: integer(print.get("file_size"))
            .filter(|value| *value > 0)
            .and_then(|value| value.try_into().ok()),
        nozzle_diameter: number(print.get("nozzle_diameter")).filter(|value| *value > 0.0),
        bed_type: string(print.get("bed_type")).filter(|value| !value.is_empty()),
    })
}

/// H2-class firmware nests the timelapse flag inside `ipcam`.
fn timelapse(print: &Map<String, Value>) -> Option<Timelapse> {
    let state = string(print.get("timelapse")).or_else(|| {
        string(
            print
                .get("ipcam")
                .and_then(|ipcam| ipcam.get("timelapse"))
                .or_else(|| print.get("ipcam")?.get("timelapse_stat")),
        )
    })?;
    Some(Timelapse {
        recording: state == "enable",
        progress: None,
        ready: None,
    })
}

fn gcode_position(print: &Map<String, Value>) -> Option<GcodePosition> {
    Some(GcodePosition {
        // ponytail: Z height is not reliably present in pushall, same as Go.
        z_mm: 0.0,
        current_line: integer(print.get("mc_print_line_number"))
            .or_else(|| integer(print.get("cur_line_num")))?,
        total_lines: integer(print.get("total_line_num"))?,
    })
}

fn firmware_version(print: &Map<String, Value>) -> Option<String> {
    string(print.get("ota_version")).filter(|value| !value.is_empty())
}

fn ams_humidity(index: i64) -> Option<(&'static str, &'static str)> {
    match index {
        1 => Some(("< 10%", "very dry")),
        2 => Some(("10-20%", "dry")),
        3 => Some(("20-30%", "moderate")),
        4 => Some(("30-40%", "slightly humid")),
        5 => Some(("> 40%", "humid")),
        _ => None,
    }
}

fn ams_data(print: &Map<String, Value>) -> Option<AmsData> {
    let mut units: Vec<AmsUnit> = print
        .get("ams")
        .and_then(|ams| ams.get("ams"))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().map(ams_unit).collect())
        .unwrap_or_default();
    units.extend(virtual_tray_units(print));
    (!units.is_empty()).then_some(AmsData { units })
}

fn ams_unit(unit: &Value) -> AmsUnit {
    let humidity = integer(unit.get("humidity")).and_then(ams_humidity);
    AmsUnit {
        id: integer(unit.get("id"))
            .and_then(|value| value.try_into().ok())
            .unwrap_or_default(),
        humidity_range: humidity.map(|(range, _)| range),
        humidity_level: humidity.map(|(_, level)| level),
        temperature: number(unit.get("temp")).filter(|value| *value > 0.0),
        trays: unit
            .get("tray")
            .and_then(Value::as_array)
            .map(|trays| {
                trays
                    .iter()
                    .map(|tray| AmsTray {
                        slot: integer(tray.get("id"))
                            .and_then(|value| value.try_into().ok())
                            .unwrap_or_default(),
                        filament_type: string(tray.get("tray_type"))
                            .filter(|value| !value.is_empty()),
                        color: string(tray.get("tray_color")).filter(|value| !value.is_empty()),
                        remaining_percent: integer(tray.get("remain"))
                            .and_then(|value| value.try_into().ok()),
                        nozzle_temp_min: integer(tray.get("nozzle_temp_min"))
                            .and_then(|value| value.try_into().ok()),
                        nozzle_temp_max: integer(tray.get("nozzle_temp_max"))
                            .and_then(|value| value.try_into().ok()),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// External spool holders report outside the AMS array: `vt_tray` on the A1
/// Mini, `vir_slot` on the H2. Each loaded holder becomes a single-slot unit,
/// conventionally numbered 254.
fn virtual_tray_units(print: &Map<String, Value>) -> Vec<AmsUnit> {
    let trays = print
        .get("vt_tray")
        .into_iter()
        .chain(
            print
                .get("vir_slot")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )
        .filter(|tray| {
            string(tray.get("tray_type")).is_some_and(|value| !value.is_empty())
                || virtual_tray_color(tray).is_some()
        });
    trays
        .map(|tray| AmsUnit {
            id: integer(tray.get("id"))
                .and_then(|value| value.try_into().ok())
                .unwrap_or(254),
            humidity_range: None,
            humidity_level: None,
            temperature: None,
            trays: vec![AmsTray {
                slot: 0,
                filament_type: string(tray.get("tray_type")).filter(|value| !value.is_empty()),
                color: virtual_tray_color(tray),
                remaining_percent: integer(tray.get("remain"))
                    .filter(|value| *value > 0)
                    .and_then(|value| value.try_into().ok()),
                nozzle_temp_min: integer(tray.get("nozzle_temp_min"))
                    .filter(|value| *value > 0)
                    .and_then(|value| value.try_into().ok()),
                nozzle_temp_max: integer(tray.get("nozzle_temp_max"))
                    .filter(|value| *value > 0)
                    .and_then(|value| value.try_into().ok()),
            }],
        })
        .collect()
}

fn virtual_tray_color(tray: &Value) -> Option<String> {
    string(tray.get("tray_color")).filter(|value| !value.is_empty() && value != "00000000")
}

/// SD card state lives in `home_flag` bits [8:9] rather than a dedicated field.
fn sd_card_state(print: &Map<String, Value>) -> Option<&'static str> {
    match (integer(print.get("home_flag"))? >> 8) & 0x3 {
        0 => Some("none"),
        1 => Some("normal"),
        2 => Some("abnormal"),
        3 => Some("readonly"),
        _ => None,
    }
}

/// eMMC support is advertised by bit 17 of the hex `fun2` capability mask.
fn has_emmc(print: &Map<String, Value>) -> Option<bool> {
    let raw = string(print.get("fun2")).filter(|value| !value.is_empty())?;
    let bits = u64::from_str_radix(raw.trim(), 16).ok()?;
    (bits & (1 << 17) != 0).then_some(true)
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
    control: SslStream<TcpStream>,
    connector: SslConnector,
    profile: Profile,
    fingerprint: Option<String>,
    /// Idle budget for one command, reply, or transfer chunk.
    timeout: Duration,
    /// DER-encoded control session, re-decoded per data connection.
    ///
    /// OpenSSL retires an `SSL_SESSION` once it has been used to resume, so the
    /// second transfer would be refused with `522 session reuse required`.
    /// Decoding a fresh object from these bytes keeps every data connection
    /// resumable, which is what the printer's FTP server demands.
    session: Option<Vec<u8>>,
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
        let timeout = profile.timeout();
        let connector = tls_connector()?;
        let (control, _) = open_tls(
            &connector,
            profile,
            FTP_PORT,
            fingerprint,
            true,
            deadline_after(timeout)?,
        )?;
        let session = control
            .ssl()
            .session()
            .and_then(|session| session.to_der().ok());
        let mut connection = Self {
            control,
            connector,
            profile: profile.clone(),
            fingerprint: fingerprint.map(str::to_owned),
            timeout,
            session,
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
        // RFC 4217 leaves the data channel in the clear until PROT P, and the
        // printer refuses a data connection that does not resume this session.
        connection.expect_command("PBSZ 0", &[200])?;
        connection.expect_command("PROT P", &[200])?;
        Ok(connection)
    }

    fn list(&mut self, path: &str) -> Result<Vec<FileEntry>, Error> {
        let socket = self.passive_socket()?;
        let reply = self.command(&format!("LIST {path}"))?;
        if !matches!(reply.code, 125 | 150) {
            return Err(Error::FileTransfer);
        }
        let mut data = self.start_data_tls(socket)?;
        let bytes = read_data(&mut data, self.timeout)?;
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
            if all.len() > MAX_LIST_ENTRIES {
                return Err(Error::FileTransfer);
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
        let socket = self.passive_socket()?;
        let reply = self.command(&format!("RETR {path}"))?;
        if !matches!(reply.code, 125 | 150) {
            return Err(Error::FileTransfer);
        }
        let mut data = self.start_data_tls(socket)?;
        let result = copy_data(&mut data, destination);
        drop(data);
        self.expect(&[226, 250])?;
        result
    }

    fn upload(&mut self, path: &str, source: &mut dyn Read) -> Result<u64, Error> {
        let socket = self.passive_socket()?;
        let reply = self.command(&format!("STOR {path}"))?;
        if !matches!(reply.code, 125 | 150) {
            return Err(Error::FileTransfer);
        }
        let mut data = self.start_data_tls(socket)?;
        let result = copy_data(source, &mut data);
        let _ = data.flush();
        let _ = data.shutdown();
        drop(data);
        self.expect(&[226, 250])?;
        result
    }

    /// Opens the passive data socket without handshaking.
    ///
    /// The printer's FTP server does not service the data channel's TLS
    /// handshake until the transfer command has been accepted on the control
    /// channel, so the handshake is deferred to [`Self::start_data_tls`].
    fn passive_socket(&mut self) -> Result<TcpStream, Error> {
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
        let deadline = deadline_after(self.timeout)?;
        connect_socket(address, deadline)
    }

    fn start_data_tls(&mut self, socket: TcpStream) -> Result<SslStream<TcpStream>, Error> {
        let deadline = deadline_after(self.timeout)?;
        let session = match &self.session {
            Some(der) => Some(SslSession::from_der(der).map_err(|_| Error::Tls)?),
            None => None,
        };
        let (data, _) = open_tls_socket(
            &self.connector,
            &self.profile,
            socket,
            self.fingerprint.as_deref(),
            true,
            deadline,
            session.as_deref(),
        )?;
        set_socket_idle_timeout(data.get_ref(), self.timeout)?;
        Ok(data)
    }

    fn command(&mut self, command: &str) -> Result<FtpReply, Error> {
        if command.contains(['\r', '\n']) {
            return Err(Error::InvalidDevicePath);
        }
        set_socket_idle_timeout(self.control.get_ref(), self.timeout)?;
        // The A1 firmware parses one TLS record per command, so the terminator
        // has to share a record with the verb or every later reply is off by one.
        let line = format!("{command}\r\n");
        self.control
            .write_all(line.as_bytes())
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
            set_socket_idle_timeout(self.control.get_ref(), self.timeout)?;
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

fn read_data(data: &mut SslStream<TcpStream>, timeout: Duration) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        set_socket_idle_timeout(data.get_ref(), timeout)?;
        let read = match data.read(&mut buffer) {
            Ok(read) => read,
            // The printer closes the data connection to mark the end of a
            // listing, sometimes without sending a TLS close_notify first.
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => 0,
            Err(error) => return Err(map_io(error)),
        };
        if read == 0 {
            return Ok(bytes);
        }
        if (bytes.len() + read) as u64 > MAX_FTP_LISTING_SIZE {
            return Err(Error::FileTransfer);
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
}

/// Streams a transfer, bounded by the socket idle timeouts set by the caller
/// rather than by a wall clock, so a large file is not cut off part way.
fn copy_data(source: &mut dyn Read, destination: &mut dyn Write) -> Result<u64, Error> {
    let mut buffer = [0; 8192];
    let mut copied = 0_u64;
    loop {
        let read = match source.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => 0,
            Err(error) => return Err(map_io(error)),
        };
        if read == 0 {
            return Ok(copied);
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
                modified_at: entry.modified_at,
                metadata: BTreeMap::new(),
            }
        })
        .collect()
}

struct ParsedFtpEntry {
    name: String,
    entry_type: FileEntryType,
    size_bytes: Option<i64>,
    modified_at: Option<String>,
}

fn parse_ftp_list_line(line: &str) -> Option<ParsedFtpEntry> {
    let entry_type = match line.as_bytes().first().copied()? {
        b'd' => FileEntryType::Directory,
        b'-' => FileEntryType::File,
        _ => return None,
    };
    // Mode, links, owner, group, size, month, day, and time or year. The name
    // is whatever follows, taken whole so runs of spaces inside it survive.
    let mut rest = line;
    let mut size = "";
    let (mut month, mut day, mut time_or_year) = ("", "", "");
    for field in 0..8 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        match field {
            4 => size = &rest[..end],
            5 => month = &rest[..end],
            6 => day = &rest[..end],
            7 => time_or_year = &rest[..end],
            _ => {}
        }
        rest = &rest[end..];
    }
    let name = rest.trim_start();
    (!name.is_empty()).then(|| ParsedFtpEntry {
        name: name.to_owned(),
        entry_type,
        size_bytes: (entry_type == FileEntryType::File)
            .then(|| size.parse().ok())
            .flatten(),
        modified_at: ftp_modified_at(month, day, time_or_year),
    })
}

/// Unix `LIST` output carries either a clock time (recent entries, year
/// implied) or a year (older entries). The implied year is the most recent one
/// that does not place the entry in the future.
fn ftp_modified_at(month: &str, day: &str, time_or_year: &str) -> Option<String> {
    let month = match month {
        "Jan" => Month::January,
        "Feb" => Month::February,
        "Mar" => Month::March,
        "Apr" => Month::April,
        "May" => Month::May,
        "Jun" => Month::June,
        "Jul" => Month::July,
        "Aug" => Month::August,
        "Sep" => Month::September,
        "Oct" => Month::October,
        "Nov" => Month::November,
        "Dec" => Month::December,
        _ => return None,
    };
    let day: u8 = day.parse().ok()?;
    let now = OffsetDateTime::now_utc();
    let (year, hour, minute) = match time_or_year.split_once(':') {
        Some((hour, minute)) => {
            let candidate = Date::from_calendar_date(now.year(), month, day).ok()?;
            let year = if candidate > now.date() {
                now.year() - 1
            } else {
                now.year()
            };
            (year, hour.parse().ok()?, minute.parse().ok()?)
        }
        None => (time_or_year.parse().ok()?, 0, 0),
    };
    let date = Date::from_calendar_date(year, month, day).ok()?;
    let time = Time::from_hms(hour, minute, 0).ok()?;
    date.with_time(time).assume_utc().format(&Rfc3339).ok()
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        net::TcpListener,
        sync::{Arc, mpsc},
        thread,
    };

    use openssl::{
        asn1::Asn1Time,
        bn::BigNum,
        hash::MessageDigest,
        pkey::PKey,
        rsa::Rsa,
        ssl::{SslAcceptor, SslMethod},
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
            b"drwxr-xr-x 1 root root 0 Jan 01 12:00 models\r\n-rw-r--r--   1 root root    12 Jan 01 12:00 big  cube.3mf\r\n",
            "/",
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].device_path, "sdcard:/models");
        assert_eq!(entries[1].size_bytes, Some(12));
        // Runs of spaces inside a name are part of the name on the printer.
        assert_eq!(entries[1].name, "big  cube.3mf");
        assert_eq!(entries[1].device_path, "sdcard:/big  cube.3mf");
    }

    #[test]
    fn ftp_timestamps_resolve_the_year_the_listing_leaves_implicit() {
        // A clock time means the most recent occurrence, never the future.
        let year = OffsetDateTime::now_utc().year();
        let recent = ftp_modified_at("Jan", "1", "12:00").unwrap();
        assert!(
            recent == format!("{year}-01-01T12:00:00Z")
                || recent == format!("{}-01-01T12:00:00Z", year - 1),
            "{recent}"
        );
        assert!(recent <= OffsetDateTime::now_utc().format(&Rfc3339).unwrap());
        // A year in place of the clock means midnight on that day.
        assert_eq!(
            ftp_modified_at("Jun", "19", "2024").as_deref(),
            Some("2024-06-19T00:00:00Z")
        );
        assert_eq!(ftp_modified_at("Foo", "19", "2024"), None);
    }

    #[test]
    fn external_spool_holders_become_a_virtual_ams_unit() {
        let print = serde_json::from_str::<Value>(
            r#"{"vt_tray":{"id":"254","tray_type":"PLA","tray_color":"F6DA5AFF","remain":"-1","nozzle_temp_min":"190","nozzle_temp_max":"240"},
                "vir_slot":[{"id":"255","tray_type":"","tray_color":"00000000"}]}"#,
        )
        .unwrap();
        let print = print.as_object().unwrap();

        let units = ams_data(print).unwrap().units;

        // The empty H2 slot is dropped; only the loaded holder is reported.
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].id, 254);
        assert_eq!(units[0].trays[0].slot, 0);
        assert_eq!(units[0].trays[0].filament_type.as_deref(), Some("PLA"));
        assert_eq!(units[0].trays[0].color.as_deref(), Some("F6DA5AFF"));
        // Unknown remaining filament is omitted rather than reported as -1%.
        assert_eq!(units[0].trays[0].remaining_percent, None);
        assert_eq!(units[0].trays[0].nozzle_temp_max, Some(240));
    }

    #[test]
    fn command_rejections_are_scoped_to_the_active_sequence() {
        let rejection: Value = serde_json::from_str(
            r#"{"print":{"sequence_id":"7","result":"fail","reason":"verification failed"}}"#,
        )
        .unwrap();
        assert!(matches!(
            command_rejection(&rejection, Some("7")),
            Err(Error::UnsignedCommand)
        ));
        assert!(command_rejection(&rejection, Some("other")).is_ok());
    }

    #[test]
    fn command_rejections_select_the_envelope_with_the_active_sequence() {
        let report = json!({
            "print": {"sequence_id": "unrelated", "gcode_state": "IDLE"},
            "system": {
                "sequence_id": "active",
                "result": "fail",
                "reason": "verification failed"
            }
        });

        assert!(matches!(
            command_rejection(&report, Some("active")),
            Err(Error::UnsignedCommand)
        ));
        assert!(report_matches_sequence(&report, Some("active")));
        assert!(!report_matches_sequence(&report, Some("missing")));
    }

    #[test]
    fn command_acknowledgements_are_scoped_to_the_active_sequence() {
        let stale_status: Value =
            serde_json::from_str(r#"{"print":{"sequence_id":"previous","gcode_state":"IDLE"}}"#)
                .unwrap();
        let acknowledgement: Value =
            serde_json::from_str(r#"{"print":{"sequence_id":"current","result":"success"}}"#)
                .unwrap();

        assert!(!report_matches_sequence(&stale_status, Some("current")));
        assert!(report_matches_sequence(&acknowledgement, Some("current")));
    }

    #[test]
    fn partial_packets_stay_buffered_until_the_rest_arrives() {
        let mut packet = vec![0x30];
        mqtt_remaining_length(&mut packet, 300).unwrap();
        packet.extend_from_slice(&[7; 300]);

        // A read timeout can land anywhere, so every split must be resumable.
        for split in [1, 2, 3, 10, packet.len() - 1] {
            let mut buffer = packet[..split].to_vec();
            assert!(take_mqtt_packet(&mut buffer).unwrap().is_none(), "{split}");
            buffer.extend_from_slice(&packet[split..]);
            let taken = take_mqtt_packet(&mut buffer).unwrap().unwrap();
            assert_eq!(taken.kind, 0x30);
            assert_eq!(taken.payload.len(), 300);
            assert!(buffer.is_empty());
        }
    }

    #[test]
    fn maps_the_firmware_pause_state_and_rounds_reported_fan_speeds() {
        let status = parse_status(
            br#"{"print":{"gcode_state":"PAUSE","subtask_name":"cube.3mf","mc_percent":10,"cooling_fan_speed":"7","big_fan1_speed":15}}"#,
        )
        .unwrap();

        assert_eq!(status.state, PrinterState::Paused);
        assert!(status.job.is_some());
        assert_eq!(status.fans["partCooling"], 47);
        assert_eq!(status.fans["auxiliary"], 100);
    }

    #[test]
    fn starts_project_files_from_the_printer_sd_card() {
        let payload: Value = serde_json::from_str(
            &job_start_payload("/models/cube.3mf", JobStartOptions::default()).unwrap(),
        )
        .unwrap();

        assert_eq!(payload["print"]["command"], "project_file");
        assert_eq!(payload["print"]["url"], "file:///sdcard/models/cube.3mf");
        assert_eq!(payload["print"]["param"], "Metadata/plate_1.gcode");
    }

    #[test]
    fn builds_chamber_ledctrl_payload_and_acknowledges_reported_state() {
        let payload: Value =
            serde_json::from_str(&ledctrl_payload("chamber_light", LightState::On)).unwrap();
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

    #[test]
    fn poll_status_reuses_a_cached_session_instead_of_reconnecting() {
        // The server accepts exactly one TCP connection and answers two
        // status exchanges over it — a `poll_status` that reconnected on the
        // second call would hang waiting on a connection nothing accepts.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = address.ip().to_string();
        let port = address.port();
        let acceptor = test_acceptor();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let mut stream = acceptor.accept(socket).unwrap();
            let connect = read_test_packet(&mut stream);
            assert_eq!(connect.kind, 0x10);
            stream.write_all(&[0x20, 0x02, 0, 0]).unwrap();
            stream.flush().unwrap();
            let subscribe = read_test_packet(&mut stream);
            assert_eq!(subscribe.kind, 0x82);
            let id = &subscribe.payload[..2];
            let stale_report = br#"{"print":{"gcode_state":"PRINTING","mc_percent":75}}"#;
            let mut stale_payload = Vec::new();
            mqtt_string(&mut stale_payload, "device/SN001/report").unwrap();
            stale_payload.extend_from_slice(stale_report);
            let mut subscribe_and_stale = vec![0x90, 0x03, id[0], id[1], 0, 0x30];
            mqtt_remaining_length(&mut subscribe_and_stale, stale_payload.len()).unwrap();
            subscribe_and_stale.extend_from_slice(&stale_payload);
            stream.write_all(&subscribe_and_stale).unwrap();
            stream.flush().unwrap();

            for _ in 0..2 {
                let publish = read_test_packet(&mut stream);
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

        let profile = Profile::with_timeout(host, "SN001", true, Duration::from_secs(2)).unwrap();
        let connector = tls_connector().unwrap();
        let deadline = deadline_after(profile.timeout()).unwrap();
        let (stream, _) = open_tls(&connector, &profile, port, None, false, deadline).unwrap();
        let mut mqtt = MqttConnection::new(stream, profile.mqtt_topics(), deadline);
        mqtt.connect("access-code").unwrap();
        mqtt.subscribe().unwrap();
        let identity = profile.connection_identity(Some("access-code"), None);
        let client = Client::new(profile);
        *client.mqtt.lock().unwrap() = Some(CachedConnection { identity, mqtt });
        let first = client
            .poll_status(Some("access-code"), None, Duration::from_secs(2))
            .unwrap();
        assert!(
            client.mqtt.lock().unwrap().is_some(),
            "a successful poll keeps the session cached"
        );
        let second = client
            .poll_status(Some("access-code"), None, Duration::from_secs(2))
            .unwrap();

        server.join().unwrap();
        assert_eq!(first.state, PrinterState::Idle);
        assert_eq!(second.state, PrinterState::Idle);
        let changed_access_code = client
            .profile
            .connection_identity(Some("different-access-code"), None);
        assert!(
            matching_cached_connection(&mut client.mqtt.lock().unwrap(), changed_access_code)
                .is_none(),
            "credentials changes must discard an authenticated cached session"
        );
    }

    #[test]
    fn mqtt_lock_wait_is_bounded_by_the_status_deadline() {
        let profile =
            Profile::with_timeout("127.0.0.1", "SN001", true, Duration::from_millis(20)).unwrap();
        let client = Arc::new(Client::new(profile));
        let holder = client.clone();
        let (locked_tx, locked_rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            let _guard = holder.mqtt.lock().unwrap();
            locked_tx.send(()).unwrap();
            thread::sleep(Duration::from_millis(100));
        });
        locked_rx.recv().unwrap();

        let started = Instant::now();
        let result = client.poll_status(Some("access-code"), None, Duration::from_millis(20));

        assert!(matches!(result, Err(Error::Timeout)));
        assert!(started.elapsed() < Duration::from_millis(80));
        thread.join().unwrap();
    }

    #[test]
    fn cached_status_retry_shares_one_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = address.ip().to_string();
        let port = address.port();
        let acceptor = test_acceptor();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let mut stream = acceptor.accept(socket).unwrap();
            assert_eq!(read_test_packet(&mut stream).kind, 0x10);
            stream.write_all(&[0x20, 0x02, 0, 0]).unwrap();
            stream.flush().unwrap();
            let subscribe = read_test_packet(&mut stream);
            let id = &subscribe.payload[..2];
            stream.write_all(&[0x90, 0x03, id[0], id[1], 0]).unwrap();
            stream.flush().unwrap();
            assert_eq!(read_test_packet(&mut stream).kind, 0x30);
            thread::sleep(Duration::from_millis(100));
        });

        let profile = Profile::with_timeout(host, "SN001", true, Duration::from_secs(2)).unwrap();
        let connector = tls_connector().unwrap();
        let deadline = deadline_after(profile.timeout()).unwrap();
        let (stream, _) = open_tls(&connector, &profile, port, None, false, deadline).unwrap();
        let mut mqtt = MqttConnection::new(stream, profile.mqtt_topics(), deadline);
        mqtt.connect("access-code").unwrap();
        mqtt.subscribe().unwrap();
        let identity = profile.connection_identity(Some("access-code"), None);
        let client = Client::new(profile);
        *client.mqtt.lock().unwrap() = Some(CachedConnection { identity, mqtt });

        let started = Instant::now();
        let result = client.poll_status(Some("access-code"), None, Duration::from_millis(20));

        assert!(matches!(result, Err(Error::Timeout)));
        assert!(started.elapsed() < Duration::from_millis(80));
        server.join().unwrap();
    }

    #[test]
    fn commands_reuse_a_cached_session_with_sequence_correlated_reports() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = address.ip().to_string();
        let port = address.port();
        let acceptor = test_acceptor();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let mut stream = acceptor.accept(socket).unwrap();
            let connect = read_test_packet(&mut stream);
            assert_eq!(connect.kind, 0x10);
            stream.write_all(&[0x20, 0x02, 0, 0]).unwrap();
            let subscribe = read_test_packet(&mut stream);
            let id = &subscribe.payload[..2];
            stream.write_all(&[0x90, 0x03, id[0], id[1], 0]).unwrap();
            stream.flush().unwrap();

            for _ in 0..2 {
                let command = read_test_packet(&mut stream);
                let command = mqtt_publish_payload(command.kind, &command.payload).unwrap();
                let sequence = payload_sequence_id(&command).unwrap();
                let refresh = read_test_packet(&mut stream);
                assert!(is_pushall_payload(
                    &mqtt_publish_payload(refresh.kind, &refresh.payload).unwrap()
                ));

                for report in [
                    json!({"print": {"sequence_id": sequence, "result": "success"}}),
                    json!({"print": {"gcode_state": "IDLE", "mc_percent": 0}}),
                ] {
                    let mut payload = Vec::new();
                    mqtt_string(&mut payload, "device/SN001/report").unwrap();
                    payload.extend_from_slice(report.to_string().as_bytes());
                    let mut packet = vec![0x30];
                    mqtt_remaining_length(&mut packet, payload.len()).unwrap();
                    packet.extend_from_slice(&payload);
                    stream.write_all(&packet).unwrap();
                }
                stream.flush().unwrap();
            }
        });

        let profile = Profile::with_timeout(host, "SN001", true, Duration::from_secs(2)).unwrap();
        let connector = tls_connector().unwrap();
        let deadline = deadline_after(profile.timeout()).unwrap();
        let (stream, _) = open_tls(&connector, &profile, port, None, false, deadline).unwrap();
        let mut mqtt = MqttConnection::new(stream, profile.mqtt_topics(), deadline);
        mqtt.connect("access-code").unwrap();
        mqtt.subscribe().unwrap();
        let identity = profile.connection_identity(Some("access-code"), None);
        let client = Client::new(profile);
        *client.mqtt.lock().unwrap() = Some(CachedConnection { identity, mqtt });

        client.emergency_stop(Some("access-code"), None).unwrap();
        client.emergency_stop(Some("access-code"), None).unwrap();

        server.join().unwrap();
        assert!(client.mqtt.lock().unwrap().is_some());
    }

    fn mqtt_server() -> (String, u16, mpsc::Receiver<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = address.ip().to_string();
        let acceptor = test_acceptor();
        let (sender, credentials) = mpsc::channel();
        let server = thread::spawn(move || {
            for status_request in [false, true] {
                let (socket, _) = listener.accept().unwrap();
                let mut stream = acceptor.accept(socket).unwrap();
                let connect = read_test_packet(&mut stream);
                assert_eq!(connect.kind, 0x10);
                sender.send(connect_password(&connect.payload)).unwrap();
                stream.write_all(&[0x20, 0x02, 0, 0]).unwrap();
                stream.flush().unwrap();

                if !status_request {
                    continue;
                }
                let subscribe = read_test_packet(&mut stream);
                assert_eq!(subscribe.kind, 0x82);
                let id = &subscribe.payload[..2];
                stream.write_all(&[0x90, 0x03, id[0], id[1], 0]).unwrap();
                stream.flush().unwrap();
                let publish = read_test_packet(&mut stream);
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

    fn read_test_packet(stream: &mut impl Read) -> MqttPacket {
        let mut pending = Vec::new();
        loop {
            if let Some(packet) = take_mqtt_packet(&mut pending).unwrap() {
                return packet;
            }
            let mut chunk = [0; 512];
            let read = stream.read(&mut chunk).unwrap();
            assert!(read > 0, "stream closed mid-packet");
            pending.extend_from_slice(&chunk[..read]);
        }
    }

    /// The printer's FTP server refuses a data connection whose TLS session was
    /// already used to resume (`522 session reuse required`), so the connection
    /// keeps the control session as DER and decodes a fresh object per
    /// transfer. Guard that a re-decoded session still resumes.
    #[test]
    fn ftp_commands_and_their_terminator_share_one_record() {
        // The A1 firmware reads a whole command per TLS record, so a reply is
        // only ever in step when the CRLF travels with the verb.
        let acceptor = std::sync::Arc::new(test_acceptor());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, records) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut stream = acceptor.accept(listener.accept().unwrap().0).unwrap();
            let mut buffer = [0; 512];
            let read = stream.read(&mut buffer).unwrap();
            sender.send(buffer[..read].to_vec()).unwrap();
            stream.write_all(b"331 \r\n").unwrap();
            stream.flush().unwrap();
        });

        let connector = tls_connector().unwrap();
        let socket = TcpStream::connect(address).unwrap();
        let ssl = connector
            .configure()
            .unwrap()
            .verify_hostname(false)
            .into_ssl("SN001")
            .unwrap();
        let mut control = SslStream::new(ssl, socket).unwrap();
        control.connect().unwrap();
        let mut connection = FtpsConnection {
            control,
            connector,
            profile: Profile::new("127.0.0.1", "SN001", true).unwrap(),
            fingerprint: None,
            timeout: Duration::from_secs(5),
            session: None,
            pending: Vec::new(),
            last_reply: String::new(),
        };

        let reply = connection.command("USER bblp").unwrap();

        server.join().unwrap();
        assert_eq!(reply.code, 331);
        assert_eq!(records.recv().unwrap(), b"USER bblp\r\n");
    }

    #[test]
    fn a_der_encoded_session_resumes_more_than_once() {
        let acceptor = std::sync::Arc::new(test_acceptor());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            for stream in listener.incoming().take(3) {
                let acceptor = acceptor.clone();
                thread::spawn(move || {
                    let _ = acceptor.accept(stream.unwrap());
                });
            }
        });

        let connector = tls_connector().unwrap();
        let handshake = |session: Option<&SslSessionRef>| {
            let socket = TcpStream::connect(address).unwrap();
            let mut ssl = connector
                .configure()
                .unwrap()
                .verify_hostname(false)
                .into_ssl("SN001")
                .unwrap();
            if let Some(session) = session {
                unsafe { ssl.set_session(session) }.unwrap();
            }
            let mut stream = SslStream::new(ssl, socket).unwrap();
            stream.connect().unwrap();
            stream
        };

        let control = handshake(None);
        let der = control.ssl().session().unwrap().to_der().unwrap();

        for attempt in 1..=2 {
            let session = SslSession::from_der(&der).unwrap();
            let data = handshake(Some(&session));
            assert!(
                data.ssl().session_reused(),
                "transfer {attempt} did not resume the control session"
            );
        }
    }

    fn test_acceptor() -> SslAcceptor {
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

        let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        acceptor.set_private_key(&key).unwrap();
        acceptor.set_certificate(&certificate).unwrap();
        acceptor.build()
    }
}
