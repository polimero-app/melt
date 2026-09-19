use std::{fmt, time::Duration};

use serde::Serialize;
use thiserror::Error;

use crate::{
    bambu, config, firmware_updates::FirmwareUpdateReport, moonraker, trace::SharedTracer,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DriverInfo {
    pub name: &'static str,
    pub description: &'static str,
}

pub fn verify(profile: &Profile, access_code: Option<&str>) -> Result<Option<String>, DriverError> {
    match profile {
        Profile::Moonraker(_) => status(profile, access_code, None).map(|_| None),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .verify(access_code)
            .map_err(DriverError::Bambu),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Driver {
    BambuLan,
    Moonraker,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub status: bool,
    pub discovery: bool,
    pub camera_stream: bool,
    pub camera_snapshot: bool,
    pub file_list: bool,
    pub file_download: bool,
    pub file_upload: bool,
    pub file_delete: bool,
    pub job_start: bool,
    pub job_pause: bool,
    pub job_resume: bool,
    pub job_cancel: bool,
    pub emergency_stop: bool,
    pub temperature_read: bool,
    pub temperature_write: bool,
    pub motion_control: bool,
    pub tls_refresh: bool,
    pub fan_control: bool,
    pub light_control: bool,
    pub speed_control: bool,
    pub firmware_update_check: bool,
    pub ams_drying: bool,
}

impl Capabilities {
    pub fn supports(self, operation: Operation) -> bool {
        match operation {
            Operation::Status | Operation::Verify => self.status,
            Operation::Discovery => self.discovery,
            Operation::CameraStream => self.camera_stream,
            Operation::CameraSnapshot => self.camera_snapshot,
            Operation::FileList => self.file_list,
            Operation::FileDownload => self.file_download,
            Operation::FileUpload => self.file_upload,
            Operation::FileDelete => self.file_delete,
            Operation::JobStart => self.job_start,
            Operation::JobPause => self.job_pause,
            Operation::JobResume => self.job_resume,
            Operation::JobCancel => self.job_cancel,
            Operation::EmergencyStop => self.emergency_stop,
            Operation::TemperatureSet => self.temperature_write,
            Operation::MotionHome | Operation::MotionJog => self.motion_control,
            Operation::TlsRefresh => self.tls_refresh,
            Operation::FanSet => self.fan_control,
            Operation::LightSet => self.light_control,
            Operation::SpeedSet => self.speed_control,
            Operation::FirmwareUpdateCheck => self.firmware_update_check,
            Operation::AmsDrying => self.ams_drying,
        }
    }
}

impl Driver {
    pub fn parse(name: &str) -> Result<Self, DriverError> {
        match name {
            "bambu-lan" => Ok(Self::BambuLan),
            "moonraker" => Ok(Self::Moonraker),
            _ => Err(DriverError::Unknown(name.to_owned())),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::BambuLan => "bambu-lan",
            Self::Moonraker => "moonraker",
        }
    }

    pub fn requires_access_code(self) -> bool {
        matches!(self, Self::BambuLan)
    }

    pub fn supports(self, operation: Operation) -> bool {
        self.capabilities().supports(operation)
    }

    pub fn capabilities(self) -> Capabilities {
        match self {
            Self::BambuLan => Capabilities {
                status: true,
                discovery: true,
                camera_stream: true,
                camera_snapshot: true,
                file_list: true,
                file_download: true,
                file_upload: true,
                file_delete: true,
                job_start: true,
                job_pause: true,
                job_resume: true,
                job_cancel: true,
                emergency_stop: true,
                temperature_read: true,
                temperature_write: true,
                motion_control: true,
                tls_refresh: true,
                fan_control: true,
                light_control: true,
                speed_control: true,
                firmware_update_check: true,
                ams_drying: true,
            },
            Self::Moonraker => Capabilities {
                status: true,
                file_list: true,
                file_download: true,
                file_upload: true,
                file_delete: true,
                job_start: true,
                job_pause: true,
                job_resume: true,
                job_cancel: true,
                emergency_stop: true,
                temperature_read: true,
                temperature_write: true,
                motion_control: true,
                fan_control: true,
                speed_control: true,
                firmware_update_check: true,
                ..Capabilities::default()
            },
        }
    }
}

impl fmt::Display for Driver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Status,
    Verify,
    Discovery,
    CameraStream,
    CameraSnapshot,
    FileList,
    FileDownload,
    FileUpload,
    FileDelete,
    JobStart,
    JobPause,
    JobResume,
    JobCancel,
    EmergencyStop,
    TemperatureSet,
    MotionHome,
    MotionJog,
    TlsRefresh,
    FanSet,
    LightSet,
    SpeedSet,
    FirmwareUpdateCheck,
    AmsDrying,
}

#[derive(Clone, Debug)]
pub enum Profile {
    Bambu(bambu::Profile),
    Moonraker(moonraker::Profile),
}

impl Profile {
    pub fn driver(&self) -> Driver {
        match self {
            Self::Bambu(_) => Driver::BambuLan,
            Self::Moonraker(_) => Driver::Moonraker,
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        self.driver().capabilities()
    }

    pub fn bambu_runtime_capabilities(&self) -> Option<bambu::RuntimeCapabilities> {
        match self {
            Self::Bambu(profile) => Some(profile.default_capabilities()),
            Self::Moonraker(_) => None,
        }
    }

    /// Stable ownership key for resources that must be unique per physical
    /// printer. Profile names are only a fallback for drivers without a serial.
    pub fn physical_printer_key(&self, profile_name: &str) -> String {
        match self {
            Self::Bambu(profile) => format!("bambu:{}", profile.serial().to_ascii_uppercase()),
            Self::Moonraker(_) => format!("profile:{}", profile_name.to_ascii_lowercase()),
        }
    }
}

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("unknown printer driver {0:?}")]
    Unknown(String),
    #[error("invalid {0} printer profile")]
    InvalidProfile(Driver),
    #[error("invalid printer timeout")]
    InvalidTimeout,
    #[error("{0} does not support {1:?}")]
    UnsupportedOperation(Driver, Operation),
    #[error("printer status request failed")]
    Moonraker(#[source] moonraker::Error),
    #[error("printer camera request failed")]
    Camera(#[source] bambu::CameraError),
    #[error("Bambu printer request failed")]
    Bambu(#[source] bambu::TransportError),
}

pub fn registered() -> [DriverInfo; 2] {
    [
        DriverInfo {
            name: "bambu-lan",
            description: "Bambu Lab printers over LAN mode",
        },
        DriverInfo {
            name: "moonraker",
            description: "Moonraker-compatible Klipper printers",
        },
    ]
}

pub fn profile(config: &config::Profile) -> Result<Profile, DriverError> {
    profile_with_timeout(config, parse_timeout(&config.timeout)?)
}

/// A status poll only needs to know "is it reachable", so it doesn't need to
/// wait as long as a real operation (e.g. a print-job command) might.
pub const STATUS_POLL_TIMEOUT: Duration = Duration::from_secs(3);

/// Like [`profile`], but clamps the configured timeout to
/// [`STATUS_POLL_TIMEOUT`] — for use by the periodic status poller only, not
/// by command execution which should honor the user-configured timeout.
pub fn profile_for_status(config: &config::Profile) -> Result<Profile, DriverError> {
    let timeout = parse_timeout(&config.timeout)?.min(STATUS_POLL_TIMEOUT);
    profile_with_timeout(config, timeout)
}

fn profile_with_timeout(
    config: &config::Profile,
    timeout: Duration,
) -> Result<Profile, DriverError> {
    match Driver::parse(&config.driver)? {
        Driver::BambuLan => {
            bambu::Profile::with_timeout(&config.host, &config.serial, config.insecure, timeout)
                .map(|profile| profile.with_model(&config.model))
                .map(Profile::Bambu)
                .map_err(|_| DriverError::InvalidProfile(Driver::BambuLan))
        }
        Driver::Moonraker => moonraker::Profile::new(&config.host, config.insecure, timeout)
            .map(Profile::Moonraker)
            .map_err(|_| DriverError::InvalidProfile(Driver::Moonraker)),
    }
}

/// Attaches a protocol tracer to an already-resolved driver profile, so every
/// subsequent request made through it is recorded.
pub fn attach_tracer(profile: Profile, tracer: SharedTracer) -> Profile {
    match profile {
        Profile::Bambu(profile) => Profile::Bambu(profile.with_tracer(tracer)),
        Profile::Moonraker(profile) => Profile::Moonraker(profile.with_tracer(tracer)),
    }
}

pub fn status(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
) -> Result<moonraker::Status, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.status(access_code))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .status(access_code, tls_fingerprint)
            .map_err(DriverError::Bambu),
    }
}

pub fn firmware_update_status(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    refresh: bool,
) -> Result<FirmwareUpdateReport, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.firmware_update_status(access_code, refresh))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .firmware_update_status(access_code, tls_fingerprint, refresh)
            .map_err(DriverError::Bambu),
    }
}

pub fn capture_tls_fingerprint(profile: &Profile) -> Result<String, DriverError> {
    match profile {
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .capture_fingerprint()
            .map_err(DriverError::Bambu),
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::TlsRefresh,
        )),
    }
}

pub fn camera_snapshot(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<Vec<u8>, DriverError> {
    match profile {
        Profile::Bambu(profile) => bambu::snapshot(profile, access_code, tls_fingerprint, timeout)
            .map_err(DriverError::Camera),
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::CameraSnapshot,
        )),
    }
}

pub fn camera_stream(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<bambu::MjpegStream, DriverError> {
    match profile {
        Profile::Bambu(profile) => {
            bambu::open_mjpeg_stream(profile, access_code, tls_fingerprint, timeout)
                .map_err(DriverError::Camera)
        }
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::CameraStream,
        )),
    }
}

/// Opens the native H.264/RTP camera source for a WebRTC gateway.
pub fn camera_h264_stream(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<bambu::H264Stream, DriverError> {
    match profile {
        Profile::Bambu(profile) => {
            bambu::open_h264_stream(profile, access_code, tls_fingerprint, timeout)
                .map_err(DriverError::Camera)
        }
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::CameraStream,
        )),
    }
}

/// Opens an H.264 camera stream that also produces bounded JPEG previews from
/// each decodable access unit. The original RTP packets remain available.
pub fn camera_h264_stream_with_previews(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<bambu::H264Stream, DriverError> {
    match profile {
        Profile::Bambu(profile) => {
            bambu::open_decoded_h264_stream(profile, access_code, tls_fingerprint, timeout)
                .map_err(DriverError::Camera)
        }
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::CameraStream,
        )),
    }
}

pub fn file_roots(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
) -> Result<Vec<moonraker::FileRoot>, DriverError> {
    match profile {
        Profile::Moonraker(_) => Ok(moonraker::Client::file_roots()),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .file_roots(access_code, tls_fingerprint)
            .map_err(DriverError::Bambu),
    }
}

pub fn file_list(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    device_path: &str,
    recursive: bool,
) -> Result<moonraker::FileList, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.file_list(access_code, device_path, recursive))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .file_list(access_code, tls_fingerprint, device_path, recursive)
            .map_err(DriverError::Bambu),
    }
}

pub fn download_to(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    device_path: &str,
    destination: &mut dyn std::io::Write,
) -> Result<u64, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.download_to(access_code, device_path, destination))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .download_to(access_code, tls_fingerprint, device_path, destination)
            .map_err(DriverError::Bambu),
    }
}

pub fn file_thumbnail_to(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    device_path: &str,
    plate: u32,
    destination: &mut dyn std::io::Write,
) -> Result<u64, DriverError> {
    match profile {
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .thumbnail_to(
                access_code,
                tls_fingerprint,
                device_path,
                plate,
                destination,
            )
            .map_err(DriverError::Bambu),
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::FileDownload,
        )),
    }
}

pub fn upload_file(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    source: &std::path::Path,
    device_path: &str,
    overwrite: bool,
) -> Result<u64, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.upload_file(access_code, source, device_path, overwrite))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .upload_file(access_code, tls_fingerprint, source, device_path, overwrite)
            .map_err(DriverError::Bambu),
    }
}

pub fn delete_file(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    device_path: &str,
) -> Result<(), DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.delete_file(access_code, device_path))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .delete_file(access_code, tls_fingerprint, device_path)
            .map_err(DriverError::Bambu),
    }
}

pub fn job_start(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    device_path: &str,
    options: bambu::JobStartOptions,
) -> Result<moonraker::JobResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.job_start(access_code, device_path))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .job_start(access_code, tls_fingerprint, device_path, options)
            .map_err(DriverError::Bambu),
    }
}

pub fn job_pause(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
) -> Result<moonraker::JobResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.job_pause(access_code))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .job_pause(access_code, tls_fingerprint)
            .map_err(DriverError::Bambu),
    }
}

pub fn job_resume(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
) -> Result<moonraker::JobResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.job_resume(access_code))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .job_resume(access_code, tls_fingerprint)
            .map_err(DriverError::Bambu),
    }
}

pub fn job_cancel(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
) -> Result<moonraker::JobResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.job_cancel(access_code))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .job_cancel(access_code, tls_fingerprint)
            .map_err(DriverError::Bambu),
    }
}

pub fn emergency_stop(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
) -> Result<(), DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.emergency_stop(access_code))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .emergency_stop(access_code, tls_fingerprint)
            .map_err(DriverError::Bambu),
    }
}

pub fn temperature_set(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    targets: moonraker::TemperatureTargets,
) -> Result<moonraker::TemperatureResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.temperature_set(access_code, targets))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .temperature_set(access_code, tls_fingerprint, targets)
            .map_err(DriverError::Bambu),
    }
}

pub fn motion_home(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    axes: &[moonraker::Axis],
) -> Result<moonraker::MotionResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.motion_home(access_code, axes))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .motion_home(access_code, tls_fingerprint, axes)
            .map_err(DriverError::Bambu),
    }
}

pub fn motion_jog(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    delta: moonraker::JogDelta,
) -> Result<moonraker::MotionResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.motion_jog(access_code, delta))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .motion_jog(access_code, tls_fingerprint, delta)
            .map_err(DriverError::Bambu),
    }
}

pub fn fan_set(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    fan: &str,
    speed_percent: u8,
) -> Result<moonraker::FanResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.fan_set(access_code, fan, speed_percent))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .fan_set(access_code, tls_fingerprint, fan, speed_percent)
            .map_err(DriverError::Bambu),
    }
}

pub fn light_set(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    light: &str,
    state: moonraker::LightState,
) -> Result<moonraker::LightResult, DriverError> {
    match profile {
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .light_set(access_code, tls_fingerprint, light, state)
            .map_err(DriverError::Bambu),
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::LightSet,
        )),
    }
}

pub fn ams_drying_set(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    ams_id: u32,
    request: &moonraker::DryingRequest,
) -> Result<moonraker::DryingResult, DriverError> {
    match profile {
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .ams_drying_set(access_code, tls_fingerprint, ams_id, request)
            .map_err(DriverError::Bambu),
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::AmsDrying,
        )),
    }
}

pub fn speed_set(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    speed_profile: &str,
) -> Result<moonraker::SpeedResult, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.speed_set(access_code, speed_profile))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .speed_set(access_code, tls_fingerprint, speed_profile)
            .map_err(DriverError::Bambu),
    }
}

pub fn parse_timeout(value: &str) -> Result<Duration, DriverError> {
    let (amount, unit) = value
        .trim()
        .strip_suffix("ms")
        .map(|amount| (amount, "ms"))
        .or_else(|| value.trim().strip_suffix('s').map(|amount| (amount, "s")))
        .or_else(|| value.trim().strip_suffix('m').map(|amount| (amount, "m")))
        .or_else(|| value.trim().strip_suffix('h').map(|amount| (amount, "h")))
        .ok_or(DriverError::InvalidTimeout)?;
    let amount = amount
        .parse::<u64>()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or(DriverError::InvalidTimeout)?;
    match unit {
        "ms" => Ok(Duration::from_millis(amount)),
        "s" => Ok(Duration::from_secs(amount)),
        "m" => amount
            .checked_mul(60)
            .map(Duration::from_secs)
            .ok_or(DriverError::InvalidTimeout),
        "h" => amount
            .checked_mul(60 * 60)
            .map(Duration::from_secs)
            .ok_or(DriverError::InvalidTimeout),
        _ => unreachable!("timeout units are fixed above"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Profile as ConfigProfile;

    #[test]
    fn registered_drivers_match_the_current_product() {
        assert_eq!(
            registered().map(|driver| driver.name),
            ["bambu-lan", "moonraker"]
        );
    }

    #[test]
    fn driver_profiles_are_validated_by_their_registered_driver() {
        let bambu = profile(&ConfigProfile {
            driver: "bambu-lan".into(),
            host: "printer.local".into(),
            serial: "01S09C450100XXX".into(),
            model: "P1S".into(),
            timeout: "10s".into(),
            insecure: false,
            created: String::new(),
            updated: String::new(),
        })
        .unwrap();
        assert_eq!(bambu.driver(), Driver::BambuLan);
        assert!(bambu.driver().supports(Operation::Status));
        assert!(bambu.driver().requires_access_code());

        let moonraker = profile(&ConfigProfile {
            driver: "moonraker".into(),
            host: "printer.local".into(),
            serial: String::new(),
            model: String::new(),
            timeout: "250ms".into(),
            insecure: false,
            created: String::new(),
            updated: String::new(),
        })
        .unwrap();
        assert_eq!(moonraker.driver(), Driver::Moonraker);
        assert!(moonraker.driver().supports(Operation::Status));
        assert!(moonraker.driver().supports(Operation::Verify));
    }

    #[test]
    fn profiles_reject_unknown_drivers_and_bad_timeouts() {
        let mut config = ConfigProfile {
            driver: "unknown".into(),
            host: "printer.local".into(),
            serial: String::new(),
            model: String::new(),
            timeout: "10s".into(),
            insecure: false,
            created: String::new(),
            updated: String::new(),
        };
        assert!(matches!(profile(&config), Err(DriverError::Unknown(_))));

        config.driver = "moonraker".into();
        config.timeout = "0s".into();
        assert!(matches!(profile(&config), Err(DriverError::InvalidTimeout)));
    }

    #[test]
    fn moonraker_advertises_only_operations_implemented_by_the_http_driver() {
        let capabilities = Driver::Moonraker.capabilities();

        assert!(capabilities.status);
        assert!(capabilities.file_list);
        assert!(capabilities.file_download);
        assert!(capabilities.file_upload);
        assert!(capabilities.job_start);
        assert!(capabilities.temperature_write);
        assert!(capabilities.motion_control);
        assert!(capabilities.fan_control);
        assert!(capabilities.speed_control);
        assert!(!capabilities.camera_snapshot);
        assert!(!capabilities.camera_stream);
        assert!(!capabilities.light_control);
    }

    #[test]
    fn bambu_advertises_authenticated_lan_operations() {
        let capabilities = Driver::BambuLan.capabilities();

        assert!(capabilities.status);
        assert!(capabilities.camera_snapshot);
        assert!(capabilities.camera_stream);
        assert!(capabilities.file_list);
        assert!(capabilities.file_download);
        assert!(capabilities.file_upload);
        assert!(capabilities.job_start);
        assert!(capabilities.emergency_stop);
        assert!(capabilities.temperature_write);
        assert!(capabilities.motion_control);
        assert!(capabilities.fan_control);
        assert!(capabilities.light_control);
        assert!(capabilities.speed_control);
        assert!(capabilities.discovery);
        assert!(capabilities.tls_refresh);
    }

    #[test]
    fn profile_capabilities_expose_tunnel_storage_metadata() {
        let profile = Profile::Bambu(
            bambu::Profile::new("printer.local", "SN001", true)
                .unwrap()
                .with_model("H2D"),
        );

        assert!(profile.capabilities().file_list);
        assert!(profile.capabilities().file_upload);
        assert!(profile.capabilities().job_start);
        assert_eq!(
            profile.bambu_runtime_capabilities().unwrap().model_family,
            bambu::ModelFamily::H2
        );
    }

    #[test]
    fn rejects_light_control_for_moonraker_without_connecting() {
        let profile = Profile::Moonraker(
            moonraker::Profile::new("printer.local", false, Duration::from_secs(1)).unwrap(),
        );

        assert!(matches!(
            light_set(
                &profile,
                None,
                None,
                "chamber_light",
                moonraker::LightState::On
            ),
            Err(DriverError::UnsupportedOperation(
                Driver::Moonraker,
                Operation::LightSet
            ))
        ));
    }

    #[test]
    fn rejects_ams_drying_for_moonraker_without_connecting() {
        let profile = Profile::Moonraker(
            moonraker::Profile::new("printer.local", false, Duration::from_secs(1)).unwrap(),
        );
        assert!(!Driver::Moonraker.supports(Operation::AmsDrying));
        assert!(Driver::BambuLan.supports(Operation::AmsDrying));
        assert!(matches!(
            ams_drying_set(&profile, None, None, 0, &moonraker::DryingRequest::Stop),
            Err(DriverError::UnsupportedOperation(
                Driver::Moonraker,
                Operation::AmsDrying
            ))
        ));
    }
}
