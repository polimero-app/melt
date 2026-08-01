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
    client: bambu::Client,
    connection: Option<bambu::PersistentConnection>,
}

#[derive(Default)]
pub struct ConnectionPool {
    moonraker: Mutex<HashMap<String, Arc<moonraker::Client>>>,
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
                // ponytail: pool entries aren't invalidated on a profile edit
                // (host/timeout/insecure) while the app is running; a changed
                // printer picks up new settings on its next reconnect after
                // an error, or after an app restart. Revisit if that proves
                // surprising in practice.
                let client = {
                    let mut clients =
                        self.moonraker.lock().unwrap_or_else(|error| error.into_inner());
                    match clients.get(name) {
                        Some(client) => client.clone(),
                        None => {
                            let client = Arc::new(
                                moonraker::Client::new(profile.clone())
                                    .map_err(drivers::DriverError::Moonraker)?,
                            );
                            clients.insert(name.to_string(), client.clone());
                            client
                        }
                    }
                };
                client.status(access_code).map_err(drivers::DriverError::Moonraker)
            }
            drivers::Profile::Bambu(profile) => {
                let session = {
                    let mut sessions =
                        self.bambu.lock().unwrap_or_else(|error| error.into_inner());
                    sessions
                        .entry(name.to_string())
                        .or_insert_with(|| {
                            Arc::new(Mutex::new(BambuSession {
                                client: bambu::Client::new(profile.clone()),
                                connection: None,
                            }))
                        })
                        .clone()
                };
                let session = &mut *session.lock().unwrap_or_else(|error| error.into_inner());
                // Cheap (no I/O): keeps access-code/timeout/insecure current
                // even though the live `connection`, if any, stays open.
                session.client = bambu::Client::new(profile.clone());
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
