//! Per-printer connection reuse for status polling.
//!
//! `drivers::status` (a one-shot call, still used for non-polling callers)
//! always opens a fresh connection. The status poller instead routes through
//! [`ConnectionPool`], which keeps one connection per printer name alive
//! across polls: a cached [`moonraker::Client`] for Moonraker (its
//! `reqwest::blocking::Client` pools its own TCP connections), and a cached,
//! live MQTT session for Bambu (see [`bambu::Client::poll_status`]).
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{bambu, drivers, moonraker};

struct BambuSession {
    identity: [u8; 32],
    client: bambu::Client,
    connection: Option<bambu::PersistentConnection>,
}

struct MoonrakerSession {
    identity: String,
    client: Arc<moonraker::Client>,
}

#[derive(Default)]
pub struct ConnectionPool {
    moonraker: Mutex<HashMap<String, MoonrakerSession>>,
    bambu: Mutex<HashMap<String, Arc<Mutex<BambuSession>>>>,
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
                let identity = profile.connection_identity();
                let client = {
                    let mut clients =
                        self.moonraker.lock().unwrap_or_else(|error| error.into_inner());
                    let replace = clients
                        .get(name)
                        .is_none_or(|session| session.identity != identity);
                    if replace {
                        let client = Arc::new(
                            moonraker::Client::new(profile.clone())
                                .map_err(drivers::DriverError::Moonraker)?,
                        );
                        clients.insert(
                            name.to_string(),
                            MoonrakerSession {
                                identity,
                                client,
                            },
                        );
                    }
                    clients[name].client.clone()
                };
                client.status(access_code).map_err(drivers::DriverError::Moonraker)
            }
            drivers::Profile::Bambu(profile) => {
                let identity = profile.connection_identity(access_code, tls_fingerprint);
                let session = {
                    let mut sessions =
                        self.bambu.lock().unwrap_or_else(|error| error.into_inner());
                    let replace = sessions.get(name).is_none_or(|session| {
                        session
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .identity
                            != identity
                    });
                    if replace {
                        sessions.insert(
                            name.to_string(),
                            Arc::new(Mutex::new(BambuSession {
                                identity,
                                client: bambu::Client::new(profile.clone()),
                                connection: None,
                            })),
                        );
                    }
                    sessions[name].clone()
                };
                let session = &mut *session.lock().unwrap_or_else(|error| error.into_inner());
                session
                    .client
                    .poll_status(&mut session.connection, access_code, tls_fingerprint)
                    .map_err(drivers::DriverError::Bambu)
            }
        }
    }

    /// Drops pooled connections for printers no longer present in the config.
    pub fn retain(&self, keep: impl Fn(&str) -> bool) {
        if let Ok(mut clients) = self.moonraker.lock() {
            clients.retain(|name, _| keep(name));
        }
        if let Ok(mut sessions) = self.bambu.lock() {
            sessions.retain(|name, _| keep(name));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
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

    fn status_server(state: &'static str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).unwrap();
            let body = format!(
                r#"{{"result":{{"status":{{"print_stats":{{"state":"{state}"}}}}}}}}"#
            );
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
