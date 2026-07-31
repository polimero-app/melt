//! Moonraker's HTTP status API, independent of profile storage and UI code.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use reqwest::{
    Method, StatusCode,
    blocking::{Client as HttpClient, ClientBuilder},
    header::{ACCEPT, HeaderValue},
    redirect::Policy,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
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
    #[error("invalid device path")]
    InvalidDevicePath,
    #[error("Moonraker response is missing a requested printer object")]
    MissingObject,
    #[error("Moonraker returned an unexpected state")]
    UnexpectedState,
    #[error("Moonraker operation timed out")]
    Timeout,
    #[error("Moonraker does not support {0}")]
    Unsupported(&'static str),
    #[error("invalid temperature target")]
    InvalidTemperatureTarget,
    #[error("invalid relative motion request")]
    InvalidJog,
    #[error("unknown speed profile")]
    InvalidSpeedProfile,
    #[error("destination file already exists")]
    FileAlreadyExists,
    #[error("upload destination is a directory")]
    DirectoryDestination,
    #[error("Moonraker returned an unexpected upload status")]
    UnexpectedUploadStatus,
    #[error("local file operation failed")]
    LocalIo(#[source] std::io::Error),
}

#[derive(Debug)]
pub struct Client {
    profile: Profile,
    http: HttpClient,
    transfer_http: HttpClient,
}

impl Client {
    pub fn new(profile: Profile) -> Result<Self, Error> {
        let http = ClientBuilder::new()
            .timeout(profile.timeout)
            .danger_accept_invalid_certs(profile.insecure)
            .redirect(Policy::none())
            .build()
            .map_err(Error::Client)?;
        let transfer_http = ClientBuilder::new()
            .connect_timeout(profile.timeout)
            .danger_accept_invalid_certs(profile.insecure)
            .redirect(Policy::none())
            .build()
            .map_err(Error::Client)?;
        Ok(Self {
            profile,
            http,
            transfer_http,
        })
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

    pub fn file_roots() -> Vec<FileRoot> {
        vec![FileRoot {
            name: "gcodes",
            description: "Moonraker gcode storage",
            writable: true,
            metadata: BTreeMap::new(),
        }]
    }

    pub fn file_list(
        &self,
        access_code: Option<&str>,
        device_path: &str,
        recursive: bool,
    ) -> Result<FileList, Error> {
        let device_path = normalize_device_path(device_path)?;
        let mut entries: Vec<FileEntry> =
            if recursive {
                let files: Vec<Value> = self.json_request(
                    Method::GET,
                    "server/files/list",
                    &[("root", "gcodes".into())],
                    access_code,
                )?;
                files
                    .iter()
                    .filter_map(file_entry_from_list_item)
                    .filter(|entry| under_device_path(&entry.path, &device_path))
                    .collect()
            } else {
                let directory: DirectoryListing = self.json_request(
                    Method::GET,
                    "server/files/directory",
                    &[("path", moonraker_path(&device_path))],
                    access_code,
                )?;
                directory
                    .dirs
                    .iter()
                    .filter_map(|item| file_entry_from_directory_item(item, &device_path, true))
                    .chain(directory.files.iter().filter_map(|item| {
                        file_entry_from_directory_item(item, &device_path, false)
                    }))
                    .collect()
            };
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(FileList { entries })
    }

    pub fn download_to(
        &self,
        access_code: Option<&str>,
        device_path: &str,
        destination: &mut dyn Write,
    ) -> Result<u64, Error> {
        let device_path = normalize_device_path(device_path)?;
        let relative = device_path.trim_start_matches('/');
        if relative.is_empty() {
            return Err(Error::InvalidDevicePath);
        }
        let mut response = self.response(
            &self.transfer_http,
            Method::GET,
            &format!("server/files/gcodes/{relative}"),
            &[],
            access_code,
        )?;
        std::io::copy(&mut response, destination).map_err(Error::LocalIo)
    }

    pub fn upload_file(
        &self,
        access_code: Option<&str>,
        source: &Path,
        device_path: &str,
        overwrite: bool,
    ) -> Result<u64, Error> {
        let metadata = fs::metadata(source).map_err(Error::LocalIo)?;
        if !metadata.is_file() {
            return Err(Error::LocalIo(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "upload source is not a regular file",
            )));
        }
        let device_path = normalize_device_path(device_path)?;
        let (parent, filename) = device_path
            .rsplit_once('/')
            .filter(|(_, filename)| !filename.is_empty())
            .ok_or(Error::InvalidDevicePath)?;
        let parent = if parent.is_empty() { "/" } else { parent };
        match self.remote_file_type(access_code, parent, filename)? {
            Some(FileEntryType::Directory) => return Err(Error::DirectoryDestination),
            Some(FileEntryType::File) if !overwrite => return Err(Error::FileAlreadyExists),
            _ => {}
        }

        let form = reqwest::blocking::multipart::Form::new()
            .text("root", "gcodes")
            .text("path", parent.trim_start_matches('/').to_owned())
            .part(
                "file",
                reqwest::blocking::multipart::Part::reader(
                    File::open(source).map_err(Error::LocalIo)?,
                )
                .file_name(filename.to_owned()),
            );
        let url = self.endpoint("server/files/upload");
        let mut request = self
            .transfer_http
            .post(url)
            .header(ACCEPT, "application/json")
            .multipart(form);
        if let Some(access_code) = access_code.filter(|value| !value.is_empty()) {
            let value = HeaderValue::from_str(access_code).map_err(|_| Error::InvalidAccessCode)?;
            request = request.header("X-Api-Key", value);
        }
        let response = request.send().map_err(Error::Transport)?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => return Err(Error::Authentication),
            status if !status.is_success() => return Err(Error::HttpStatus(status)),
            StatusCode::CREATED => {}
            _ => return Err(Error::UnexpectedUploadStatus),
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
        envelope.result.ok_or(Error::MissingResult)?;
        Ok(metadata.len())
    }

    pub fn job_start(
        &self,
        access_code: Option<&str>,
        device_path: &str,
    ) -> Result<JobResult, Error> {
        let device_path = normalize_device_path(device_path)?;
        let filename = device_path.trim_start_matches('/');
        if filename.is_empty() {
            return Err(Error::InvalidDevicePath);
        }
        self.post(
            "printer/print/start",
            &[("filename", filename.to_owned())],
            access_code,
        )?;
        self.wait_for_state(access_code, PrinterState::Printing)
    }

    pub fn job_pause(&self, access_code: Option<&str>) -> Result<JobResult, Error> {
        self.post("printer/print/pause", &[], access_code)?;
        self.wait_for_state(access_code, PrinterState::Paused)
    }

    pub fn job_resume(&self, access_code: Option<&str>) -> Result<JobResult, Error> {
        self.post("printer/print/resume", &[], access_code)?;
        self.wait_for_state(access_code, PrinterState::Printing)
    }

    pub fn job_cancel(&self, access_code: Option<&str>) -> Result<JobResult, Error> {
        self.post("printer/print/cancel", &[], access_code)?;
        self.wait_for_state(access_code, PrinterState::Idle)
    }

    pub fn emergency_stop(&self, access_code: Option<&str>) -> Result<(), Error> {
        self.post("printer/emergency_stop", &[], access_code)
    }

    pub fn temperature_set(
        &self,
        access_code: Option<&str>,
        targets: TemperatureTargets,
    ) -> Result<TemperatureResult, Error> {
        if targets.chamber_celsius.is_some() {
            return Err(Error::Unsupported(
                "stock Klipper has no portable chamber heater command",
            ));
        }
        validate_temperature_targets(&targets)?;
        if targets.nozzle_celsius.is_none() && targets.bed_celsius.is_none() {
            return Err(Error::InvalidTemperatureTarget);
        }

        let mut objects = Vec::new();
        if targets.nozzle_celsius.is_some() {
            objects.push("extruder");
        }
        if targets.bed_celsius.is_some() {
            objects.push("heater_bed");
        }
        let available = self.object_status(access_code, &objects)?;
        if targets.nozzle_celsius.is_some() && !available.contains_key("extruder") {
            return Err(Error::MissingObject);
        }
        if targets.bed_celsius.is_some() && !available.contains_key("heater_bed") {
            return Err(Error::MissingObject);
        }

        let mut lines = Vec::new();
        if let Some(nozzle) = targets.nozzle_celsius {
            lines.push(format!("M104 S{nozzle}"));
        }
        if let Some(bed) = targets.bed_celsius {
            lines.push(format!("M140 S{bed}"));
        }
        self.gcode(access_code, &lines.join("\n"))?;

        let mut acknowledged = TemperatureTargets::default();
        if targets.nozzle_celsius.is_some() {
            acknowledged.nozzle_celsius =
                Some(self.wait_for_temperature_target(access_code, "extruder")?);
        }
        if targets.bed_celsius.is_some() {
            acknowledged.bed_celsius =
                Some(self.wait_for_temperature_target(access_code, "heater_bed")?);
        }
        Ok(TemperatureResult {
            targets: acknowledged,
        })
    }

    pub fn motion_home(
        &self,
        access_code: Option<&str>,
        axes: &[Axis],
    ) -> Result<MotionResult, Error> {
        let mut gcode = String::from("G28");
        for axis in axes {
            gcode.push(' ');
            gcode.push(axis.letter());
        }
        self.gcode(access_code, &gcode)?;
        Ok(MotionResult {
            state: MotionState::Accepted,
        })
    }

    pub fn motion_jog(
        &self,
        access_code: Option<&str>,
        delta: JogDelta,
    ) -> Result<MotionResult, Error> {
        validate_jog(&delta)?;
        let mut parts = Vec::new();
        if let Some(x) = delta.x_millimeters {
            parts.push(format!("X{x:.3}"));
        }
        if let Some(y) = delta.y_millimeters {
            parts.push(format!("Y{y:.3}"));
        }
        if let Some(z) = delta.z_millimeters {
            parts.push(format!("Z{z:.3}"));
        }
        if parts.is_empty() {
            return Err(Error::InvalidJog);
        }
        parts.push(format!("F{}", delta.feedrate_mm_per_min));
        let moved = self.gcode(access_code, &format!("G91\nG1 {}", parts.join(" ")));
        let restored = self.gcode(access_code, "G90");
        moved?;
        restored?;
        Ok(MotionResult {
            state: MotionState::Accepted,
        })
    }

    pub fn fan_set(
        &self,
        access_code: Option<&str>,
        fan: &str,
        speed_percent: u8,
    ) -> Result<FanResult, Error> {
        if fan != "partCooling" {
            return Err(Error::Unsupported(
                "stock Klipper only exposes the portable partCooling fan",
            ));
        }
        let pwm = (u16::from(speed_percent) * 255 + 50) / 100;
        self.gcode(access_code, &format!("M106 S{pwm}"))?;
        self.wait_for_fan_speed(access_code, speed_percent)?;
        Ok(FanResult {
            fan: fan.to_owned(),
            speed_percent,
        })
    }

    pub fn speed_set(
        &self,
        access_code: Option<&str>,
        speed_profile: &str,
    ) -> Result<SpeedResult, Error> {
        let percent = speed_percent(speed_profile).ok_or(Error::InvalidSpeedProfile)?;
        self.gcode(access_code, &format!("M220 S{percent}"))?;
        self.wait_for_speed_factor(access_code, percent)?;
        Ok(SpeedResult {
            speed_profile: speed_profile.to_owned(),
        })
    }

    fn post(
        &self,
        endpoint: &str,
        query: &[(&str, String)],
        access_code: Option<&str>,
    ) -> Result<(), Error> {
        self.json_request::<Value>(Method::POST, endpoint, query, access_code)
            .map(|_| ())
    }

    fn remote_file_type(
        &self,
        access_code: Option<&str>,
        parent: &str,
        filename: &str,
    ) -> Result<Option<FileEntryType>, Error> {
        match self.file_list(access_code, parent, false) {
            Ok(listing) => Ok(listing
                .entries
                .into_iter()
                .find(|entry| entry.name == filename)
                .map(|entry| entry.entry_type)),
            Err(Error::HttpStatus(StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn gcode(&self, access_code: Option<&str>, script: &str) -> Result<(), Error> {
        self.post(
            "printer/gcode/script",
            &[("script", script.to_owned())],
            access_code,
        )
    }

    fn wait_for_state(
        &self,
        access_code: Option<&str>,
        expected: PrinterState,
    ) -> Result<JobResult, Error> {
        let deadline = Instant::now() + self.profile.timeout;
        loop {
            let status = self.status(access_code)?;
            if status.state == expected {
                return Ok(JobResult { state: expected });
            }
            if status.state == PrinterState::Error {
                return Err(Error::UnexpectedState);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            thread::sleep(Duration::from_millis(300));
        }
    }

    fn wait_for_temperature_target(
        &self,
        access_code: Option<&str>,
        object: &str,
    ) -> Result<f64, Error> {
        let deadline = Instant::now() + self.profile.timeout;
        loop {
            let objects = self.object_status(access_code, &[object])?;
            if let Some(target) = objects
                .get(object)
                .and_then(Value::as_object)
                .and_then(|status| number(status, "target"))
            {
                return Ok(target);
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            thread::sleep(Duration::from_millis(300));
        }
    }

    fn wait_for_fan_speed(&self, access_code: Option<&str>, expected: u8) -> Result<(), Error> {
        let deadline = Instant::now() + self.profile.timeout;
        loop {
            let objects = self.object_status(access_code, &["fan"])?;
            if objects
                .get("fan")
                .and_then(Value::as_object)
                .and_then(|status| number(status, "speed"))
                .is_some_and(|speed| (speed * 100.0 - f64::from(expected)).abs() <= 1.0)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            thread::sleep(Duration::from_millis(300));
        }
    }

    fn wait_for_speed_factor(&self, access_code: Option<&str>, expected: u16) -> Result<(), Error> {
        let deadline = Instant::now() + self.profile.timeout;
        loop {
            let objects = self.object_status(access_code, &["gcode_move"])?;
            if objects
                .get("gcode_move")
                .and_then(Value::as_object)
                .and_then(|status| number(status, "speed_factor"))
                .is_some_and(|factor| (factor * 100.0 - f64::from(expected)).abs() < 0.5)
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            thread::sleep(Duration::from_millis(300));
        }
    }

    fn object_status(
        &self,
        access_code: Option<&str>,
        objects: &[&str],
    ) -> Result<BTreeMap<String, Value>, Error> {
        let query = objects
            .iter()
            .map(|object| (*object, String::new()))
            .collect::<Vec<_>>();
        let payload: StatusPayload =
            self.json_request(Method::GET, "printer/objects/query", &query, access_code)?;
        Ok(payload.status)
    }

    fn json_request<T: DeserializeOwned>(
        &self,
        method: Method,
        endpoint: &str,
        query: &[(&str, String)],
        access_code: Option<&str>,
    ) -> Result<T, Error> {
        let response = self.response(&self.http, method, endpoint, query, access_code)?;
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
        serde_json::from_value(envelope.result.ok_or(Error::MissingResult)?)
            .map_err(|_| Error::InvalidResponse)
    }

    fn response(
        &self,
        http: &HttpClient,
        method: Method,
        endpoint: &str,
        query: &[(&str, String)],
        access_code: Option<&str>,
    ) -> Result<reqwest::blocking::Response, Error> {
        let mut url = self.endpoint(endpoint);
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        let mut request = http.request(method, url).header(ACCEPT, "application/json");
        if let Some(access_code) = access_code.filter(|value| !value.is_empty()) {
            let value = HeaderValue::from_str(access_code).map_err(|_| Error::InvalidAccessCode)?;
            request = request.header("X-Api-Key", value);
        }
        let response = request.send().map_err(Error::Transport)?;
        match response.status() {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(Error::Authentication),
            status if !status.is_success() => Err(Error::HttpStatus(status)),
            _ => Ok(response),
        }
    }

    fn endpoint(&self, endpoint: &str) -> Url {
        let mut url = self.profile.base_url.clone();
        let path = format!(
            "{}/{}",
            url.path().trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        );
        url.set_path(&path);
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FileRoot {
    pub name: &'static str,
    pub description: &'static str,
    pub writable: bool,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileEntryType {
    File,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub root: &'static str,
    pub path: String,
    pub device_path: String,
    #[serde(rename = "type")]
    pub entry_type: FileEntryType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FileList {
    pub entries: Vec<FileEntry>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MotionState {
    Accepted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct MotionResult {
    pub state: MotionState,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TemperatureTargets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nozzle_celsius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bed_celsius: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chamber_celsius: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TemperatureResult {
    pub targets: TemperatureTargets,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FanResult {
    pub fan: String,
    #[serde(rename = "speedPercent")]
    pub speed_percent: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LightState {
    On,
    Off,
}

impl LightState {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LightResult {
    pub light: String,
    pub state: LightState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedResult {
    pub speed_profile: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct JobResult {
    pub state: PrinterState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    fn letter(self) -> char {
        match self {
            Self::X => 'X',
            Self::Y => 'Y',
            Self::Z => 'Z',
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JogDelta {
    pub x_millimeters: Option<f64>,
    pub y_millimeters: Option<f64>,
    pub z_millimeters: Option<f64>,
    pub feedrate_mm_per_min: u16,
}

#[derive(Deserialize)]
struct DirectoryListing {
    #[serde(default)]
    dirs: Vec<Value>,
    #[serde(default)]
    files: Vec<Value>,
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

fn normalize_device_path(value: &str) -> Result<String, Error> {
    if value.contains('\0') || value.contains('\\') || value.chars().any(char::is_control) {
        return Err(Error::InvalidDevicePath);
    }
    let trimmed = value.trim_matches('/');
    if trimmed.is_empty() {
        return Ok("/".into());
    }
    let segments = trimmed.split('/').collect::<Vec<_>>();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || matches!(*segment, "." | ".."))
    {
        return Err(Error::InvalidDevicePath);
    }
    Ok(format!("/{}", segments.join("/")))
}

fn moonraker_path(device_path: &str) -> String {
    if device_path == "/" {
        "gcodes".into()
    } else {
        format!("gcodes/{}", device_path.trim_start_matches('/'))
    }
}

fn under_device_path(entry_path: &str, requested_path: &str) -> bool {
    requested_path == "/"
        || entry_path == requested_path
        || entry_path
            .strip_prefix(requested_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn file_entry_from_list_item(value: &Value) -> Option<FileEntry> {
    let object = value.as_object()?;
    let path = object.get("path")?.as_str()?;
    file_entry(path, object, FileEntryType::File)
}

fn file_entry_from_directory_item(
    value: &Value,
    parent: &str,
    directory: bool,
) -> Option<FileEntry> {
    let object = value.as_object()?;
    let key = if directory { "dirname" } else { "filename" };
    let path = object
        .get(key)
        .and_then(Value::as_str)
        .map(|name| format!("{}/{}", parent.trim_end_matches('/'), name))
        .or_else(|| {
            object
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })?;
    file_entry(
        &path,
        object,
        if directory {
            FileEntryType::Directory
        } else {
            FileEntryType::File
        },
    )
}

fn file_entry(
    raw_path: &str,
    object: &Map<String, Value>,
    entry_type: FileEntryType,
) -> Option<FileEntry> {
    let path = normalize_device_path(raw_path.strip_prefix("gcodes/").unwrap_or(raw_path)).ok()?;
    let name = path.rsplit('/').next()?.to_owned();
    if name.is_empty() {
        return None;
    }
    let size_bytes = (entry_type == FileEntryType::File)
        .then(|| object.get("size").and_then(json_i64))
        .flatten();
    let modified_at = object
        .get("modified")
        .and_then(json_i64)
        .and_then(|seconds| OffsetDateTime::from_unix_timestamp(seconds).ok())
        .and_then(|time| time.format(&Rfc3339).ok());
    Some(FileEntry {
        name,
        root: "gcodes",
        device_path: format!("gcodes:{path}"),
        path,
        entry_type,
        size_bytes,
        modified_at,
        metadata: BTreeMap::new(),
    })
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|value| value.is_finite())
                .map(|value| value as i64)
        })
        .or_else(|| value.as_str()?.parse().ok())
}

fn validate_temperature_targets(targets: &TemperatureTargets) -> Result<(), Error> {
    validate_temperature(targets.nozzle_celsius, 300.0)?;
    validate_temperature(targets.bed_celsius, 120.0)
}

fn validate_temperature(value: Option<f64>, maximum: f64) -> Result<(), Error> {
    if value.is_some_and(|value| !value.is_finite() || !(0.0..=maximum).contains(&value)) {
        return Err(Error::InvalidTemperatureTarget);
    }
    Ok(())
}

fn validate_jog(delta: &JogDelta) -> Result<(), Error> {
    if delta.feedrate_mm_per_min == 0 || delta.feedrate_mm_per_min > 24_000 {
        return Err(Error::InvalidJog);
    }
    for distance in [
        delta.x_millimeters,
        delta.y_millimeters,
        delta.z_millimeters,
    ] {
        if distance
            .is_some_and(|distance| !distance.is_finite() || !(-10.0..=10.0).contains(&distance))
        {
            return Err(Error::InvalidJog);
        }
    }
    Ok(())
}

fn speed_percent(profile: &str) -> Option<u16> {
    match profile {
        "silent" => Some(20),
        "standard" => Some(100),
        "sport" => Some(150),
        "ludicrous" => Some(300),
        _ => None,
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

    #[test]
    fn file_requests_reject_path_traversal_before_contacting_the_printer() {
        let profile = Profile::new("http://127.0.0.1:9", false, DEFAULT_TIMEOUT).unwrap();
        let client = Client::new(profile).unwrap();

        for path in ["/../moonraker.conf", "/unsafe\nname.gcode"] {
            assert!(
                matches!(
                    client.file_list(None, path, false),
                    Err(Error::InvalidDevicePath)
                ),
                "{path:?}"
            );
        }
    }

    #[test]
    fn file_listing_uses_the_gcodes_root_without_exposing_parent_paths() {
        let (host, requests, server) = scripted_server(vec![
            r#"{"result":{"dirs":[{"dirname":"models"}],"files":[{"filename":"cube.gcode","size":12,"modified":1700000000}]}}"#,
        ]);
        let client = Client::new(Profile::new(&host, false, DEFAULT_TIMEOUT).unwrap()).unwrap();

        let listing = client.file_list(Some("api-key"), "/", false).unwrap();

        server.join().unwrap();
        let request = requests.recv().unwrap();
        assert!(request.starts_with("GET /server/files/directory?path=gcodes "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-api-key: api-key\r\n")
        );
        assert_eq!(listing.entries.len(), 2);
        assert_eq!(listing.entries[0].device_path, "gcodes:/cube.gcode");
        assert_eq!(listing.entries[1].device_path, "gcodes:/models");
        assert_eq!(listing.entries[0].size_bytes, Some(12));
    }

    #[test]
    fn jog_restores_absolute_coordinates_after_an_accepted_move() {
        let (host, requests, server) =
            scripted_server(vec![r#"{"result":"ok"}"#, r#"{"result":"ok"}"#]);
        let client = Client::new(Profile::new(&host, false, DEFAULT_TIMEOUT).unwrap()).unwrap();

        let result = client
            .motion_jog(
                None,
                JogDelta {
                    x_millimeters: Some(1.0),
                    feedrate_mm_per_min: 1500,
                    ..JogDelta::default()
                },
            )
            .unwrap();

        server.join().unwrap();
        let first = requests.recv().unwrap();
        let second = requests.recv().unwrap();
        assert_eq!(result.state, MotionState::Accepted);
        assert!(first.contains("script=G91%0AG1+X1.000+F1500"));
        assert!(second.contains("script=G90"));
    }

    #[test]
    fn upload_checks_the_destination_before_streaming_a_regular_file() {
        let (host, requests, server) = scripted_server(vec![
            r#"{"result":{"dirs":[],"files":[]}}"#,
            r#"__201__{"result":{"item":{"path":"gcodes/copy.toml"}}}"#,
        ]);
        let client = Client::new(Profile::new(&host, false, DEFAULT_TIMEOUT).unwrap()).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let expected_size = std::fs::metadata(&source).unwrap().len();

        let copied = client
            .upload_file(None, &source, "/copy.toml", false)
            .unwrap();

        server.join().unwrap();
        let preflight = requests.recv().unwrap();
        let upload = requests.recv().unwrap();
        assert_eq!(copied, expected_size);
        assert!(preflight.starts_with("GET /server/files/directory?path=gcodes "));
        assert!(upload.starts_with("POST /server/files/upload "));
        assert!(
            upload
                .to_ascii_lowercase()
                .contains("content-type: multipart/form-data; boundary=")
        );
    }

    #[test]
    fn temperature_set_checks_the_heater_then_reads_back_the_target() {
        let (host, requests, server) = scripted_server(vec![
            r#"{"result":{"status":{"extruder":{"temperature":25,"target":0}}}}"#,
            r#"{"result":"ok"}"#,
            r#"{"result":{"status":{"extruder":{"temperature":25,"target":215}}}}"#,
        ]);
        let client = Client::new(Profile::new(&host, false, DEFAULT_TIMEOUT).unwrap()).unwrap();

        let result = client
            .temperature_set(
                None,
                TemperatureTargets {
                    nozzle_celsius: Some(215.0),
                    ..TemperatureTargets::default()
                },
            )
            .unwrap();

        server.join().unwrap();
        let available = requests.recv().unwrap();
        let command = requests.recv().unwrap();
        let acknowledged = requests.recv().unwrap();
        assert_eq!(result.targets.nozzle_celsius, Some(215.0));
        assert!(available.starts_with("GET /printer/objects/query?extruder= "));
        assert!(command.contains("script=M104+S215"));
        assert!(acknowledged.starts_with("GET /printer/objects/query?extruder= "));
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

    fn scripted_server(
        bodies: Vec<&'static str>,
    ) -> (
        String,
        std::sync::mpsc::Receiver<String>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            for body in bodies {
                let (status, body) = body
                    .strip_prefix("__201__")
                    .map_or(("200 OK", body), |body| ("201 Created", body));
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut chunk).unwrap();
                    request.extend_from_slice(&chunk[..read]);
                }
                let header_end = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .expect("request headers")
                    + 4;
                let content_length = String::from_utf8_lossy(&request[..header_end])
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or_default();
                while request.len() - header_end < content_length {
                    let read = stream.read(&mut chunk).unwrap();
                    request.extend_from_slice(&chunk[..read]);
                }
                sender.send(String::from_utf8(request).unwrap()).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        (host, requests, server)
    }
}
