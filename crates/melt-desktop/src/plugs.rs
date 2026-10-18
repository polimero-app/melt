//! Smart plug commands, error codes and the background status poller.
//! Config and keychain work runs on blocking threads; device I/O goes
//! through the shared [`PlugManager`].

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Instant,
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
use tokio::sync::Semaphore;

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
    schedule: State<'_, SharedSchedule>,
) -> Result<PlugStatus, CommandError> {
    let (dir, key) = current_key(&id).await?;
    let result = manager.status(&key, false, resolver(dir, id.clone())).await;
    publish(&app, &manager);
    if result.is_ok() {
        lock(&schedule).succeeded(&id, &key.revision, Instant::now());
    }
    result.map_err(plug_error)
}

/// Switches once with readback; never retried.
#[tauri::command]
pub async fn plug_set_power(
    id: String,
    state: PowerState,
    app: AppHandle,
    manager: State<'_, Arc<PlugManager>>,
    schedule: State<'_, SharedSchedule>,
) -> Result<PowerChange, CommandError> {
    let (dir, key) = current_key(&id).await?;
    let revision = {
        let (dir, id) = (dir.clone(), id.clone());
        move || plugs::revision(dir, &id)
    };
    let result = manager
        .set_power(&key, state, resolver(dir, id.clone()), revision)
        .await;
    publish(&app, &manager);
    if result.is_ok() {
        lock(&schedule).succeeded(&id, &key.revision, Instant::now());
    }
    result.map_err(plug_error)
}

#[tauri::command(async)]
pub fn plug_observations(manager: State<'_, Arc<PlugManager>>) -> HashMap<String, PlugObservation> {
    manager.observations()
}

/// Polls every plug for the app's lifetime. Each plug backs off on its own,
/// so an unreachable, locked or password-less plug is retried at most every
/// 60 s; an auth-blocked plug fails without touching the network. Reads run
/// detached, so a slow plug never delays another.
pub fn start_poller(app: AppHandle, manager: Arc<PlugManager>, schedule: SharedSchedule) {
    tauri::async_runtime::spawn(async move {
        let permits = Arc::new(Semaphore::new(POLL_CONCURRENCY));
        loop {
            poll(&app, &manager, &schedule, &permits).await;
            tokio::time::sleep(monitor::DEFAULT_INTERVAL).await;
        }
    });
}

/// One tick: starts a read for every due plug that is not already being
/// read. Keys come from a fresh list every time: the manager resets a slot
/// on any revision change, so stale summaries would thrash sessions.
async fn poll(
    app: &AppHandle,
    manager: &Arc<PlugManager>,
    schedule: &SharedSchedule,
    permits: &Arc<Semaphore>,
) {
    let Ok(dir) = dir() else { return };
    let summaries = {
        let dir = dir.clone();
        match blocking(move || plugs::list(dir).map_err(plug_error)).await {
            Ok(summaries) => summaries,
            Err(_) => return,
        }
    };
    let ids: HashSet<String> = summaries.iter().map(|plug| plug.id.clone()).collect();
    let known = manager.observations().len();
    manager.retain(&ids);
    if manager.observations().len() != known {
        publish(app, manager);
    }

    let revisions: Vec<(String, String)> = summaries
        .iter()
        .map(|plug| (plug.id.clone(), plug.updated.clone()))
        .collect();
    let due = lock(schedule).take_due(&revisions, Instant::now());
    for summary in summaries.iter().filter(|plug| due.contains(&plug.id)) {
        let (id, revision) = (summary.id.clone(), summary.updated.clone());
        let Ok(key) = PlugKey::try_from(summary) else {
            lock(schedule).finished(&id, &revision, false, Instant::now());
            continue;
        };
        let resolve = resolver(dir.clone(), id.clone());
        let (app, manager, schedule, permits) = (
            app.clone(),
            manager.clone(),
            schedule.clone(),
            permits.clone(),
        );
        tauri::async_runtime::spawn(async move {
            let before = manager.observations().remove(&id);
            // A nested task, so a panic still counts as a failed read.
            let read = tauri::async_runtime::spawn({
                let manager = manager.clone();
                async move {
                    let Ok(_permit) = permits.acquire_owned().await else {
                        return false;
                    };
                    manager.status(&key, true, resolve).await.is_ok()
                }
            });
            let ok = matches!(read.await, Ok(true));
            lock(&schedule).finished(&id, &revision, ok, Instant::now());
            if manager.observations().remove(&id) != before {
                publish(&app, &manager);
            }
        });
    }
}

/// Which plugs the poller reads next. Keyed by plug ID and profile revision:
/// a new revision (an edit) starts with a fresh backoff.
#[derive(Default)]
pub struct Schedule(HashMap<String, Slot>);

struct Slot {
    revision: String,
    backoff: Backoff,
    due: Instant,
    in_flight: bool,
}

impl Schedule {
    /// Syncs to `plugs` (`(id, revision)`), then returns the due plugs that
    /// are not already being read and marks them in flight.
    fn take_due(&mut self, plugs: &[(String, String)], now: Instant) -> Vec<String> {
        self.0
            .retain(|id, _| plugs.iter().any(|(plug, _)| plug == id));
        let mut due = Vec::new();
        for (id, revision) in plugs {
            let fresh = || Slot {
                revision: revision.clone(),
                backoff: Backoff::new(monitor::DEFAULT_INTERVAL),
                due: now,
                in_flight: false,
            };
            let slot = self.0.entry(id.clone()).or_insert_with(fresh);
            if slot.revision != *revision {
                *slot = fresh();
            }
            if !slot.in_flight && slot.due <= now {
                slot.in_flight = true;
                due.push(id.clone());
            }
        }
        due
    }

    /// A poll finished. Results for an older revision are ignored.
    fn finished(&mut self, id: &str, revision: &str, ok: bool, now: Instant) {
        if let Some(slot) = self.slot(id, revision) {
            slot.in_flight = false;
            slot.due = now + slot.backoff.after_result(ok);
        }
    }

    /// An explicit read or write succeeded: the plug is healthy again.
    fn succeeded(&mut self, id: &str, revision: &str, now: Instant) {
        if let Some(slot) = self.slot(id, revision) {
            slot.due = now + slot.backoff.after_result(true);
        }
    }

    fn slot(&mut self, id: &str, revision: &str) -> Option<&mut Slot> {
        self.0.get_mut(id).filter(|slot| slot.revision == revision)
    }
}

/// Shared between the poller and the explicit commands.
pub type SharedSchedule = Arc<Mutex<Schedule>>;

/// The schedule holds no invariant a panic could break mid-update.
fn lock(schedule: &SharedSchedule) -> MutexGuard<'_, Schedule> {
    schedule.lock().unwrap_or_else(PoisonError::into_inner)
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

    use std::time::{Duration, Instant};

    use super::{Schedule, link_target, plug_error};

    fn plugs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(id, revision)| (id.to_string(), revision.to_string()))
            .collect()
    }

    #[test]
    fn a_plug_in_flight_never_delays_a_due_plug() {
        let mut schedule = Schedule::default();
        let start = Instant::now();
        let both = plugs(&[("slow", "r1"), ("fast", "r1")]);
        assert_eq!(schedule.take_due(&both, start), ["slow", "fast"]);
        schedule.finished("fast", "r1", true, start);

        let next = start + Duration::from_secs(5);
        assert_eq!(schedule.take_due(&both, next), ["fast"]);
        schedule.finished("slow", "r1", true, next);
        assert!(schedule.take_due(&both, next).is_empty());
        assert_eq!(
            schedule.take_due(&both, next + Duration::from_secs(5)),
            ["slow"]
        );
    }

    #[test]
    fn failures_back_off_until_the_revision_changes() {
        let mut schedule = Schedule::default();
        let start = Instant::now();
        let old = plugs(&[("plug", "r1")]);
        assert_eq!(schedule.take_due(&old, start), ["plug"]);
        for _ in 0..4 {
            schedule.finished("plug", "r1", false, start);
        }
        let soon = start + Duration::from_secs(10);
        assert!(schedule.take_due(&old, soon).is_empty());

        let edited = plugs(&[("plug", "r2")]);
        assert_eq!(schedule.take_due(&edited, soon), ["plug"]);
        // A late result from the old revision changes nothing.
        schedule.finished("plug", "r1", false, soon);
        schedule.finished("plug", "r2", true, soon);
        assert_eq!(
            schedule.take_due(&edited, soon + Duration::from_secs(5)),
            ["plug"]
        );
    }

    #[test]
    fn an_explicit_success_resets_the_backoff() {
        let mut schedule = Schedule::default();
        let start = Instant::now();
        let one = plugs(&[("plug", "r1")]);
        for _ in 0..4 {
            schedule.take_due(&one, start);
            schedule.finished("plug", "r1", false, start);
        }
        schedule.succeeded("plug", "r1", start);
        assert_eq!(
            schedule.take_due(&one, start + Duration::from_secs(5)),
            ["plug"]
        );
    }

    #[test]
    fn removed_plugs_are_forgotten() {
        let mut schedule = Schedule::default();
        let start = Instant::now();
        schedule.take_due(&plugs(&[("plug", "r1")]), start);
        assert!(schedule.take_due(&[], start).is_empty());
        assert_eq!(
            schedule.take_due(&plugs(&[("plug", "r1")]), start),
            ["plug"]
        );
    }

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
