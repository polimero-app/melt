//! Smart plugs: profiles, attachments and relay types. Drivers form a closed
//! set; `tapo` is the only one today. Nothing here touches the network.

mod profiles;

pub use profiles::{
    PlugRemoveResult, PlugRequest, PlugSummary, PlugUpdate, commit_create, commit_update, link,
    list, list_for, prepare_create, prepare_update, remove, resolve, resolve_by_id, revision,
    unlink,
};

use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    config::{ConfigError, PlugProfile},
    keychain::SecretError,
};

pub const TAPO: &str = "tapo";
pub const DRIVERS: &[&str] = &[TAPO];
/// Exact match on the model the device reports.
pub const QUALIFIED_MODELS: &[&str] = &["P110"];
pub const DEFAULT_TIMEOUT: &str = "10s";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerState {
    On,
    Off,
}

/// Relay state as shown to users; `Unknown` while the plug is unreachable.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayState {
    On,
    Off,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub model: String,
    pub hardware: String,
    pub firmware: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlugStatus {
    pub device: DeviceInfo,
    pub relay: PowerState,
    pub observed_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub state: PowerState,
    pub observed_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PowerChange {
    pub requested: PowerState,
    pub status: PlugStatus,
}

/// A plug profile with its password, ready to connect. Deliberately not
/// `Debug`: it holds a secret.
pub struct ResolvedPlug {
    pub name: String,
    pub profile: PlugProfile,
    pub password: String,
    pub timeout: Duration,
}

/// No variant carries upstream text: device and network details are mapped
/// to these variants and dropped.
#[derive(Debug, Error)]
pub enum PlugError {
    #[error("plug name is required")]
    MissingName,
    #[error("invalid plug name")]
    InvalidName,
    #[error("unsupported plug driver {0:?}")]
    UnknownDriver(String),
    #[error("invalid plug host")]
    InvalidHost,
    #[error("plug username must be 1-254 characters without control characters")]
    InvalidUsername,
    #[error("plug password must not be empty or contain control characters")]
    InvalidPassword,
    #[error("invalid plug timeout")]
    InvalidTimeout,
    #[error("plug {0:?} not found")]
    NotFound(String),
    #[error("plug {0:?} already exists")]
    AlreadyExists(String),
    #[error("printer profile {0:?} not found")]
    PrinterNotFound(String),
    #[error("camera {0:?} not found")]
    CameraNotFound(String),
    #[error("plug {plug:?} is not linked to {target:?}")]
    NotLinked { plug: String, target: String },
    #[error("plug authentication failed")]
    Authentication,
    #[error("plug rejected the login; enable third-party compatibility in the Tapo app")]
    CompatibilityDisabled,
    #[error("plug locked out further logins")]
    Lockout,
    #[error("unsupported plug device {0:?}")]
    UnsupportedDevice(String),
    #[error("plug speaks an unsupported protocol")]
    UnsupportedProtocol,
    #[error("plug rejected the request")]
    DeviceRejected,
    #[error("plug connection failed")]
    Connect,
    #[error("plug connection timed out")]
    Timeout,
    #[error("plug sent a malformed response")]
    MalformedResponse,
    #[error("could not confirm the plug relay is {requested:?}")]
    PowerUnconfirmed {
        requested: PowerState,
        last_observed: Option<Observation>,
    },
    #[error("plug relay is {observed:?} instead of {requested:?}")]
    PowerMismatch {
        requested: PowerState,
        observed: PowerState,
    },
    #[error("plug profile changed during the operation")]
    ProfileChanged,
    #[error("stored plug password is unavailable")]
    MissingSecret,
    #[error("keychain operation failed")]
    Secret(#[source] SecretError),
    #[error("cannot save config")]
    Config(#[source] ConfigError),
    #[error("cannot save config and could not restore the stored plug password")]
    RollbackFailed,
}
