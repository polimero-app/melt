use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Condvar, LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use openssl::rand::rand_bytes;
use polimero_core::{
    AppInfo,
    config::{Config, ConfigError, Profile, config_dir},
    diagnostics,
    drivers::{self, Capabilities, DriverError, Operation},
    keychain::{SERVICE, SecretError, SecretStore, SystemKeychain, account},
    monitor, moonraker,
    pool::ConnectionPool,
    preferences::{self, NotificationPreferences, PreferencesError, Slicer},
    profiles,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

mod preview;
mod webrtc_camera;

#[tauri::command(async)]
fn app_info() -> AppInfo {
    polimero_core::app_info()
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
    timeout: String,
    insecure: bool,
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

#[derive(Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorEntry {
    name: String,
    driver: String,
    status: Option<moonraker::Status>,
    error: Option<CommandError>,
}

struct CachedMonitor {
    next_poll: Instant,
    sampled_at: Instant,
    backoff: monitor::Backoff,
    entry: MonitorEntry,
}

#[derive(Clone, Default)]
struct MonitorState {
    entries: Arc<Mutex<BTreeMap<String, CachedMonitor>>>,
    previous: Arc<Mutex<HashMap<String, MonitorEntry>>>,
    pool: Arc<ConnectionPool>,
}

#[derive(Default)]
struct CameraWebRtcState {
    generation: AtomicU64,
    session: Mutex<Option<webrtc_camera::Session>>,
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
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TemperatureRequest {
    name: String,
    nozzle_celsius: Option<f64>,
    bed_celsius: Option<f64>,
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
/// `Polimero` folder under the user's documents, created on first use.
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
    let dir = base.join("Polimero");
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
    #[serde(default)]
    search: String,
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
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LibraryPrintRequest {
    printer: String,
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

fn preview_cache_key(request: &FilePreviewRequest) -> String {
    format!(
        "{}:{}:{}:{}",
        request.name,
        request.device_path,
        request.size_bytes.unwrap_or_default(),
        request.modified_at.as_deref().unwrap_or_default()
    )
}

#[tauri::command(async)]
fn configured_printers() -> Result<Vec<PrinterSummary>, CommandError> {
    Config::load()
        .map(|config| {
            config
                .sorted_profiles()
                .into_iter()
                .map(|profile| PrinterSummary {
                    name: profile.name,
                    driver: profile.profile.driver,
                    host: profile.profile.host,
                    serial: profile.profile.serial,
                    timeout: profile.profile.timeout,
                    insecure: profile.profile.insecure,
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
fn discover_printers() -> Result<Vec<polimero_core::bambu::DiscoveredPrinter>, CommandError> {
    polimero_core::bambu::discover(Duration::from_secs(5))
        .map_err(|_| CommandError::new("discoveryFailed"))
}

#[tauri::command(async)]
fn printer_capabilities(name: String) -> Result<PrinterCapabilities, CommandError> {
    let config = Config::load().map_err(|_| unreadable_config())?;
    let profile = config
        .get_profile(&name.to_ascii_lowercase())
        .ok_or_else(|| CommandError::new("profileNotFound"))?;
    let driver = drivers::profile(profile).map_err(|_| CommandError::new("profileInvalid"))?;
    Ok(PrinterCapabilities {
        name,
        capabilities: driver.driver().capabilities(),
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
        if let Some(entry) = cached_ui_monitor_entry(&state, &name) {
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
fn cached_ui_monitor_entry(state: &MonitorState, name: &str) -> Option<MonitorEntry> {
    let entries = state.entries.lock().ok()?;
    let cached = entries.get(&name.to_ascii_lowercase())?;
    (cached.sampled_at.elapsed() <= MAX_UI_CACHE_AGE).then(|| cached.entry.clone())
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

const STATUS_CACHE_FILE: &str = "polimero-status-cache.json";
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

fn start_monitor_worker(app: tauri::AppHandle, state: MonitorState) {
    thread::spawn(move || {
        let mut previous_entries: Option<Vec<MonitorEntry>> = None;
        loop {
            let cycle_started = Instant::now();
            if let Ok(entries) = collect_monitored_printers(&state) {
                if previous_entries.as_ref() != Some(&entries) {
                    use tauri::Emitter;
                    save_status_cache(&entries);
                    let _ = app.emit("monitoring-updated", entries.clone());
                    emit_status_notifications(&app, &state);
                    previous_entries = Some(entries);
                }
            }
            thread::sleep(Duration::from_secs(5).saturating_sub(cycle_started.elapsed()));
        }
    });
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

    let entry = monitor_printer(&state.pool, name.clone(), profile, generation)?;
    if !state.pool.is_current_generation(generation) {
        return None;
    }
    if let Ok(mut entries) = state.entries.lock() {
        if !state.pool.is_current_generation(generation) {
            return None;
        }
        let cached = entries.entry(name).or_insert_with(|| CachedMonitor {
            next_poll: now,
            sampled_at: now,
            backoff: monitor::Backoff::new(monitor::DEFAULT_INTERVAL),
            entry: entry.clone(),
        });
        let delay = cached.backoff.after_result(entry.error.is_none());
        cached.next_poll = now + delay;
        cached.sampled_at = Instant::now();
        cached.entry = entry.clone();
    }
    Some(entry)
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
        },
        Err(error) => MonitorEntry {
            name,
            driver,
            status: None,
            error: Some(operation_error(error, Operation::Status)),
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
        entries.push(moonraker::FileEntry {
            name,
            root: "library",
            device_path: path.clone(),
            path,
            entry_type,
            size_bytes: (!metadata.is_dir()).then_some(metadata.len() as i64),
            modified_at: metadata.modified().ok().and_then(format_modified),
            metadata: BTreeMap::new(),
        });
    }
    let search = request.search.trim().to_ascii_lowercase();
    if !search.is_empty() {
        entries.retain(|entry| entry.name.to_ascii_lowercase().contains(&search));
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
            search: String::new(),
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
fn print_library_file(
    request: LibraryPrintRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::JobResult, CommandError> {
    let absolute = resolve_absolute_path(&request.path)?;
    let filename = absolute
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CommandError::new("libraryPathInvalid"))?;
    let (generation, printer) =
        pooled_desktop_printer(&state, &request.printer, Operation::FileUpload)?;
    let device_path = format!("/{filename}");
    drivers::upload_file(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &absolute,
        &device_path,
        true,
    )
    .map_err(|error| operation_error(error, Operation::FileUpload))?;
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
            polimero_core::bambu::JobStartOptions::default(),
        )
    });
    invalidate(&state, &request.printer);
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
                polimero_core::bambu::JobStartOptions::default(),
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
    let mut destination = std::fs::File::create(&request.destination)
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
        destination: &mut destination,
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
    match result {
        Ok(bytes) => {
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
    let cache_key = preview_cache_key(&request);
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
    let printer = desktop_printer(&request.name, Operation::FileDownload)?;
    let mut bytes = Vec::new();
    drivers::download_to(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        &request.device_path,
        &mut bytes,
    )
    .map_err(|error| operation_error(error, Operation::FileDownload))?;
    render_preview(&extension, bytes, cache_key, &state)
}

#[tauri::command(async)]
fn library_file_preview(
    request: LibraryPreviewRequest,
    state: tauri::State<'_, PreviewState>,
) -> Result<FilePreviewResponse, CommandError> {
    let cache_key = format!(
        "library:{}:{}:{}",
        request.path,
        request.size_bytes.unwrap_or_default(),
        request.modified_at.as_deref().unwrap_or_default()
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
    if !matches!(extension.as_str(), "3mf" | "stl" | "obj") {
        return Err(CommandError::new("thumbnailUnsupported"));
    }
    let _permit = RENDER_SEMAPHORE.acquire();
    let absolute = resolve_absolute_path(&request.path)?;
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
            cache_preview(&state, &cache_key, &preview);
            return Ok(preview);
        }
    }
    let bytes = std::fs::read(&absolute).map_err(|_| CommandError::new("libraryUnavailable"))?;
    render_preview(&extension, bytes, cache_key, &state)
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

fn cache_preview(state: &PreviewState, cache_key: &str, preview: &FilePreviewResponse) {
    if let Ok(mut cache) = state.cache.lock() {
        cache.insert(cache_key.to_owned(), preview.clone());
    }
    write_disk_preview_cache(cache_key, preview);
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
    let mut entry = archive.by_name(&name).ok()?;
    let mut thumbnail = Vec::new();
    entry.read_to_end(&mut thumbnail).ok()?;
    Some(thumbnail)
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
) -> Result<FilePreviewResponse, CommandError> {
    const MAX_RASTERIZE_BYTES: usize = 32 * 1024 * 1024;
    if extension == "3mf" {
        if let Some(thumbnail) = extract_3mf_thumbnail(Cursor::new(&bytes)) {
            let preview = FilePreviewResponse {
                kind: "png",
                data: STANDARD.encode(thumbnail),
            };
            cache_preview(state, &cache_key, &preview);
            return Ok(preview);
        }
        if bytes.len() > MAX_RASTERIZE_BYTES {
            return Err(CommandError::new("thumbnailTooLarge"));
        }
        let png =
            preview::rasterize_3mf(&bytes).map_err(|_| CommandError::new("thumbnailInvalid"))?;
        let preview = FilePreviewResponse {
            kind: "png",
            data: STANDARD.encode(png),
        };
        cache_preview(state, &cache_key, &preview);
        return Ok(preview);
    }
    if matches!(extension, "stl" | "obj") {
        if bytes.len() > MAX_RASTERIZE_BYTES {
            return Err(CommandError::new("thumbnailTooLarge"));
        }
        let png = preview::rasterize(extension, &bytes)
            .map_err(|_| CommandError::new("thumbnailInvalid"))?;
        let preview = FilePreviewResponse {
            kind: "png",
            data: STANDARD.encode(png),
        };
        cache_preview(state, &cache_key, &preview);
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
fn printer_camera_snapshot(name: String) -> Result<CameraSnapshot, CommandError> {
    let printer = desktop_printer(&name, Operation::CameraSnapshot)?;
    let image = drivers::camera_snapshot(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        Duration::from_secs(10),
    )
    .map_err(|error| operation_error(error, Operation::CameraSnapshot))?;
    Ok(CameraSnapshot {
        data_url: format!("data:image/jpeg;base64,{}", STANDARD.encode(image)),
    })
}

#[tauri::command(async)]
fn printer_camera_stream(name: String) -> Result<CameraStream, CommandError> {
    let printer = desktop_printer(&name, Operation::CameraStream)?;
    let stream = drivers::camera_stream(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        Duration::from_secs(10),
    )
    .map_err(|error| operation_error(error, Operation::CameraStream))?;
    start_camera_server(stream).map(|url| CameraStream { url })
}

#[tauri::command(async)]
fn printer_camera_webrtc_offer(
    name: String,
    offer: String,
    state: tauri::State<'_, CameraWebRtcState>,
) -> Result<CameraWebRtcAnswer, CommandError> {
    let printer = desktop_printer(&name, Operation::CameraStream)?;
    let generation = state.generation.fetch_add(1, Ordering::AcqRel) + 1;
    let _ = state
        .session
        .lock()
        .map_err(|_| CommandError::new("cameraPreviewUnavailable"))?
        .take();
    let (sdp, session) = webrtc_camera::start(
        printer.driver,
        printer.access_code,
        printer.tls_fingerprint,
        offer,
    )
    .map_err(|error| CommandError::new("cameraPreviewUnavailable").with_detail(error))?;
    let mut active = state
        .session
        .lock()
        .map_err(|_| CommandError::new("cameraPreviewUnavailable"))?;
    if state.generation.load(Ordering::Acquire) != generation {
        return Err(CommandError::new("cameraPreviewUnavailable")
            .with_detail("WebRTC request was superseded"));
    }
    active.replace(session);
    Ok(CameraWebRtcAnswer {
        kind: "answer",
        sdp,
    })
}

#[tauri::command(async)]
fn printer_camera_webrtc_stop(
    state: tauri::State<'_, CameraWebRtcState>,
) -> Result<(), CommandError> {
    state.generation.fetch_add(1, Ordering::AcqRel);
    state
        .session
        .lock()
        .map_err(|_| CommandError::new("cameraPreviewUnavailable"))?
        .take();
    Ok(())
}

fn start_camera_server(
    mut stream: polimero_core::bambu::MjpegStream,
) -> Result<String, CommandError> {
    let unavailable = || CommandError::new("cameraPreviewUnavailable");
    let mut token = [0_u8; 16];
    rand_bytes(&mut token).map_err(|_| unavailable())?;
    let path = format!("/stream/{}", hex(&token));

    let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|_| unavailable())?;
    let address = listener.local_addr().map_err(|_| unavailable())?;
    listener.set_nonblocking(true).map_err(|_| unavailable())?;
    let expected = path.clone();
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((socket, _)) => {
                    proxy_camera_stream(&mut stream, socket, &expected);
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(_) => return,
            }
        }
    });
    Ok(format!("http://127.0.0.1:{}{path}", address.port()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn proxy_camera_stream(stream: &mut dyn Read, mut socket: TcpStream, expected: &str) {
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
        let _ = std::io::copy(stream, &mut socket);
    }
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
fn diagnostics_report() -> Result<diagnostics::Report, CommandError> {
    Config::load()
        .map(|config| {
            diagnostics::report(
                config
                    .sorted_profiles()
                    .into_iter()
                    .map(|profile| profile.profile),
            )
        })
        .map_err(|_| unreadable_config())
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
    if !driver_kind.supports(operation) {
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
        Operation::FileDownload => "fileDownload",
        Operation::FileUpload => "fileUpload",
        Operation::FileDelete => "fileDelete",
        Operation::Verify => "verification",
        _ => "operation",
    }
}

fn operation_error(error: DriverError, operation: Operation) -> CommandError {
    let code = match error {
        DriverError::Unknown(_) | DriverError::InvalidProfile(_) | DriverError::InvalidTimeout => {
            return CommandError::new("profileInvalid");
        }
        DriverError::UnsupportedOperation(_, _)
        | DriverError::Moonraker(polimero_core::moonraker::Error::Unsupported(_))
        | DriverError::Bambu(polimero_core::bambu::TransportError::Unsupported(_)) => {
            "driverUnsupported"
        }
        DriverError::Moonraker(polimero_core::moonraker::Error::Authentication)
        | DriverError::Bambu(
            polimero_core::bambu::TransportError::Authentication
            | polimero_core::bambu::TransportError::UnsignedCommand
            | polimero_core::bambu::TransportError::Pin(_),
        ) => "printerAuthFailed",
        DriverError::Moonraker(polimero_core::moonraker::Error::Timeout)
        | DriverError::Bambu(polimero_core::bambu::TransportError::Timeout) => "printerTimeout",
        DriverError::Camera(polimero_core::bambu::CameraError::MissingAccessCode) => {
            "cameraAccessCodeUnavailable"
        }
        DriverError::Camera(
            polimero_core::bambu::CameraError::Pin(_)
            | polimero_core::bambu::CameraError::MissingCertificate,
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
) -> Result<profiles::RemoveResult, CommandError> {
    let dir = config_dir().map_err(|_| unreadable_config())?;
    let result = profiles::remove(dir, &SystemKeychain, &name).map_err(create_error)?;
    state.pool.remove(&name);
    invalidate(&state, &name);
    Ok(result)
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(polimero_cli::run(
            &args,
            &mut std::io::stdout(),
            &mut std::io::stderr(),
        ));
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(MonitorState::default())
        .manage(CameraWebRtcState::default())
        .manage(PreviewState::default())
        .manage(TransferState::default())
        .manage(SlicerState::default())
        .setup(|app| {
            let state = app.state::<MonitorState>().inner().clone();
            start_monitor_worker(app.handle().clone(), state);
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
            printer_status,
            create_configured_printer,
            preview_printer_tls,
            refresh_printer_tls,
            printer_files,
            library_files,
            library_delete_file,
            library_add_file,
            library_file_preview,
            set_library_path,
            print_library_file,
            printer_job_action,
            printer_temperature_set,
            printer_fan_set,
            printer_motion_home,
            printer_motion_jog,
            printer_light_set,
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
            printer_camera_webrtc_offer,
            printer_camera_webrtc_stop,
            diagnostics_report,
            remove_configured_printer
        ])
        .run(tauri::generate_context!())
        .expect("error while running Polimero");
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
        time::Duration,
    };

    use polimero_core::{
        drivers::{self, Operation},
        moonraker,
    };

    use super::{
        DesktopPrinter, MonitorEntry, MonitorState, Profile, cached_monitor_entry_at,
        camera_preview_request, ensure_state, extract_3mf_thumbnail, load_status_cache_from,
        operation_error, save_status_cache_to,
    };

    const TOKEN: &str = "/stream/0123456789abcdef0123456789abcdef";

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
    fn status_cache_round_trips_the_last_monitoring_payload() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_status_cache_from(dir.path()).is_none());

        let entries = vec![MonitorEntry {
            name: "attic-p1s".into(),
            driver: "bambu-lan".into(),
            status: None,
            error: None,
        }];
        save_status_cache_to(dir.path(), &entries);

        let cached = load_status_cache_from(dir.path()).unwrap();
        assert_eq!(cached, serde_json::to_value(&entries).unwrap());
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

    #[test]
    fn prefers_a_thumbnail_named_png_over_others() {
        let bytes = zip_with_entries(&[
            ("preview/icon.png", b"icon"),
            ("Metadata/PLATE_1.png", b"plate"),
            ("Metadata/thumbnail_middle.png", b"thumb"),
        ]);
        let thumbnail = extract_3mf_thumbnail(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(thumbnail, b"thumb");
    }

    /// Real slicers don't all name their preview the same way, so anything
    /// exact-path matching would miss files a "thumbnail"-named search
    /// still finds; falling back to the first PNG covers those too.
    #[test]
    fn falls_back_to_the_first_png_when_none_are_named_thumbnail() {
        let bytes = zip_with_entries(&[
            ("3D/3dmodel.model", b"<model/>"),
            ("preview/icon.png", b"icon"),
            ("preview/other.png", b"other"),
        ]);
        let thumbnail = extract_3mf_thumbnail(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(thumbnail, b"icon");
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
}
