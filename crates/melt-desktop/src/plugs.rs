//! Smart plug commands, error codes and the background status poller.
//! Config and keychain work runs on blocking threads; device I/O goes
//! through the shared [`PlugManager`].

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
};

use melt_core::{
    config::{PlugTarget, config_dir},
    keychain::SystemKeychain,
    monitor::{self, Backoff},
    plugs::{
        self, PlugError, PlugRemoveResult, PlugRequest, PlugStatus, PlugSummary, PowerChange,
        PowerState, ResolvedPlug,
        manager::{PlugKey, PlugManager, PlugObservation},
    },
};
use tauri::{AppHandle, Emitter, State};
use tokio::{sync::Semaphore, task::JoinSet, time::Instant};

use crate::{CommandError, unreadable_config};

const UPDATED_EVENT: &str = "plugs-updated";
const POLL_CONCURRENCY: usize = 4;

#[tauri::command]
pub async fn plug_list() -> Result<Vec<PlugSummary>, CommandError> {
    let dir = dir()?;
    blocking(move || plugs::list(dir).map_err(|_| unreadable_config())).await
}

/// Saves only after the plug answers with the given credentials.
#[tauri::command]
pub async fn plug_add(request: PlugRequest) -> Result<PlugSummary, CommandError> {
    let dir = dir()?;
    let plug = {
        let dir = dir.clone();
        blocking(move || plugs::prepare_create(dir, request).map_err(plug_error)).await?
    };
    plugs::verify(&plug).await.map_err(plug_error)?;
    blocking(move || plugs::commit_create(dir, &SystemKeychain, plug).map_err(plug_error)).await
}

/// An empty password keeps the stored one. Verifies only when the
/// connection details change.
#[tauri::command]
pub async fn plug_edit(
    name: String,
    request: PlugRequest,
    app: AppHandle,
    manager: State<'_, Arc<PlugManager>>,
) -> Result<PlugSummary, CommandError> {
    let dir = dir()?;
    let update = {
        let dir = dir.clone();
        blocking(move || {
            plugs::prepare_update(dir, &SystemKeychain, &name, request).map_err(plug_error)
        })
        .await?
    };
    if update.needs_verify {
        plugs::verify(&update.plug).await.map_err(plug_error)?;
    }
    let summary =
        blocking(move || plugs::commit_update(dir, &SystemKeychain, update).map_err(plug_error))
            .await?;
    manager.evict(&summary.id);
    publish(&app, &manager);
    Ok(summary)
}

#[tauri::command]
pub async fn plug_remove(
    name: String,
    app: AppHandle,
    manager: State<'_, Arc<PlugManager>>,
) -> Result<PlugRemoveResult, CommandError> {
    let dir = dir()?;
    let result =
        blocking(move || plugs::remove(dir, &SystemKeychain, &name).map_err(plug_error)).await?;
    manager.evict(&result.id);
    publish(&app, &manager);
    Ok(result)
}

/// Exactly one of `printer` and `camera` (names).
#[tauri::command]
pub async fn plug_link(
    plug: String,
    printer: Option<String>,
    camera: Option<String>,
) -> Result<PlugSummary, CommandError> {
    let target = link_target(printer, camera)?;
    let dir = dir()?;
    blocking(move || plugs::link(dir, &plug, &target).map_err(plug_error)).await
}

/// Exactly one of `printer` and `camera` (names).
#[tauri::command]
pub async fn plug_unlink(
    plug: String,
    printer: Option<String>,
    camera: Option<String>,
) -> Result<PlugSummary, CommandError> {
    let target = link_target(printer, camera)?;
    let dir = dir()?;
    blocking(move || plugs::unlink(dir, &plug, &target).map_err(plug_error)).await
}

/// An explicit read: clears an authentication block.
#[tauri::command]
pub async fn plug_status(
    id: String,
    app: AppHandle,
    manager: State<'_, Arc<PlugManager>>,
) -> Result<PlugStatus, CommandError> {
    let (dir, key) = current_key(&id).await?;
    let result = manager.status(&key, false, resolver(dir, id)).await;
    publish(&app, &manager);
    result.map_err(plug_error)
}

/// Switches once with readback; never retried.
#[tauri::command]
pub async fn plug_set_power(
    id: String,
    state: PowerState,
    app: AppHandle,
    manager: State<'_, Arc<PlugManager>>,
) -> Result<PowerChange, CommandError> {
    let (dir, key) = current_key(&id).await?;
    let revision = {
        let (dir, id) = (dir.clone(), id.clone());
        move || plugs::revision(dir, &id)
    };
    let result = manager
        .set_power(&key, state, resolver(dir, id), revision)
        .await;
    publish(&app, &manager);
    result.map_err(plug_error)
}

#[tauri::command(async)]
pub fn plug_observations(manager: State<'_, Arc<PlugManager>>) -> HashMap<String, PlugObservation> {
    manager.observations()
}

/// Polls every plug for the app's lifetime. Each plug backs off on its own,
/// so an unreachable, locked or password-less plug is retried at most every
/// 60 s; an auth-blocked plug fails without touching the network.
pub fn start_poller(app: AppHandle, manager: Arc<PlugManager>) {
    tauri::async_runtime::spawn(async move {
        let permits = Arc::new(Semaphore::new(POLL_CONCURRENCY));
        let mut schedule = HashMap::new();
        loop {
            poll(&app, &manager, &permits, &mut schedule).await;
            tokio::time::sleep(monitor::DEFAULT_INTERVAL).await;
        }
    });
}

/// One cycle. Keys come from a fresh list every time: the manager resets a
/// slot on any revision change, so stale summaries would thrash sessions.
async fn poll(
    app: &AppHandle,
    manager: &Arc<PlugManager>,
    permits: &Arc<Semaphore>,
    schedule: &mut HashMap<String, (Backoff, Instant)>,
) {
    let Ok(dir) = dir() else { return };
    let summaries = {
        let dir = dir.clone();
        match blocking(move || plugs::list(dir).map_err(plug_error)).await {
            Ok(summaries) => summaries,
            Err(_) => return,
        }
    };
    let before = manager.observations();
    let ids: HashSet<String> = summaries.iter().map(|plug| plug.id.clone()).collect();
    manager.retain(&ids);
    schedule.retain(|id, _| ids.contains(id));

    let now = Instant::now();
    let mut tasks = JoinSet::new();
    for summary in &summaries {
        let (backoff, due) = schedule
            .entry(summary.id.clone())
            .or_insert_with(|| (Backoff::new(monitor::DEFAULT_INTERVAL), now));
        if *due > now {
            continue;
        }
        let Ok(key) = PlugKey::try_from(summary) else {
            *due = now + backoff.after_result(false);
            continue;
        };
        let (manager, permits) = (manager.clone(), permits.clone());
        let resolve = resolver(dir.clone(), summary.id.clone());
        tasks.spawn(async move {
            let ok = match permits.acquire_owned().await {
                Ok(_permit) => manager.status(&key, true, resolve).await.is_ok(),
                Err(_) => false,
            };
            (key.id, ok)
        });
    }
    while let Some(joined) = tasks.join_next().await {
        if let Ok((id, ok)) = joined
            && let Some((backoff, due)) = schedule.get_mut(&id)
        {
            *due = Instant::now() + backoff.after_result(ok);
        }
    }
    if manager.observations() != before {
        publish(app, manager);
    }
}

fn publish(app: &AppHandle, manager: &PlugManager) {
    let _ = app.emit(UPDATED_EVENT, manager.observations());
}

fn dir() -> Result<PathBuf, CommandError> {
    config_dir().map_err(|_| unreadable_config())
}

async fn blocking<T, F>(work: F) -> Result<T, CommandError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| CommandError::new("internalError"))?
}

/// The plug's session key, from a fresh read of the config.
async fn current_key(id: &str) -> Result<(PathBuf, PlugKey), CommandError> {
    let dir = dir()?;
    let summaries = {
        let dir = dir.clone();
        blocking(move || plugs::list(dir).map_err(|_| unreadable_config())).await?
    };
    let summary = summaries
        .iter()
        .find(|plug| plug.id == id)
        .ok_or_else(|| CommandError::new("plugNotFound"))?;
    let key = PlugKey::try_from(summary).map_err(plug_error)?;
    Ok((dir, key))
}

fn resolver(
    dir: PathBuf,
    id: String,
) -> impl FnOnce() -> Result<ResolvedPlug, PlugError> + Send + 'static {
    move || plugs::resolve_by_id(dir, &SystemKeychain, &id)
}

fn link_target(
    printer: Option<String>,
    camera: Option<String>,
) -> Result<PlugTarget, CommandError> {
    match (printer, camera) {
        (Some(printer), None) => Ok(PlugTarget::Printer(printer)),
        (None, Some(camera)) => Ok(PlugTarget::Camera(camera)),
        _ => Err(CommandError::new("plugInvalidTarget")),
    }
}

/// No variant carries upstream text, so codes alone cross the boundary.
fn plug_error(error: PlugError) -> CommandError {
    use PlugError as E;
    CommandError::new(match error {
        E::MissingName => "profileMissingName",
        E::InvalidName => "profileInvalidName",
        E::InvalidHost => "profileInvalidHost",
        E::UnknownDriver(_) => "profileInvalid",
        E::InvalidUsername => "plugInvalidUsername",
        E::InvalidPassword => "plugInvalidPassword",
        E::InvalidTimeout => "plugInvalidTimeout",
        E::NotFound(_) => "plugNotFound",
        E::AlreadyExists(_) => "plugExists",
        E::PrinterNotFound(_) => "profileNotFound",
        E::CameraNotFound(_) => "cameraNotFound",
        E::NotLinked { .. } => "plugNotLinked",
        E::Authentication => "plugAuthFailed",
        E::CompatibilityDisabled => "plugCompatibilityDisabled",
        E::Lockout => "plugLocked",
        E::UnsupportedDevice(_) => "plugUnsupportedDevice",
        E::UnsupportedProtocol => "plugUnsupportedProtocol",
        E::Connect | E::MalformedResponse => "plugUnreachable",
        E::DeviceRejected => "plugDeviceRejected",
        E::Timeout => "plugTimeout",
        E::PowerUnconfirmed { .. } => "plugPowerUnconfirmed",
        E::PowerMismatch { .. } => "plugPowerMismatch",
        E::ProfileChanged => "plugProfileChanged",
        E::MissingSecret => "plugPasswordUnavailable",
        E::Secret(_) | E::RollbackFailed => "keychainFailed",
        E::Config(_) => "configUnwritable",
    })
}

#[cfg(test)]
mod tests {
    use melt_core::{
        config::{ConfigError, PlugTarget},
        keychain::SecretError,
        plugs::{Observation, PlugError, PowerState},
    };

    use super::{link_target, plug_error};

    #[test]
    fn plug_errors_map_to_stable_codes() {
        let cases = [
            (PlugError::MissingName, "profileMissingName"),
            (PlugError::InvalidName, "profileInvalidName"),
            (PlugError::UnknownDriver("x".into()), "profileInvalid"),
            (PlugError::InvalidHost, "profileInvalidHost"),
            (PlugError::InvalidUsername, "plugInvalidUsername"),
            (PlugError::InvalidPassword, "plugInvalidPassword"),
            (PlugError::InvalidTimeout, "plugInvalidTimeout"),
            (PlugError::NotFound("p".into()), "plugNotFound"),
            (PlugError::AlreadyExists("p".into()), "plugExists"),
            (PlugError::PrinterNotFound("x".into()), "profileNotFound"),
            (PlugError::CameraNotFound("c".into()), "cameraNotFound"),
            (
                PlugError::NotLinked {
                    plug: "p".into(),
                    target: "x".into(),
                },
                "plugNotLinked",
            ),
            (PlugError::Authentication, "plugAuthFailed"),
            (
                PlugError::CompatibilityDisabled,
                "plugCompatibilityDisabled",
            ),
            (PlugError::Lockout, "plugLocked"),
            (
                PlugError::UnsupportedDevice("P100".into()),
                "plugUnsupportedDevice",
            ),
            (PlugError::UnsupportedProtocol, "plugUnsupportedProtocol"),
            (PlugError::DeviceRejected, "plugDeviceRejected"),
            (PlugError::Connect, "plugUnreachable"),
            (PlugError::Timeout, "plugTimeout"),
            (PlugError::MalformedResponse, "plugUnreachable"),
            (
                PlugError::PowerUnconfirmed {
                    requested: PowerState::Off,
                    last_observed: Some(Observation {
                        state: PowerState::On,
                        observed_at: "2026-10-18T12:00:00Z".into(),
                    }),
                },
                "plugPowerUnconfirmed",
            ),
            (
                PlugError::PowerMismatch {
                    requested: PowerState::Off,
                    observed: PowerState::On,
                },
                "plugPowerMismatch",
            ),
            (PlugError::ProfileChanged, "plugProfileChanged"),
            (PlugError::MissingSecret, "plugPasswordUnavailable"),
            (PlugError::Secret(SecretError::NotFound), "keychainFailed"),
            (
                PlugError::Config(ConfigError::PlugNotFound),
                "configUnwritable",
            ),
            (PlugError::RollbackFailed, "keychainFailed"),
        ];
        for (error, code) in cases {
            assert_eq!(plug_error(error).code, code);
        }
    }

    #[test]
    fn link_requires_exactly_one_target() {
        assert_eq!(
            link_target(Some("workshop".into()), None).ok(),
            Some(PlugTarget::Printer("workshop".into()))
        );
        assert_eq!(
            link_target(None, Some("bench".into())).ok(),
            Some(PlugTarget::Camera("bench".into()))
        );
        for (printer, camera) in [(None, None), (Some("workshop"), Some("bench"))] {
            assert_eq!(
                link_target(printer.map(Into::into), camera.map(Into::into))
                    .err()
                    .map(|error| error.code),
                Some("plugInvalidTarget")
            );
        }
    }
}
