//! UI-agnostic Melt domain types.

pub mod bambu;
pub mod config;
pub mod diagnostics;
pub mod drivers;
pub mod keychain;
pub mod monitor;
pub mod moonraker;
pub mod pool;
pub mod preferences;
pub mod profiles;
pub mod trace;

use serde::Serialize;
use thiserror::Error;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LICENSE: &str = "AGPL-3.0-only";
pub const SOURCE_URL: &str = "https://github.com/polimero-app/melt";

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
            code: "config_error",
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub modes: [&'static str; 2],
    pub license: &'static str,
    pub source_url: &'static str,
}

pub fn app_info() -> AppInfo {
    AppInfo {
        version: VERSION,
        modes: ["cli", "gui"],
        license: LICENSE,
        source_url: SOURCE_URL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_exposes_both_modes() {
        assert_eq!(app_info().modes, ["cli", "gui"]);
        assert_eq!(app_info().license, "AGPL-3.0-only");
    }

    #[test]
    fn usage_errors_keep_the_cli_contract_code() {
        assert_eq!(AppError::usage("bad input").code, "config_error");
    }
}
