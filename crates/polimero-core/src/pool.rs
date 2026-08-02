//! Per-printer connection reuse for status polling and command execution.
//!
//! One-shot driver calls always open a fresh connection. Long-lived callers
//! instead route through [`ConnectionPool`], which keeps one connection per
//! printer name alive: a cached [`moonraker::Client`] for Moonraker (its
//! `reqwest::blocking::Client` pools TCP connections), and one serialized,
//! authenticated MQTT session for Bambu status and commands.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{bambu, drivers, moonraker};

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
}

impl ConnectionPool {
    pub fn status(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Result<moonraker::Status, drivers::DriverError> {
        match profile {
            drivers::Profile::Moonraker(profile) => {
                let client = self.moonraker_client(name, profile)?;
                client
                    .status_with_timeout(
                        access_code,
                        profile.timeout().min(drivers::STATUS_POLL_TIMEOUT),
                    )
                    .map_err(drivers::DriverError::Moonraker)
            }
            drivers::Profile::Bambu(profile) => self
                .bambu_client(name, profile, access_code, tls_fingerprint)
                .poll_status(
                    access_code,
                    tls_fingerprint,
                    profile.timeout().min(drivers::STATUS_POLL_TIMEOUT),
                )
                .map_err(drivers::DriverError::Bambu),
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
        let name = canonical_name(name);
        let identity = profile.connection_identity();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(PooledSession::Moonraker {
            identity: current,
            client,
        }) = sessions.get(&name)
            && current == &identity
        {
            return Ok(client.clone());
        }
        let client = Arc::new(
            moonraker::Client::new(profile.clone()).map_err(drivers::DriverError::Moonraker)?,
        );
        sessions.insert(
            name,
            PooledSession::Moonraker {
                identity,
                client: client.clone(),
            },
        );
        Ok(client)
    }

    fn bambu_client(
        &self,
        name: &str,
        profile: &bambu::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
    ) -> Arc<bambu::Client> {
        let name = canonical_name(name);
        let identity = profile.connection_identity(access_code, tls_fingerprint);
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(PooledSession::Bambu {
            identity: current,
            client,
        }) = sessions.get(&name)
            && current == &identity
        {
            return client.clone();
        }
        let client = Arc::new(bambu::Client::new(profile.clone()));
        sessions.insert(
            name,
            PooledSession::Bambu {
                identity,
                client: client.clone(),
            },
        );
        client
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
        sessions.retain(|name, _| keep(name));
    }
}

fn canonical_name(name: &str) -> String {
    name.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    use super::*;

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
