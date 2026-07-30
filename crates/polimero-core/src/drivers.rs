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
        matches!(
            (self, operation),
            (Self::Moonraker, Operation::Status | Operation::Verify)
        )
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
}
