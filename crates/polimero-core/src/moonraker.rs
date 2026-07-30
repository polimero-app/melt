//! Moonraker's HTTP status API, independent of profile storage and UI code.

use std::{collections::BTreeMap, io::Read, time::Duration};

use reqwest::{
    StatusCode,
    blocking::{Client as HttpClient, ClientBuilder},
    header::{ACCEPT, HeaderValue},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;
use url::Url;

pub const DEFAULT_PORT: u16 = 7125;
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_JSON_RESPONSE_BYTES: u64 = 8 << 20;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile {
    base_url: Url,
    insecure: bool,
    timeout: Duration,
}

impl Profile {
    pub fn new(host: &str, insecure: bool, timeout: Duration) -> Result<Self, Error> {
        if host.is_empty() || host.trim() != host {
            return Err(Error::InvalidProfile);
        }
        if timeout.is_zero() {
            return Err(Error::InvalidTimeout);
        }

        let host = if host.contains("://") {
            host.to_owned()
        } else {
            format!("http://{host}")
        };
        let mut base_url = Url::parse(&host).map_err(|_| Error::InvalidProfile)?;
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.host_str().is_none()
            || base_url.username() != ""
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(Error::InvalidProfile);
        }
        if base_url.scheme() == "http" && base_url.port().is_none() {
            base_url
                .set_port(Some(DEFAULT_PORT))
                .map_err(|_| Error::InvalidProfile)?;
        }
        let path = base_url.path().trim_end_matches('/').to_owned();
        base_url.set_path(&path);

        Ok(Self {
            base_url,
            insecure,
            timeout,
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid Moonraker profile")]
    InvalidProfile,
    #[error("Moonraker timeout must be greater than zero")]
    InvalidTimeout,
    #[error("cannot build Moonraker HTTP client")]
    Client(#[source] reqwest::Error),
    #[error("invalid Moonraker API key")]
    InvalidAccessCode,
    #[error("Moonraker authentication rejected")]
    Authentication,
    #[error("Moonraker request failed with status {0}")]
    HttpStatus(StatusCode),
    #[error("Moonraker transport failed")]
    Transport(#[source] reqwest::Error),
    #[error("Moonraker response is too large")]
    ResponseTooLarge,
    #[error("invalid response from Moonraker")]
    InvalidResponse,
    #[error("Moonraker response missing result")]
    MissingResult,
    #[error("Moonraker API returned an error")]
    Api,
}

#[derive(Debug)]
pub struct Client {
    profile: Profile,
    http: HttpClient,
}

impl Client {
    pub fn new(profile: Profile) -> Result<Self, Error> {
        let http = ClientBuilder::new()
            .timeout(profile.timeout)
            .danger_accept_invalid_certs(profile.insecure)
            .redirect(Policy::none())
            .build()
            .map_err(Error::Client)?;
        Ok(Self { profile, http })
    }

    pub fn status(&self, access_code: Option<&str>) -> Result<Status, Error> {
        let url = self.status_url();
        let mut request = self.http.get(url).header(ACCEPT, "application/json");
        if let Some(access_code) = access_code.filter(|value| !value.is_empty()) {
            let value = HeaderValue::from_str(access_code).map_err(|_| Error::InvalidAccessCode)?;
            request = request.header("X-Api-Key", value);
        }

        let response = request.send().map_err(Error::Transport)?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => return Err(Error::Authentication),
            status if !status.is_success() => return Err(Error::HttpStatus(status)),
            _ => {}
        }

        let body = read_response(response)?;
        let envelope: Envelope =
            serde_json::from_slice(&body).map_err(|_| Error::InvalidResponse)?;
        if envelope.error.as_ref().is_some_and(|error| {
            error.message.to_ascii_lowercase().contains("unauthorized")
                || error.message.to_ascii_lowercase().contains("forbidden")
        }) {
            return Err(Error::Authentication);
        }
        if envelope.error.is_some() {
            return Err(Error::Api);
        }
        let result = envelope.result.ok_or(Error::MissingResult)?;
        let payload: StatusPayload =
            serde_json::from_value(result).map_err(|_| Error::InvalidResponse)?;

        Ok(Status::from_objects(payload.status))
    }

    fn status_url(&self) -> Url {
        let mut url = self.profile.base_url.clone();
        let path = format!("{}/printer/objects/query", url.path().trim_end_matches('/'));
        url.set_path(&path);
        url.query_pairs_mut()
            .append_pair("webhooks", "")
            .append_pair("print_stats", "")
            .append_pair("virtual_sdcard", "")
            .append_pair("extruder", "")
            .append_pair("heater_bed", "")
            .append_pair("fan", "");
        url
    }
}

fn read_response(response: reqwest::blocking::Response) -> Result<Vec<u8>, Error> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_JSON_RESPONSE_BYTES)
    {
        return Err(Error::ResponseTooLarge);
    }
    let mut body = Vec::new();
    response
        .take(MAX_JSON_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| Error::InvalidResponse)?;
    if body.len() as u64 > MAX_JSON_RESPONSE_BYTES {
        return Err(Error::ResponseTooLarge);
    }
    Ok(body)
}

#[derive(Deserialize)]
struct Envelope {
    result: Option<Value>,
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct StatusPayload {
    status: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrinterState {
    Idle,
    Printing,
    Paused,
    Error,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Temperature {
    pub current_celsius: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_celsius: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Temperatures {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nozzle: Option<Temperature>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bed: Option<Temperature>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Job {
    pub name: String,
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub percent: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_layer: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_layers: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusWarning {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: PrinterState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperatures: Option<Temperatures>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<Job>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    pub errors: Vec<StatusError>,
    pub warnings: Vec<StatusWarning>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fans: BTreeMap<String, u8>,
}

impl Status {
    fn from_objects(status: BTreeMap<String, Value>) -> Self {
        let state = map_state(&status);
        let (temperatures, mut warnings) = map_temperatures(&status);
        let (progress, progress_warning) = map_progress(&status);
        if let Some(warning) = progress_warning {
            warnings.push(warning);
        }
        let active = matches!(state, PrinterState::Printing | PrinterState::Paused);

        Self {
            state,
            temperatures,
            job: active.then(|| map_job(&status)).flatten(),
            progress: active.then_some(progress).flatten(),
            errors: map_errors(&status, state),
            warnings,
            fans: map_fans(&status),
        }
    }
}

fn map_state(status: &BTreeMap<String, Value>) -> PrinterState {
    if object(status, "webhooks")
        .and_then(|object| string(object, "state"))
        .is_some_and(|state| matches!(state.to_ascii_lowercase().as_str(), "shutdown" | "error"))
    {
        return PrinterState::Error;
    }

    match object(status, "print_stats")
        .and_then(|object| string(object, "state"))
        .map(|state| state.to_ascii_lowercase())
        .as_deref()
    {
        Some("printing") => PrinterState::Printing,
        Some("paused") => PrinterState::Paused,
        Some("error" | "shutdown") => PrinterState::Error,
        Some("complete" | "standby" | "ready" | "cancelled" | "canceled") => PrinterState::Idle,
        _ => PrinterState::Unknown,
    }
}

fn map_temperatures(
    status: &BTreeMap<String, Value>,
) -> (Option<Temperatures>, Vec<StatusWarning>) {
    let (nozzle, nozzle_incomplete) = heater(status, "extruder");
    let (bed, bed_incomplete) = heater(status, "heater_bed");
    if nozzle.is_none() && bed.is_none() {
        return (
            None,
            vec![StatusWarning {
                code: "temperature-data-unavailable",
                message: "temperature data unavailable",
            }],
        );
    }

    let mut warnings = Vec::new();
    if nozzle_incomplete {
        warnings.push(StatusWarning {
            code: "temperature-data-unavailable",
            message: "nozzle temperature reading unavailable",
        });
    }
    if bed_incomplete {
        warnings.push(StatusWarning {
            code: "temperature-data-unavailable",
            message: "bed temperature reading unavailable",
        });
    }
    (Some(Temperatures { nozzle, bed }), warnings)
}

fn heater(status: &BTreeMap<String, Value>, name: &str) -> (Option<Temperature>, bool) {
    let Some(object) = object(status, name) else {
        return (None, false);
    };
    let current = number(object, "temperature");
    let target = number(object, "target");
    (
        current.map(|current_celsius| Temperature {
            current_celsius,
            target_celsius: target,
        }),
        current.is_none() && target.is_some(),
    )
}

fn map_progress(status: &BTreeMap<String, Value>) -> (Option<Progress>, Option<StatusWarning>) {
    let Some(progress) =
        object(status, "virtual_sdcard").and_then(|object| number(object, "progress"))
    else {
        return (
            None,
            Some(StatusWarning {
                code: "progress-unavailable",
                message: "progress unavailable",
            }),
        );
    };
    let percent = (progress * 100.0).round().clamp(0.0, 100.0) as u8;
    let info = object(status, "print_stats")
        .and_then(|object| object.get("info"))
        .and_then(Value::as_object);
    (
        Some(Progress {
            percent,
            current_layer: info.and_then(|info| unsigned(info, "current_layer")),
            total_layers: info.and_then(|info| unsigned(info, "total_layer")),
        }),
        None,
    )
}

fn map_job(status: &BTreeMap<String, Value>) -> Option<Job> {
    let name = object(status, "print_stats")?
        .get("filename")?
        .as_str()?
        .to_owned();
    (!name.is_empty()).then(|| Job {
        id: synthetic_job_id(&name),
        name,
    })
}

fn map_errors(status: &BTreeMap<String, Value>, state: PrinterState) -> Vec<StatusError> {
    if state != PrinterState::Error {
        return Vec::new();
    }
    let message = object(status, "webhooks")
        .filter(|object| {
            string(object, "state").is_some_and(|state| {
                matches!(state.to_ascii_lowercase().as_str(), "shutdown" | "error")
            })
        })
        .and_then(|object| string(object, "state_message"))
        .and_then(first_line)
        .or_else(|| {
            object(status, "print_stats")
                .and_then(|object| string(object, "message"))
                .and_then(first_line)
        })
        .unwrap_or_else(|| "printer reported an error state".to_owned());
    vec![StatusError {
        code: "printer-error",
        message,
    }]
}

fn map_fans(status: &BTreeMap<String, Value>) -> BTreeMap<String, u8> {
    let Some(speed) = object(status, "fan").and_then(|object| number(object, "speed")) else {
        return BTreeMap::new();
    };
    if !(0.0..=1.0).contains(&speed) {
        return BTreeMap::new();
    }
    BTreeMap::from([("partCooling".to_owned(), (speed * 100.0).round() as u8)])
}

fn object<'a>(status: &'a BTreeMap<String, Value>, name: &str) -> Option<&'a Map<String, Value>> {
    status.get(name)?.as_object()
}

fn string(object: &Map<String, Value>, name: &str) -> Option<String> {
    object.get(name)?.as_str().map(str::to_owned)
}

fn number(object: &Map<String, Value>, name: &str) -> Option<f64> {
    let value = object.get(name)?;
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn unsigned(object: &Map<String, Value>, name: &str) -> Option<u32> {
    number(object, name).and_then(|value| (value >= 0.0).then_some(value as u32))
}

fn first_line(message: String) -> Option<String> {
    let message = message.lines().next()?.trim();
    (!message.is_empty()).then(|| message.to_owned())
}

fn synthetic_job_id(name: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("klipper-{hash:x}")
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    use super::*;

    #[test]
    fn profile_validation_normalizes_the_moonraker_base_url() {
        let profile = Profile::new("http://printer.local/api/", false, DEFAULT_TIMEOUT).unwrap();
        assert_eq!(profile.base_url().as_str(), "http://printer.local:7125/api");

        for host in [
            "",
            "ftp://printer.local",
            "https://user@printer.local",
            "printer.local?x=1",
        ] {
            assert!(
                Profile::new(host, false, DEFAULT_TIMEOUT).is_err(),
                "{host}"
            );
        }
    }

    #[test]
    fn status_queries_moonraker_with_the_api_key_and_maps_typed_fields() {
        let (host, request, server) = server(
            r#"{"result":{"status":{"print_stats":{"state":"printing","filename":"cube.gcode","info":{"current_layer":42,"total_layer":120}},"virtual_sdcard":{"progress":0.605},"extruder":{"temperature":212.5,"target":220},"heater_bed":{"temperature":59.5,"target":60},"fan":{"speed":0.605}}}}"#,
        );
        let client = Client::new(Profile::new(&host, false, DEFAULT_TIMEOUT).unwrap()).unwrap();

        let status = client.status(Some("secret-key")).unwrap();

        let request = request.recv().unwrap();
        server.join().unwrap();
        assert!(request.starts_with("GET /printer/objects/query?"));
        assert!(request.contains("webhooks=&"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-api-key: secret-key\r\n")
        );
        assert_eq!(status.state, PrinterState::Printing);
        assert_eq!(status.job.unwrap().name, "cube.gcode");
        assert_eq!(status.progress.unwrap().percent, 61);
        assert_eq!(
            status.temperatures.unwrap().nozzle.unwrap().current_celsius,
            212.5
        );
        assert_eq!(status.fans["partCooling"], 61);
    }

    #[test]
    fn status_reports_missing_data_and_hides_a_terminal_job() {
        let (host, _request, server) = server(
            r#"{"result":{"status":{"print_stats":{"state":"complete","filename":"cube.gcode"},"virtual_sdcard":{"progress":1},"extruder":{"target":210}}}}"#,
        );
        let client = Client::new(Profile::new(&host, false, DEFAULT_TIMEOUT).unwrap()).unwrap();

        let status = client.status(None).unwrap();

        server.join().unwrap();
        assert_eq!(status.state, PrinterState::Idle);
        assert!(status.job.is_none() && status.progress.is_none());
        assert!(status.temperatures.is_none());
        assert_eq!(
            status.warnings,
            vec![StatusWarning {
                code: "temperature-data-unavailable",
                message: "temperature data unavailable",
            },]
        );
    }

    #[test]
    fn status_reports_the_klipper_error_reason() {
        let (host, _request, server) = server(
            r#"{"result":{"status":{"webhooks":{"state":"shutdown","state_message":" "},"print_stats":{"state":"error","message":"Move out of range\nMore detail"}}}}"#,
        );
        let client = Client::new(Profile::new(&host, false, DEFAULT_TIMEOUT).unwrap()).unwrap();

        let status = client.status(None).unwrap();

        server.join().unwrap();
        assert_eq!(
            status.errors,
            vec![StatusError {
                code: "printer-error",
                message: "Move out of range".into(),
            }]
        );
    }

    fn server(
        body: &'static str,
    ) -> (
        String,
        std::sync::mpsc::Receiver<String>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let (sender, request) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                request.extend_from_slice(&chunk[..read]);
            }
            sender.send(String::from_utf8(request).unwrap()).unwrap();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        (host, request, server)
    }
}
