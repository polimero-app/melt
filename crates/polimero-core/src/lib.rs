//! UI-agnostic Polimero domain types.

pub mod config;
pub mod drivers;
pub mod keychain;
pub mod profiles;

use serde::Serialize;
use thiserror::Error;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
#[error("{message}")]
pub struct AppError {
    pub exit_code: i32,
    pub code: &'static str,
    pub message: String,
}

impl AppError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: 2,
            code: "config-error",
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub modes: [&'static str; 2],
}

pub fn app_info() -> AppInfo {
    AppInfo {
        version: VERSION,
        modes: ["cli", "gui"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_exposes_both_modes() {
        assert_eq!(app_info().modes, ["cli", "gui"]);
    }

    #[test]
    fn usage_errors_keep_the_cli_contract_code() {
        assert_eq!(AppError::usage("bad input").code, "config-error");
    }
}
