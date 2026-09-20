//! Per-printer connection reuse for status polling and command execution.
//!
//! One-shot driver calls always open a fresh connection. Long-lived callers
//! instead route through [`ConnectionPool`], which keeps one connection per
//! printer name alive: a cached [`moonraker::Client`] for Moonraker (its
//! `reqwest::blocking::Client` pools TCP connections), and one serialized,
//! authenticated MQTT session for Bambu status and commands.
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use crate::{bambu, drivers, firmware_updates::FirmwareUpdateReport, moonraker};

const BAMBU_SERVICE_INTERVAL: Duration = Duration::from_millis(100);
const BAMBU_SERVICE_SLICE: Duration = Duration::from_millis(25);

enum PooledSession {
    Moonraker {
        identity: String,
        client: Arc<moonraker::Client>,
    },
    Bambu {
        identity: [u8; 32],
        client: Arc<bambu::Client>,
    },
}

#[derive(Default)]
pub struct ConnectionPool {
    sessions: Mutex<HashMap<String, PooledSession>>,
    lifecycle: AtomicU64,
    lifecycle_gate: RwLock<()>,
}

impl ConnectionPool {
    pub fn lifecycle_generation(&self) -> u64 {
        self.lifecycle.load(Ordering::Acquire)
    }

    pub fn is_current_generation(&self, generation: u64) -> bool {
        self.lifecycle_generation() == generation
    }

    /// Runs an operation only while its configuration generation remains
    /// current. Lifecycle mutations wait for an admitted operation to finish,
    /// preventing a removed or refreshed profile from being reinserted later.
    pub fn with_current_generation<T>(
        &self,
        generation: u64,
        operation: impl FnOnce(&Self) -> T,
    ) -> Option<T> {
        let _gate = self
            .lifecycle_gate
            .read()
            .unwrap_or_else(|error| error.into_inner());
        if !self.is_current_generation(generation) {
            return None;
        }
        Some(operation(self))
    }

    pub fn status(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Result<moonraker::Status, drivers::DriverError> {
        self.status_at(None, name, profile, access_code, tls_fingerprint)
            .expect("an unconstrained pool operation is always current")
    }

    pub fn status_if_current(
        &self,
        generation: u64,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Option<Result<moonraker::Status, drivers::DriverError>> {
        let result = self.status_at(
            Some(generation),
            name,
            profile,
            access_code,
            tls_fingerprint,
        )?;
        self.is_current_generation(generation).then_some(result)
    }

    pub fn bambu_runtime_capabilities(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Result<Option<bambu::RuntimeCapabilities>, drivers::DriverError> {
        let drivers::Profile::Bambu(profile) = profile else {
            return Ok(None);
        };
        self.with_bambu(name, profile, access_code, tls_fingerprint, |client| {
            client.runtime_capabilities(access_code, tls_fingerprint)
        })
        .map(Some)
    }

    pub fn firmware_update_status(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        refresh: bool,
    ) -> Result<FirmwareUpdateReport, drivers::DriverError> {
        self.firmware_update_status_with_sources(
            name,
            profile,
            access_code,
            tls_fingerprint,
            refresh,
            false,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn firmware_update_status_with_sources(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        refresh: bool,
        include_history: bool,
        include_public_catalogue: bool,
    ) -> Result<FirmwareUpdateReport, drivers::DriverError> {
        match profile {
            drivers::Profile::Moonraker(profile) => self
                .moonraker_client(name, profile)?
                .firmware_update_status(access_code, refresh)
                .map_err(drivers::DriverError::Moonraker),
            drivers::Profile::Bambu(profile) => {
                self.with_bambu(name, profile, access_code, tls_fingerprint, |client| {
                    client.firmware_update_status_with_sources(
                        access_code,
                        tls_fingerprint,
                        refresh,
                        include_history,
                        include_public_catalogue,
                    )
                })
            }
        }
    }

    fn status_at(
        &self,
        generation: Option<u64>,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Option<Result<moonraker::Status, drivers::DriverError>> {
        match profile {
            drivers::Profile::Moonraker(profile) => {
                let client = match self.moonraker_client_at(generation, name, profile)? {
                    Ok(client) => client,
                    Err(error) => return Some(Err(error)),
                };
                Some(
                    client
                        .status_with_timeout(
                            access_code,
                            profile.timeout().min(drivers::STATUS_POLL_TIMEOUT),
                        )
                        .map_err(drivers::DriverError::Moonraker),
                )
            }
            drivers::Profile::Bambu(profile) => {
                let client =
                    self.bambu_client_at(generation, name, profile, access_code, tls_fingerprint)?;
                Some(
                    client
                        .poll_status(
                            access_code,
                            tls_fingerprint,
                            profile.timeout().min(drivers::STATUS_POLL_TIMEOUT),
                        )
                        .map_err(drivers::DriverError::Bambu),
                )
            }
        }
    }

    pub fn job_start(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        device_path: &str,
        options: bambu::JobStartOptions,
    ) -> Result<moonraker::JobResult, drivers::DriverError> {
        match profile {
            drivers::Profile::Moonraker(profile) => self
                .moonraker_client(name, profile)?
                .job_start(access_code, device_path)
                .map_err(drivers::DriverError::Moonraker),
            drivers::Profile::Bambu(profile) => {
                self.with_bambu(name, profile, access_code, tls_fingerprint, |client| {
                    client.job_start(access_code, tls_fingerprint, device_path, options)
                })
            }
        }
    }

    pub fn job_pause(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Result<moonraker::JobResult, drivers::DriverError> {
        self.job_control(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| client.job_pause(access_code, tls_fingerprint),
            |client| client.job_pause(access_code),
        )
    }

    pub fn job_resume(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Result<moonraker::JobResult, drivers::DriverError> {
        self.job_control(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| client.job_resume(access_code, tls_fingerprint),
            |client| client.job_resume(access_code),
        )
    }

    pub fn job_cancel(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Result<moonraker::JobResult, drivers::DriverError> {
        self.job_control(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| client.job_cancel(access_code, tls_fingerprint),
            |client| client.job_cancel(access_code),
        )
    }

    pub fn emergency_stop(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Result<(), drivers::DriverError> {
        self.execute(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| client.emergency_stop(access_code),
            |client| client.emergency_stop(access_code, tls_fingerprint),
        )
    }

    pub fn temperature_set(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        targets: moonraker::TemperatureTargets,
    ) -> Result<moonraker::TemperatureResult, drivers::DriverError> {
        let moonraker_targets = targets.clone();
        self.execute(
            name,
            profile,
            access_code,
            tls_fingerprint,
            move |client| client.temperature_set(access_code, moonraker_targets),
            move |client| client.temperature_set(access_code, tls_fingerprint, targets),
        )
    }

    pub fn temperature_set_item(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        item: &str,
        target_celsius: f64,
    ) -> Result<moonraker::TemperatureResult, drivers::DriverError> {
        self.execute(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| {
                let targets = match item {
                    "nozzle" => moonraker::TemperatureTargets {
                        nozzle_celsius: Some(target_celsius),
                        ..Default::default()
                    },
                    "bed" => moonraker::TemperatureTargets {
                        bed_celsius: Some(target_celsius),
                        ..Default::default()
                    },
                    _ => {
                        return Err(moonraker::Error::Unsupported(
                            "requested temperature control",
                        ));
                    }
                };
                client.temperature_set(access_code, targets)
            },
            |client| {
                client.temperature_set_item(access_code, tls_fingerprint, item, target_celsius)
            },
        )
    }

    pub fn fan_set(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        fan: &str,
        speed_percent: u8,
    ) -> Result<moonraker::FanResult, drivers::DriverError> {
        self.execute(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| client.fan_set(access_code, fan, speed_percent),
            |client| client.fan_set(access_code, tls_fingerprint, fan, speed_percent),
        )
    }

    pub fn motion_home(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        axes: &[moonraker::Axis],
    ) -> Result<moonraker::MotionResult, drivers::DriverError> {
        self.execute(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| client.motion_home(access_code, axes),
            |client| client.motion_home(access_code, tls_fingerprint, axes),
        )
    }

    pub fn motion_jog(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        delta: moonraker::JogDelta,
    ) -> Result<moonraker::MotionResult, drivers::DriverError> {
        self.execute(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| client.motion_jog(access_code, delta),
            |client| client.motion_jog(access_code, tls_fingerprint, delta),
        )
    }

    pub fn light_set(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        light: &str,
        state: moonraker::LightState,
    ) -> Result<moonraker::LightResult, drivers::DriverError> {
        let drivers::Profile::Bambu(profile) = profile else {
            return Err(drivers::DriverError::UnsupportedOperation(
                drivers::Driver::Moonraker,
                drivers::Operation::LightSet,
            ));
        };
        self.with_bambu(name, profile, access_code, tls_fingerprint, |client| {
            client.light_set(access_code, tls_fingerprint, light, state)
        })
    }

    pub fn ams_drying_set(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        ams_id: u32,
        request: &moonraker::DryingRequest,
    ) -> Result<moonraker::DryingResult, drivers::DriverError> {
        let drivers::Profile::Bambu(profile) = profile else {
            return Err(drivers::DriverError::UnsupportedOperation(
                drivers::Driver::Moonraker,
                drivers::Operation::AmsDrying,
            ));
        };
        self.with_bambu(name, profile, access_code, tls_fingerprint, |client| {
            client.ams_drying_set(access_code, tls_fingerprint, ams_id, request)
        })
    }

    pub fn speed_set(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        speed_profile: &str,
    ) -> Result<moonraker::SpeedResult, drivers::DriverError> {
        self.execute(
            name,
            profile,
            access_code,
            tls_fingerprint,
            |client| client.speed_set(access_code, speed_profile),
            |client| client.speed_set(access_code, tls_fingerprint, speed_profile),
        )
    }

    fn job_control(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        bambu_operation: impl FnOnce(
            &bambu::Client,
        ) -> Result<moonraker::JobResult, bambu::TransportError>,
        moonraker_operation: impl FnOnce(
            &moonraker::Client,
        ) -> Result<moonraker::JobResult, moonraker::Error>,
    ) -> Result<moonraker::JobResult, drivers::DriverError> {
        self.execute(
            name,
            profile,
            access_code,
            tls_fingerprint,
            moonraker_operation,
            bambu_operation,
        )
    }

    fn execute<T>(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        moonraker_operation: impl FnOnce(&moonraker::Client) -> Result<T, moonraker::Error>,
        bambu_operation: impl FnOnce(&bambu::Client) -> Result<T, bambu::TransportError>,
    ) -> Result<T, drivers::DriverError> {
        match profile {
            drivers::Profile::Moonraker(profile) => {
                let client = self.moonraker_client(name, profile)?;
                moonraker_operation(&client).map_err(drivers::DriverError::Moonraker)
            }
            drivers::Profile::Bambu(profile) => {
                self.with_bambu(name, profile, access_code, tls_fingerprint, bambu_operation)
            }
        }
    }

    fn moonraker_client(
        &self,
        name: &str,
        profile: &moonraker::Profile,
    ) -> Result<Arc<moonraker::Client>, drivers::DriverError> {
        self.moonraker_client_at(None, name, profile)
            .expect("an unconstrained pool operation is always current")
    }

    fn moonraker_client_at(
        &self,
        generation: Option<u64>,
        name: &str,
        profile: &moonraker::Profile,
    ) -> Option<Result<Arc<moonraker::Client>, drivers::DriverError>> {
        let name = canonical_name(name);
        let identity = profile.connection_identity();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if generation.is_some_and(|generation| generation != self.lifecycle_generation()) {
            return None;
        }
        if let Some(PooledSession::Moonraker {
            identity: current,
            client,
        }) = sessions.get(&name)
            && current == &identity
        {
            return Some(Ok(client.clone()));
        }
        let client = match moonraker::Client::new(profile.clone()) {
            Ok(client) => Arc::new(client),
            Err(error) => return Some(Err(drivers::DriverError::Moonraker(error))),
        };
        let replaced = sessions.insert(
            name,
            PooledSession::Moonraker {
                identity,
                client: client.clone(),
            },
        );
        drop(sessions);
        drop(replaced);
        Some(Ok(client))
    }

    fn bambu_client(
        &self,
        name: &str,
        profile: &bambu::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Arc<bambu::Client> {
        self.bambu_client_at(None, name, profile, access_code, tls_fingerprint)
            .expect("an unconstrained pool operation is always current")
    }

    fn bambu_client_at(
        &self,
        generation: Option<u64>,
        name: &str,
        profile: &bambu::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Option<Arc<bambu::Client>> {
        let name = canonical_name(name);
        let identity = profile.connection_identity(access_code, tls_fingerprint);
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if generation.is_some_and(|generation| generation != self.lifecycle_generation()) {
            return None;
        }
        if let Some(PooledSession::Bambu {
            identity: current,
            client,
        }) = sessions.get(&name)
            && current == &identity
        {
            return Some(client.clone());
        }
        let client = Arc::new(bambu::Client::new(profile.clone()));
        spawn_bambu_worker(Arc::downgrade(&client));
        let replaced = sessions.insert(
            name,
            PooledSession::Bambu {
                identity,
                client: client.clone(),
            },
        );
        drop(sessions);
        drop(replaced);
        Some(client)
    }

    fn with_bambu<T>(
        &self,
        name: &str,
        profile: &bambu::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        operation: impl FnOnce(&bambu::Client) -> Result<T, bambu::TransportError>,
    ) -> Result<T, drivers::DriverError> {
        let client = self.bambu_client(name, profile, access_code, tls_fingerprint);
        operation(&client).map_err(drivers::DriverError::Bambu)
    }

    /// Drops pooled connections for printers no longer present in the config.
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let removed = sessions
            .keys()
            .filter(|name| !keep(name))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|name| sessions.remove(&name))
            .collect::<Vec<_>>();
        drop(sessions);
        drop(removed);
    }

    /// Immediately drops the pooled connection for one configured printer.
    pub fn remove(&self, name: &str) {
        let _gate = self
            .lifecycle_gate
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.lifecycle.fetch_add(1, Ordering::AcqRel);
        let removed = sessions.remove(&canonical_name(name));
        drop(sessions);
        drop(removed);
    }
}

fn spawn_bambu_worker(client: Weak<bambu::Client>) {
    thread::spawn(move || {
        loop {
            thread::sleep(BAMBU_SERVICE_INTERVAL);
            let Some(client) = client.upgrade() else {
                break;
            };
            client.service_cached_status(BAMBU_SERVICE_SLICE);
            drop(client);
        }
    });
}

fn canonical_name(name: &str) -> String {
    name.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingTracer(std::sync::Mutex<Vec<crate::trace::TraceEvent>>);

    impl crate::trace::ProtocolTracer for RecordingTracer {
        fn record(&self, event: crate::trace::TraceEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn moonraker_profile_edits_replace_the_cached_client() {
        let (first_host, first_server) = status_server("standby");
        let (second_host, second_server) = status_server("printing");
        let pool = ConnectionPool::default();
        let first = drivers::Profile::Moonraker(
            moonraker::Profile::new(&first_host, false, Duration::from_secs(2)).unwrap(),
        );
        let second = drivers::Profile::Moonraker(
            moonraker::Profile::new(&second_host, false, Duration::from_secs(2)).unwrap(),
        );

        let initial = pool.status("printer", &first, None, None).unwrap();
        let edited = pool.status("printer", &second, None, None).unwrap();

        first_server.join().unwrap();
        second_server.join().unwrap();
        assert_eq!(initial.state, moonraker::PrinterState::Idle);
        assert_eq!(edited.state, moonraker::PrinterState::Printing);
    }

    #[test]
    fn moonraker_commands_reuse_the_cached_http_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            for _ in 0..2 {
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                let body = r#"{"result":{}}"#;
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                stream.flush().unwrap();
            }
        });
        let profile = drivers::Profile::Moonraker(
            moonraker::Profile::new(&format!("http://{address}"), false, Duration::from_secs(2))
                .unwrap(),
        );
        let pool = ConnectionPool::default();

        pool.emergency_stop("printer", &profile, None, None)
            .unwrap();
        pool.emergency_stop("printer", &profile, None, None)
            .unwrap();

        server.join().unwrap();
    }

    #[test]
    fn bambu_credentials_replace_and_retain_evicts_pooled_sessions() {
        let profile = bambu::Profile::new("printer.local", "SN001", true).unwrap();
        let pool = ConnectionPool::default();
        let first = pool.bambu_client("printer", &profile, Some("old-code"), None);
        let reused = pool.bambu_client("PRINTER", &profile, Some("old-code"), None);
        let replaced = pool.bambu_client("printer", &profile, Some("new-code"), None);

        assert!(Arc::ptr_eq(&first, &reused));
        assert!(!Arc::ptr_eq(&first, &replaced));
        pool.retain(|_| false);
        assert!(pool.sessions.lock().unwrap().is_empty());
    }

    #[test]
    fn moonraker_names_are_canonicalized_for_connection_reuse() {
        let profile =
            moonraker::Profile::new("http://printer.local", false, Duration::from_secs(2)).unwrap();
        let pool = ConnectionPool::default();

        let first = pool.moonraker_client("Printer", &profile).unwrap();
        let reused = pool.moonraker_client("printer", &profile).unwrap();

        assert!(Arc::ptr_eq(&first, &reused));
        assert_eq!(pool.sessions.lock().unwrap().len(), 1);
    }

    #[test]
    fn attaching_a_tracer_replaces_then_reuses_the_moonraker_client() {
        let (host, server) = status_server("standby");
        let profile = moonraker::Profile::new(&host, false, Duration::from_secs(2)).unwrap();
        let tracer = Arc::new(RecordingTracer::default());
        let pool = ConnectionPool::default();
        let untraced = pool.moonraker_client("printer", &profile).unwrap();
        let traced = profile.with_tracer(tracer.clone());

        let replacement = pool.moonraker_client("printer", &traced).unwrap();
        let reused = pool.moonraker_client("printer", &traced).unwrap();
        assert!(!Arc::ptr_eq(&untraced, &replacement));
        assert!(Arc::ptr_eq(&replacement, &reused));

        let status = pool
            .status("printer", &drivers::Profile::Moonraker(traced), None, None)
            .unwrap();
        server.join().unwrap();
        assert_eq!(status.state, moonraker::PrinterState::Idle);
        assert_eq!(tracer.0.lock().unwrap().len(), 2);
    }

    #[test]
    fn attaching_a_tracer_replaces_then_reuses_the_bambu_client() {
        let profile = bambu::Profile::new("printer.local", "SN001", true).unwrap();
        let tracer = Arc::new(RecordingTracer::default());
        let pool = ConnectionPool::default();
        let untraced = pool.bambu_client("printer", &profile, Some("access-code"), None);
        let traced = profile.with_tracer(tracer);

        let replacement = pool.bambu_client("printer", &traced, Some("access-code"), None);
        let reused = pool.bambu_client("printer", &traced, Some("access-code"), None);

        assert!(!Arc::ptr_eq(&untraced, &replacement));
        assert!(Arc::ptr_eq(&replacement, &reused));
    }

    #[test]
    fn changing_drivers_replaces_the_previous_pooled_client() {
        let moonraker =
            moonraker::Profile::new("http://printer.local", false, Duration::from_secs(2)).unwrap();
        let bambu = bambu::Profile::new("printer.local", "SN001", true).unwrap();
        let pool = ConnectionPool::default();

        let moonraker_client = pool.moonraker_client("printer", &moonraker).unwrap();
        let old_moonraker = Arc::downgrade(&moonraker_client);
        drop(moonraker_client);
        let bambu_client = pool.bambu_client("printer", &bambu, Some("access-code"), None);
        assert!(old_moonraker.upgrade().is_none());

        let old_bambu = Arc::downgrade(&bambu_client);
        drop(bambu_client);
        let _moonraker_client = pool.moonraker_client("printer", &moonraker).unwrap();
        assert!(old_bambu.upgrade().is_none());
        assert_eq!(pool.sessions.lock().unwrap().len(), 1);
    }

    #[test]
    fn remove_canonicalizes_names_and_releases_the_pooled_client() {
        let profile =
            moonraker::Profile::new("http://printer.local", false, Duration::from_secs(2)).unwrap();
        let pool = ConnectionPool::default();
        let client = pool.moonraker_client("Workshop", &profile).unwrap();
        let weak = Arc::downgrade(&client);
        drop(client);

        pool.remove("WORKSHOP");

        assert!(pool.sessions.lock().unwrap().is_empty());
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn removed_generation_cannot_reinsert_an_old_monitor_session() {
        let profile = bambu::Profile::new("printer.local", "SN001", true).unwrap();
        let pool = ConnectionPool::default();
        let old_generation = pool.lifecycle_generation();

        pool.remove("printer");

        assert!(
            pool.bambu_client_at(
                Some(old_generation),
                "printer",
                &profile,
                Some("access-code"),
                None,
            )
            .is_none()
        );
        assert!(pool.sessions.lock().unwrap().is_empty());
        assert!(
            pool.bambu_client_at(
                Some(pool.lifecycle_generation()),
                "printer",
                &profile,
                Some("access-code"),
                None,
            )
            .is_some()
        );
    }

    #[test]
    fn lifecycle_mutation_waits_for_an_admitted_command_lease() {
        let pool = Arc::new(ConnectionPool::default());
        let generation = pool.lifecycle_generation();
        let leased_pool = pool.clone();
        let (admitted_tx, admitted_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let command = thread::spawn(move || {
            leased_pool
                .with_current_generation(generation, |_| {
                    admitted_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                })
                .unwrap();
        });
        admitted_rx.recv().unwrap();
        let removing_pool = pool.clone();
        let (removed_tx, removed_rx) = mpsc::channel();
        let removal = thread::spawn(move || {
            removing_pool.remove("printer");
            removed_tx.send(()).unwrap();
        });

        assert!(
            removed_rx.recv_timeout(Duration::from_millis(20)).is_err(),
            "removal must wait until the admitted command releases its lease"
        );
        release_tx.send(()).unwrap();
        removed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        command.join().unwrap();
        removal.join().unwrap();
        assert!(
            pool.with_current_generation(generation, |_| ()).is_none(),
            "the old generation cannot admit a command after removal"
        );
    }

    #[test]
    fn status_invalidated_while_in_flight_returns_none() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::channel();
        let (respond_tx, respond_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..read]);
            }
            request_tx.send(()).unwrap();
            respond_rx.recv().unwrap();
            let body = r#"{"result":{"status":{"print_stats":{"state":"standby"}}}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let profile = drivers::Profile::Moonraker(
            moonraker::Profile::new(&format!("http://{address}"), false, Duration::from_secs(2))
                .unwrap(),
        );
        let pool = Arc::new(ConnectionPool::default());
        let generation = pool.lifecycle_generation();
        let polling_pool = pool.clone();
        let poll = thread::spawn(move || {
            polling_pool.status_if_current(generation, "printer", &profile, None, None)
        });
        request_rx.recv().unwrap();

        pool.remove("printer");
        respond_tx.send(()).unwrap();

        assert!(poll.join().unwrap().is_none());
        server.join().unwrap();
    }

    #[test]
    fn retain_evicts_sessions_after_the_pool_lock_is_poisoned() {
        let profile =
            moonraker::Profile::new("http://printer.local", false, Duration::from_secs(2)).unwrap();
        let pool = ConnectionPool::default();
        let _client = pool.moonraker_client("printer", &profile).unwrap();
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _sessions = pool.sessions.lock().unwrap();
            panic!("poison the connection pool for the regression test");
        }));

        pool.retain(|_| false);

        assert!(
            pool.sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty()
        );
    }

    fn status_server(state: &'static str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).unwrap();
            let body =
                format!(r#"{{"result":{{"status":{{"print_stats":{{"state":"{state}"}}}}}}}}"#);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (format!("http://{address}"), server)
    }
}
