//! Long-lived plug sessions for the desktop: one serialized slot per plug,
//! revision checks, the auth block and the observations the UI polls.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::Serialize;
use tokio::{
    sync::{Mutex as AsyncMutex, OwnedMutexGuard},
    time::{Instant, timeout_at},
};

use super::{
    DeviceInfo, Observation, PlugError, PlugStatus, PlugSummary, PowerChange, PowerState,
    RelayState, ResolvedPlug, Session, switch, with_deadline,
};
use crate::drivers;

/// What identifies one plug session: its ID, the profile revision it was
/// built from and the per-operation deadline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlugKey {
    pub id: String,
    pub revision: String,
    pub timeout: Duration,
}

impl TryFrom<&PlugSummary> for PlugKey {
    type Error = PlugError;

    fn try_from(summary: &PlugSummary) -> Result<Self, PlugError> {
        Ok(Self {
            id: summary.id.clone(),
            revision: summary.updated.clone(),
            timeout: drivers::parse_timeout(&summary.timeout)
                .map_err(|_| PlugError::InvalidTimeout)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlugObservation {
    pub relay: RelayState,
    pub connection: Connection,
    /// The last relay reading; kept as stale while the plug is unreachable.
    pub last: Option<Observation>,
    pub device: Option<DeviceInfo>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Connection {
    Unknown,
    Online,
    Unreachable,
    AuthFailed,
    Locked,
    Unsupported,
}

struct Slot {
    revision: String,
    /// The [`Entry::generation`] this slot's session belongs to.
    generation: u64,
    session: Option<Session>,
    /// The authentication error that stopped automatic logins, if any.
    blocked: Option<PlugError>,
}

struct Entry {
    slot: Arc<AsyncMutex<Slot>>,
    /// Bumped by `evict`: the slot's session and block are dropped by the
    /// next operation, and results from before the bump are discarded.
    generation: u64,
}

/// Lock order: `slots`, then `observations`.
#[derive(Default)]
pub struct PlugManager {
    /// Never held across `.await`.
    slots: Mutex<HashMap<String, Entry>>,
    /// Written only while the plug's slot lock and `slots` are held, so
    /// results land in completion order and never after an evict/retain.
    observations: Mutex<HashMap<String, PlugObservation>>,
}

impl PlugManager {
    /// Reads the relay from the device. `automatic` polls stop at an auth
    /// block; explicit calls clear it. `resolve` runs only to open a session.
    pub async fn status<R>(
        &self,
        key: &PlugKey,
        automatic: bool,
        resolve: R,
    ) -> Result<PlugStatus, PlugError>
    where
        R: FnOnce() -> Result<ResolvedPlug, PlugError> + Send + 'static,
    {
        let deadline = Instant::now() + key.timeout;
        let mut slot = self.enter(key, deadline).await?;
        if automatic && let Some(blocked) = &slot.blocked {
            return Err(auth_error(blocked).expect("only auth errors block"));
        }
        slot.blocked = None;
        let result = async {
            let session = connected(&mut slot.session, resolve, deadline).await?;
            with_deadline(deadline, session.status()).await
        }
        .await;
        self.settle(&key.id, &mut slot, result.as_ref());
        result
    }

    /// Switches the relay once with readback. Revalidates the profile after
    /// taking the slot and again after the pre-read, immediately before send.
    pub async fn set_power<R, C>(
        &self,
        key: &PlugKey,
        state: PowerState,
        resolve: R,
        current_revision: C,
    ) -> Result<PowerChange, PlugError>
    where
        R: FnOnce() -> Result<ResolvedPlug, PlugError> + Send + 'static,
        C: Fn() -> Result<Option<String>, PlugError> + Clone + Send + 'static,
    {
        let deadline = Instant::now() + key.timeout;
        let mut slot = self.enter(key, deadline).await?;
        slot.blocked = None;
        let checked = check_revision(key, deadline, current_revision.clone()).await;
        if let Err(error) = checked {
            // Nothing reached the device: leave the observation alone.
            slot.session = None;
            return Err(error);
        }
        let last = self
            .observations
            .lock()
            .unwrap()
            .get(&key.id)
            .and_then(|seen| seen.last.clone());
        let generation = slot.generation;
        let cell = OwnedMutexGuard::mutex(&slot).clone();
        let result = async {
            let session = connected(&mut slot.session, resolve, deadline).await?;
            switch(session, state, deadline, last, || async {
                check_revision(key, deadline, current_revision).await?;
                let slots = self.slots.lock().unwrap();
                match slots.get(&key.id) {
                    None => Err(PlugError::NotFound(key.id.clone())),
                    Some(entry)
                        if !Arc::ptr_eq(&entry.slot, &cell) || entry.generation != generation =>
                    {
                        Err(PlugError::ProfileChanged)
                    }
                    Some(_) => Ok(()),
                }
            })
            .await
        }
        .await;
        self.settle(
            &key.id,
            &mut slot,
            result.as_ref().map(|change| &change.status),
        );
        result
    }

    pub fn observations(&self) -> HashMap<String, PlugObservation> {
        self.observations.lock().unwrap().clone()
    }

    /// Drops the plug's session, auth block and observation. The slot stays,
    /// so an operation in flight still serializes with the next one; its
    /// result is discarded.
    pub fn evict(&self, id: &str) {
        let mut slots = self.slots.lock().unwrap();
        if let Some(entry) = slots.get_mut(id) {
            entry.generation += 1;
        }
        self.observations.lock().unwrap().remove(id);
    }

    /// Forgets every plug not in `ids`. Those plugs are gone from the config,
    /// so an operation that recreates a slot fails to resolve.
    pub fn retain(&self, ids: &HashSet<String>) {
        let mut slots = self.slots.lock().unwrap();
        slots.retain(|id, _| ids.contains(id));
        self.observations
            .lock()
            .unwrap()
            .retain(|id, _| ids.contains(id));
    }

    /// Takes the plug's slot lock within `deadline`, resetting the slot if
    /// the profile revision changed or the plug was evicted.
    async fn enter(
        &self,
        key: &PlugKey,
        deadline: Instant,
    ) -> Result<OwnedMutexGuard<Slot>, PlugError> {
        let cell = self
            .slots
            .lock()
            .unwrap()
            .entry(key.id.clone())
            .or_insert_with(|| Entry {
                slot: Arc::new(AsyncMutex::new(Slot {
                    revision: key.revision.clone(),
                    generation: 0,
                    session: None,
                    blocked: None,
                })),
                generation: 0,
            })
            .slot
            .clone();
        let mut slot = timeout_at(deadline, cell.lock_owned())
            .await
            .map_err(|_| PlugError::Timeout)?;
        // A slot retained away meanwhile keeps its generation; `settle`
        // discards its result.
        let generation = self
            .slots
            .lock()
            .unwrap()
            .get(&key.id)
            .map_or(slot.generation, |entry| entry.generation);
        if slot.revision != key.revision || slot.generation != generation {
            slot.revision = key.revision.clone();
            slot.generation = generation;
            slot.session = None;
            slot.blocked = None;
        }
        Ok(slot)
    }

    /// Applies a device result to the slot and records it, still under the
    /// slot lock. Results from an evicted or retained-away slot are dropped.
    fn settle(
        &self,
        id: &str,
        slot: &mut OwnedMutexGuard<Slot>,
        result: Result<&PlugStatus, &PlugError>,
    ) {
        if let Err(error) = result {
            slot.session = None;
            if let Some(blocked) = auth_error(error) {
                slot.blocked = Some(blocked);
            }
        }
        // Held until the observation is written, so an evict or retain cannot
        // slip in between the check and the write.
        let slots = self.slots.lock().unwrap();
        let current = slots.get(id).is_some_and(|entry| {
            Arc::ptr_eq(&entry.slot, OwnedMutexGuard::mutex(slot))
                && entry.generation == slot.generation
        });
        if !current {
            slot.session = None;
            return;
        }
        let mut observations = self.observations.lock().unwrap();
        let seen = observations
            .entry(id.to_owned())
            .or_insert(PlugObservation {
                relay: RelayState::Unknown,
                connection: Connection::Unknown,
                last: None,
                device: None,
            });
        match result {
            Ok(status) => {
                seen.relay = relay(status.relay);
                seen.connection = Connection::Online;
                seen.last = Some(Observation {
                    state: status.relay,
                    observed_at: status.observed_at.clone(),
                });
                seen.device = Some(status.device.clone());
            }
            // A fresh device reading, just not the requested one.
            Err(PlugError::PowerMismatch { observed, .. }) => {
                seen.relay = relay(*observed);
                seen.connection = Connection::Online;
                seen.last = Some(Observation {
                    state: *observed,
                    observed_at: crate::profiles::now(),
                });
            }
            Err(error) => {
                seen.relay = RelayState::Unknown;
                seen.connection = connection(error);
                if let PlugError::PowerUnconfirmed {
                    last_observed: Some(before),
                    ..
                } = error
                {
                    // The pre-read is at least as fresh as what we had.
                    seen.last = Some(before.clone());
                }
            }
        }
    }
}

async fn check_revision<C>(key: &PlugKey, deadline: Instant, current: C) -> Result<(), PlugError>
where
    C: FnOnce() -> Result<Option<String>, PlugError> + Send + 'static,
{
    match with_deadline(deadline, blocking(current)).await? {
        Some(revision) if revision == key.revision => Ok(()),
        Some(_) => Err(PlugError::ProfileChanged),
        None => Err(PlugError::NotFound(key.id.clone())),
    }
}

/// Opens a session if the slot has none: `resolve` on the blocking pool,
/// then connect, both within `deadline`.
async fn connected<R>(
    session: &mut Option<Session>,
    resolve: R,
    deadline: Instant,
) -> Result<&mut Session, PlugError>
where
    R: FnOnce() -> Result<ResolvedPlug, PlugError> + Send + 'static,
{
    if session.is_none() {
        let plug = with_deadline(deadline, blocking(resolve)).await?;
        *session = Some(with_deadline(deadline, Session::connect(&plug)).await?);
    }
    Ok(session.as_mut().expect("connected above"))
}

async fn blocking<T, F>(work: F) -> Result<T, PlugError>
where
    F: FnOnce() -> Result<T, PlugError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|join| std::panic::resume_unwind(join.into_panic()))
}

/// A copy of `error` if it stops automatic logins.
fn auth_error(error: &PlugError) -> Option<PlugError> {
    match error {
        PlugError::Authentication => Some(PlugError::Authentication),
        PlugError::CompatibilityDisabled => Some(PlugError::CompatibilityDisabled),
        PlugError::Lockout => Some(PlugError::Lockout),
        PlugError::PowerUnconfirmed { cause, .. } => auth_error(cause),
        _ => None,
    }
}

fn connection(error: &PlugError) -> Connection {
    match error {
        PlugError::Authentication | PlugError::CompatibilityDisabled => Connection::AuthFailed,
        PlugError::Lockout => Connection::Locked,
        PlugError::UnsupportedDevice(_)
        | PlugError::UnsupportedProtocol
        | PlugError::IdentityUnverified
        | PlugError::IdentityMismatch => Connection::Unsupported,
        PlugError::PowerUnconfirmed { cause, .. } => connection(cause),
        _ => Connection::Unreachable,
    }
}

fn relay(state: PowerState) -> RelayState {
    match state {
        PowerState::On => RelayState::On,
        PowerState::Off => RelayState::Off,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    use tokio::sync::oneshot;

    use super::*;
    use crate::{
        config::PlugProfile,
        plugs::{DEFAULT_TIMEOUT, FAKES, FakeSession, FakeStep},
    };

    const ID: &str = "11111111111111111111111111111111";

    fn key(id: &str, revision: &str) -> PlugKey {
        PlugKey {
            id: id.into(),
            revision: revision.into(),
            timeout: Duration::from_secs(10),
        }
    }

    fn plug(host: &str) -> ResolvedPlug {
        ResolvedPlug {
            name: "bench".into(),
            profile: PlugProfile {
                id: ID.into(),
                device_id: Some("test-p110".into()),
                driver: "fake".into(),
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

    /// Installs a fake for `host`; returns its shared relay.
    fn install(host: &str, fake: FakeSession) -> Arc<Mutex<PowerState>> {
        let relay = fake.relay.clone();
        FAKES.lock().unwrap().insert(host.into(), fake);
        relay
    }

    fn installed(host: &str) -> bool {
        FAKES.lock().unwrap().contains_key(host)
    }

    fn resolver(host: &str) -> impl FnOnce() -> Result<ResolvedPlug, PlugError> + Send + 'static {
        let host = host.to_owned();
        move || Ok(plug(&host))
    }

    fn revision(
        value: Option<&str>,
    ) -> impl Fn() -> Result<Option<String>, PlugError> + Clone + Send {
        let value = value.map(str::to_owned);
        move || Ok(value.clone())
    }

    /// A resolver that holds the slot lock until released: it reports on
    /// `entered` once called, then blocks until `release` fires.
    fn gated(
        host: &str,
    ) -> (
        impl FnOnce() -> Result<ResolvedPlug, PlugError> + Send + 'static,
        oneshot::Receiver<()>,
        mpsc::Sender<()>,
    ) {
        let (entered_tx, entered) = oneshot::channel();
        let (release, release_rx) = mpsc::channel();
        let host = host.to_owned();
        let resolve = move || {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(plug(&host))
        };
        (resolve, entered, release)
    }

    fn observation(manager: &PlugManager, id: &str) -> PlugObservation {
        manager.observations().remove(id).expect("observation")
    }

    #[tokio::test]
    async fn external_relay_change_shows_on_next_status() {
        let host = "manager-external";
        let relay = install(host, FakeSession::new(PowerState::On, []));
        let manager = PlugManager::default();
        let k = key(ID, "r1");

        let status = manager.status(&k, true, resolver(host)).await.unwrap();
        assert_eq!(status.relay, PowerState::On);

        // The physical button or the Tapo app switches the relay.
        *relay.lock().unwrap() = PowerState::Off;
        let status = manager.status(&k, true, resolver(host)).await.unwrap();
        assert_eq!(status.relay, PowerState::Off);
        let seen = observation(&manager, ID);
        assert_eq!(seen.relay, RelayState::Off);
        assert_eq!(seen.connection, Connection::Online);
        assert_eq!(seen.last.unwrap().state, PowerState::Off);
        assert_eq!(seen.device.unwrap().model, "P110");
    }

    #[tokio::test]
    async fn session_is_reused_until_revision_changes() {
        let host = "manager-reuse";
        install(host, FakeSession::new(PowerState::On, []));
        let manager = PlugManager::default();
        let resolves = Arc::new(AtomicUsize::new(0));
        let counting = |resolves: &Arc<AtomicUsize>| {
            let resolves = resolves.clone();
            move || {
                resolves.fetch_add(1, Ordering::SeqCst);
                Ok(plug(host))
            }
        };

        for _ in 0..3 {
            manager
                .status(&key(ID, "r1"), true, counting(&resolves))
                .await
                .unwrap();
        }
        assert_eq!(resolves.load(Ordering::SeqCst), 1, "one connect");

        install(host, FakeSession::new(PowerState::Off, []));
        let status = manager
            .status(&key(ID, "r2"), true, counting(&resolves))
            .await
            .unwrap();
        assert_eq!(status.relay, PowerState::Off);
        assert_eq!(resolves.load(Ordering::SeqCst), 2, "reconnect on revision");
        assert!(!installed(host), "new session came from the second fake");
    }

    /// Runs a write queued behind a poll that holds the slot; `edit` runs
    /// while the write waits.
    async fn queued_write(host: &str, edit: impl FnOnce(&Mutex<Option<String>>)) -> PlugError {
        let relay = install(host, FakeSession::new(PowerState::Off, []));
        let manager = Arc::new(PlugManager::default());
        let stored = Arc::new(Mutex::new(Some("r1".to_owned())));

        let (resolve, entered, release) = gated(host);
        let poll = tokio::spawn({
            let manager = manager.clone();
            async move { manager.status(&key(ID, "r1"), true, resolve).await }
        });
        entered.await.unwrap();
        let write = tokio::spawn({
            let manager = manager.clone();
            let stored = stored.clone();
            let host = host.to_owned();
            async move {
                let current = move || Ok(stored.lock().unwrap().clone());
                manager
                    .set_power(&key(ID, "r1"), PowerState::On, resolver(&host), current)
                    .await
            }
        });
        tokio::task::yield_now().await;
        edit(&stored);
        release.send(()).unwrap();

        assert_eq!(poll.await.unwrap().unwrap().relay, PowerState::Off);
        let error = write.await.unwrap().unwrap_err();
        assert_eq!(*relay.lock().unwrap(), PowerState::Off, "nothing sent");
        error
    }

    #[tokio::test]
    async fn queued_write_after_edit_returns_profile_changed_and_sends_nothing() {
        let error = queued_write("manager-edit", |stored| {
            *stored.lock().unwrap() = Some("r2".into());
        })
        .await;
        assert!(matches!(error, PlugError::ProfileChanged), "{error:?}");
    }

    #[tokio::test]
    async fn queued_write_after_removal_returns_not_found() {
        let error = queued_write("manager-removed", |stored| {
            *stored.lock().unwrap() = None;
        })
        .await;
        assert!(matches!(error, PlugError::NotFound(_)), "{error:?}");
    }

    #[tokio::test]
    async fn auth_failure_blocks_automatic_status_until_explicit_or_revision_change() {
        let host = "manager-auth";
        let manager = PlugManager::default();
        let k = key(ID, "r1");
        let refusing = |error| FakeSession::new(PowerState::On, [FakeStep::Err(error)]);

        install(host, refusing(PlugError::Authentication));
        let error = manager.status(&k, true, resolver(host)).await.unwrap_err();
        assert!(matches!(error, PlugError::Authentication), "{error:?}");
        assert_eq!(observation(&manager, ID).connection, Connection::AuthFailed);

        // Blocked: automatic polls make no login attempt.
        install(host, FakeSession::new(PowerState::On, []));
        for _ in 0..2 {
            let error = manager.status(&k, true, resolver(host)).await.unwrap_err();
            assert!(matches!(error, PlugError::Authentication), "{error:?}");
        }
        assert!(installed(host), "no connect while blocked");

        // An explicit request tries again and clears the block.
        manager.status(&k, false, resolver(host)).await.unwrap();
        assert!(!installed(host));
        assert_eq!(observation(&manager, ID).connection, Connection::Online);

        // Lockout blocks too, with its own error, until the revision changes.
        manager.evict(ID);
        install(host, refusing(PlugError::Lockout));
        let error = manager.status(&k, true, resolver(host)).await.unwrap_err();
        assert!(matches!(error, PlugError::Lockout), "{error:?}");
        assert_eq!(observation(&manager, ID).connection, Connection::Locked);
        install(host, FakeSession::new(PowerState::Off, []));
        let error = manager.status(&k, true, resolver(host)).await.unwrap_err();
        assert!(matches!(error, PlugError::Lockout), "{error:?}");
        assert!(installed(host));
        let status = manager
            .status(&key(ID, "r2"), true, resolver(host))
            .await
            .unwrap();
        assert_eq!(status.relay, PowerState::Off);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_plug_does_not_delay_another_plug() {
        let slow_id = "22222222222222222222222222222222";
        install(
            "manager-slow",
            FakeSession::new(PowerState::On, [FakeStep::Ok, FakeStep::Hang]),
        );
        install("manager-fast", FakeSession::new(PowerState::Off, []));
        let manager = Arc::new(PlugManager::default());
        // Open the slow plug's session, then hang its next read.
        let slow = tokio::spawn({
            let manager = manager.clone();
            async move {
                manager
                    .status(&key(slow_id, "r1"), true, resolver("manager-slow"))
                    .await
            }
        });

        let started = Instant::now();
        let status = manager
            .status(&key(ID, "r1"), true, resolver("manager-fast"))
            .await
            .unwrap();
        assert_eq!(status.relay, PowerState::Off);
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert!(!slow.is_finished());

        let error = slow.await.unwrap().unwrap_err();
        assert!(matches!(error, PlugError::Timeout), "{error:?}");
    }

    #[tokio::test]
    async fn poll_finishing_after_write_cannot_overwrite_newer_observation() {
        let host = "manager-order";
        install(host, FakeSession::new(PowerState::Off, []));
        let manager = Arc::new(PlugManager::default());

        // The poll is slow; the write is issued while it is in flight.
        let (resolve, entered, release) = gated(host);
        let poll = tokio::spawn({
            let manager = manager.clone();
            async move { manager.status(&key(ID, "r1"), true, resolve).await }
        });
        entered.await.unwrap();
        let write = tokio::spawn({
            let manager = manager.clone();
            async move {
                manager
                    .set_power(
                        &key(ID, "r1"),
                        PowerState::On,
                        resolver("unused"),
                        revision(Some("r1")),
                    )
                    .await
            }
        });
        tokio::task::yield_now().await;
        release.send(()).unwrap();

        assert_eq!(poll.await.unwrap().unwrap().relay, PowerState::Off);
        let change = write.await.unwrap().unwrap();
        assert_eq!(change.status.relay, PowerState::On);
        let seen = observation(&manager, ID);
        assert_eq!(seen.relay, RelayState::On);
        assert_eq!(seen.last.unwrap().state, PowerState::On);
    }

    #[tokio::test(start_paused = true)]
    async fn lock_wait_counts_against_the_deadline() {
        let host = "manager-deadline";
        install(
            host,
            FakeSession::new(PowerState::On, [FakeStep::Ok, FakeStep::Hang]),
        );
        let manager = Arc::new(PlugManager::default());
        let busy = tokio::spawn({
            let manager = manager.clone();
            async move { manager.status(&key(ID, "r1"), true, resolver(host)).await }
        });
        tokio::task::yield_now().await;

        let mut short = key(ID, "r1");
        short.timeout = Duration::from_secs(2);
        let started = Instant::now();
        let error = manager
            .status(&short, false, resolver(host))
            .await
            .unwrap_err();
        assert!(matches!(error, PlugError::Timeout), "{error:?}");
        assert_eq!(started.elapsed(), Duration::from_secs(2));
        assert!(!busy.is_finished(), "the lock was never acquired");
        busy.abort();
    }

    #[tokio::test]
    async fn failure_keeps_last_observation_as_stale() {
        let host = "manager-stale";
        install(
            host,
            FakeSession::new(
                PowerState::On,
                [
                    FakeStep::Ok,
                    FakeStep::Ok,
                    FakeStep::Err(PlugError::Connect),
                ],
            ),
        );
        let manager = PlugManager::default();
        let k = key(ID, "r1");
        manager.status(&k, true, resolver(host)).await.unwrap();
        let error = manager.status(&k, true, resolver(host)).await.unwrap_err();
        assert!(matches!(error, PlugError::Connect), "{error:?}");

        let seen = observation(&manager, ID);
        assert_eq!(seen.relay, RelayState::Unknown);
        assert_eq!(seen.connection, Connection::Unreachable);
        assert_eq!(seen.last.unwrap().state, PowerState::On);
    }

    #[tokio::test]
    async fn retain_and_evict_drop_observations() {
        install("manager-retain-a", FakeSession::new(PowerState::On, []));
        install("manager-retain-b", FakeSession::new(PowerState::On, []));
        let other = "33333333333333333333333333333333";
        let manager = PlugManager::default();
        manager
            .status(&key(ID, "r1"), true, resolver("manager-retain-a"))
            .await
            .unwrap();
        manager
            .status(&key(other, "r1"), true, resolver("manager-retain-b"))
            .await
            .unwrap();

        manager.retain(&HashSet::from([ID.to_owned()]));
        assert_eq!(manager.observations().into_keys().collect::<Vec<_>>(), [ID]);
        manager.evict(ID);
        assert!(manager.observations().is_empty());

        // The evicted session is gone: the next status connects again.
        let error = manager
            .status(&key(ID, "r1"), true, resolver("manager-retain-a"))
            .await
            .unwrap_err();
        assert!(matches!(error, PlugError::Connect), "{error:?}");
    }

    #[tokio::test]
    async fn write_errors_record_mismatch_and_unconfirmed() {
        let host = "manager-write-errors";
        let manager = PlugManager::default();
        let k = key(ID, "r1");
        let write = || manager.set_power(&k, PowerState::On, resolver(host), revision(Some("r1")));

        // connect read, pre-read, send, readback says Off.
        let steps = [
            FakeStep::Ok,
            FakeStep::Ok,
            FakeStep::Ok,
            FakeStep::Status(PowerState::Off),
        ];
        install(host, FakeSession::new(PowerState::Off, steps));
        let error = write().await.unwrap_err();
        assert!(
            matches!(error, PlugError::PowerMismatch { .. }),
            "{error:?}"
        );
        let seen = observation(&manager, ID);
        assert_eq!(seen.relay, RelayState::Off);
        assert_eq!(seen.connection, Connection::Online);

        // The session was dropped: a new one, whose readback is lost.
        let steps = [
            FakeStep::Ok,
            FakeStep::Ok,
            FakeStep::Ok,
            FakeStep::Err(PlugError::Connect),
        ];
        install(host, FakeSession::new(PowerState::Off, steps));
        let error = write().await.unwrap_err();
        assert!(
            matches!(error, PlugError::PowerUnconfirmed { .. }),
            "{error:?}"
        );
        let seen = observation(&manager, ID);
        assert_eq!(seen.relay, RelayState::Unknown);
        assert_eq!(seen.connection, Connection::Unreachable);
        assert_eq!(seen.last.unwrap().state, PowerState::Off, "pre-read kept");
    }

    /// Starts a status on `host` whose session opens and then hangs, so the
    /// task holds the slot lock until its deadline. Returns once it hangs.
    async fn hanging(
        manager: &Arc<PlugManager>,
        host: &'static str,
    ) -> tokio::task::JoinHandle<Result<PlugStatus, PlugError>> {
        install(
            host,
            FakeSession::new(PowerState::On, [FakeStep::Ok, FakeStep::Hang]),
        );
        let task = tokio::spawn({
            let manager = manager.clone();
            async move { manager.status(&key(ID, "r1"), true, resolver(host)).await }
        });
        // The fake is taken and the read hangs within one poll of the task.
        while installed(host) {
            tokio::task::yield_now().await;
        }
        task
    }

    #[tokio::test(start_paused = true)]
    async fn evict_keeps_operations_on_one_plug_serialized() {
        let host = "manager-evict-serial";
        let manager = Arc::new(PlugManager::default());
        let first = hanging(&manager, host).await;

        manager.evict(ID);
        install(host, FakeSession::new(PowerState::Off, []));
        let second = tokio::time::timeout(
            Duration::from_secs(1),
            manager.status(&key(ID, "r1"), false, resolver(host)),
        )
        .await;
        assert!(second.is_err(), "second operation ran beside the first");
        assert!(installed(host), "no second session while the first runs");

        let error = first.await.unwrap().unwrap_err();
        assert!(matches!(error, PlugError::Timeout), "{error:?}");
        assert!(
            manager.observations().is_empty(),
            "an operation from before the evict records nothing"
        );

        // The evict dropped the old session: the next call opens a new one.
        let status = manager
            .status(&key(ID, "r1"), false, resolver(host))
            .await
            .unwrap();
        assert_eq!(status.relay, PowerState::Off);
        assert!(!installed(host));
    }

    #[tokio::test(start_paused = true)]
    async fn retained_away_plug_gets_no_observation_from_in_flight_operation() {
        let manager = Arc::new(PlugManager::default());
        let first = hanging(&manager, "manager-retain-ghost").await;
        manager.retain(&HashSet::new());
        let error = first.await.unwrap().unwrap_err();
        assert!(matches!(error, PlugError::Timeout), "{error:?}");
        assert!(manager.observations().is_empty());
    }

    #[tokio::test]
    async fn evicted_operation_cannot_dispatch_after_resolving_credentials() {
        let host = "manager-evict-before-dispatch";
        let relay = install(host, FakeSession::new(PowerState::On, []));
        let manager = Arc::new(PlugManager::default());
        let (resolve, entered, release) = gated(host);
        let write = tokio::spawn({
            let manager = manager.clone();
            async move {
                manager
                    .set_power(
                        &key(ID, "r1"),
                        PowerState::Off,
                        resolve,
                        revision(Some("r1")),
                    )
                    .await
            }
        });
        entered.await.unwrap();
        manager.evict(ID);
        release.send(()).unwrap();

        let result = write.await.unwrap();
        assert_eq!(
            *relay.lock().unwrap(),
            PowerState::On,
            "no relay command after eviction"
        );
        assert!(matches!(result, Err(PlugError::ProfileChanged)));
        assert!(manager.observations().is_empty());
    }

    #[tokio::test]
    async fn authentication_failure_during_readback_blocks_automatic_logins() {
        for (suffix, error) in [
            ("lockout", PlugError::Lockout),
            ("auth", PlugError::Authentication),
            ("compatibility", PlugError::CompatibilityDisabled),
        ] {
            let host = format!("manager-readback-{suffix}");
            install(
                &host,
                FakeSession::new(
                    PowerState::On,
                    [
                        FakeStep::Ok,
                        FakeStep::Ok,
                        FakeStep::Ok,
                        FakeStep::Err(error),
                    ],
                ),
            );
            let manager = PlugManager::default();
            let k = key(ID, "r1");
            let result = manager
                .set_power(&k, PowerState::Off, resolver(&host), revision(Some("r1")))
                .await;
            assert!(matches!(result, Err(PlugError::PowerUnconfirmed { .. })));
            install(&host, FakeSession::new(PowerState::Off, []));

            let poll = manager.status(&k, true, resolver(&host)).await;

            assert!(
                matches!(
                    poll,
                    Err(PlugError::Lockout
                        | PlugError::Authentication
                        | PlugError::CompatibilityDisabled)
                ),
                "{poll:?}"
            );
            assert!(
                installed(&host),
                "automatic poll must not authenticate again"
            );
            assert_eq!(
                observation(&manager, ID).connection,
                if suffix == "lockout" {
                    Connection::Locked
                } else {
                    Connection::AuthFailed
                }
            );
        }
    }

    #[tokio::test]
    async fn changes_during_pre_read_are_rechecked_without_an_eviction_hook() {
        for removed in [false, true] {
            let host = format!(
                "manager-pre-read-{}",
                if removed { "remove" } else { "edit" }
            );
            let (entered_tx, entered) = oneshot::channel();
            let (release, release_rx) = oneshot::channel();
            let relay = install(
                &host,
                FakeSession::new(
                    PowerState::On,
                    [
                        FakeStep::Ok,
                        FakeStep::ReadGate {
                            entered: entered_tx,
                            release: release_rx,
                        },
                    ],
                ),
            );
            let manager = Arc::new(PlugManager::default());
            let stored = Arc::new(Mutex::new(Some("r1".to_owned())));
            let write = tokio::spawn({
                let manager = manager.clone();
                let stored = stored.clone();
                async move {
                    manager
                        .set_power(
                            &key(ID, "r1"),
                            PowerState::Off,
                            resolver(&host),
                            move || Ok(stored.lock().unwrap().clone()),
                        )
                        .await
                }
            });
            entered.await.unwrap();
            *stored.lock().unwrap() = if removed { None } else { Some("r2".into()) };
            release.send(()).unwrap();

            let result = write.await.unwrap();
            assert_eq!(*relay.lock().unwrap(), PowerState::On);
            if removed {
                assert!(matches!(result, Err(PlugError::NotFound(_))));
            } else {
                assert!(matches!(result, Err(PlugError::ProfileChanged)));
            }
        }
    }
}
