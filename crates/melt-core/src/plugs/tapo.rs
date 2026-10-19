//! `tapo` crate adapter. Vendor types and error text stay in this file.

use ::tapo::{ApiClient, Error, PlugEnergyMonitoringHandler, TapoResponseError};

use super::{DeviceInfo, PlugError, PlugStatus, PowerState, ReadFailure, ResolvedPlug};

pub(super) struct TapoSession {
    handler: PlugEnergyMonitoringHandler,
}

impl TapoSession {
    pub(super) async fn login(plug: &ResolvedPlug) -> Result<Self, PlugError> {
        let handler = ApiClient::new(plug.profile.username.as_str(), plug.password.as_str())
            .with_timeout(plug.timeout)
            .p110(plug.profile.host.as_str())
            .await
            .map_err(map_error)?;
        Ok(Self { handler })
    }

    pub(super) async fn read(&self) -> Result<PlugStatus, ReadFailure> {
        let info = self.handler.get_device_info().await.map_err(|error| {
            match session_expired(&error) {
                true => ReadFailure::Expired,
                false => ReadFailure::Plug(map_error(error)),
            }
        })?;
        Ok(PlugStatus {
            device: DeviceInfo {
                device_id: info.device_id,
                model: info.model,
                hardware: info.hw_ver,
                firmware: info.fw_ver,
            },
            relay: if info.device_on {
                PowerState::On
            } else {
                PowerState::Off
            },
            observed_at: crate::profiles::now(),
        })
    }

    /// Logs in again with the credentials the client already holds.
    pub(super) async fn reconnect(&mut self) -> Result<(), PlugError> {
        self.handler.refresh_session().await.map_err(map_error)?;
        Ok(())
    }

    pub(super) async fn send(&self, state: PowerState) -> Result<(), PlugError> {
        match state {
            PowerState::On => self.handler.on().await,
            PowerState::Off => self.handler.off().await,
        }
        .map_err(map_error)
    }
}

fn session_expired(error: &Error) -> bool {
    matches!(
        error,
        Error::Tapo(
            TapoResponseError::Unauthorized {
                kind: "SESSION_TIMEOUT" | "SESSION_EXPIRED" | "TPAP_SESSION_INVALID",
                ..
            } | TapoResponseError::DeviceError {
                kind: "SESSION_EXPIRED",
                ..
            }
        )
    )
}

/// Drops all upstream text. An expired session maps to `DeviceRejected`;
/// reads check [`session_expired`] first and reconnect once.
fn map_error(error: Error) -> PlugError {
    use TapoResponseError as R;
    match error {
        Error::Tapo(R::Unauthorized { kind, .. }) => match kind {
            "TPAP_AUTH_ATTEMPTS_LIMIT" => PlugError::Lockout,
            "FORBIDDEN" => PlugError::CompatibilityDisabled,
            "SESSION_TIMEOUT" | "SESSION_EXPIRED" | "TPAP_SESSION_INVALID" => {
                PlugError::DeviceRejected
            }
            // HASH_MISMATCH, TPAP_CREDENTIALS, TPAP_HASH_MISMATCH, LOGIN and
            // any handshake refusal `tapo` adds later.
            _ => PlugError::Authentication,
        },
        Error::Tapo(R::DeviceError { kind: "LOGIN", .. }) => PlugError::Authentication,
        // Includes SESSION_EXPIRED: the device answered with an error code.
        Error::Tapo(R::DeviceError { .. }) => PlugError::DeviceRejected,
        Error::UnsupportedProtocol { .. } => PlugError::UnsupportedProtocol,
        Error::Http(error) if error.is_timeout() => PlugError::Timeout,
        Error::Http(_) | Error::DeviceNotFound | Error::Other(_) => PlugError::Connect,
        // Serde, Validation, EmptyResult, ResponseError, HttpError, and any
        // variant a later `tapo` adds: unconfirmed, never a refusal.
        _ => PlugError::MalformedResponse,
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Read, net::TcpListener, time::Duration};

    use super::*;

    fn unauthorized(kind: &'static str) -> Error {
        Error::Tapo(TapoResponseError::Unauthorized {
            kind,
            description: "secret-pw".into(),
        })
    }

    fn device_error(kind: &'static str) -> Error {
        Error::Tapo(TapoResponseError::DeviceError { code: -1, kind })
    }

    /// A reqwest timeout: the listener accepts but never answers.
    fn http_timeout() -> Error {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let hold = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.read(&mut [0; 1024]);
            std::thread::sleep(Duration::from_millis(500));
        });
        let error = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap()
            .get(url)
            .send()
            .unwrap_err();
        hold.join().unwrap();
        assert!(error.is_timeout());
        Error::Http(error)
    }

    fn http_other() -> Error {
        let error = reqwest::blocking::Client::new()
            .get("http://")
            .send()
            .unwrap_err();
        assert!(!error.is_timeout());
        Error::Http(error)
    }

    #[test]
    fn tapo_errors_map_to_plug_errors() {
        use PlugError as P;
        let serde = serde_json::from_str::<u8>("x").unwrap_err();
        let cases: Vec<(&str, Error, PlugError, bool)> = vec![
            (
                "lockout",
                unauthorized("TPAP_AUTH_ATTEMPTS_LIMIT"),
                P::Lockout,
                false,
            ),
            (
                "forbidden",
                unauthorized("FORBIDDEN"),
                P::CompatibilityDisabled,
                false,
            ),
            (
                "hash",
                unauthorized("HASH_MISMATCH"),
                P::Authentication,
                false,
            ),
            (
                "tpap creds",
                unauthorized("TPAP_CREDENTIALS"),
                P::Authentication,
                false,
            ),
            (
                "tpap hash",
                unauthorized("TPAP_HASH_MISMATCH"),
                P::Authentication,
                false,
            ),
            (
                "login unauthorized",
                unauthorized("LOGIN"),
                P::Authentication,
                false,
            ),
            (
                "login device",
                device_error("LOGIN"),
                P::Authentication,
                false,
            ),
            (
                "session timeout",
                unauthorized("SESSION_TIMEOUT"),
                P::DeviceRejected,
                true,
            ),
            (
                "session expired",
                unauthorized("SESSION_EXPIRED"),
                P::DeviceRejected,
                true,
            ),
            (
                "tpap session",
                unauthorized("TPAP_SESSION_INVALID"),
                P::DeviceRejected,
                true,
            ),
            (
                "device session",
                device_error("SESSION_EXPIRED"),
                P::DeviceRejected,
                true,
            ),
            (
                "protocol",
                Error::UnsupportedProtocol {
                    protocol: "AES",
                    description: "secret-pw".into(),
                },
                P::UnsupportedProtocol,
                false,
            ),
            ("http timeout", http_timeout(), P::Timeout, false),
            ("http other", http_other(), P::Connect, false),
            ("not found", Error::DeviceNotFound, P::Connect, false),
            (
                "device error",
                device_error("UNKNOWN"),
                P::DeviceRejected,
                false,
            ),
            ("serde", Error::Serde(serde), P::MalformedResponse, false),
            (
                "validation",
                Error::Validation {
                    field: "x".into(),
                    message: "secret-pw".into(),
                },
                P::MalformedResponse,
                false,
            ),
            (
                "empty",
                Error::Tapo(TapoResponseError::EmptyResult),
                P::MalformedResponse,
                false,
            ),
            (
                "response",
                Error::Tapo(TapoResponseError::ResponseError {
                    description: "secret-pw".into(),
                }),
                P::MalformedResponse,
                false,
            ),
            (
                "http status",
                Error::Tapo(TapoResponseError::HttpError {
                    status_code: 500,
                    description: "secret-pw".into(),
                }),
                P::MalformedResponse,
                false,
            ),
        ];
        // ponytail: `Error::Other` needs `anyhow` to construct; not a dependency here.
        for (name, error, expected, expired) in cases {
            assert_eq!(session_expired(&error), expired, "{name}: session_expired");
            let mapped = map_error(error);
            assert_eq!(
                std::mem::discriminant(&mapped),
                std::mem::discriminant(&expected),
                "{name}: got {mapped:?}"
            );
        }
    }

    #[test]
    fn plug_error_display_never_contains_upstream_text() {
        let mapped = map_error(unauthorized("HASH_MISMATCH"));
        assert!(!mapped.to_string().contains("secret-pw"));
        assert!(!format!("{mapped:?}").contains("secret-pw"));
    }
}
