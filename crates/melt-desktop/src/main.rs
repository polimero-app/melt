use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Condvar, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use melt_core::{
    AppInfo,
    bambu::{CameraFrame, CameraFrameKind, CameraManager, CameraSubscription, PresenceCache},
    config::{Config, ConfigError, Profile, config_dir},
    diagnostics,
    drivers::{self, Capabilities, DriverError, Operation},
    firmware_updates::FirmwareUpdateReport,
    keychain::{SERVICE, SecretError, SecretStore, SystemKeychain, account},
    monitor, moonraker,
    pool::ConnectionPool,
    preferences::{self, NotificationPreferences, PreferencesError, Slicer},
    profiles,
};
use openssl::rand::rand_bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

mod preview;
mod webrtc_camera;

#[tauri::command(async)]
fn app_info() -> AppInfo {
    melt_core::app_info()
}

/// Errors cross the IPC boundary as stable codes so the interface can render
/// them in the user's language instead of English strings built in Rust.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandError {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    operation: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl CommandError {
    fn new(code: &'static str) -> Self {
        Self {
            code,
            operation: None,
            state: None,
            detail: None,
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn of(code: &'static str, operation: Operation) -> Self {
        Self {
            code,
            operation: Some(operation_slug(operation)),
            state: None,
            detail: None,
        }
    }
}

#[derive(Default)]
struct PreferencesState {
    write_lock: Mutex<()>,
}

#[derive(Clone, Default)]
struct PresenceState {
    cache: Arc<Mutex<PresenceCache>>,
    scan_lock: Arc<Mutex<()>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreferencesResponse {
    notifications: NotificationPreferences,
    slicers: Vec<Slicer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotificationPreferencesRequest {
    completion: bool,
    failure: bool,
    disconnection: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlicerRequest {
    name: String,
    path: String,
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlicerEnabledRequest {
    name: String,
    enabled: bool,
}

fn preferences_error(error: PreferencesError) -> CommandError {
    CommandError::new(match error {
        PreferencesError::UnsupportedVersion { .. } | PreferencesError::Malformed(_) => {
            "preferencesUnreadable"
        }
        PreferencesError::ConcurrentUpdate | PreferencesError::Io(_) => "preferencesUnwritable",
        PreferencesError::SlicerNameMissing => "slicerNameMissing",
        PreferencesError::SlicerPathMissing => "slicerPathMissing",
        PreferencesError::SlicerAlreadyExists => "slicerAlreadyExists",
        PreferencesError::SlicerNotFound => "slicerNotFound",
    })
}

fn preferences_response(mut preferences: preferences::Preferences) -> PreferencesResponse {
    preferences.slicers =
        preferences::merge_slicers(&preferences.slicers, &preferences::detect_slicers());
    PreferencesResponse {
        notifications: preferences.notifications,
        slicers: preferences.slicers,
    }
}

#[tauri::command(async)]
fn get_preferences() -> Result<PreferencesResponse, CommandError> {
    preferences::Preferences::load()
        .map(preferences_response)
        .map_err(preferences_error)
}

#[tauri::command(async)]
fn update_notification_preferences(
    request: NotificationPreferencesRequest,
    state: tauri::State<'_, PreferencesState>,
) -> Result<NotificationPreferences, CommandError> {
    let _guard = state
        .write_lock
        .lock()
        .map_err(|_| CommandError::new("preferencesUnwritable"))?;
    let dir = preferences::preferences_dir().map_err(preferences_error)?;
    let mut stored = preferences::Preferences::open(&dir).map_err(preferences_error)?;
    stored.notifications = NotificationPreferences {
        completion: request.completion,
        failure: request.failure,
        disconnection: request.disconnection,
    };
    stored.save(&dir).map_err(preferences_error)?;
    Ok(stored.notifications)
}

#[tauri::command(async)]
fn save_slicer(
    request: SlicerRequest,
    state: tauri::State<'_, PreferencesState>,
) -> Result<Vec<Slicer>, CommandError> {
    let _guard = state
        .write_lock
        .lock()
        .map_err(|_| CommandError::new("preferencesUnwritable"))?;
    let dir = preferences::preferences_dir().map_err(preferences_error)?;
    let mut stored = preferences::Preferences::open(&dir).map_err(preferences_error)?;
    stored
        .set_slicer(Slicer {
            name: request.name,
            path: request.path,
            enabled: request.enabled,
        })
        .map_err(preferences_error)?;
    stored.save(&dir).map_err(preferences_error)?;
    Ok(stored.slicers)
}

#[tauri::command(async)]
fn set_slicer_enabled(
    request: SlicerEnabledRequest,
    state: tauri::State<'_, PreferencesState>,
) -> Result<Vec<Slicer>, CommandError> {
    let _guard = state
        .write_lock
        .lock()
        .map_err(|_| CommandError::new("preferencesUnwritable"))?;
    let dir = preferences::preferences_dir().map_err(preferences_error)?;
    let mut stored = preferences::Preferences::open(&dir).map_err(preferences_error)?;
    let slicer = stored
        .slicers
        .iter_mut()
        .find(|slicer| slicer.name.eq_ignore_ascii_case(request.name.trim()))
        .ok_or_else(|| CommandError::new("slicerNotFound"))?;
    slicer.enabled = request.enabled;
    stored.save(&dir).map_err(preferences_error)?;
    Ok(stored.slicers)
}

#[tauri::command(async)]
fn remove_slicer(
    name: String,
    state: tauri::State<'_, PreferencesState>,
) -> Result<Vec<Slicer>, CommandError> {
    let _guard = state
        .write_lock
        .lock()
        .map_err(|_| CommandError::new("preferencesUnwritable"))?;
    let dir = preferences::preferences_dir().map_err(preferences_error)?;
    let mut stored = preferences::Preferences::open(&dir).map_err(preferences_error)?;
    stored.remove_slicer(&name).map_err(preferences_error)?;
    stored.save(&dir).map_err(preferences_error)?;
    Ok(stored.slicers)
}

#[tauri::command(async)]
fn detect_slicers() -> Vec<Slicer> {
    preferences::detect_slicers()
}

fn unreadable_config() -> CommandError {
    CommandError::new("configUnreadable")
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrinterSummary {
    name: String,
    driver: String,
    host: String,
    serial: String,
    model: String,
    timeout: String,
    insecure: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    presence: Option<PrinterPresenceSummary>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PrinterPresenceSummary {
    last_seen_unix_ms: u64,
    model: String,
    firmware: Option<String>,
    suggested_host: Option<String>,
}

struct DesktopPrinter {
    name: String,
    driver: drivers::Profile,
    access_code: Option<String>,
    tls_fingerprint: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrinterCapabilities {
    name: String,
    capabilities: Capabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    bambu: Option<melt_core::bambu::RuntimeCapabilities>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CameraSnapshot {
    data_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CameraStream {
    url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CameraH264Stream {
    url: String,
    codec: String,
}

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorEntry {
    name: String,
    driver: String,
    status: Option<moonraker::Status>,
    error: Option<CommandError>,
    /// True when `status` is the last successful sample retained across a
    /// failed refresh rather than a result from the current attempt.
    stale: bool,
    connection_state: MonitorConnectionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    observed_at: Option<String>,
}

/// A non-sensitive health check for the operating system credential store.
/// It deliberately returns no credential material; its purpose is to let the
/// interface distinguish a locked keychain from a printer that is unreachable.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct KeychainProbe {
    available: bool,
    reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum MonitorConnectionState {
    Connecting,
    Synchronizing,
    Live,
    Recovering,
    Offline,
}

const MONITOR_OFFLINE_AFTER: Duration = Duration::from_secs(30);

struct CachedMonitor {
    next_poll: Instant,
    sampled_at: Instant,
    started_at: Instant,
    observed_at: Option<Instant>,
    backoff: monitor::Backoff,
    entry: MonitorEntry,
}

#[derive(Clone, Default)]
struct MonitorState {
    entries: Arc<Mutex<BTreeMap<String, CachedMonitor>>>,
    previous: Arc<Mutex<HashMap<String, MonitorEntry>>>,
    pool: Arc<ConnectionPool>,
}

const FIRMWARE_UPDATE_CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const FIRMWARE_UPDATE_STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct FirmwareUpdateEntry {
    profile: String,
    driver: String,
    report: FirmwareUpdateReport,
    checked_at: String,
    stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<CommandError>,
}

struct CachedFirmwareUpdate {
    checked_at: Instant,
    generation: u64,
    entry: FirmwareUpdateEntry,
}

#[derive(Clone, Default)]
struct FirmwareUpdateState {
    entries: Arc<Mutex<BTreeMap<String, CachedFirmwareUpdate>>>,
    pool: Arc<ConnectionPool>,
}

#[derive(Default)]
struct WebRtcSessions {
    generations: HashMap<String, u64>,
    active: HashMap<String, webrtc_camera::Session>,
    profile_keys: HashMap<String, String>,
}

#[derive(Default)]
struct CameraWebRtcState {
    generation: AtomicU64,
    sessions: Mutex<WebRtcSessions>,
}

struct CameraProxy {
    generation: u64,
    stop: Arc<AtomicBool>,
}

#[derive(Clone, Copy)]
enum CameraProxyFormat {
    Mjpeg,
    H264,
}

#[derive(Clone, Default)]
struct CameraStreamState {
    generation: Arc<AtomicU64>,
    active: Arc<Mutex<HashMap<String, CameraProxy>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CameraWebRtcAnswer {
    #[serde(rename = "type")]
    kind: &'static str,
    sdp: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationEvent {
    kind: &'static str,
    printer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobActionRequest {
    name: String,
    action: String,
    #[serde(default)]
    device_path: Option<String>,
    #[serde(default)]
    options: melt_core::bambu::JobStartOptions,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemperatureRequest {
    name: String,
    #[serde(default)]
    temperature: Option<String>,
    #[serde(default)]
    target_celsius: Option<f64>,
    #[serde(default)]
    nozzle_celsius: Option<f64>,
    #[serde(default)]
    bed_celsius: Option<f64>,
    #[serde(default)]
    chamber_celsius: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FanRequest {
    name: String,
    fan: String,
    speed_percent: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TlsRefreshRequest {
    name: String,
    fingerprint: String,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JogRequest {
    name: String,
    x_millimeters: Option<f64>,
    y_millimeters: Option<f64>,
    z_millimeters: Option<f64>,
    #[serde(default = "default_feedrate")]
    feedrate_mm_per_min: u16,
}

fn default_feedrate() -> u16 {
    1500
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LightRequest {
    name: String,
    light: String,
    on: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AmsDryingRequest {
    name: String,
    ams_id: u32,
    drying: moonraker::DryingRequest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpeedRequest {
    name: String,
    speed_profile: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDownloadRequest {
    name: String,
    device_path: String,
    destination: String,
    #[serde(default)]
    transfer_id: Option<String>,
    #[serde(default)]
    total_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TransferProgress {
    transfer_id: String,
    direction: &'static str,
    bytes_transferred: u64,
    total_bytes: Option<u64>,
    complete: bool,
}

#[derive(Clone, Default)]
struct TransferState {
    canceled: Arc<Mutex<HashSet<String>>>,
}

struct ProgressWriter<'a> {
    destination: &'a mut std::fs::File,
    app: tauri::AppHandle,
    state: TransferState,
    transfer_id: String,
    total_bytes: Option<u64>,
    transferred: u64,
    last_report: u64,
}

impl std::io::Write for ProgressWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self
            .state
            .canceled
            .lock()
            .map(|canceled| canceled.contains(&self.transfer_id))
            .unwrap_or(false)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "transfer canceled",
            ));
        }
        let written = self.destination.write(bytes)?;
        self.transferred += written as u64;
        if self.transferred.saturating_sub(self.last_report) >= 256 * 1024 {
            use tauri::Emitter;
            let _ = self.app.emit(
                "transfer-progress",
                TransferProgress {
                    transfer_id: self.transfer_id.clone(),
                    direction: "download",
                    bytes_transferred: self.transferred,
                    total_bytes: self.total_bytes,
                    complete: false,
                },
            );
            self.last_report = self.transferred;
        }
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.destination.flush()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileListRequest {
    name: String,
    #[serde(default = "root_path")]
    path: String,
    #[serde(default)]
    search: String,
}

fn root_path() -> String {
    "/".to_owned()
}

/// The file library can browse anywhere on disk, so this only rejects
/// values that can't be a real path (empty, or containing a NUL byte, which
/// every OS filesystem API already refuses).
fn resolve_absolute_path(value: &str) -> Result<std::path::PathBuf, CommandError> {
    if value.is_empty() || value.contains('\0') {
        return Err(CommandError::new("libraryPathInvalid"));
    }
    std::fs::canonicalize(value).map_err(|_| CommandError::new("libraryUnavailable"))
}

/// Where the file library opens when no explicit path is given: the last
/// folder the user browsed (persisted in preferences), falling back to a
/// `Melt` folder under the user's documents, created on first use.
fn default_library_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, CommandError> {
    if let Ok(preferences) = preferences::Preferences::load() {
        if let Some(path) = preferences.library_path {
            if let Ok(resolved) = resolve_absolute_path(&path) {
                if resolved.is_dir() {
                    return Ok(resolved);
                }
            }
        }
    }
    let base = app
        .path()
        .document_dir()
        .map_err(|_| CommandError::new("libraryUnavailable"))?;
    let dir = base.join("Melt");
    std::fs::create_dir_all(&dir).map_err(|_| CommandError::new("libraryUnavailable"))?;
    Ok(dir)
}

/// Breadcrumb trail from the filesystem root down to `path`, letting the UI
/// jump to any ancestor without the frontend having to parse OS-specific
/// path separators itself.
fn library_breadcrumbs(path: &std::path::Path) -> Vec<LibraryBreadcrumb> {
    let mut breadcrumbs: Vec<LibraryBreadcrumb> = path
        .ancestors()
        .map(|ancestor| LibraryBreadcrumb {
            name: ancestor
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| ancestor.to_string_lossy().into_owned()),
            path: ancestor.to_string_lossy().into_owned(),
        })
        .collect();
    breadcrumbs.reverse();
    breadcrumbs
}

fn format_modified(modified: std::time::SystemTime) -> Option<String> {
    time::OffsetDateTime::from(modified)
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileUploadRequest {
    name: String,
    source: String,
    device_path: String,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileDeleteRequest {
    name: String,
    device_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FilePreviewRequest {
    name: String,
    device_path: String,
    #[serde(default)]
    size_bytes: Option<i64>,
    #[serde(default)]
    modified_at: Option<String>,
}

/// The file library lives on the local machine, not on any printer, so this
/// only ever names a slicer and a path inside the library.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OpenWithSlicerRequest {
    path: String,
    slicer: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FilePreviewResponse {
    kind: &'static str,
    data: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryListRequest {
    /// Absolute path to browse; omitted (or empty) resolves to the last
    /// visited folder, or a default under the user's documents.
    #[serde(default)]
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryBreadcrumb {
    name: String,
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LibraryListResponse {
    path: String,
    parent: Option<String>,
    breadcrumbs: Vec<LibraryBreadcrumb>,
    entries: Vec<moonraker::FileEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryDeleteRequest {
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryAddRequest {
    source: String,
    directory: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryPreviewRequest {
    path: String,
    #[serde(default)]
    size_bytes: Option<i64>,
    #[serde(default)]
    modified_at: Option<String>,
    /// Renders at `preview::LARGE_SIZE` for the file preview dialog instead of
    /// the grid thumbnail size.
    #[serde(default)]
    large: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryPrintRequest {
    printer: String,
    path: String,
    #[serde(default)]
    plate: Option<u32>,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryInspectRequest {
    path: String,
}

impl Clone for FilePreviewResponse {
    fn clone(&self) -> Self {
        Self {
            kind: self.kind,
            data: self.data.clone(),
        }
    }
}

#[derive(Default)]
struct PreviewState {
    cache: Mutex<PreviewCache>,
}

/// Watches the folder the file library last listed, so the UI can re-list
/// it when something outside the app adds, removes or rewrites a file.
/// Replacing the watcher drops the previous one, which stops that watch.
#[derive(Default)]
struct LibraryWatchState {
    watcher: Mutex<Option<(std::path::PathBuf, notify::RecommendedWatcher)>>,
}

fn watch_library_dir(app: &tauri::AppHandle, directory: &std::path::Path) {
    let state = app.state::<LibraryWatchState>();
    let mut current = state
        .watcher
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if current
        .as_ref()
        .is_some_and(|(watched, _)| watched == directory)
    {
        return;
    }
    let emitter = app.clone();
    let path = directory.to_string_lossy().into_owned();
    let handler = move |event: notify::Result<notify::Event>| {
        // Reads don't change the listing, and a file being copied in emits
        // plenty of modify events; the UI debounces those into one re-list.
        if event.is_ok_and(|event| !event.kind.is_access()) {
            let _ = emitter.emit("library-changed", &path);
        }
    };
    // ponytail: best-effort, a folder that can't be watched still lists and
    // refreshes on navigation or the refresh button.
    *current = notify::recommended_watcher(handler)
        .and_then(|mut watcher| {
            notify::Watcher::watch(&mut watcher, directory, notify::RecursiveMode::NonRecursive)?;
            Ok(watcher)
        })
        .ok()
        .map(|watcher| (directory.to_path_buf(), watcher));
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SlicerProcessInfo {
    pid: u32,
    slicer: String,
    file_name: String,
}

struct ManagedSlicer {
    child: Arc<Mutex<std::process::Child>>,
    info: SlicerProcessInfo,
}

#[derive(Clone, Default)]
struct SlicerState {
    processes: Arc<Mutex<HashMap<u32, ManagedSlicer>>>,
}

#[derive(Default)]
struct PreviewCache {
    entries: HashMap<String, FilePreviewResponse>,
    order: VecDeque<String>,
    bytes: usize,
}

impl PreviewCache {
    const MAX_BYTES: usize = 64 * 1024 * 1024;

    fn get(&mut self, key: &str) -> Option<FilePreviewResponse> {
        let preview = self.entries.get(key)?.clone();
        self.order.retain(|entry| entry != key);
        self.order.push_back(key.to_owned());
        Some(preview)
    }

    fn insert(&mut self, key: String, preview: FilePreviewResponse) {
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.data.len());
        }
        self.order.retain(|entry| entry != &key);
        self.bytes += preview.data.len();
        self.entries.insert(key.clone(), preview);
        self.order.push_back(key);
        while self.bytes > Self::MAX_BYTES {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed.data.len());
            }
        }
    }
}

const PREVIEW_CACHE_VERSION: &str = "zip-first-model-v1";

fn preview_cache_key(request: &FilePreviewRequest, printer_key: &str) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        PREVIEW_CACHE_VERSION,
        printer_key,
        request.device_path,
        request.size_bytes.unwrap_or_default(),
        request.modified_at.as_deref().unwrap_or_default()
    )
}

#[tauri::command(async)]
fn configured_printers(
    state: tauri::State<'_, PresenceState>,
) -> Result<Vec<PrinterSummary>, CommandError> {
    let presence = state
        .cache
        .lock()
        .map_err(|_| CommandError::new("monitorUnavailable"))?;
    Config::load()
        .map(|config| {
            config
                .sorted_profiles()
                .into_iter()
                .map(|profile| {
                    let observed = presence.get(&profile.profile.serial);
                    PrinterSummary {
                        name: profile.name,
                        driver: profile.profile.driver,
                        host: profile.profile.host,
                        serial: profile.profile.serial,
                        model: profile.profile.model,
                        timeout: profile.profile.timeout,
                        insecure: profile.profile.insecure,
                        presence: observed.map(|observed| PrinterPresenceSummary {
                            last_seen_unix_ms: observed.last_seen_unix_ms,
                            model: observed.model.clone(),
                            firmware: observed.firmware.clone(),
                            suggested_host: observed.suggested_host.clone(),
                        }),
                    }
                })
                .collect()
        })
        .map_err(|_| unreadable_config())
}

#[tauri::command(async)]
fn registered_drivers() -> Vec<drivers::DriverInfo> {
    drivers::registered().to_vec()
}

#[tauri::command(async)]
fn discover_printers(
    state: tauri::State<'_, PresenceState>,
) -> Result<Vec<melt_core::bambu::DiscoveredPrinter>, CommandError> {
    let _scan = state
        .scan_lock
        .lock()
        .map_err(|_| CommandError::new("discoveryFailed"))?;
    let discovered = melt_core::bambu::discover(Duration::from_secs(5))
        .map_err(|_| CommandError::new("discoveryFailed"))?;
    update_presence_cache(&state, &discovered, Duration::from_secs(60));
    Ok(discovered)
}

#[tauri::command(async)]
fn printer_capabilities(
    name: String,
    state: tauri::State<'_, MonitorState>,
) -> Result<PrinterCapabilities, CommandError> {
    let config = Config::load().map_err(|_| unreadable_config())?;
    let profile = config
        .get_profile(&name.to_ascii_lowercase())
        .ok_or_else(|| CommandError::new("profileNotFound"))?;
    let driver = drivers::profile(profile).map_err(|_| CommandError::new("profileInvalid"))?;
    let mut bambu = driver.bambu_runtime_capabilities();
    if bambu.is_some() {
        let kind = driver.driver();
        let discovered = access_code(&profile.driver, &name, kind)
            .and_then(|access_code| {
                tls_fingerprint(&profile.driver, &name, kind, profile.insecure)
                    .map(|fingerprint| (access_code, fingerprint))
            })
            .ok()
            .and_then(|(access_code, fingerprint)| {
                state
                    .pool
                    .bambu_runtime_capabilities(
                        &name,
                        &driver,
                        access_code.as_deref(),
                        fingerprint.as_deref(),
                    )
                    .ok()
                    .flatten()
            });
        if discovered.is_some() {
            bambu = discovered;
        }
    }
    if let Some(runtime) = bambu.as_mut()
        && let Ok(entries) = state.entries.lock()
        && let Some(extension) = entries
            .get(&name.to_ascii_lowercase())
            .and_then(|cached| cached.entry.status.as_ref())
            .and_then(|status| status.extensions.bambu_lan.as_ref())
    {
        if let Some(count) = extension.extruder_count {
            runtime.extruder_count = Some(count);
        }
        runtime.mqtt_alive_supported = extension
            .mqtt_alive_supported
            .or(runtime.mqtt_alive_supported);
        if extension.ams.is_some() {
            runtime.ams_supported = Some(true);
        }
        if extension.emmc_storage == Some(true) {
            runtime.storage_transport = melt_core::bambu::StorageTransport::Tunnel6000;
            if !runtime
                .storage_volumes
                .contains(&melt_core::bambu::StorageVolume::Emmc)
            {
                runtime
                    .storage_volumes
                    .push(melt_core::bambu::StorageVolume::Emmc);
            }
        }
    }
    if let Some(runtime) = bambu.as_mut()
        && let Ok(entries) = state.entries.lock()
        && let Some(version) = entries
            .get(&name.to_ascii_lowercase())
            .and_then(|cached| cached.entry.status.as_ref())
            .and_then(|status| status.firmware_version.clone())
    {
        runtime.firmware_version = Some(version);
    }
    Ok(PrinterCapabilities {
        name,
        capabilities: driver.capabilities(),
        bambu,
    })
}

#[tauri::command(async)]
fn monitored_printers(
    state: tauri::State<'_, MonitorState>,
) -> Result<Vec<MonitorEntry>, CommandError> {
    collect_monitored_printers(&state)
}

/// Returns the last persisted monitoring payload for immediate UI paint.
/// The frontend requests this after registering its event listener so the
/// startup snapshot cannot be lost before the webview is ready.
#[tauri::command(async)]
fn cached_monitoring() -> Option<serde_json::Value> {
    load_status_cache()
}

#[tauri::command(async)]
fn cached_firmware_updates(
    state: tauri::State<'_, FirmwareUpdateState>,
) -> Result<Vec<FirmwareUpdateEntry>, CommandError> {
    let generation = state.pool.lifecycle_generation();
    let entries = state
        .entries
        .lock()
        .map_err(|_| CommandError::new("firmwareUpdatesUnavailable"))?;
    Ok(entries
        .values()
        .filter(|cached| cached.generation == generation)
        .map(current_firmware_entry)
        .collect())
}

#[tauri::command(async)]
fn printer_firmware_updates(
    name: String,
    refresh: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, FirmwareUpdateState>,
) -> Result<FirmwareUpdateEntry, CommandError> {
    let config = Config::load().map_err(|_| unreadable_config())?;
    let normalized = name.to_ascii_lowercase();
    let profile = config
        .get_profile(&normalized)
        .ok_or_else(|| CommandError::new("profileNotFound"))?
        .clone();
    if !refresh && let Some(entry) = fresh_firmware_entry(&state, &normalized) {
        return Ok(entry);
    }
    check_firmware_updates(&state, &app, normalized, profile, refresh)
}

fn fresh_firmware_entry(state: &FirmwareUpdateState, name: &str) -> Option<FirmwareUpdateEntry> {
    let generation = state.pool.lifecycle_generation();
    let entries = state.entries.lock().ok()?;
    let cached = entries.get(&name.to_ascii_lowercase())?;
    (cached.generation == generation && cached.checked_at.elapsed() <= FIRMWARE_UPDATE_CACHE_TTL)
        .then(|| current_firmware_entry(cached))
}

fn current_firmware_entry(cached: &CachedFirmwareUpdate) -> FirmwareUpdateEntry {
    let mut entry = cached.entry.clone();
    entry.stale |= cached.checked_at.elapsed() > FIRMWARE_UPDATE_STALE_AFTER;
    entry
}

fn check_firmware_updates(
    state: &FirmwareUpdateState,
    app: &tauri::AppHandle,
    name: String,
    profile: Profile,
    refresh: bool,
) -> Result<FirmwareUpdateEntry, CommandError> {
    let generation = state.pool.lifecycle_generation();
    let driver = drivers::profile(&profile).map_err(|_| CommandError::new("profileInvalid"))?;
    let kind = driver.driver();
    let access_code = access_code(&profile.driver, &name, kind)?;
    let fingerprint = tls_fingerprint(&profile.driver, &name, kind, profile.insecure)?;
    let result = state.pool.firmware_update_status(
        &name,
        &driver,
        access_code.as_deref(),
        fingerprint.as_deref(),
        refresh,
    );
    if !state.pool.is_current_generation(generation) {
        return Err(CommandError::new("profileNotFound"));
    }
    match result {
        Ok(report) => {
            let entry = FirmwareUpdateEntry {
                profile: name.clone(),
                driver: kind.name().into(),
                report,
                checked_at: format_modified(SystemTime::now()).unwrap_or_default(),
                stale: false,
                error: None,
            };
            let changed = store_firmware_update(state, &name, generation, entry.clone());
            if changed {
                let _ = app.emit("firmware-updates-updated", &entry);
            }
            Ok(entry)
        }
        Err(error) => {
            let error = operation_error(error, Operation::FirmwareUpdateCheck);
            if let Some((entry, changed)) = retain_firmware_update_failure(state, &name, &error)
                && changed
            {
                let _ = app.emit("firmware-updates-updated", entry);
            }
            Err(error)
        }
    }
}

fn store_firmware_update(
    state: &FirmwareUpdateState,
    name: &str,
    generation: u64,
    entry: FirmwareUpdateEntry,
) -> bool {
    let Ok(mut entries) = state.entries.lock() else {
        return false;
    };
    let changed = entries
        .get(name)
        .is_none_or(|cached| cached.entry != entry || cached.generation != generation);
    entries.insert(
        name.to_owned(),
        CachedFirmwareUpdate {
            checked_at: Instant::now(),
            generation,
            entry,
        },
    );
    changed
}

fn retain_firmware_update_failure(
    state: &FirmwareUpdateState,
    name: &str,
    error: &CommandError,
) -> Option<(FirmwareUpdateEntry, bool)> {
    let mut entries = state.entries.lock().ok()?;
    let cached = entries.get_mut(name)?;
    let previous = cached.entry.clone();
    cached.entry.stale = true;
    cached.entry.error = Some(error.clone());
    Some((cached.entry.clone(), cached.entry != previous))
}

fn invalidate_firmware_updates(state: &FirmwareUpdateState, name: &str) {
    if let Ok(mut entries) = state.entries.lock() {
        entries.remove(&name.to_ascii_lowercase());
    }
}

/// Tries to read the configured authentication credential without exposing it.
/// On Secret Service based desktops this access can also cause the OS to
/// present its normal unlock prompt; Melt never attempts to unlock it
/// itself.
#[tauri::command(async)]
fn probe_keychain(name: String) -> Result<KeychainProbe, CommandError> {
    let config = Config::load().map_err(|_| unreadable_config())?;
    let profile = config
        .get_profile(&name.to_ascii_lowercase())
        .ok_or_else(|| CommandError::new("profileNotFound"))?;
    let driver = drivers::profile(profile).map_err(|_| CommandError::new("profileInvalid"))?;
    let kind = driver.driver();

    match access_code(&profile.driver, &name, kind) {
        Ok(_) => Ok(KeychainProbe {
            available: true,
            reason: None,
        }),
        Err(error) if error.code == "accessCodeUnavailable" => Ok(KeychainProbe {
            available: true,
            reason: Some("accessCodeMissing"),
        }),
        Err(error) if error.code == "keychainFailed" => Ok(KeychainProbe {
            available: false,
            reason: Some("keychainUnavailable"),
        }),
        Err(error) => Err(error),
    }
}

/// Like [`monitored_printers`], but polls (and caches) just one printer —
/// for refreshing after an action scoped to a single printer instead of
/// re-probing every configured one.
#[tauri::command(async)]
fn printer_status(
    name: String,
    state: tauri::State<'_, MonitorState>,
) -> Result<MonitorEntry, CommandError> {
    for _ in 0..2 {
        let generation = state.pool.lifecycle_generation();
        let config = Config::load().map_err(|_| unreadable_config())?;
        let profile = config
            .get_profile(&name.to_ascii_lowercase())
            .ok_or_else(|| CommandError::new("profileNotFound"))?
            .clone();
        if let Some(entry) = cached_ui_monitor_entry(&state, &name, generation) {
            return Ok(entry);
        }
        if let Some(entry) = cached_monitor_entry_at(&state, name.clone(), profile, generation) {
            return Ok(entry);
        }
    }
    Err(CommandError::new("monitorUnavailable"))
}

/// Printer selection should repaint from the last known state immediately.
/// The monitor worker remains responsible for refreshing this snapshot; a
/// live probe is only needed when no recent entry exists.
fn cached_ui_monitor_entry(
    state: &MonitorState,
    name: &str,
    generation: u64,
) -> Option<MonitorEntry> {
    if !state.pool.is_current_generation(generation) {
        return None;
    }
    let entries = state.entries.lock().ok()?;
    let cached = entries.get(&name.to_ascii_lowercase())?;
    (cached.sampled_at.elapsed() <= MAX_UI_CACHE_AGE
        && state.pool.is_current_generation(generation))
    .then(|| cached.entry.clone())
}

fn collect_monitored_printers(state: &MonitorState) -> Result<Vec<MonitorEntry>, CommandError> {
    let generation = state.pool.lifecycle_generation();
    let config = Config::load().map_err(|_| unreadable_config())?;
    let profiles = config.sorted_profiles();
    let mut entries = state
        .entries
        .lock()
        .map_err(|_| CommandError::new("monitorUnavailable"))?;
    entries.retain(|name, _| profiles.iter().any(|profile| profile.name == *name));
    drop(entries);
    state
        .pool
        .retain(|name| profiles.iter().any(|profile| profile.name == name));

    // ponytail: fan out one thread per printer instead of `monitor::DEFAULT_WORKERS`-bounded
    // pool; printer counts are small (single-digit) so a cap isn't worth the complexity yet.
    let collected = thread::scope(|scope| {
        profiles
            .into_iter()
            .map(|named| {
                scope
                    .spawn(|| cached_monitor_entry_at(state, named.name, named.profile, generation))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|handle| handle.join().expect("monitor thread panicked"))
            .collect()
    });
    if !state.pool.is_current_generation(generation) {
        return Err(CommandError::new("monitorUnavailable"));
    }
    Ok(collected)
}

const STATUS_CACHE_FILE: &str = "melt-status-cache.json";
const MAX_UI_CACHE_AGE: Duration = Duration::from_secs(30);

/// Last known `monitoring-updated` payload, persisted as raw JSON so a
/// restart can repaint it immediately instead of showing nothing until the
/// first live poll completes. Values are never round-tripped back into
/// `MonitorEntry` (some of its fields borrow `&'static str`, which can't
/// implement `Deserialize`) — the cache exists only to be forwarded
/// untouched as an event payload.
fn save_status_cache(entries: &[MonitorEntry]) {
    if let Ok(dir) = config_dir() {
        save_status_cache_to(&dir, entries);
    }
}

fn save_status_cache_to(dir: &std::path::Path, entries: &[MonitorEntry]) {
    let Ok(bytes) = serde_json::to_vec(entries) else {
        return;
    };
    let _ = std::fs::create_dir_all(dir).and_then(|()| {
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        temp.write_all(&bytes)?;
        temp.persist(dir.join(STATUS_CACHE_FILE))
            .map_err(|error| error.error)?;
        Ok(())
    });
}

fn load_status_cache() -> Option<serde_json::Value> {
    load_status_cache_from(&config_dir().ok()?)
}

fn load_status_cache_from(dir: &std::path::Path) -> Option<serde_json::Value> {
    let bytes = std::fs::read(dir.join(STATUS_CACHE_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Every successful poll stamps a new `observed_at`, which the UI needs to
/// show a reading's age but which would otherwise make each 5 s poll look
/// like a change worth writing to disk. The cache is rewritten only when
/// something else changed, or at a slow heartbeat so the ages a restart
/// repaints from it don't drift arbitrarily far behind reality.
const STATUS_CACHE_HEARTBEAT: Duration = Duration::from_secs(300);

fn without_observed_at(entries: &[MonitorEntry]) -> Vec<MonitorEntry> {
    entries
        .iter()
        .cloned()
        .map(|entry| MonitorEntry {
            observed_at: None,
            ..entry
        })
        .collect()
}

#[derive(Default)]
struct StatusCacheThrottle {
    saved: Option<(Vec<MonitorEntry>, Instant)>,
}

impl StatusCacheThrottle {
    /// Records `entries` as saved and returns true when they should be
    /// written; false when only freshness changed since the last write.
    fn should_save(&mut self, entries: &[MonitorEntry], now: Instant) -> bool {
        let comparable = without_observed_at(entries);
        if let Some((saved, saved_at)) = &self.saved
            && *saved == comparable
            && now.duration_since(*saved_at) < STATUS_CACHE_HEARTBEAT
        {
            return false;
        }
        self.saved = Some((comparable, now));
        true
    }
}

fn start_monitor_worker(app: tauri::AppHandle, state: MonitorState, presence: PresenceState) {
    thread::spawn(move || {
        let mut previous_entries: Option<Vec<MonitorEntry>> = None;
        let mut cache_throttle = StatusCacheThrottle::default();
        loop {
            let cycle_started = Instant::now();
            if let Ok(entries) = collect_monitored_printers(&state) {
                if update_presence_from_monitor(&presence, &entries) {
                    let _ = app.emit("presence-updated", ());
                }
                if previous_entries.as_ref() != Some(&entries) {
                    use tauri::Emitter;
                    if cache_throttle.should_save(&entries, Instant::now()) {
                        save_status_cache(&entries);
                    }
                    let _ = app.emit("monitoring-updated", entries.clone());
                    emit_status_notifications(&app, &state);
                    previous_entries = Some(entries);
                }
            }
            thread::sleep(Duration::from_secs(5).saturating_sub(cycle_started.elapsed()));
        }
    });
}

fn start_firmware_update_worker(app: tauri::AppHandle, state: FirmwareUpdateState) {
    thread::spawn(move || {
        // Let the status monitor establish the shared sessions before doing
        // slower, non-essential update checks.
        thread::sleep(Duration::from_secs(10));
        loop {
            let cycle_started = Instant::now();
            if let Ok(config) = Config::load() {
                let profiles = config.sorted_profiles();
                for batch in profiles.chunks(usize::from(monitor::DEFAULT_WORKERS)) {
                    thread::scope(|scope| {
                        for named in batch.iter().cloned() {
                            let app = &app;
                            let state = &state;
                            scope.spawn(move || {
                                let _ = check_firmware_updates(
                                    state,
                                    app,
                                    named.name,
                                    named.profile,
                                    false,
                                );
                            });
                        }
                    });
                }
            }
            thread::sleep(FIRMWARE_UPDATE_CACHE_TTL.saturating_sub(cycle_started.elapsed()));
        }
    });
}

fn update_presence_from_monitor(state: &PresenceState, entries: &[MonitorEntry]) -> bool {
    let configured = Config::load()
        .ok()
        .map(|config| {
            config
                .sorted_profiles()
                .into_iter()
                .filter(|profile| profile.profile.driver == "bambu-lan")
                .map(|profile| {
                    (
                        profile.name.to_ascii_lowercase(),
                        (
                            profile.profile.serial,
                            profile.profile.host,
                            profile.profile.model,
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let now_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX.into()) as u64;
    let Ok(mut cache) = state.cache.lock() else {
        return false;
    };
    let mut changed = false;
    for entry in entries
        .iter()
        .filter(|entry| entry.error.is_none() && !entry.stale)
    {
        let Some((serial, configured_host, model)) =
            configured.get(&entry.name.to_ascii_lowercase())
        else {
            continue;
        };
        let Some(status) = &entry.status else {
            continue;
        };
        let Some(reported_host) = status
            .extensions
            .bambu_lan
            .as_ref()
            .and_then(|extension| extension.reported_ip.as_deref())
        else {
            continue;
        };
        let before = cache.get(serial).map(presence_identity);
        let after = cache
            .observe_status_address(
                serial,
                reported_host,
                Some(configured_host),
                model,
                status.firmware_version.as_deref(),
                now_unix_ms,
            )
            .map(presence_identity);
        changed |= before != after;
    }
    changed
}

fn presence_identity(
    presence: &melt_core::bambu::PrinterPresence,
) -> (String, Option<String>, String, Option<String>) {
    (
        presence.host.clone(),
        presence.suggested_host.clone(),
        presence.model.clone(),
        presence.firmware.clone(),
    )
}

fn start_presence_worker(app: tauri::AppHandle, state: PresenceState) {
    const SCAN_WINDOW: Duration = Duration::from_secs(4);
    const CYCLE: Duration = Duration::from_secs(15);
    const RETENTION: Duration = Duration::from_secs(60);
    thread::spawn(move || {
        loop {
            let cycle_started = Instant::now();
            let discovered = state
                .scan_lock
                .lock()
                .ok()
                .and_then(|_scan| melt_core::bambu::discover(SCAN_WINDOW).ok());
            if let Some(discovered) = discovered {
                update_presence_cache(&state, &discovered, RETENTION);
                let _ = app.emit("presence-updated", ());
            }
            thread::sleep(CYCLE.saturating_sub(cycle_started.elapsed()));
        }
    });
}

fn update_presence_cache(
    state: &PresenceState,
    discovered: &[melt_core::bambu::DiscoveredPrinter],
    retention: Duration,
) {
    let configured = Config::load()
        .ok()
        .map(|config| {
            config
                .sorted_profiles()
                .into_iter()
                .filter(|profile| profile.profile.driver == "bambu-lan")
                .filter(|profile| !profile.profile.serial.trim().is_empty())
                .map(|profile| {
                    (
                        profile.profile.serial.trim().to_ascii_uppercase(),
                        profile.profile.host,
                    )
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let now_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX.into()) as u64;
    if let Ok(mut cache) = state.cache.lock() {
        for printer in discovered {
            let host = configured
                .get(&printer.serial.trim().to_ascii_uppercase())
                .map(String::as_str);
            cache.observe(printer, host, now_unix_ms);
        }
        cache.retain_seen_since(now_unix_ms.saturating_sub(retention.as_millis() as u64));
    }
}

fn emit_status_notifications(app: &tauri::AppHandle, state: &MonitorState) {
    let current = state
        .entries
        .lock()
        .ok()
        .map(|entries| {
            entries
                .iter()
                .map(|(name, cached)| (name.clone(), cached.entry.clone()))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let preferences = preferences::Preferences::load()
        .ok()
        .map(|value| value.notifications)
        .unwrap_or_default();
    let mut previous = match state.previous.lock() {
        Ok(previous) => previous,
        Err(_) => return,
    };
    for (name, entry) in &current {
        let Some(old) = previous.get(name) else {
            continue;
        };
        let old_state = old.status.as_ref().map(|status| status.state);
        let new_state = entry.status.as_ref().map(|status| status.state);
        let kind = if preferences.completion
            && matches!(
                old_state,
                Some(moonraker::PrinterState::Printing | moonraker::PrinterState::Paused)
            )
            && new_state == Some(moonraker::PrinterState::Idle)
        {
            Some("completion")
        } else if preferences.failure
            && new_state == Some(moonraker::PrinterState::Error)
            && old_state != new_state
        {
            Some("failure")
        } else if preferences.disconnection && old.error.is_none() && entry.error.is_some() {
            Some("disconnection")
        } else {
            None
        };
        if let Some(kind) = kind {
            let _ = app.emit(
                "printer-notification",
                NotificationEvent {
                    kind,
                    printer: name.clone(),
                },
            );
            let body = match kind {
                "completion" => "Print finished.",
                "failure" => "Print failed or was aborted.",
                _ => "Connection lost.",
            };
            let _ = app.notification().builder().title(name).body(body).show();
        }
    }
    *previous = current;
}

fn cached_monitor_entry_at(
    state: &MonitorState,
    name: String,
    profile: Profile,
    generation: u64,
) -> Option<MonitorEntry> {
    if !state.pool.is_current_generation(generation) {
        return None;
    }
    let now = Instant::now();
    if let Ok(entries) = state.entries.lock() {
        if let Some(cached) = entries.get(&name).filter(|cached| cached.next_poll > now) {
            return state
                .pool
                .is_current_generation(generation)
                .then(|| cached.entry.clone());
        }
    }

    if let Ok(mut entries) = state.entries.lock() {
        entries
            .entry(name.clone())
            .or_insert_with(|| CachedMonitor {
                next_poll: now,
                sampled_at: now,
                started_at: now,
                observed_at: None,
                backoff: monitor::Backoff::new(monitor::DEFAULT_INTERVAL),
                entry: MonitorEntry {
                    name: name.clone(),
                    driver: profile.driver.clone(),
                    status: None,
                    error: None,
                    stale: true,
                    connection_state: MonitorConnectionState::Connecting,
                    observed_at: None,
                },
            });
    }

    let Some(mut entry) = monitor_printer(&state.pool, name.clone(), profile, generation) else {
        if let Ok(mut entries) = state.entries.lock() {
            entries.remove(&name);
        }
        return None;
    };
    if !state.pool.is_current_generation(generation) {
        return None;
    }
    if let Ok(mut entries) = state.entries.lock() {
        if !state.pool.is_current_generation(generation) {
            return None;
        }
        let previous = entries.get(&name);
        if entry.status.is_none()
            && let Some(previous) = previous
            && let Some(status) = previous.entry.status.clone()
        {
            entry.status = Some(status);
            entry.observed_at = previous.entry.observed_at.clone();
            entry.stale = true;
        }
        let started_at = previous.map_or(now, |cached| cached.started_at);
        let observed_at = if entry.error.is_none() {
            Some(now)
        } else {
            previous.and_then(|cached| cached.observed_at)
        };
        entry.connection_state =
            monitor_connection_state(now, started_at, observed_at, entry.error.is_some());
        let cached = entries.entry(name).or_insert_with(|| CachedMonitor {
            next_poll: now,
            sampled_at: now,
            started_at,
            observed_at,
            backoff: monitor::Backoff::new(monitor::DEFAULT_INTERVAL),
            entry: entry.clone(),
        });
        let delay = cached.backoff.after_result(entry.error.is_none());
        cached.next_poll = now + delay;
        cached.sampled_at = Instant::now();
        cached.observed_at = observed_at;
        cached.entry = entry.clone();
    }
    Some(entry)
}

fn monitor_connection_state(
    now: Instant,
    started_at: Instant,
    observed_at: Option<Instant>,
    failed: bool,
) -> MonitorConnectionState {
    if !failed {
        return MonitorConnectionState::Live;
    }
    match observed_at {
        Some(observed) if now.saturating_duration_since(observed) < MONITOR_OFFLINE_AFTER => {
            MonitorConnectionState::Recovering
        }
        Some(_) => MonitorConnectionState::Offline,
        None if now.saturating_duration_since(started_at) < MONITOR_OFFLINE_AFTER => {
            MonitorConnectionState::Synchronizing
        }
        None => MonitorConnectionState::Offline,
    }
}

fn monitor_printer(
    pool: &ConnectionPool,
    name: String,
    profile: Profile,
    generation: u64,
) -> Option<MonitorEntry> {
    let driver = profile.driver.clone();
    let driver_profile = match drivers::profile(&profile) {
        Ok(profile) => profile,
        Err(_) => {
            return Some(MonitorEntry {
                name,
                driver,
                status: None,
                error: Some(CommandError::new("profileInvalid")),
                stale: true,
                connection_state: MonitorConnectionState::Offline,
                observed_at: None,
            });
        }
    };
    let kind = driver_profile.driver();
    if !kind.supports(Operation::Status) {
        return Some(MonitorEntry {
            name,
            driver,
            status: None,
            error: Some(CommandError::of("driverUnsupported", Operation::Status)),
            stale: true,
            connection_state: MonitorConnectionState::Offline,
            observed_at: None,
        });
    }
    let access_code = match access_code(&profile.driver, &name, kind) {
        Ok(access_code) => access_code,
        Err(error) => {
            return Some(MonitorEntry {
                name,
                driver,
                status: None,
                error: Some(error),
                stale: true,
                connection_state: MonitorConnectionState::Offline,
                observed_at: None,
            });
        }
    };
    let tls_fingerprint = match tls_fingerprint(&profile.driver, &name, kind, profile.insecure) {
        Ok(fingerprint) => fingerprint,
        Err(error) => {
            return Some(MonitorEntry {
                name,
                driver,
                status: None,
                error: Some(error),
                stale: true,
                connection_state: MonitorConnectionState::Offline,
                observed_at: None,
            });
        }
    };
    let status = pool.status_if_current(
        generation,
        &name,
        &driver_profile,
        access_code.as_deref(),
        tls_fingerprint.as_deref(),
    )?;
    Some(match status {
        Ok(status) => MonitorEntry {
            name,
            driver,
            status: Some(status),
            error: None,
            stale: false,
            connection_state: MonitorConnectionState::Live,
            observed_at: time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .ok(),
        },
        Err(error) => MonitorEntry {
            name,
            driver,
            status: None,
            error: Some(operation_error(error, Operation::Status)),
            stale: true,
            connection_state: MonitorConnectionState::Synchronizing,
            observed_at: None,
        },
    })
}

#[tauri::command(async)]
fn create_configured_printer(
    request: profiles::CreateRequest,
) -> Result<profiles::CreateResult, CommandError> {
    let dir = config_dir().map_err(|_| unreadable_config())?;
    profiles::create(dir, &SystemKeychain, request).map_err(create_error)
}

#[tauri::command(async)]
fn update_configured_printer(
    name: String,
    request: profiles::CreateRequest,
    state: tauri::State<'_, MonitorState>,
    firmware_state: tauri::State<'_, FirmwareUpdateState>,
) -> Result<profiles::UpdateResult, CommandError> {
    let dir = config_dir().map_err(|_| unreadable_config())?;
    let result = profiles::update(dir, &SystemKeychain, &name, request).map_err(create_error)?;
    state.pool.remove(&name);
    invalidate(&state, &name);
    invalidate_firmware_updates(&firmware_state, &name);
    if result.name != name {
        state.pool.remove(&result.name);
        invalidate(&state, &result.name);
        invalidate_firmware_updates(&firmware_state, &result.name);
    }
    Ok(result)
}

fn create_error(error: profiles::ProfileError) -> CommandError {
    CommandError::new(match error {
        profiles::ProfileError::MissingName => "profileMissingName",
        profiles::ProfileError::InvalidName => "profileInvalidName",
        profiles::ProfileError::InvalidHost => "profileInvalidHost",
        profiles::ProfileError::InvalidAccessCode => "profileInvalidAccessCode",
        profiles::ProfileError::InvalidTlsFingerprint => "profileInvalidFingerprint",
        profiles::ProfileError::MissingAccessCode => "profileMissingAccessCode",
        profiles::ProfileError::Config(ConfigError::ProfileAlreadyExists) => "profileExists",
        profiles::ProfileError::Driver(error) => {
            return operation_error(error, Operation::Verify);
        }
        profiles::ProfileError::Secret(_) | profiles::ProfileError::RollbackFailed => {
            "keychainFailed"
        }
        profiles::ProfileError::Config(_) => "configUnwritable",
        profiles::ProfileError::NotFound(_) => "profileNotFound",
    })
}

#[tauri::command(async)]
fn refresh_printer_tls(
    request: TlsRefreshRequest,
    state: tauri::State<'_, MonitorState>,
    firmware_state: tauri::State<'_, FirmwareUpdateState>,
) -> Result<profiles::TlsRefreshResult, CommandError> {
    if !request.confirmed {
        return Err(CommandError::new("tlsUnconfirmed"));
    }
    let dir = config_dir().map_err(|_| unreadable_config())?;
    let result =
        profiles::store_tls_fingerprint(dir, &SystemKeychain, &request.name, &request.fingerprint)
            .map_err(create_error)?;
    state.pool.remove(&request.name);
    invalidate(&state, &request.name);
    invalidate_firmware_updates(&firmware_state, &request.name);
    Ok(result)
}

#[tauri::command(async)]
fn preview_printer_tls(name: String) -> Result<String, CommandError> {
    let dir = config_dir().map_err(|_| unreadable_config())?;
    profiles::preview_tls(dir, &name, None).map_err(create_error)
}

#[tauri::command(async)]
fn printer_files(request: FileListRequest) -> Result<moonraker::FileList, CommandError> {
    let printer = desktop_printer(&request.name, Operation::FileList)?;
    let mut files = drivers::file_list(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &request.path,
        false,
    )
    .map_err(|error| operation_error(error, Operation::FileList))?;
    let search = request.search.trim().to_ascii_lowercase();
    if !search.is_empty() {
        files
            .entries
            .retain(|entry| entry.name.to_ascii_lowercase().contains(&search));
    }
    files.entries.sort_by(|left, right| {
        let left_rank = (left.entry_type != moonraker::FileEntryType::Directory) as u8;
        let right_rank = (right.entry_type != moonraker::FileEntryType::Directory) as u8;
        left_rank
            .cmp(&right_rank)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(files)
}

#[tauri::command(async)]
fn library_files(
    request: LibraryListRequest,
    app: tauri::AppHandle,
) -> Result<LibraryListResponse, CommandError> {
    let absolute = if request.path.trim().is_empty() {
        default_library_dir(&app)?
    } else {
        resolve_absolute_path(&request.path)?
    };
    let read_dir =
        std::fs::read_dir(&absolute).map_err(|_| CommandError::new("libraryUnavailable"))?;
    let mut entries = Vec::new();
    for entry in read_dir {
        let Ok(entry) = entry else { continue };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let entry_type = if metadata.is_dir() {
            moonraker::FileEntryType::Directory
        } else {
            moonraker::FileEntryType::File
        };
        let path = entry.path().to_string_lossy().into_owned();
        let media_type = moonraker::infer_file_media_type(&path);
        entries.push(moonraker::FileEntry {
            name,
            root: "library",
            device_path: path.clone(),
            path,
            entry_type,
            media_type,
            size_bytes: (!metadata.is_dir()).then_some(metadata.len() as i64),
            modified_at: metadata.modified().ok().and_then(format_modified),
            metadata: BTreeMap::new(),
        });
    }
    entries.sort_by(|left, right| {
        let left_rank = (left.entry_type != moonraker::FileEntryType::Directory) as u8;
        let right_rank = (right.entry_type != moonraker::FileEntryType::Directory) as u8;
        left_rank.cmp(&right_rank).then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
    });
    watch_library_dir(&app, &absolute);
    Ok(LibraryListResponse {
        parent: absolute
            .parent()
            .map(|parent| parent.to_string_lossy().into_owned()),
        breadcrumbs: library_breadcrumbs(&absolute),
        path: absolute.to_string_lossy().into_owned(),
        entries,
    })
}

#[tauri::command(async)]
fn library_delete_file(request: LibraryDeleteRequest) -> Result<(), CommandError> {
    let absolute = resolve_absolute_path(&request.path)?;
    std::fs::remove_file(&absolute).map_err(|_| CommandError::new("libraryUnavailable"))
}

#[tauri::command(async)]
fn library_add_file(
    request: LibraryAddRequest,
    app: tauri::AppHandle,
) -> Result<LibraryListResponse, CommandError> {
    let directory = resolve_absolute_path(&request.directory)?;
    let source = std::path::Path::new(&request.source);
    let filename = source
        .file_name()
        .ok_or_else(|| CommandError::new("libraryPathInvalid"))?;
    let destination = directory.join(filename);
    std::fs::copy(source, &destination).map_err(|_| CommandError::new("libraryUnavailable"))?;
    library_files(
        LibraryListRequest {
            path: directory.to_string_lossy().into_owned(),
        },
        app,
    )
}

/// Persists the folder the file library last browsed, so it reopens there
/// on the next launch instead of always resetting to the default location.
#[tauri::command(async)]
fn set_library_path(
    path: String,
    state: tauri::State<'_, PreferencesState>,
) -> Result<(), CommandError> {
    let absolute = resolve_absolute_path(&path)?;
    if !absolute.is_dir() {
        return Err(CommandError::new("libraryPathInvalid"));
    }
    let _guard = state
        .write_lock
        .lock()
        .map_err(|_| CommandError::new("preferencesUnwritable"))?;
    let dir = preferences::preferences_dir().map_err(preferences_error)?;
    let mut stored = preferences::Preferences::open(&dir).map_err(preferences_error)?;
    stored.library_path = Some(absolute.to_string_lossy().into_owned());
    stored.save(&dir).map_err(preferences_error)
}

#[tauri::command(async)]
fn inspect_library_print(
    request: LibraryInspectRequest,
) -> Result<melt_core::bambu::PrintPackage, CommandError> {
    let absolute = resolve_absolute_path(&request.path)?;
    melt_core::bambu::inspect_print_package(&absolute)
        .map_err(|_| CommandError::new("jobFileInvalid"))
}

#[tauri::command(async)]
fn print_library_file(
    request: LibraryPrintRequest,
    state: tauri::State<'_, MonitorState>,
    app: tauri::AppHandle,
) -> Result<moonraker::JobResult, CommandError> {
    let absolute = resolve_absolute_path(&request.path)?;
    let source_filename = absolute
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CommandError::new("libraryPathInvalid"))?;
    let (generation, printer) =
        pooled_desktop_printer(&state, &request.printer, Operation::FileUpload)?;
    let mut options = melt_core::bambu::JobStartOptions::default();
    let device_path = if printer.driver.driver() == drivers::Driver::BambuLan
        && source_filename.to_ascii_lowercase().ends_with(".3mf")
    {
        let inventory = state
            .entries
            .lock()
            .ok()
            .and_then(|entries| entries.get(&request.printer)?.entry.status.clone())
            .and_then(|status| status.extensions.bambu_lan)
            .and_then(|extension| extension.ams);
        let preflight = melt_core::bambu::preflight_print_package(
            &absolute,
            request.display_name.as_deref(),
            request.plate,
            inventory.as_ref(),
        )
        .map_err(|_| CommandError::new("jobFileInvalid"))?;
        if !preflight.ready {
            return Err(CommandError::new("jobFileInvalid"));
        }
        options.plate = Some(preflight.selected_plate.index);
        options.display_name = Some(preflight.names.display_name.clone());
        if let Some(mapping) = &preflight.filament_mapping {
            options.use_ams = !mapping.ams_mapping.is_empty();
            options.ams_mapping.clone_from(&mapping.ams_mapping);
            options.nozzle_mapping.clone_from(&mapping.nozzle_mapping);
        }
        let _ = app.emit(
            "print-stage",
            melt_core::bambu::PrintStageEvent {
                stage: melt_core::bambu::PrintStage::Inspect,
                percent: Some(10),
                bytes_transferred: None,
                detail: Some(preflight.names.display_name),
            },
        );
        format!("/{}", preflight.names.remote_filename)
    } else {
        format!("/{source_filename}")
    };
    let _ = app.emit(
        "print-stage",
        melt_core::bambu::PrintStageEvent {
            stage: melt_core::bambu::PrintStage::Upload,
            percent: Some(30),
            bytes_transferred: Some(0),
            detail: Some(device_path.clone()),
        },
    );
    drivers::upload_file(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &absolute,
        &device_path,
        true,
    )
    .map_err(|error| operation_error(error, Operation::FileUpload))?;
    let _ = app.emit(
        "print-stage",
        melt_core::bambu::PrintStageEvent {
            stage: melt_core::bambu::PrintStage::SendCommand,
            percent: Some(80),
            bytes_transferred: None,
            detail: None,
        },
    );
    ensure_state(
        &state,
        generation,
        &printer,
        &[moonraker::PrinterState::Idle],
        Operation::JobStart,
    )?;
    let result = current_pool_operation(&state, generation, Operation::JobStart, |pool| {
        pool.job_start(
            &request.printer,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            &device_path,
            options,
        )
    });
    invalidate(&state, &request.printer);
    if result.is_ok() {
        let _ = app.emit(
            "print-stage",
            melt_core::bambu::PrintStageEvent {
                stage: melt_core::bambu::PrintStage::Finished,
                percent: Some(100),
                bytes_transferred: None,
                detail: None,
            },
        );
    }
    result
}

#[tauri::command(async)]
fn printer_job_action(
    request: JobActionRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::JobResult, CommandError> {
    if !request.confirmed {
        return Err(CommandError::new("actionUnconfirmed"));
    }
    let operation = match request.action.as_str() {
        "start" => Operation::JobStart,
        "pause" => Operation::JobPause,
        "resume" => Operation::JobResume,
        "cancel" => Operation::JobCancel,
        _ => return Err(CommandError::new("actionUnsupported")),
    };
    let (generation, printer) = pooled_desktop_printer(&state, &request.name, operation)?;
    let allowed = match request.action.as_str() {
        "start" => &[moonraker::PrinterState::Idle][..],
        "pause" => &[moonraker::PrinterState::Printing][..],
        "resume" => &[moonraker::PrinterState::Paused][..],
        "cancel" => &[
            moonraker::PrinterState::Printing,
            moonraker::PrinterState::Paused,
        ][..],
        _ => unreachable!("action is validated above"),
    };
    ensure_state(&state, generation, &printer, allowed, operation)?;
    let device_path = if request.action == "start" {
        Some(
            request
                .device_path
                .as_deref()
                .filter(|path| !path.is_empty())
                .ok_or_else(|| CommandError::new("jobFileMissing"))?,
        )
    } else {
        None
    };
    let result = current_pool_operation(&state, generation, operation, |pool| {
        match request.action.as_str() {
            "start" => pool.job_start(
                &request.name,
                &printer.driver,
                printer.access_code.as_deref(),
                printer.tls_fingerprint.as_deref(),
                device_path.expect("start device path was validated"),
                request.options.clone(),
            ),
            "pause" => pool.job_pause(
                &request.name,
                &printer.driver,
                printer.access_code.as_deref(),
                printer.tls_fingerprint.as_deref(),
            ),
            "resume" => pool.job_resume(
                &request.name,
                &printer.driver,
                printer.access_code.as_deref(),
                printer.tls_fingerprint.as_deref(),
            ),
            "cancel" => pool.job_cancel(
                &request.name,
                &printer.driver,
                printer.access_code.as_deref(),
                printer.tls_fingerprint.as_deref(),
            ),
            _ => unreachable!("action is validated above"),
        }
    });
    invalidate(&state, &request.name);
    result
}

#[tauri::command(async)]
fn printer_temperature_set(
    request: TemperatureRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::TemperatureResult, CommandError> {
    if let (Some(temperature), Some(target_celsius)) =
        (request.temperature.as_deref(), request.target_celsius)
    {
        let (generation, printer) =
            pooled_desktop_printer(&state, &request.name, Operation::TemperatureSet)?;
        ensure_state(
            &state,
            generation,
            &printer,
            &[moonraker::PrinterState::Idle],
            Operation::TemperatureSet,
        )?;
        let result =
            current_pool_operation(&state, generation, Operation::TemperatureSet, |pool| {
                pool.temperature_set_item(
                    &request.name,
                    &printer.driver,
                    printer.access_code.as_deref(),
                    printer.tls_fingerprint.as_deref(),
                    temperature,
                    target_celsius,
                )
            });
        invalidate(&state, &request.name);
        return result;
    }
    if request.temperature.is_some() || request.target_celsius.is_some() {
        return Err(CommandError::new("temperatureTargetMissing"));
    }
    let targets = moonraker::TemperatureTargets {
        nozzle_celsius: request.nozzle_celsius,
        bed_celsius: request.bed_celsius,
        chamber_celsius: request.chamber_celsius,
    };
    if targets.nozzle_celsius.is_none()
        && targets.bed_celsius.is_none()
        && targets.chamber_celsius.is_none()
    {
        return Err(CommandError::new("temperatureTargetMissing"));
    }
    let (generation, printer) =
        pooled_desktop_printer(&state, &request.name, Operation::TemperatureSet)?;
    ensure_state(
        &state,
        generation,
        &printer,
        &[moonraker::PrinterState::Idle],
        Operation::TemperatureSet,
    )?;
    let result = current_pool_operation(&state, generation, Operation::TemperatureSet, |pool| {
        pool.temperature_set(
            &request.name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            targets,
        )
    });
    invalidate(&state, &request.name);
    result
}

#[tauri::command(async)]
fn printer_fan_set(
    request: FanRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::FanResult, CommandError> {
    let (generation, printer) = pooled_desktop_printer(&state, &request.name, Operation::FanSet)?;
    ensure_state(
        &state,
        generation,
        &printer,
        &[
            moonraker::PrinterState::Idle,
            moonraker::PrinterState::Printing,
            moonraker::PrinterState::Paused,
            moonraker::PrinterState::Error,
        ],
        Operation::FanSet,
    )?;
    let result = current_pool_operation(&state, generation, Operation::FanSet, |pool| {
        pool.fan_set(
            &request.name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            &request.fan,
            request.speed_percent,
        )
    });
    invalidate(&state, &request.name);
    result
}

#[tauri::command(async)]
fn printer_motion_home(
    name: String,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::MotionResult, CommandError> {
    let (generation, printer) = pooled_desktop_printer(&state, &name, Operation::MotionHome)?;
    ensure_state(
        &state,
        generation,
        &printer,
        &[moonraker::PrinterState::Idle],
        Operation::MotionHome,
    )?;
    let result = current_pool_operation(&state, generation, Operation::MotionHome, |pool| {
        pool.motion_home(
            &name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            &[moonraker::Axis::X, moonraker::Axis::Y, moonraker::Axis::Z],
        )
    });
    invalidate(&state, &name);
    result
}

#[tauri::command(async)]
fn printer_motion_jog(
    request: JogRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::MotionResult, CommandError> {
    if request.x_millimeters.is_none()
        && request.y_millimeters.is_none()
        && request.z_millimeters.is_none()
    {
        return Err(CommandError::new("jogTargetMissing"));
    }
    let (generation, printer) =
        pooled_desktop_printer(&state, &request.name, Operation::MotionJog)?;
    ensure_state(
        &state,
        generation,
        &printer,
        &[moonraker::PrinterState::Idle],
        Operation::MotionJog,
    )?;
    let result = current_pool_operation(&state, generation, Operation::MotionJog, |pool| {
        pool.motion_jog(
            &request.name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            moonraker::JogDelta {
                x_millimeters: request.x_millimeters,
                y_millimeters: request.y_millimeters,
                z_millimeters: request.z_millimeters,
                feedrate_mm_per_min: request.feedrate_mm_per_min,
            },
        )
    });
    invalidate(&state, &request.name);
    result
}

#[tauri::command(async)]
fn printer_light_set(
    request: LightRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::LightResult, CommandError> {
    let (generation, printer) = pooled_desktop_printer(&state, &request.name, Operation::LightSet)?;
    ensure_state(
        &state,
        generation,
        &printer,
        &[
            moonraker::PrinterState::Idle,
            moonraker::PrinterState::Printing,
            moonraker::PrinterState::Paused,
            moonraker::PrinterState::Error,
        ],
        Operation::LightSet,
    )?;
    let light_state = if request.on {
        moonraker::LightState::On
    } else {
        moonraker::LightState::Off
    };
    let result = current_pool_operation(&state, generation, Operation::LightSet, |pool| {
        pool.light_set(
            &request.name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            &request.light,
            light_state,
        )
    });
    invalidate(&state, &request.name);
    result
}

#[tauri::command(async)]
fn printer_ams_drying_set(
    request: AmsDryingRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::DryingResult, CommandError> {
    let (generation, printer) =
        pooled_desktop_printer(&state, &request.name, Operation::AmsDrying)?;
    let result = current_pool_operation(&state, generation, Operation::AmsDrying, |pool| {
        pool.ams_drying_set(
            &request.name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            request.ams_id,
            &request.drying,
        )
    });
    invalidate(&state, &request.name);
    result
}

#[tauri::command(async)]
fn printer_speed_set(
    request: SpeedRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::SpeedResult, CommandError> {
    let (generation, printer) = pooled_desktop_printer(&state, &request.name, Operation::SpeedSet)?;
    ensure_state(
        &state,
        generation,
        &printer,
        &[
            moonraker::PrinterState::Printing,
            moonraker::PrinterState::Paused,
        ],
        Operation::SpeedSet,
    )?;
    let result = current_pool_operation(&state, generation, Operation::SpeedSet, |pool| {
        pool.speed_set(
            &request.name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            &request.speed_profile,
        )
    });
    invalidate(&state, &request.name);
    result
}

#[tauri::command(async)]
fn printer_file_download(
    request: FileDownloadRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, TransferState>,
) -> Result<u64, CommandError> {
    let printer = desktop_printer(&request.name, Operation::FileDownload)?;
    let destination = std::path::Path::new(&request.destination);
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| CommandError::of("fileDestinationUnwritable", Operation::FileDownload))?;
    let transfer_id = request
        .transfer_id
        .unwrap_or_else(|| format!("download:{}", request.device_path));
    {
        use tauri::Emitter;
        let _ = app.emit(
            "transfer-progress",
            TransferProgress {
                transfer_id: transfer_id.clone(),
                direction: "download",
                bytes_transferred: 0,
                total_bytes: request.total_bytes,
                complete: false,
            },
        );
    }
    let mut progress = ProgressWriter {
        destination: temporary.as_file_mut(),
        app: app.clone(),
        state: state.inner().clone(),
        transfer_id: transfer_id.clone(),
        total_bytes: request.total_bytes,
        transferred: 0,
        last_report: 0,
    };
    let result = drivers::download_to(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &request.device_path,
        &mut progress,
    )
    .map_err(|error| operation_error(error, Operation::FileDownload));
    drop(progress);
    match result {
        Ok(bytes) => {
            let actual = temporary
                .as_file()
                .metadata()
                .map(|metadata| metadata.len())
                .map_err(|_| CommandError::new("fileIntegrityFailed"))?;
            if actual != bytes
                || request
                    .total_bytes
                    .is_some_and(|expected| expected != actual)
            {
                return Err(CommandError::new("fileIntegrityFailed"));
            }
            temporary
                .as_file_mut()
                .sync_all()
                .map_err(|_| CommandError::new("fileIntegrityFailed"))?;
            temporary.persist(destination).map_err(|_| {
                CommandError::of("fileDestinationUnwritable", Operation::FileDownload)
            })?;
            use tauri::Emitter;
            let _ = app.emit(
                "transfer-progress",
                TransferProgress {
                    transfer_id: transfer_id.clone(),
                    direction: "download",
                    bytes_transferred: bytes,
                    total_bytes: request.total_bytes,
                    complete: true,
                },
            );
            if let Ok(mut canceled) = state.canceled.lock() {
                canceled.remove(&transfer_id);
            }
            Ok(bytes)
        }
        Err(error) => Err(error),
    }
}

#[tauri::command(async)]
fn cancel_file_transfer(
    transfer_id: String,
    state: tauri::State<'_, TransferState>,
) -> Result<(), CommandError> {
    state
        .canceled
        .lock()
        .map_err(|_| CommandError::new("transferUnavailable"))?
        .insert(transfer_id);
    Ok(())
}

#[tauri::command(async)]
fn printer_file_upload(request: FileUploadRequest) -> Result<u64, CommandError> {
    let printer = desktop_printer(&request.name, Operation::FileUpload)?;
    drivers::upload_file(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        std::path::Path::new(&request.source),
        &request.device_path,
        request.overwrite,
    )
    .map_err(|error| operation_error(error, Operation::FileUpload))
}

#[tauri::command(async)]
fn printer_file_delete(
    request: FileDeleteRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<(), CommandError> {
    let printer = desktop_printer(&request.name, Operation::FileDelete)?;
    drivers::delete_file(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &request.device_path,
    )
    .map_err(|error| operation_error(error, Operation::FileDelete))?;
    invalidate(&state, &request.name);
    Ok(())
}

#[tauri::command(async)]
fn printer_file_preview(
    request: FilePreviewRequest,
    state: tauri::State<'_, PreviewState>,
) -> Result<FilePreviewResponse, CommandError> {
    let printer = desktop_printer(&request.name, Operation::FileDownload)?;
    let cache_key = preview_cache_key(
        &request,
        &printer.driver.physical_printer_key(&printer.name),
    );
    if let Some(preview) = lookup_preview_cache(&state, &cache_key) {
        return Ok(preview);
    }
    let extension = request
        .device_path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "3mf" | "stl" | "obj") {
        return Err(CommandError::new("thumbnailUnsupported"));
    }
    // Bounds concurrent renders so opening a folder with many thumbnails
    // doesn't peg every core at once; a disk-cache hit above never reaches
    // this, so cached folders stay instant regardless of the limit.
    let _permit = RENDER_SEMAPHORE.acquire();
    if extension == "3mf" {
        let mut thumbnail = Vec::new();
        if drivers::file_thumbnail_to(
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            &request.device_path,
            1,
            &mut thumbnail,
        )
        .is_ok()
            && preview::validate_png(&thumbnail)
        {
            let preview = FilePreviewResponse {
                kind: "png",
                data: STANDARD.encode(thumbnail),
            };
            cache_preview(&state, &cache_key, &preview, true);
            return Ok(preview);
        }
    }
    let mut bytes = Vec::new();
    drivers::download_to(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &request.device_path,
        &mut bytes,
    )
    .map_err(|error| operation_error(error, Operation::FileDownload))?;
    render_preview(&extension, bytes, cache_key, &state, preview::GRID_SIZE)
}

#[tauri::command(async)]
fn library_file_preview(
    request: LibraryPreviewRequest,
    state: tauri::State<'_, PreviewState>,
) -> Result<FilePreviewResponse, CommandError> {
    let size = if request.large {
        preview::LARGE_SIZE
    } else {
        preview::GRID_SIZE
    };
    // ponytail: a 3mf's embedded thumbnail is size-independent, so it lands
    // under both keys. Duplicating ~50 KB beats branching the cache lookup on
    // a fact only the renderer downstream knows.
    let cache_key = format!(
        "{}:library:{}:{}:{}:{}x{}",
        PREVIEW_CACHE_VERSION,
        request.path,
        request.size_bytes.unwrap_or_default(),
        request.modified_at.as_deref().unwrap_or_default(),
        size.0,
        size.1
    );
    if let Some(preview) = lookup_preview_cache(&state, &cache_key) {
        return Ok(preview);
    }
    let extension = request
        .path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "3mf" | "stl" | "obj" | "zip") {
        return Err(CommandError::new("thumbnailUnsupported"));
    }
    let _permit = RENDER_SEMAPHORE.acquire();
    let absolute = resolve_absolute_path(&request.path)?;
    if extension == "zip" {
        let archive =
            std::fs::File::open(&absolute).map_err(|_| CommandError::new("libraryUnavailable"))?;
        let (model_extension, bytes) = preview::first_model_in_zip(archive).map_err(|error| {
            CommandError::new(match error {
                preview::PreviewError::TooLarge => "thumbnailTooLarge",
                _ => "thumbnailInvalid",
            })
        })?;
        return render_preview(&model_extension, bytes, cache_key, &state, size);
    }
    // A local file is seekable, so a 3mf's embedded thumbnail (present on
    // nearly every slicer-exported file) can be read directly from disk
    // without loading the whole model into memory first.
    if extension == "3mf" {
        if let Some(thumbnail) = std::fs::File::open(&absolute)
            .ok()
            .and_then(extract_3mf_thumbnail)
        {
            let preview = FilePreviewResponse {
                kind: "png",
                data: STANDARD.encode(thumbnail),
            };
            cache_preview(&state, &cache_key, &preview, size == preview::GRID_SIZE);
            return Ok(preview);
        }
    }
    // Check the size before reading so an oversized model never lands in
    // memory; `take` also bounds a file that grows after the metadata check.
    let file =
        std::fs::File::open(&absolute).map_err(|_| CommandError::new("libraryUnavailable"))?;
    let length = file
        .metadata()
        .map_err(|_| CommandError::new("libraryUnavailable"))?
        .len();
    if length > preview::MAX_MODEL_BYTES as u64 {
        return Err(CommandError::new("thumbnailTooLarge"));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(preview::MAX_MODEL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CommandError::new("libraryUnavailable"))?;
    render_preview(&extension, bytes, cache_key, &state, size)
}

/// Bounds concurrent thumbnail work (disk-cache reads and CPU-heavy
/// renders alike) so opening a folder full of models doesn't try to do it
/// all at once. A plain blocking semaphore, since preview generation is
/// synchronous CPU work regardless of the `async` command wrapper.
struct Semaphore {
    count: Mutex<usize>,
    available: Condvar,
    max: usize,
}

struct SemaphorePermit<'a>(&'a Semaphore);

impl Semaphore {
    fn acquire(&self) -> SemaphorePermit<'_> {
        let mut count = self.count.lock().unwrap_or_else(|error| error.into_inner());
        while *count >= self.max {
            count = self
                .available
                .wait(count)
                .unwrap_or_else(|error| error.into_inner());
        }
        *count += 1;
        SemaphorePermit(self)
    }
}

impl Drop for SemaphorePermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut count) = self.0.count.lock() {
            *count -= 1;
            self.0.available.notify_one();
        }
    }
}

static RENDER_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore {
    count: Mutex::new(0),
    available: Condvar::new(),
    max: 2,
});

/// Checks the in-memory cache, then falls back to the on-disk thumbnail
/// cache (persisted across app restarts), populating memory on a disk hit.
fn lookup_preview_cache(state: &PreviewState, cache_key: &str) -> Option<FilePreviewResponse> {
    if let Ok(mut cache) = state.cache.lock() {
        if let Some(preview) = cache.get(cache_key) {
            return Some(preview);
        }
    }
    let preview = read_disk_preview_cache(cache_key)?;
    if let Ok(mut cache) = state.cache.lock() {
        cache.insert(cache_key.to_owned(), preview.clone());
    }
    Some(preview)
}

fn cache_preview(
    state: &PreviewState,
    cache_key: &str,
    preview: &FilePreviewResponse,
    persist: bool,
) {
    if let Ok(mut cache) = state.cache.lock() {
        cache.insert(cache_key.to_owned(), preview.clone());
    }
    if persist {
        write_disk_preview_cache(cache_key, preview);
    }
}

/// Thumbnails cache to disk keyed by a hash of (path, size, mtime) — encoded
/// into `cache_key` by the caller — so re-opening the same folder later, or
/// after restarting the app, never re-pays render cost for an unchanged
/// file. Cache misses or write failures are silently ignored: a thumbnail
/// is a nice-to-have, never worth failing the request over.
fn preview_cache_dir() -> Option<std::path::PathBuf> {
    Some(preferences::preferences_dir().ok()?.join("thumbnails"))
}

fn preview_cache_path(cache_key: &str) -> Option<std::path::PathBuf> {
    let digest = Sha256::digest(cache_key.as_bytes());
    Some(preview_cache_dir()?.join(format!("{digest:x}.png")))
}

fn read_disk_preview_cache(cache_key: &str) -> Option<FilePreviewResponse> {
    let bytes = std::fs::read(preview_cache_path(cache_key)?).ok()?;
    if !preview::validate_png(&bytes) {
        return None;
    }
    Some(FilePreviewResponse {
        kind: "png",
        data: STANDARD.encode(bytes),
    })
}

fn write_disk_preview_cache(cache_key: &str, preview: &FilePreviewResponse) {
    if preview.kind != "png" {
        return;
    }
    let Some(path) = preview_cache_path(cache_key) else {
        return;
    };
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if let Ok(bytes) = STANDARD.decode(&preview.data) {
        let _ = std::fs::write(path, bytes);
    }
}

/// Scans every PNG in the bundle and prefers one with "thumbnail" in its
/// name, falling back to the first PNG found — slicers don't all use the
/// same handful of well-known paths, so matching only exact names like
/// `Metadata/thumbnail.png` silently missed real files.
fn extract_3mf_thumbnail(reader: impl Read + std::io::Seek) -> Option<Vec<u8>> {
    let mut archive = zip::ZipArchive::new(reader).ok()?;
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let mut fallback = None;
    let mut chosen = None;
    for name in names {
        if !name.to_ascii_lowercase().ends_with(".png") {
            continue;
        }
        if name.to_ascii_lowercase().contains("thumbnail") {
            chosen = Some(name);
            break;
        }
        fallback.get_or_insert(name);
    }
    let name = chosen.or(fallback)?;
    let entry = archive.by_name(&name).ok()?;
    if entry.size() > preview::MAX_PREVIEW_BYTES as u64 {
        return None;
    }
    let mut thumbnail = Vec::new();
    entry
        .take(preview::MAX_PREVIEW_BYTES as u64 + 1)
        .read_to_end(&mut thumbnail)
        .ok()?;
    preview::validate_png(&thumbnail).then_some(thumbnail)
}

/// Renders (and caches) a model thumbnail from already-in-memory file bytes,
/// shared by the printer-hosted and local-library preview commands, which
/// differ only in how they obtain those bytes. The whole-file size only
/// gates the CPU-heavy rasterizer fallback, not the (cheap) embedded-3mf-
/// thumbnail lookup, so a large sliced project file with a baked-in
/// thumbnail still previews instantly instead of being rejected outright.
fn render_preview(
    extension: &str,
    bytes: Vec<u8>,
    cache_key: String,
    state: &PreviewState,
    size: (usize, usize),
) -> Result<FilePreviewResponse, CommandError> {
    // Large renders are session-scoped: a 1920x1080 PNG is 1-3 MB against
    // roughly 50 KB for a grid thumbnail, and the on-disk cache has no
    // eviction policy to keep a big library from filling the disk.
    let persist = size == preview::GRID_SIZE;
    if extension == "3mf" {
        // A slicer's baked-in thumbnail is its own render, with plate,
        // filament colours and supports. Our flat-shaded rasterization would
        // be sharper at the large size and a worse picture, so the embedded
        // one wins at every size.
        if let Some(thumbnail) = extract_3mf_thumbnail(Cursor::new(&bytes)) {
            let preview = FilePreviewResponse {
                kind: "png",
                data: STANDARD.encode(thumbnail),
            };
            cache_preview(state, &cache_key, &preview, persist);
            return Ok(preview);
        }
        if bytes.len() > preview::MAX_MODEL_BYTES {
            return Err(CommandError::new("thumbnailTooLarge"));
        }
        let png = preview::rasterize_3mf(&bytes, size).map_err(|error| {
            CommandError::new(match error {
                preview::PreviewError::TooLarge => "thumbnailTooLarge",
                _ => "thumbnailInvalid",
            })
        })?;
        let preview = FilePreviewResponse {
            kind: "png",
            data: STANDARD.encode(png),
        };
        cache_preview(state, &cache_key, &preview, persist);
        return Ok(preview);
    }
    if matches!(extension, "stl" | "obj") {
        if bytes.len() > preview::MAX_MODEL_BYTES {
            return Err(CommandError::new("thumbnailTooLarge"));
        }
        let png = preview::rasterize(extension, &bytes, size)
            .map_err(|_| CommandError::new("thumbnailInvalid"))?;
        let preview = FilePreviewResponse {
            kind: "png",
            data: STANDARD.encode(png),
        };
        cache_preview(state, &cache_key, &preview, persist);
        return Ok(preview);
    }
    Ok(FilePreviewResponse {
        kind: "model",
        data: STANDARD.encode(bytes),
    })
}

#[tauri::command(async)]
fn open_file_with_slicer(
    request: OpenWithSlicerRequest,
    app: tauri::AppHandle,
    state: tauri::State<'_, SlicerState>,
) -> Result<(), CommandError> {
    let preferences = preferences::Preferences::load().map_err(preferences_error)?;
    // Auto-detected slicers only exist in this merged view until a user
    // explicitly saves one, but the frontend's list (and thus every "open
    // with" menu entry) is built from the same merge, so it must be used
    // here too or every detected-but-unsaved slicer fails to open.
    let slicers = preferences::merge_slicers(&preferences.slicers, &preferences::detect_slicers());
    let slicer = slicers
        .iter()
        .find(|slicer| slicer.enabled && slicer.name == request.slicer)
        .ok_or_else(|| CommandError::new("slicerNotFound"))?;
    let absolute = resolve_absolute_path(&request.path)?;
    let filename = absolute
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("model")
        .to_owned();
    let child = std::process::Command::new(&slicer.path)
        .arg(&absolute)
        .spawn()
        .map_err(|_| CommandError::new("slicerLaunchFailed"))?;
    let pid = child.id();
    let info = SlicerProcessInfo {
        pid,
        slicer: slicer.name.clone(),
        file_name: filename,
    };
    let child = Arc::new(Mutex::new(child));
    state
        .processes
        .lock()
        .map_err(|_| CommandError::new("slicerLaunchFailed"))?
        .insert(
            pid,
            ManagedSlicer {
                child: child.clone(),
                info,
            },
        );
    let processes = state.inner().clone();
    std::thread::spawn(move || {
        let status = child.lock().ok().and_then(|mut child| child.wait().ok());
        if let Ok(mut processes) = processes.processes.lock() {
            processes.remove(&pid);
        }
        let _ = app.emit("slicer-process-exited", serde_json::json!({ "pid": pid, "success": status.is_some_and(|status| status.success()) }));
    });
    Ok(())
}

#[tauri::command(async)]
fn list_slicer_processes(
    state: tauri::State<'_, SlicerState>,
) -> Result<Vec<SlicerProcessInfo>, CommandError> {
    Ok(state
        .processes
        .lock()
        .map_err(|_| CommandError::new("slicerLaunchFailed"))?
        .values()
        .map(|process| process.info.clone())
        .collect())
}

#[tauri::command(async)]
fn stop_slicer_process(pid: u32, state: tauri::State<'_, SlicerState>) -> Result<(), CommandError> {
    let processes = state
        .processes
        .lock()
        .map_err(|_| CommandError::new("slicerLaunchFailed"))?;
    if let Some(process) = processes.get(&pid) {
        process
            .child
            .lock()
            .map_err(|_| CommandError::new("slicerLaunchFailed"))?
            .kill()
            .map_err(|_| CommandError::new("slicerLaunchFailed"))?;
    }
    Ok(())
}

#[tauri::command(async)]
fn printer_emergency_stop(
    name: String,
    state: tauri::State<'_, MonitorState>,
) -> Result<(), CommandError> {
    let printer = desktop_printer(&name, Operation::EmergencyStop)?;
    let result = state
        .pool
        .emergency_stop(
            &name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
        )
        .map_err(|error| operation_error(error, Operation::EmergencyStop));
    invalidate(&state, &name);
    result
}

#[tauri::command(async)]
fn printer_camera_snapshot(
    name: String,
    destination: Option<String>,
    manager: tauri::State<'_, CameraManager>,
) -> Result<CameraSnapshot, CommandError> {
    let printer = desktop_printer(&name, Operation::CameraSnapshot)?;
    let subscription = subscribe_camera(
        &manager,
        &printer,
        CameraFrameKind::Jpeg,
        Operation::CameraSnapshot,
    )?;
    let image = if let Some(image) = subscription.latest_jpeg() {
        image
    } else {
        loop {
            match subscription.recv_timeout(Duration::from_secs(10)) {
                Ok(frame) => {
                    if let CameraFrame::Jpeg(image) = frame.as_ref() {
                        break Arc::clone(image);
                    }
                }
                Err(_) => return Err(CommandError::new("cameraPreviewUnavailable")),
            }
        }
    };
    if let Some(destination) = destination {
        std::fs::write(&destination, image.as_ref()).map_err(|_| {
            CommandError::of("fileDestinationUnwritable", Operation::CameraSnapshot)
        })?;
    }
    Ok(CameraSnapshot {
        data_url: format!("data:image/jpeg;base64,{}", STANDARD.encode(image.as_ref())),
    })
}

#[tauri::command(async)]
fn printer_camera_stream(
    name: String,
    state: tauri::State<'_, CameraStreamState>,
    manager: tauri::State<'_, CameraManager>,
) -> Result<CameraStream, CommandError> {
    let printer = desktop_printer(&name, Operation::CameraStream)?;
    let owner_key = printer.driver.physical_printer_key(&printer.name);
    let subscription = subscribe_camera(
        &manager,
        &printer,
        CameraFrameKind::Jpeg,
        Operation::CameraStream,
    )?;
    start_camera_server(
        subscription,
        owner_key,
        state.inner().clone(),
        CameraProxyFormat::Mjpeg,
    )
    .map(|url| CameraStream { url })
}

#[tauri::command(async)]
fn printer_camera_h264_stream(
    name: String,
    state: tauri::State<'_, CameraStreamState>,
    manager: tauri::State<'_, CameraManager>,
) -> Result<CameraH264Stream, CommandError> {
    let printer = desktop_printer(&name, Operation::CameraStream)?;
    let owner_key = printer.driver.physical_printer_key(&printer.name);
    let subscription = subscribe_camera(
        &manager,
        &printer,
        CameraFrameKind::H264,
        Operation::CameraStream,
    )?;
    let codec = subscription
        .h264_parameters()
        .and_then(|parameters| h264_codec(&parameters.sps))
        .ok_or_else(|| {
            CommandError::new("cameraPreviewUnavailable")
                .with_detail("camera SDP has no usable H.264 codec parameters")
        })?;
    start_camera_server(
        subscription,
        owner_key,
        state.inner().clone(),
        CameraProxyFormat::H264,
    )
    .map(|url| CameraH264Stream { url, codec })
}

#[tauri::command(async)]
fn printer_camera_stream_stop(
    state: tauri::State<'_, CameraStreamState>,
) -> Result<(), CommandError> {
    let mut active = state
        .active
        .lock()
        .map_err(|_| CommandError::new("cameraPreviewUnavailable"))?;
    for proxy in active.values() {
        proxy.stop.store(true, Ordering::Release);
    }
    active.clear();
    Ok(())
}

#[tauri::command(async)]
fn printer_camera_webrtc_offer(
    name: String,
    offer: String,
    state: tauri::State<'_, CameraWebRtcState>,
    manager: tauri::State<'_, CameraManager>,
) -> Result<CameraWebRtcAnswer, CommandError> {
    let printer = desktop_printer(&name, Operation::CameraStream)?;
    let profile_name = name.to_ascii_lowercase();
    let name = printer.driver.physical_printer_key(&printer.name);
    let generation = state.generation.fetch_add(1, Ordering::AcqRel) + 1;
    {
        let mut sessions = state
            .sessions
            .lock()
            .map_err(|_| CommandError::new("cameraPreviewUnavailable"))?;
        sessions.generations.insert(name.clone(), generation);
        sessions.profile_keys.insert(profile_name, name.clone());
        sessions.active.remove(&name);
    }
    let subscription = subscribe_camera(
        &manager,
        &printer,
        CameraFrameKind::H264,
        Operation::CameraStream,
    )?;
    let (sdp, session) = webrtc_camera::start(subscription, offer)
        .map_err(|error| CommandError::new("cameraPreviewUnavailable").with_detail(error))?;
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| CommandError::new("cameraPreviewUnavailable"))?;
    if sessions.generations.get(&name) != Some(&generation) {
        return Err(CommandError::new("cameraPreviewUnavailable")
            .with_detail("WebRTC request was superseded"));
    }
    sessions.active.insert(name, session);
    Ok(CameraWebRtcAnswer {
        kind: "answer",
        sdp,
    })
}

#[tauri::command(async)]
fn printer_camera_webrtc_stop(
    name: Option<String>,
    state: tauri::State<'_, CameraWebRtcState>,
) -> Result<(), CommandError> {
    let generation = state.generation.fetch_add(1, Ordering::AcqRel) + 1;
    let mut sessions = state
        .sessions
        .lock()
        .map_err(|_| CommandError::new("cameraPreviewUnavailable"))?;
    if let Some(name) = name {
        let profile_name = name.to_ascii_lowercase();
        let name = sessions
            .profile_keys
            .remove(&profile_name)
            .unwrap_or(profile_name);
        sessions.generations.insert(name.clone(), generation);
        sessions.active.remove(&name);
    } else {
        sessions.generations.clear();
        sessions.active.clear();
        sessions.profile_keys.clear();
    }
    Ok(())
}

fn start_camera_server(
    subscription: CameraSubscription,
    name: String,
    state: CameraStreamState,
    format: CameraProxyFormat,
) -> Result<String, CommandError> {
    let unavailable = || CommandError::new("cameraPreviewUnavailable");
    let mut token = [0_u8; 16];
    rand_bytes(&mut token).map_err(|_| unavailable())?;
    let path = format!("/stream/{}", hex(&token));

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| unavailable())?;
    let address = listener.local_addr().map_err(|_| unavailable())?;
    listener.set_nonblocking(true).map_err(|_| unavailable())?;
    let stop = Arc::new(AtomicBool::new(false));
    let generation = state.generation.fetch_add(1, Ordering::AcqRel) + 1;
    if let Some(previous) = state.active.lock().map_err(|_| unavailable())?.insert(
        name.clone(),
        CameraProxy {
            generation,
            stop: Arc::clone(&stop),
        },
    ) {
        previous.stop.store(true, Ordering::Release);
    }
    let expected = path.clone();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline && !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((socket, _)) => {
                    match format {
                        CameraProxyFormat::Mjpeg => {
                            proxy_camera_stream(&subscription, socket, &expected, &stop);
                        }
                        CameraProxyFormat::H264 => {
                            proxy_h264_stream(&subscription, socket, &expected, &stop);
                        }
                    }
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(_) => break,
            }
        }
        if let Ok(mut active) = state.active.lock()
            && active
                .get(&name)
                .is_some_and(|proxy| proxy.generation == generation)
        {
            active.remove(&name);
        }
    });
    Ok(format!("http://127.0.0.1:{}{path}", address.port()))
}

fn h264_codec(sps: &[u8]) -> Option<String> {
    (sps.len() >= 4 && sps[0] & 0x1f == 7)
        .then(|| format!("avc1.{:02x}{:02x}{:02x}", sps[1], sps[2], sps[3]))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn proxy_camera_stream(
    subscription: &CameraSubscription,
    mut socket: TcpStream,
    expected: &str,
    stop: &AtomicBool,
) {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let mut request = [0; 4096];
    let read = socket.read(&mut request).unwrap_or_default();
    if !camera_preview_request(&request[..read], expected) {
        let _ = socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    if socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=frame\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .is_ok()
    {
        while !stop.load(Ordering::Acquire) {
            let frame = match subscription.recv_timeout(Duration::from_secs(1)) {
                Ok(frame) => frame,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let CameraFrame::Jpeg(jpeg) = frame.as_ref() else {
                continue;
            };
            let header = format!(
                "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                jpeg.len()
            );
            if socket.write_all(header.as_bytes()).is_err()
                || socket.write_all(jpeg).is_err()
                || socket.write_all(b"\r\n").is_err()
            {
                break;
            }
        }
    }
}

const H264_FRAME_HEADER_LEN: usize = 13;

fn h264_frame_header(
    length: u32,
    timestamp_us: u64,
    keyframe: bool,
) -> [u8; H264_FRAME_HEADER_LEN] {
    let mut header = [0; H264_FRAME_HEADER_LEN];
    header[..4].copy_from_slice(&length.to_be_bytes());
    header[4..12].copy_from_slice(&timestamp_us.to_be_bytes());
    header[12] = u8::from(keyframe);
    header
}

fn proxy_h264_stream(
    subscription: &CameraSubscription,
    mut socket: TcpStream,
    expected: &str,
    stop: &AtomicBool,
) {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let mut request = [0; 4096];
    let read = socket.read(&mut request).unwrap_or_default();
    if !camera_preview_request(&request[..read], expected) {
        let _ = socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    if socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        )
        .is_err()
    {
        return;
    }
    let started = Instant::now();
    let mut last_timestamp = 0_u64;
    while !stop.load(Ordering::Acquire) {
        let frame = match subscription.recv_timeout(Duration::from_secs(1)) {
            Ok(frame) => frame,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let CameraFrame::H264(access_unit) = frame.as_ref() else {
            continue;
        };
        let Some(annex_b) = access_unit.annex_b.as_deref() else {
            continue;
        };
        let Ok(length) = u32::try_from(annex_b.len()) else {
            break;
        };
        let timestamp = u64::try_from(started.elapsed().as_micros())
            .unwrap_or(u64::MAX)
            .max(last_timestamp.saturating_add(1));
        last_timestamp = timestamp;
        let header = h264_frame_header(length, timestamp, access_unit.keyframe);
        if socket.write_all(&header).is_err() || socket.write_all(annex_b).is_err() {
            break;
        }
    }
}

fn subscribe_camera(
    manager: &CameraManager,
    printer: &DesktopPrinter,
    kind: CameraFrameKind,
    operation: Operation,
) -> Result<CameraSubscription, CommandError> {
    let drivers::Profile::Bambu(profile) = &printer.driver else {
        return Err(CommandError::new("cameraPreviewUnavailable"));
    };
    let capabilities = melt_core::bambu::Client::new(profile.clone())
        .runtime_capabilities(
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
        )
        .ok();
    manager
        .subscribe(
            profile,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            Duration::from_secs(10),
            kind,
            capabilities.as_ref(),
        )
        .map_err(|error| operation_error(DriverError::Camera(error), operation))
}

fn camera_preview_request(request: &[u8], expected: &str) -> bool {
    let Ok(request) = std::str::from_utf8(request) else {
        return false;
    };
    // Any local process can reach a loopback port and forge a Host header, so the
    // unguessable path is what actually authenticates the webview.
    let Some(target) = request
        .strip_prefix("GET ")
        .and_then(|line| line.split(' ').next())
    else {
        return false;
    };
    target.len() == expected.len() && target.as_bytes().ct_eq(expected.as_bytes())
}

trait ConstantTimeEq {
    fn ct_eq(&self, other: &Self) -> bool;
}

impl ConstantTimeEq for [u8] {
    fn ct_eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other)
                .fold(0, |difference, (left, right)| difference | (left ^ right))
                == 0
    }
}

#[tauri::command(async)]
fn diagnostics_report(
    monitor: tauri::State<'_, MonitorState>,
    firmware: tauri::State<'_, FirmwareUpdateState>,
    camera: tauri::State<'_, CameraManager>,
) -> Result<diagnostics::Report, CommandError> {
    let config = Config::load().map_err(|_| unreadable_config())?;
    let profiles = config.sorted_profiles();
    let base = diagnostics::report(profiles.iter().map(|profile| profile.profile.clone()));
    let owners = camera.statuses();
    let mut bambu_reports = Vec::new();
    for (index, named) in profiles.iter().enumerate() {
        let Ok(driver) = drivers::profile(&named.profile) else {
            continue;
        };
        let drivers::Profile::Bambu(profile) = &driver else {
            continue;
        };
        let kind = driver.driver();
        let access_code = access_code(&named.profile.driver, &named.name, kind)
            .ok()
            .flatten();
        let fingerprint = tls_fingerprint(
            &named.profile.driver,
            &named.name,
            kind,
            named.profile.insecure,
        )
        .ok()
        .flatten();
        let capabilities = monitor
            .pool
            .bambu_runtime_capabilities(
                &named.name,
                &driver,
                access_code.as_deref(),
                fingerprint.as_deref(),
            )
            .ok()
            .flatten()
            .unwrap_or_else(|| profile.default_capabilities());
        let selection = melt_core::bambu::select_camera_transport(profile.host(), &capabilities);
        let key = driver.physical_printer_key(&named.name);
        let owner = owners.iter().find(|owner| owner.printer_key == key);
        let update_report = firmware.entries.lock().ok().and_then(|entries| {
            entries
                .get(&named.name)
                .map(|cached| cached.entry.report.clone())
        });
        bambu_reports.push(diagnostics::bambu_compatibility_report_with_updates(
            index + 1,
            &capabilities,
            &selection,
            fingerprint.is_some(),
            owner,
            update_report.as_ref(),
        ));
    }
    Ok(diagnostics::with_bambu(base, bambu_reports))
}

fn desktop_printer(name: &str, operation: Operation) -> Result<DesktopPrinter, CommandError> {
    let name = name.to_ascii_lowercase();
    let config = Config::load().map_err(|_| unreadable_config())?;
    let profile = config
        .get_profile(&name)
        .cloned()
        .ok_or_else(|| CommandError::new("profileNotFound"))?;
    let driver = drivers::profile(&profile).map_err(|_| CommandError::new("profileInvalid"))?;
    let driver_kind = driver.driver();
    if !driver.capabilities().supports(operation) {
        return Err(CommandError::of("driverUnsupported", operation));
    }
    let access_code = access_code(&profile.driver, &name, driver_kind)?;
    let tls_fingerprint = tls_fingerprint(&profile.driver, &name, driver_kind, profile.insecure)?;
    Ok(DesktopPrinter {
        name,
        driver,
        access_code,
        tls_fingerprint,
    })
}

fn pooled_desktop_printer(
    state: &MonitorState,
    name: &str,
    operation: Operation,
) -> Result<(u64, DesktopPrinter), CommandError> {
    let generation = state.pool.lifecycle_generation();
    desktop_printer(name, operation).map(|printer| (generation, printer))
}

fn access_code(
    driver: &str,
    name: &str,
    driver_kind: drivers::Driver,
) -> Result<Option<String>, CommandError> {
    match SystemKeychain.get(SERVICE, &account(driver, name, "access-code")) {
        Ok(access_code) => Ok(Some(access_code)),
        Err(SecretError::NotFound) if driver_kind.requires_access_code() => {
            Err(CommandError::new("accessCodeUnavailable"))
        }
        Err(SecretError::NotFound) => Ok(None),
        Err(SecretError::Unavailable(_)) if driver_kind == drivers::Driver::Moonraker => Ok(None),
        Err(SecretError::Unavailable(_)) => Err(CommandError::new("keychainFailed")),
    }
}

fn tls_fingerprint(
    driver: &str,
    name: &str,
    driver_kind: drivers::Driver,
    insecure: bool,
) -> Result<Option<String>, CommandError> {
    if driver_kind != drivers::Driver::BambuLan || insecure {
        return Ok(None);
    }
    SystemKeychain
        .get(SERVICE, &account(driver, name, "tls-fingerprint"))
        .map(Some)
        .map_err(|_| CommandError::new("tlsCredentialsUnavailable"))
}

/// A cached entry is up to one poll interval stale, so an action the user just
/// confirmed would otherwise be reported against the pre-action state.
fn invalidate(state: &MonitorState, name: &str) {
    if let Ok(mut entries) = state.entries.lock() {
        entries.remove(&name.to_ascii_lowercase());
    }
}

fn ensure_state(
    state: &MonitorState,
    generation: u64,
    printer: &DesktopPrinter,
    allowed: &[moonraker::PrinterState],
    operation: Operation,
) -> Result<(), CommandError> {
    if !state.pool.is_current_generation(generation) {
        return Err(CommandError::new("profileNotFound"));
    }
    if let Ok(entries) = state.entries.lock()
        && let Some(cached) = entries.get(&printer.name)
        && cached.sampled_at.elapsed() <= Duration::from_secs(5)
        && let Some(status) = cached.entry.status.as_ref()
        && cached.entry.error.is_none()
    {
        return check_allowed_state(status.state, allowed, operation);
    }

    let status = state
        .pool
        .status_if_current(
            generation,
            &printer.name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
        )
        .ok_or_else(|| CommandError::new("profileNotFound"))?
        .map_err(|error| operation_error(error, operation))?;
    check_allowed_state(status.state, allowed, operation)
}

fn current_pool_operation<T>(
    state: &MonitorState,
    generation: u64,
    operation: Operation,
    run: impl FnOnce(&ConnectionPool) -> Result<T, DriverError>,
) -> Result<T, CommandError> {
    state
        .pool
        .with_current_generation(generation, run)
        .ok_or_else(|| CommandError::new("profileNotFound"))?
        .map_err(|error| operation_error(error, operation))
}

fn check_allowed_state(
    state: moonraker::PrinterState,
    allowed: &[moonraker::PrinterState],
    operation: Operation,
) -> Result<(), CommandError> {
    if allowed.contains(&state) {
        return Ok(());
    }
    Err(CommandError {
        code: "printerWrongState",
        operation: Some(operation_slug(operation)),
        state: Some(state_slug(state)),
        detail: None,
    })
}

fn state_slug(state: moonraker::PrinterState) -> &'static str {
    match state {
        moonraker::PrinterState::Idle => "idle",
        moonraker::PrinterState::Printing => "printing",
        moonraker::PrinterState::Paused => "paused",
        moonraker::PrinterState::Error => "error",
        moonraker::PrinterState::Unknown => "unknown",
    }
}

fn operation_slug(operation: Operation) -> &'static str {
    match operation {
        Operation::Status => "status",
        Operation::FileList => "fileList",
        Operation::JobPause => "jobPause",
        Operation::JobResume => "jobResume",
        Operation::JobCancel => "jobCancel",
        Operation::JobStart => "jobStart",
        Operation::EmergencyStop => "emergencyStop",
        Operation::CameraSnapshot => "cameraSnapshot",
        Operation::CameraStream => "cameraStream",
        Operation::TemperatureSet => "temperatureSet",
        Operation::FanSet => "fanSet",
        Operation::MotionHome => "motionHome",
        Operation::MotionJog => "motionJog",
        Operation::LightSet => "lightSet",
        Operation::SpeedSet => "speedSet",
        Operation::AmsDrying => "amsDrying",
        Operation::FileDownload => "fileDownload",
        Operation::FileUpload => "fileUpload",
        Operation::FileDelete => "fileDelete",
        Operation::Verify => "verification",
        Operation::FirmwareUpdateCheck => "firmwareUpdateCheck",
        _ => "operation",
    }
}

fn operation_error(error: DriverError, operation: Operation) -> CommandError {
    let code = match error {
        DriverError::Unknown(_) | DriverError::InvalidProfile(_) | DriverError::InvalidTimeout => {
            return CommandError::new("profileInvalid");
        }
        DriverError::UnsupportedOperation(_, _)
        | DriverError::Moonraker(melt_core::moonraker::Error::Unsupported(_))
        | DriverError::Bambu(melt_core::bambu::TransportError::Unsupported(_)) => {
            "driverUnsupported"
        }
        DriverError::Moonraker(melt_core::moonraker::Error::Authentication)
        | DriverError::Bambu(
            melt_core::bambu::TransportError::Authentication
            | melt_core::bambu::TransportError::Pin(_),
        ) => "printerAuthFailed",
        DriverError::Bambu(melt_core::bambu::TransportError::UnsignedCommand) => {
            "printerSigningRequired"
        }
        DriverError::Bambu(melt_core::bambu::TransportError::AuthorizationRequired(_)) => {
            "printerSigningRequired"
        }
        DriverError::Bambu(melt_core::bambu::TransportError::AuthorizationConflict(_)) => {
            "printerAuthorizationConflict"
        }
        DriverError::Moonraker(melt_core::moonraker::Error::Timeout)
        | DriverError::Bambu(melt_core::bambu::TransportError::Timeout) => "printerTimeout",
        DriverError::Moonraker(melt_core::moonraker::Error::UpdateRefreshRejected) => {
            "firmwareRefreshRejected"
        }
        DriverError::Camera(melt_core::bambu::CameraError::MissingAccessCode) => {
            "cameraAccessCodeUnavailable"
        }
        DriverError::Camera(
            melt_core::bambu::CameraError::Pin(_)
            | melt_core::bambu::CameraError::MissingCertificate,
        ) => "cameraTlsFailed",
        DriverError::Moonraker(_) | DriverError::Bambu(_) | DriverError::Camera(_) => {
            "printerOperationFailed"
        }
    };
    CommandError::of(code, operation)
}

#[tauri::command(async)]
fn remove_configured_printer(
    name: String,
    state: tauri::State<'_, MonitorState>,
    firmware_state: tauri::State<'_, FirmwareUpdateState>,
) -> Result<profiles::RemoveResult, CommandError> {
    let dir = config_dir().map_err(|_| unreadable_config())?;
    let result = profiles::remove(dir, &SystemKeychain, &name).map_err(create_error)?;
    state.pool.remove(&name);
    invalidate(&state, &name);
    invalidate_firmware_updates(&firmware_state, &name);
    Ok(result)
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(melt_cli::run(
            &args,
            &mut std::io::stdout(),
            &mut std::io::stderr(),
        ));
    }

    let pool = Arc::new(ConnectionPool::default());
    let monitor_state = MonitorState {
        pool: pool.clone(),
        ..MonitorState::default()
    };
    let firmware_update_state = FirmwareUpdateState {
        entries: Arc::default(),
        pool,
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(monitor_state)
        .manage(firmware_update_state)
        .manage(PresenceState::default())
        .manage(CameraManager::default())
        .manage(CameraStreamState::default())
        .manage(CameraWebRtcState::default())
        .manage(PreviewState::default())
        .manage(LibraryWatchState::default())
        .manage(TransferState::default())
        .manage(SlicerState::default())
        .setup(|app| {
            let state = app.state::<MonitorState>().inner().clone();
            let firmware_state = app.state::<FirmwareUpdateState>().inner().clone();
            let presence = app.state::<PresenceState>().inner().clone();
            start_monitor_worker(app.handle().clone(), state, presence.clone());
            start_firmware_update_worker(app.handle().clone(), firmware_state);
            start_presence_worker(app.handle().clone(), presence);
            Ok(())
        })
        .manage(PreferencesState::default())
        .invoke_handler(tauri::generate_handler![
            app_info,
            get_preferences,
            update_notification_preferences,
            save_slicer,
            set_slicer_enabled,
            remove_slicer,
            detect_slicers,
            configured_printers,
            registered_drivers,
            discover_printers,
            printer_capabilities,
            monitored_printers,
            cached_monitoring,
            cached_firmware_updates,
            printer_firmware_updates,
            probe_keychain,
            printer_status,
            create_configured_printer,
            update_configured_printer,
            preview_printer_tls,
            refresh_printer_tls,
            printer_files,
            library_files,
            library_delete_file,
            library_add_file,
            library_file_preview,
            set_library_path,
            print_library_file,
            inspect_library_print,
            printer_job_action,
            printer_temperature_set,
            printer_fan_set,
            printer_motion_home,
            printer_motion_jog,
            printer_light_set,
            printer_ams_drying_set,
            printer_speed_set,
            printer_file_download,
            cancel_file_transfer,
            printer_file_upload,
            printer_file_delete,
            printer_file_preview,
            open_file_with_slicer,
            list_slicer_processes,
            stop_slicer_process,
            printer_emergency_stop,
            printer_camera_snapshot,
            printer_camera_stream,
            printer_camera_h264_stream,
            printer_camera_stream_stop,
            printer_camera_webrtc_offer,
            printer_camera_webrtc_stop,
            diagnostics_report,
            remove_configured_printer
        ])
        .run(tauri::generate_context!())
        .expect("error while running Melt");
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use melt_core::{
        drivers::{self, Operation},
        firmware_updates::{FirmwareUpdateReport, FirmwareUpdateSource},
        moonraker,
    };

    use super::{
        CachedFirmwareUpdate, CachedMonitor, DesktopPrinter, FIRMWARE_UPDATE_STALE_AFTER,
        FirmwareUpdateEntry, FirmwareUpdateState, MonitorConnectionState, MonitorEntry,
        MonitorState, Profile, STATUS_CACHE_HEARTBEAT, StatusCacheThrottle,
        cached_monitor_entry_at, cached_ui_monitor_entry, camera_preview_request,
        current_firmware_entry, ensure_state, extract_3mf_thumbnail, h264_codec, h264_frame_header,
        invalidate_firmware_updates, load_status_cache_from, monitor_connection_state,
        operation_error, retain_firmware_update_failure, save_status_cache_to,
        store_firmware_update,
    };

    const TOKEN: &str = "/stream/0123456789abcdef0123456789abcdef";

    fn firmware_entry() -> FirmwareUpdateEntry {
        FirmwareUpdateEntry {
            profile: "printer".into(),
            driver: "moonraker".into(),
            report: FirmwareUpdateReport::unsupported(
                FirmwareUpdateSource::MoonrakerUpdateManager,
                "test",
                "test",
            ),
            checked_at: "2026-09-19T12:00:00Z".into(),
            stale: false,
            error: None,
        }
    }

    #[test]
    fn firmware_cache_suppresses_unchanged_entries_and_invalidates_by_name() {
        let state = FirmwareUpdateState::default();
        let generation = state.pool.lifecycle_generation();
        let entry = firmware_entry();

        assert!(store_firmware_update(
            &state,
            "printer",
            generation,
            entry.clone()
        ));
        assert!(!store_firmware_update(&state, "printer", generation, entry));
        invalidate_firmware_updates(&state, "Printer");
        assert!(state.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn firmware_cache_marks_old_and_failed_results_stale_without_losing_them() {
        let state = FirmwareUpdateState::default();
        state.entries.lock().unwrap().insert(
            "printer".into(),
            CachedFirmwareUpdate {
                checked_at: Instant::now() - FIRMWARE_UPDATE_STALE_AFTER - Duration::from_secs(1),
                generation: state.pool.lifecycle_generation(),
                entry: firmware_entry(),
            },
        );
        let aged = {
            let entries = state.entries.lock().unwrap();
            current_firmware_entry(entries.get("printer").unwrap())
        };
        assert!(aged.stale);

        let error = super::CommandError::new("printerTimeout");
        let (failed, changed) = retain_firmware_update_failure(&state, "printer", &error).unwrap();
        assert!(changed);
        assert!(failed.stale);
        assert_eq!(failed.error, Some(error));
        assert_eq!(failed.report, firmware_entry().report);
    }

    #[test]
    fn moonraker_timeouts_use_the_stable_printer_timeout_code() {
        let error = operation_error(
            drivers::DriverError::Moonraker(moonraker::Error::Timeout),
            Operation::Status,
        );

        assert_eq!(error.code, "printerTimeout");
        assert_eq!(error.operation, Some("status"));
    }

    #[test]
    fn unsigned_commands_report_a_signing_requirement_not_bad_credentials() {
        let error = operation_error(
            drivers::DriverError::Bambu(melt_core::bambu::TransportError::UnsignedCommand),
            Operation::JobStart,
        );

        assert_eq!(error.code, "printerSigningRequired");
    }

    #[test]
    fn status_cache_round_trips_the_last_monitoring_payload() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_status_cache_from(dir.path()).is_none());

        let entries = vec![MonitorEntry {
            name: "attic-p1s".into(),
            driver: "bambu-lan".into(),
            status: None,
            error: None,
            stale: false,
            connection_state: MonitorConnectionState::Connecting,
            observed_at: None,
        }];
        save_status_cache_to(dir.path(), &entries);

        let cached = load_status_cache_from(dir.path()).unwrap();
        assert_eq!(cached, serde_json::to_value(&entries).unwrap());
    }

    #[test]
    fn status_cache_skips_writes_that_only_refresh_observed_at() {
        let entry = |observed_at: &str, stale: bool| MonitorEntry {
            name: "attic-p1s".into(),
            driver: "bambu-lan".into(),
            status: None,
            error: None,
            stale,
            connection_state: MonitorConnectionState::Live,
            observed_at: Some(observed_at.into()),
        };
        let start = Instant::now();
        let mut throttle = StatusCacheThrottle::default();

        assert!(throttle.should_save(&[entry("t0", false)], start));
        assert!(!throttle.should_save(&[entry("t1", false)], start + Duration::from_secs(5)));
        assert!(throttle.should_save(&[entry("t2", true)], start + Duration::from_secs(10)));
        assert!(!throttle.should_save(&[entry("t3", true)], start + Duration::from_secs(15)));
        assert!(throttle.should_save(
            &[entry("t4", true)],
            start + Duration::from_secs(10) + STATUS_CACHE_HEARTBEAT,
        ));
    }

    #[test]
    fn monitor_lifecycle_expires_recovery_and_initial_synchronization() {
        let started = Instant::now();
        let recent = started + Duration::from_secs(20);
        let expired = started + Duration::from_secs(31);

        assert_eq!(
            monitor_connection_state(recent, started, Some(started), true),
            MonitorConnectionState::Recovering
        );
        assert_eq!(
            monitor_connection_state(expired, started, Some(started), true),
            MonitorConnectionState::Offline
        );
        assert_eq!(
            monitor_connection_state(recent, started, None, true),
            MonitorConnectionState::Synchronizing
        );
        assert_eq!(
            monitor_connection_state(expired, started, None, true),
            MonitorConnectionState::Offline
        );
        assert_eq!(
            monitor_connection_state(expired, started, Some(started), false),
            MonitorConnectionState::Live
        );
    }

    #[test]
    fn cached_status_rejects_an_invalidated_generation() {
        let state = MonitorState::default();
        let generation = state.pool.lifecycle_generation();
        let now = Instant::now();
        state.entries.lock().unwrap().insert(
            "printer".into(),
            CachedMonitor {
                next_poll: now + Duration::from_secs(5),
                sampled_at: now,
                started_at: now,
                observed_at: None,
                backoff: melt_core::monitor::Backoff::new(Duration::from_secs(5)),
                entry: MonitorEntry {
                    name: "printer".into(),
                    driver: "moonraker".into(),
                    status: None,
                    error: None,
                    stale: false,
                    connection_state: MonitorConnectionState::Connecting,
                    observed_at: None,
                },
            },
        );

        state.pool.remove("printer");

        assert!(cached_ui_monitor_entry(&state, "printer", generation).is_none());
        assert!(state.entries.lock().unwrap().contains_key("printer"));
    }

    #[test]
    fn removed_printer_discards_an_in_flight_monitor_result() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::channel();
        let (respond_tx, respond_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
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
        let state = MonitorState::default();
        let generation = state.pool.lifecycle_generation();
        let profile = Profile {
            driver: "moonraker".into(),
            host: format!("http://{address}"),
            serial: String::new(),
            model: String::new(),
            timeout: "2s".into(),
            insecure: false,
            created: String::new(),
            updated: String::new(),
        };
        let monitor_state = state.clone();
        let monitor = thread::spawn(move || {
            cached_monitor_entry_at(&monitor_state, "printer".into(), profile, generation)
        });
        request_rx.recv().unwrap();

        state.pool.remove("printer");
        respond_tx.send(()).unwrap();

        assert!(monitor.join().unwrap().is_none());
        server.join().unwrap();
        assert!(state.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn failed_refresh_retains_the_last_successful_status_as_stale() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
            }
            let body = r#"{"result":{"status":{"print_stats":{"state":"printing"}}}}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();

            // Accept the refresh and close without a response to simulate a
            // transient disconnect after a known printing state.
            let _ = listener.accept().unwrap();
        });
        let state = MonitorState::default();
        let generation = state.pool.lifecycle_generation();
        let profile = Profile {
            driver: "moonraker".into(),
            host: format!("http://{address}"),
            serial: String::new(),
            model: String::new(),
            timeout: "250ms".into(),
            insecure: false,
            created: String::new(),
            updated: String::new(),
        };

        let first =
            cached_monitor_entry_at(&state, "printer".into(), profile.clone(), generation).unwrap();
        assert_eq!(
            first.status.as_ref().map(|status| status.state),
            Some(moonraker::PrinterState::Printing)
        );
        assert!(!first.stale);
        let observed_at = first.observed_at.clone();
        state
            .entries
            .lock()
            .unwrap()
            .get_mut("printer")
            .unwrap()
            .next_poll = Instant::now();

        let failed =
            cached_monitor_entry_at(&state, "printer".into(), profile, generation).unwrap();
        server.join().unwrap();

        assert_eq!(
            failed.status.as_ref().map(|status| status.state),
            Some(moonraker::PrinterState::Printing)
        );
        assert!(failed.error.is_some());
        assert!(failed.stale);
        assert_eq!(failed.observed_at, observed_at);
    }

    #[test]
    fn stale_command_state_checks_reuse_the_pooled_client() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            for body in [
                r#"{"result":{"status":{"print_stats":{"state":"standby"}}}}"#,
                r#"{"result":{}}"#,
            ] {
                loop {
                    let mut line = String::new();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                stream.flush().unwrap();
            }
        });
        let profile =
            moonraker::Profile::new(&format!("http://{address}"), false, Duration::from_secs(2))
                .unwrap();
        let printer = DesktopPrinter {
            name: "printer".into(),
            driver: drivers::Profile::Moonraker(profile),
            access_code: None,
            tls_fingerprint: None,
        };
        let state = MonitorState::default();

        ensure_state(
            &state,
            state.pool.lifecycle_generation(),
            &printer,
            &[moonraker::PrinterState::Idle],
            Operation::EmergencyStop,
        )
        .unwrap();
        state
            .pool
            .emergency_stop(
                &printer.name,
                &printer.driver,
                printer.access_code.as_deref(),
                printer.tls_fingerprint.as_deref(),
            )
            .unwrap();

        server.join().unwrap();
    }

    fn zip_with_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buffer = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            for (name, contents) in entries {
                zip.start_file(*name, options).unwrap();
                std::io::Write::write_all(&mut zip, contents).unwrap();
            }
            zip.finish().unwrap();
        }
        buffer
    }

    fn test_png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn prefers_a_thumbnail_named_png_over_others() {
        let icon = test_png(1, 1);
        let plate = test_png(2, 1);
        let thumb = test_png(3, 1);
        let bytes = zip_with_entries(&[
            ("preview/icon.png", &icon),
            ("Metadata/PLATE_1.png", &plate),
            ("Metadata/thumbnail_middle.png", &thumb),
        ]);
        let thumbnail = extract_3mf_thumbnail(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(thumbnail, thumb);
    }

    /// Real slicers don't all name their preview the same way, so anything
    /// exact-path matching would miss files a "thumbnail"-named search
    /// still finds; falling back to the first PNG covers those too.
    #[test]
    fn falls_back_to_the_first_png_when_none_are_named_thumbnail() {
        let icon = test_png(1, 1);
        let other = test_png(2, 1);
        let bytes = zip_with_entries(&[
            ("3D/3dmodel.model", b"<model/>"),
            ("preview/icon.png", &icon),
            ("preview/other.png", &other),
        ]);
        let thumbnail = extract_3mf_thumbnail(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(thumbnail, icon);
    }

    #[test]
    fn finds_no_thumbnail_when_the_bundle_has_no_png() {
        let bytes = zip_with_entries(&[("3D/3dmodel.model", b"<model/>")]);
        assert!(extract_3mf_thumbnail(std::io::Cursor::new(bytes)).is_none());
    }

    #[test]
    fn camera_preview_accepts_only_the_issued_token() {
        assert!(camera_preview_request(
            format!("GET {TOKEN} HTTP/1.1\r\nHost: 127.0.0.1:4000\r\n\r\n").as_bytes(),
            TOKEN
        ));
        // A forged Host header is no longer enough on its own.
        assert!(!camera_preview_request(
            b"GET /stream HTTP/1.1\r\nHost: 127.0.0.1:4000\r\n\r\n",
            TOKEN
        ));
        assert!(!camera_preview_request(
            b"GET /stream/0123456789abcdef0123456789abcdee HTTP/1.1\r\nHost: localhost\r\n\r\n",
            TOKEN
        ));
        assert!(!camera_preview_request(
            format!("GET {TOKEN}/../ HTTP/1.1\r\n\r\n").as_bytes(),
            TOKEN
        ));
        assert!(!camera_preview_request(b"", TOKEN));
    }

    #[test]
    fn h264_codec_comes_from_the_sps_profile_and_level() {
        assert_eq!(
            h264_codec(&[0x67, 0x64, 0x0c, 0x1f]),
            Some("avc1.640c1f".to_owned())
        );
        assert_eq!(h264_codec(&[0x68, 0x64, 0x0c, 0x1f]), None);
        assert_eq!(h264_codec(&[0x67, 0x64]), None);
    }

    #[test]
    fn h264_frame_header_is_network_ordered() {
        assert_eq!(
            h264_frame_header(0x01020304, 0x0102030405060708, true),
            [1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 7, 8, 1]
        );
    }
}
