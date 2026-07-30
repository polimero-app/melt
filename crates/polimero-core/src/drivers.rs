use std::{fmt, time::Duration};

use serde::Serialize;
use thiserror::Error;

use crate::{bambu, config, moonraker};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DriverInfo {
    pub name: &'static str,
    pub description: &'static str,
}

pub fn verify(profile: &Profile, access_code: Option<&str>) -> Result<(), DriverError> {
    match profile {
        Profile::Moonraker(_) => status(profile, access_code).map(|_| ()),
        Profile::Bambu(_) => Err(DriverError::UnsupportedOperation(
            Driver::BambuLan,
            Operation::Verify,
        )),
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

    pub fn supports(self, operation: Operation) -> bool {
        match operation {
            Operation::Status | Operation::Verify => self.capabilities().status,
            Operation::Discovery => self.capabilities().discovery,
            Operation::CameraStream => self.capabilities().camera_stream,
            Operation::CameraSnapshot => self.capabilities().camera_snapshot,
            Operation::FileList => self.capabilities().file_list,
            Operation::FileDownload => self.capabilities().file_download,
            Operation::FileUpload => self.capabilities().file_upload,
            Operation::JobStart => self.capabilities().job_start,
            Operation::JobPause => self.capabilities().job_pause,
            Operation::JobResume => self.capabilities().job_resume,
            Operation::JobCancel => self.capabilities().job_cancel,
            Operation::EmergencyStop => self.capabilities().emergency_stop,
            Operation::TemperatureSet => self.capabilities().temperature_write,
            Operation::MotionHome | Operation::MotionJog => self.capabilities().motion_control,
            Operation::TlsRefresh => self.capabilities().tls_refresh,
            Operation::FanSet => self.capabilities().fan_control,
            Operation::LightSet => self.capabilities().light_control,
            Operation::SpeedSet => self.capabilities().speed_control,
        }
    }

    pub fn capabilities(self) -> Capabilities {
        match self {
            // Bambu LAN operations require the MQTT/TLS transport, which is
            // intentionally not claimed until that transport exists.
            Self::BambuLan => Capabilities::default(),
            Self::Moonraker => Capabilities {
                status: true,
                file_list: true,
                file_download: true,
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
    match Driver::parse(&config.driver)? {
        Driver::BambuLan => bambu::Profile::new(&config.host, &config.serial, config.insecure)
            .map(Profile::Bambu)
            .map_err(|_| DriverError::InvalidProfile(Driver::BambuLan)),
        Driver::Moonraker => moonraker::Profile::new(
            &config.host,
            config.insecure,
            parse_timeout(&config.timeout)?,
        )
        .map(Profile::Moonraker)
        .map_err(|_| DriverError::InvalidProfile(Driver::Moonraker)),
    }
}

pub fn status(
    profile: &Profile,
    access_code: Option<&str>,
) -> Result<moonraker::Status, DriverError> {
    match profile {
        Profile::Moonraker(profile) => moonraker::Client::new(profile.clone())
            .and_then(|client| client.status(access_code))
            .map_err(DriverError::Moonraker),
        Profile::Bambu(_) => Err(DriverError::UnsupportedOperation(
            Driver::BambuLan,
            Operation::Status,
        )),
    }
}

fn parse_timeout(value: &str) -> Result<Duration, DriverError> {
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
            timeout: "10s".into(),
            insecure: false,
            created: String::new(),
            updated: String::new(),
        })
        .unwrap();
        assert_eq!(bambu.driver(), Driver::BambuLan);
        assert!(!bambu.driver().supports(Operation::Status));
        assert!(matches!(
            status(&bambu, None),
            Err(DriverError::UnsupportedOperation(
                Driver::BambuLan,
                Operation::Status
            ))
        ));

        let moonraker = profile(&ConfigProfile {
            driver: "moonraker".into(),
            host: "printer.local".into(),
            serial: String::new(),
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
        assert!(capabilities.job_start);
        assert!(capabilities.temperature_write);
        assert!(capabilities.motion_control);
        assert!(capabilities.fan_control);
        assert!(capabilities.speed_control);
        assert!(!capabilities.camera_snapshot);
        assert!(!capabilities.camera_stream);
        assert!(!capabilities.light_control);
    }
}
