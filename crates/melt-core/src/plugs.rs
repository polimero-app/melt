//! Smart plugs: profiles, attachments, relay types and power control.
//! Drivers form a closed set; `tapo` is the only one today. Only [`Session`]
//! and the functions built on it touch the network.

pub mod manager;
mod profiles;
mod tapo;

pub use profiles::{
    PlugRemoveResult, PlugRequest, PlugSummary, PlugUpdate, commit_create, commit_update, link,
    list, list_for, prepare_create, prepare_update, remove, resolve, resolve_by_id, revision,
    unlink,
};

use std::time::Duration;

use tokio::time::{Instant, timeout_at};

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

/// An authenticated connection to one qualified plug. Opaque: vendor types
/// never leave this module.
pub struct Session {
    inner: Inner,
}

enum Inner {
    Tapo(tapo::TapoSession),
    #[cfg(test)]
    Fake(FakeSession),
}

/// Why a driver read failed. `Expired` stays inside this module: a read
/// reconnects once, and a second expiry surfaces as `DeviceRejected`.
enum ReadFailure {
    Expired,
    Plug(PlugError),
}

impl Session {
    /// Logs in and reads device info; refuses any model not in
    /// [`QUALIFIED_MODELS`].
    pub async fn connect(plug: &ResolvedPlug) -> Result<Session, PlugError> {
        let inner = match plug.profile.driver.as_str() {
            TAPO => Inner::Tapo(tapo::TapoSession::login(plug).await?),
            #[cfg(test)]
            FAKE => Inner::Fake(
                FAKES
                    .lock()
                    .unwrap()
                    .remove(&plug.profile.host)
                    .ok_or(PlugError::Connect)?,
            ),
            other => return Err(PlugError::UnknownDriver(other.to_owned())),
        };
        let mut session = Session { inner };
        session.status().await?;
        Ok(session)
    }

    /// Reads the device; reconnects once if the session expired.
    pub async fn status(&mut self) -> Result<PlugStatus, PlugError> {
        let status = match self.read().await {
            Err(ReadFailure::Expired) => {
                self.reconnect().await?;
                self.read().await
            }
            read => read,
        }
        .map_err(|failure| match failure {
            ReadFailure::Expired => PlugError::DeviceRejected,
            ReadFailure::Plug(error) => error,
        })?;
        if !QUALIFIED_MODELS.contains(&status.device.model.as_str()) {
            return Err(PlugError::UnsupportedDevice(status.device.model));
        }
        Ok(status)
    }

    async fn read(&mut self) -> Result<PlugStatus, ReadFailure> {
        match &mut self.inner {
            Inner::Tapo(session) => session.read().await,
            #[cfg(test)]
            Inner::Fake(session) => session.read().await,
        }
    }

    async fn reconnect(&mut self) -> Result<(), PlugError> {
        match &mut self.inner {
            Inner::Tapo(session) => session.reconnect().await,
            #[cfg(test)]
            Inner::Fake(session) => {
                session.reconnects += 1;
                Ok(())
            }
        }
    }

    /// Never reconnects or retries: a write goes out at most once.
    async fn send(&mut self, state: PowerState) -> Result<(), PlugError> {
        match &mut self.inner {
            Inner::Tapo(session) => session.send(state).await,
            #[cfg(test)]
            Inner::Fake(session) => session.send(state).await,
        }
    }
}

/// Connects and reads status; never switches the relay.
pub async fn verify(plug: &ResolvedPlug) -> Result<PlugStatus, PlugError> {
    with_deadline(Instant::now() + plug.timeout, async {
        Session::connect(plug).await?.status().await
    })
    .await
}

pub async fn status(plug: &ResolvedPlug) -> Result<PlugStatus, PlugError> {
    verify(plug).await
}

/// One-shot power change: connect, then [`switch`], all within `plug.timeout`.
pub async fn set_power(plug: &ResolvedPlug, state: PowerState) -> Result<PowerChange, PlugError> {
    let deadline = Instant::now() + plug.timeout;
    let mut session = with_deadline(deadline, Session::connect(plug)).await?;
    switch(&mut session, state, deadline, None).await
}

async fn with_deadline<T>(
    deadline: Instant,
    operation: impl Future<Output = Result<T, PlugError>>,
) -> Result<T, PlugError> {
    timeout_at(deadline, operation)
        .await
        .unwrap_or(Err(PlugError::Timeout))
}

/// Pre-read, send once, read back. See the "Power state and command results"
/// design: confirmed, mismatch, rejected or unconfirmed. `last` is what the
/// caller observed before; a successful pre-read replaces it.
pub(crate) async fn switch(
    session: &mut Session,
    state: PowerState,
    deadline: Instant,
    last: Option<Observation>,
) -> Result<PowerChange, PlugError> {
    let before = with_deadline(deadline, session.status()).await?;
    let last = Some(Observation {
        state: before.relay,
        observed_at: before.observed_at,
    })
    .or(last);
    let unconfirmed = || PlugError::PowerUnconfirmed {
        requested: state,
        last_observed: last.clone(),
    };
    match timeout_at(deadline, session.send(state)).await {
        Ok(Ok(())) => {}
        Ok(Err(
            refusal @ (PlugError::Authentication
            | PlugError::Lockout
            | PlugError::CompatibilityDisabled
            | PlugError::DeviceRejected),
        )) => return Err(refusal),
        Ok(Err(_)) | Err(_) => return Err(unconfirmed()),
    }
    match timeout_at(deadline, session.status()).await {
        Ok(Ok(status)) if status.relay == state => Ok(PowerChange {
            requested: state,
            status,
        }),
        Ok(Ok(status)) => Err(PlugError::PowerMismatch {
            requested: state,
            observed: status.relay,
        }),
        Ok(Err(_)) | Err(_) => Err(unconfirmed()),
    }
}

#[cfg(test)]
const FAKE: &str = "fake";

/// Test-only fake sessions by host, so tests can go through
/// [`Session::connect`] with `driver: fake`.
#[cfg(test)]
pub(crate) static FAKES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, FakeSession>>,
> = std::sync::LazyLock::new(Default::default);

#[cfg(test)]
pub(crate) enum FakeStep {
    /// A read answers this relay state.
    Status(PowerState),
    /// The operation behaves normally: a read returns the shared relay, a
    /// send sets it.
    Ok,
    Err(PlugError),
    /// A read fails with an expired session.
    Expired,
    Hang,
}

/// Each read or send takes the next scripted step; with none left it acts as
/// [`FakeStep::Ok`].
#[cfg(test)]
pub(crate) struct FakeSession {
    pub(crate) model: String,
    pub(crate) relay: std::sync::Arc<std::sync::Mutex<PowerState>>,
    pub(crate) steps: std::collections::VecDeque<FakeStep>,
    pub(crate) sends: Vec<PowerState>,
    pub(crate) reconnects: usize,
}

#[cfg(test)]
impl FakeSession {
    pub(crate) fn new(relay: PowerState, steps: impl IntoIterator<Item = FakeStep>) -> Self {
        Self {
            model: "P110".into(),
            relay: std::sync::Arc::new(std::sync::Mutex::new(relay)),
            steps: steps.into_iter().collect(),
            sends: Vec::new(),
            reconnects: 0,
        }
    }

    async fn read(&mut self) -> Result<PlugStatus, ReadFailure> {
        let relay = match self.steps.pop_front().unwrap_or(FakeStep::Ok) {
            FakeStep::Status(state) => state,
            FakeStep::Ok => *self.relay.lock().unwrap(),
            FakeStep::Err(error) => return Err(ReadFailure::Plug(error)),
            FakeStep::Expired => return Err(ReadFailure::Expired),
            FakeStep::Hang => std::future::pending().await,
        };
        Ok(PlugStatus {
            device: DeviceInfo {
                model: self.model.clone(),
                hardware: "1.0".into(),
                firmware: "1.0.0".into(),
            },
            relay,
            observed_at: crate::profiles::now(),
        })
    }

    async fn send(&mut self, state: PowerState) -> Result<(), PlugError> {
        self.sends.push(state);
        match self.steps.pop_front().unwrap_or(FakeStep::Ok) {
            FakeStep::Ok => *self.relay.lock().unwrap() = state,
            FakeStep::Err(error) => return Err(error),
            FakeStep::Hang => std::future::pending().await,
            FakeStep::Status(_) | FakeStep::Expired => panic!("not a send step"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn session(fake: FakeSession) -> Session {
        Session {
            inner: Inner::Fake(fake),
        }
    }

    fn fake(session: &Session) -> &FakeSession {
        match &session.inner {
            Inner::Fake(fake) => fake,
            Inner::Tapo(_) => unreachable!(),
        }
    }

    fn deadline() -> Instant {
        Instant::now() + Duration::from_secs(10)
    }

    fn plug(host: &str) -> ResolvedPlug {
        ResolvedPlug {
            name: "bench".into(),
            profile: PlugProfile {
                id: "11111111111111111111111111111111".into(),
                driver: FAKE.into(),
                host: host.into(),
                username: "user@example.com".into(),
                timeout: DEFAULT_TIMEOUT.into(),
                printers: Vec::new(),
                cameras: Vec::new(),
                created: "2026-10-18T00:00:00Z".into(),
                updated: "2026-10-18T00:00:00Z".into(),
            },
            password: "pw".into(),
            timeout: Duration::from_secs(10),
        }
    }

    #[tokio::test]
    async fn confirmed_when_readback_matches() {
        let mut s = session(FakeSession::new(PowerState::Off, []));
        let change = switch(&mut s, PowerState::On, deadline(), None)
            .await
            .unwrap();
        assert_eq!(change.requested, PowerState::On);
        assert_eq!(change.status.relay, PowerState::On);
        assert_eq!(fake(&s).sends, [PowerState::On]);
    }

    #[tokio::test]
    async fn already_on_is_confirmed() {
        let mut s = session(FakeSession::new(PowerState::On, []));
        let change = switch(&mut s, PowerState::On, deadline(), None)
            .await
            .unwrap();
        assert_eq!(change.status.relay, PowerState::On);
        assert_eq!(fake(&s).sends, [PowerState::On]);
    }

    #[tokio::test]
    async fn mismatch_when_readback_differs() {
        let steps = [
            FakeStep::Ok,
            FakeStep::Ok,
            FakeStep::Status(PowerState::Off),
        ];
        let mut s = session(FakeSession::new(PowerState::Off, steps));
        let error = switch(&mut s, PowerState::On, deadline(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                PlugError::PowerMismatch {
                    requested: PowerState::On,
                    observed: PowerState::Off
                }
            ),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn lost_response_after_send_is_unconfirmed_and_not_replayed() {
        let steps = [FakeStep::Ok, FakeStep::Err(PlugError::Connect)];
        let mut s = session(FakeSession::new(PowerState::Off, steps));
        let stale = Observation {
            state: PowerState::On,
            observed_at: "2000-01-01T00:00:00Z".into(),
        };
        let error = switch(&mut s, PowerState::On, deadline(), Some(stale))
            .await
            .unwrap_err();
        let PlugError::PowerUnconfirmed {
            requested,
            last_observed: Some(last),
        } = error
        else {
            panic!("{error:?}");
        };
        assert_eq!(requested, PowerState::On);
        assert_eq!(last.state, PowerState::Off, "last is the pre-read");
        assert_ne!(last.observed_at, "2000-01-01T00:00:00Z");
        assert_eq!(fake(&s).sends, [PowerState::On]);
    }

    #[tokio::test(start_paused = true)]
    async fn hang_during_send_hits_deadline_as_unconfirmed() {
        let mut s = session(FakeSession::new(
            PowerState::On,
            [FakeStep::Ok, FakeStep::Hang],
        ));
        let error = switch(&mut s, PowerState::Off, deadline(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(
                error,
                PlugError::PowerUnconfirmed {
                    requested: PowerState::Off,
                    last_observed: Some(Observation {
                        state: PowerState::On,
                        ..
                    })
                }
            ),
            "{error:?}"
        );
        assert_eq!(fake(&s).sends, [PowerState::Off]);
    }

    #[tokio::test(start_paused = true)]
    async fn hang_during_readback_is_unconfirmed() {
        let steps = [FakeStep::Ok, FakeStep::Ok, FakeStep::Hang];
        let mut s = session(FakeSession::new(PowerState::Off, steps));
        let error = switch(&mut s, PowerState::On, deadline(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(error, PlugError::PowerUnconfirmed { .. }),
            "{error:?}"
        );
        assert_eq!(fake(&s).sends, [PowerState::On]);
    }

    #[tokio::test]
    async fn failed_pre_read_sends_nothing() {
        let steps = [FakeStep::Err(PlugError::Connect)];
        let mut s = session(FakeSession::new(PowerState::Off, steps));
        let error = switch(&mut s, PowerState::On, deadline(), None)
            .await
            .unwrap_err();
        assert!(matches!(error, PlugError::Connect), "{error:?}");
        assert!(fake(&s).sends.is_empty());
    }

    #[tokio::test]
    async fn device_refusal_passes_through() {
        for refusal in [PlugError::Authentication, PlugError::DeviceRejected] {
            let steps = [FakeStep::Ok, FakeStep::Err(refusal)];
            let mut s = session(FakeSession::new(PowerState::Off, steps));
            let error = switch(&mut s, PowerState::On, deadline(), None)
                .await
                .unwrap_err();
            assert!(
                matches!(error, PlugError::Authentication | PlugError::DeviceRejected),
                "{error:?}"
            );
            assert_eq!(fake(&s).sends, [PowerState::On]);
        }
    }

    #[tokio::test]
    async fn malformed_reply_to_send_is_unconfirmed() {
        let steps = [FakeStep::Ok, FakeStep::Err(PlugError::MalformedResponse)];
        let mut s = session(FakeSession::new(PowerState::Off, steps));
        let error = switch(&mut s, PowerState::On, deadline(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(error, PlugError::PowerUnconfirmed { .. }),
            "{error:?}"
        );
    }

    #[tokio::test]
    async fn unqualified_model_is_rejected_on_connect_and_status() {
        let mut p100 = FakeSession::new(PowerState::Off, []);
        p100.model = "P100".into();
        FAKES.lock().unwrap().insert("192.0.2.31".into(), p100);
        let error = status(&plug("192.0.2.31")).await.unwrap_err();
        assert!(
            matches!(&error, PlugError::UnsupportedDevice(m) if m == "P100"),
            "{error:?}"
        );

        // The IP now answers as a different device mid-session: no write.
        let mut swapped = FakeSession::new(PowerState::Off, []);
        swapped.model = "P100".into();
        let mut s = session(swapped);
        let error = s.status().await.unwrap_err();
        assert!(
            matches!(error, PlugError::UnsupportedDevice(_)),
            "{error:?}"
        );
        let error = switch(&mut s, PowerState::On, deadline(), None)
            .await
            .unwrap_err();
        assert!(
            matches!(error, PlugError::UnsupportedDevice(_)),
            "{error:?}"
        );
        assert!(fake(&s).sends.is_empty());
    }

    #[tokio::test]
    async fn connect_and_verify_go_through_the_fake_driver() {
        FAKES
            .lock()
            .unwrap()
            .insert("192.0.2.30".into(), FakeSession::new(PowerState::On, []));
        let status = verify(&plug("192.0.2.30")).await.unwrap();
        assert_eq!(status.relay, PowerState::On);
        assert_eq!(status.device.model, "P110");
    }

    #[tokio::test]
    async fn expired_session_read_reconnects_once() {
        let mut s = session(FakeSession::new(PowerState::On, [FakeStep::Expired]));
        assert_eq!(s.status().await.unwrap().relay, PowerState::On);
        assert_eq!(fake(&s).reconnects, 1);

        let mut s = session(FakeSession::new(
            PowerState::On,
            [FakeStep::Expired, FakeStep::Expired],
        ));
        let error = s.status().await.unwrap_err();
        assert!(matches!(error, PlugError::DeviceRejected), "{error:?}");
        assert_eq!(fake(&s).reconnects, 1);
    }
}
