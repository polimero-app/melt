# IP Cameras Phase 2 (Desktop) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** In the desktop app, users can add, edit, remove and link IP cameras, watch any camera from a Cameras view, and pick a linked camera as a printer's feed in the control view. One camera upstream is shared by all viewers, and stopping one viewer never stops another.

**Architecture:** `CameraManager` (`melt-core/src/bambu/camera_manager.rs`) gains `subscribe_external` next to the Bambu `subscribe`. Both go through one private startup path. That path locks per source key, never manager-wide, while connecting, and gives the owner thread a reopen closure instead of Bambu-only config. The desktop swaps its two camera states for one `CameraViewers` registry keyed by a backend-generated `viewerId`. The `printer_camera_*` commands become `camera_*` commands that take a `source` (`{printer}` or `{camera}`). The UI moves the camera lifecycle out of `App.vue` into `camera-session.ts` and `CameraViewer.vue`, then adds a Cameras view and a feed picker.

**Tech Stack:** Rust 2024, Tauri 2, Vue 3 + TypeScript, vitest. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-08-ip-cameras-design.md`, section "Requirements carried to Phase 2 (desktop)".

**Phase 1 API used:** `cameras::{resolve, open_stream, verify, create, update, remove, list, link, unlink, select}`, `ResolvedCamera`, `StreamQuality`, `CameraRequest`, `CameraSummary` (`printers`, `selectedBy`), and `cameras::CameraError`.

## Global Constraints

- **No new crates or npm packages.**
- **Bambu behavior is unchanged:** same owner key (`bambu:<SERIAL>`), transport selection, fallback, reconnect backoff, fatal Pin/Identity errors and H.264 parameter check.
- **External cameras are H.264 only.** MJPEG and WebCodecs reuse the existing converters, and WebRTC reuses `webrtc_camera::start`.
- **Passwords go from UI to backend only**, on add and edit. They are never returned, logged or put in an error `detail`. On edit, an empty password keeps the stored one (Phase 1 rule).
- **Isolation:** camera commands never call `invalidate`, the monitor pool or the printer status cache. A camera error is shown only in its viewer.
- **No hidden thumbnail sessions and no periodic camera polling.** An open viewer's stream state is the camera status.
- **Tests:** stdlib `#[test]`. Network tests bind `127.0.0.1:0` only. UI logic is tested in pure `.ts` modules; vitest runs in the `node` environment, so `.vue` files are checked by `vue-tsc` and the attended check.
- **i18n:** every new key goes in `en` and `pt-BR`. The `MessageKey` union plus `vue-tsc` enforce parity.
- **Commits:** conventional messages, one logical change each. Every commit passes the verification commands below. End each message with `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.

**Verification commands**

```bash
cargo test -p melt-core --locked
cargo test -p melt-desktop --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
bun run --cwd ui test
bun run --cwd ui build
```

## Review Focus

1. **Stalled camera:** an unreachable camera must not block subscriptions to other sources. Test: Task 1 `an_unreachable_camera_does_not_stall_another`.
2. **One viewer stops only itself.** Test: Task 2 `stopping_one_viewer_leaves_the_others`. The attended check covers a fast view switch.
3. **Stale snapshot:** a frame older than 2 s, or one from before a reconnect, is never returned. Test: Task 1 `fresh_jpeg_rejects_old_frames`; the clear on stream failure is checked in review.
4. **Orphans after a webview reload:** WebRTC sessions don't end when the peer goes away, so the UI calls `camera_viewer_stop` with no id once on mount.
5. **Superseded starts:** when a start result arrives for a stale request, the UI stops that `viewerId` at once.

## Decisions (the draft plan guessed differently)

| Topic | Decision | Why |
|---|---|---|
| Move `camera_manager.rs` out of `bambu/` | No | Noise only; the type is already re-exported from `bambu` |
| Per-source startup | `ManagerState.startups: BTreeMap<key, Arc<Mutex<()>>>`. Manager lock is held only to look up or insert | Simplest way to serialize startups of one key without blocking others |
| Reconnect | `run_owner` takes `Box<dyn FnMut() -> Result<Source, Reopen> + Send>` (`Reopen::{Retry, Fatal}`) | Replaces Bambu-only `SourceConfig`; external cameras need nothing else |
| External owner `Source` | Existing `Source::H264(Box<H264Stream>)`, transport `CameraOwnerTransport::RtspH264` | `rtsp::H264Stream` is already shared |
| External owner `selection` | Neutral `CameraSelection { preferred: Unknown, source: Unknown, .. }` | Bambu diagnostics only. Desktop diagnostics match owners by printer key, so camera owners are ignored |
| Snapshot freshness | `latest_jpeg: Option<(Arc<[u8]>, Instant)>`, `fresh_jpeg(max_age)`, cleared when the stream fails | An owner's generation never changes across reconnects, so clearing the frame on failure is enough |
| Viewer ids | Backend `CameraViewers` registry; random 128-bit hex `viewerId` from every start | Replaces `CameraWebRtcState` and `CameraStreamState` and their generation and supersede maps |
| Effective printer feed | Resolved in the UI from `camera_list` (`selectedBy`). `{printer}` always means the built-in feed | No `effective_feed` command; the UI already needs the list for the picker |
| Old commands | Deleted. Task 2 renames the UI call sites in the same commit | Each commit stays green; no wrappers |
| Edit and remove versus running owners | No manager invalidation. Edit changes the revision, so the next subscribe makes a new owner. The UI restarts or stops its viewer after either action | One code path fewer |

---

### Task 1: Per-source startup, external subscriptions, fresh snapshots (melt-core)

**Files:**
- Modify: `crates/melt-core/src/bambu/camera_manager.rs` (`subscribe` ~300–380, `open_source` ~395, `run_owner` ~505–575, tests ~605+)
- Modify: `crates/melt-core/src/cameras.rs` (`StreamQuality` derives `Deserialize`), `crates/melt-core/src/cameras/profiles.rs` (`CameraRequest` derives `Deserialize`, `rename_all = "camelCase"`)

- [ ] **Step 1: Shared startup path.** Split `subscribe` into Bambu-specific setup and a private generic helper. No behavior change yet; the helper still takes the manager-wide lock.

```rust
pub(crate) enum Reopen { Retry, Fatal }
type Reopener = Box<dyn FnMut() -> Result<Source, Reopen> + Send>;

struct Opened {
    source: Source,
    selection: CameraSelection,
    repair_rtp_timestamps: bool,
    reopen: Reopener,
}

fn subscribe_with<E>(
    &self,
    key: String,
    revision: [u8; 32],
    kind: CameraFrameKind,
    open: impl FnOnce() -> Result<Opened, E>,
    unsupported: impl Fn() -> E,
    stream_error: impl Fn(std::io::Error) -> E,
) -> Result<CameraSubscription, E>
```

  Bambu's `open` runs `open_source` and builds its reopen closure from the transport that was actually opened. `Pin` and `Identity` map to `Reopen::Fatal`; everything else maps to `Retry`. `run_owner(owner, source, reopen)` calls `reopen()`: `Err(Fatal)` breaks the loop, `Err(Retry)` continues. The deadline and backoff logic stays as it is. Delete `SourceConfig`. Change the H.264 check to `owner.transport == CameraOwnerTransport::MjpegTls`.

- [ ] **Step 2: `subscribe_external`.**

```rust
pub fn subscribe_external(
    &self,
    camera: ResolvedCamera,
    quality: StreamQuality,
    kind: CameraFrameKind,
) -> Result<CameraSubscription, cameras::CameraError>
```

  - Key `camera:<id>:high|low`.
  - Revision is SHA-256 over host, port, username, password and quality, separated by `\0`.
  - `open` calls `cameras::open_stream(&camera, quality, false)` and wraps the result as `Source::H264`. `run_owner` turns preview decode on while JPEG subscribers exist.
  - The reopen closure owns `camera`. `Authentication`, `DigestRequired`, `UnsupportedCodec` and `UnknownDriver` are `Fatal`; every other error is `Retry`.
  - `repair_rtp_timestamps = false`.
  - Add the `CameraOwnerTransport::RtspH264` variant.
  - `unsupported` is unreachable for external cameras (always H.264); pass `|| CameraError::UnsupportedCodec`.

- [ ] **Step 3: Failing stall test.** Add this test helper and test:

```rust
fn external(id: &str, port: u16) -> ResolvedCamera {
    ResolvedCamera {
        name: id.into(),
        profile: CameraProfile {
            id: id.into(), driver: "tapo".into(), host: "127.0.0.1".into(),
            username: "melt".into(), rtsp_port: Some(port), timeout: "3s".into(),
            created: String::new(), updated: String::new(),
        },
        password: "secret".into(),
        timeout: Duration::from_secs(3),
    }
}

#[test]
fn an_unreachable_camera_does_not_stall_another() {
    // The backlog completes connect; DESCRIBE then waits for the 3 s read timeout.
    let silent = TcpListener::bind("127.0.0.1:0").unwrap();
    let silent_port = silent.local_addr().unwrap().port();
    let refused = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let manager = CameraManager::default();
    let stalled = {
        let manager = manager.clone();
        thread::spawn(move || {
            manager.subscribe_external(external("a", silent_port), StreamQuality::High, CameraFrameKind::H264).is_err()
        })
    };
    thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    assert!(manager.subscribe_external(external("b", refused), StreamQuality::High, CameraFrameKind::H264).is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(stalled.join().unwrap());
}
```

  Run: `cargo test -p melt-core --locked an_unreachable_camera_does_not_stall_another`. Expected: FAIL; B waits about 3 s behind A's lock.

- [ ] **Step 4: Per-key startup.** In `subscribe_with`:
  1. Lock the manager. Try reuse: the revision matches, and `try_subscribe` succeeds or returns the `unsupported` error. Otherwise clone or insert `startups[key]`, then unlock.
  2. Lock the startup mutex and try reuse again, because another caller may have finished meanwhile.
  3. Remove the stale owner from `owners` and `stop()` it.
  4. Call `open()` with no manager lock held.
  5. Lock the manager briefly to bump `generation` and insert the owner. Spawn `run_owner`.

  Add `// ponytail: startups is never pruned; it is bounded by printers + cameras × 2.`

  Run the stall test (expected: PASS), then `cargo test -p melt-core --locked camera_manager` (expected: all existing Bambu tests pass unchanged).

- [ ] **Step 5: Freshness.** Store `latest_jpeg: Option<(Arc<[u8]>, Instant)>`. Replace `CameraSubscription::latest_jpeg()` with `fresh_jpeg(max_age: Duration) -> Option<Arc<[u8]>>`. In `run_owner`, set `latest_jpeg = None` as soon as a read fails, before the reconnect wait. `has_jpeg` in `status()` keeps working. Test with the existing `owner()` helper:

```rust
#[test]
fn fresh_jpeg_rejects_old_frames() {
    let owner = owner();
    let subscription = owner.subscribe(CameraFrameKind::Jpeg);
    owner.publish(CameraFrame::Jpeg(Arc::from(&b"jpeg"[..])));
    assert!(subscription.fresh_jpeg(Duration::from_secs(2)).is_some());
    assert!(subscription.fresh_jpeg(Duration::ZERO).is_none());
}
```

- [ ] **Step 6: Deserialize derives** on `StreamQuality` (lowercase) and `CameraRequest` (camelCase, so `rtspPort`). Update the desktop's one `latest_jpeg` call site (`main.rs` ~3258) to `fresh_jpeg(Duration::from_secs(2))`. Run all the Rust verification commands. Expected: PASS.

- [ ] **Step 7: Commit** `feat(camera): start camera sources independently and share external camera streams`.

### Task 2: Viewer registry and `camera_*` commands (melt-desktop)

**Files:**
- Modify: `crates/melt-desktop/src/main.rs` (states ~427–455, camera commands ~3246–3416, `start_camera_server` ~3419, `subscribe_camera` ~3591, `.manage` ~3981, `generate_handler!` ~3998, tests)
- Modify: `ui/src/App.vue` (call sites at ~1985, 2010, 2054, 2109, 2164–2165, 2180 only)

- [ ] **Step 1: Failing registry test.**

```rust
#[test]
fn stopping_one_viewer_leaves_the_others() {
    let viewers = CameraViewers::default();
    let (a, b) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let first = viewers.insert(Viewer::Proxy(Arc::clone(&a))).unwrap();
    viewers.insert(Viewer::Proxy(Arc::clone(&b))).unwrap();
    viewers.stop(Some(&first));
    assert!(a.load(Ordering::Acquire) && !b.load(Ordering::Acquire));
    viewers.stop(None);
    assert!(b.load(Ordering::Acquire));
}
```

- [ ] **Step 2: `CameraViewers`.**

```rust
enum Viewer { WebRtc(webrtc_camera::Session), Proxy(Arc<AtomicBool>) }
// Drop: Proxy sets its stop flag; WebRtc's own Drop stops and joins its thread.

#[derive(Clone, Default)]
struct CameraViewers(Arc<Mutex<HashMap<String, Viewer>>>);
// insert(viewer) -> Result<String, CommandError>: id = hex(rand 16 bytes)
// stop(Option<&str>): remove one entry (Some) or drain all (None) under the
//   lock, and drop the removed entries after unlocking, because
//   Session::drop joins a thread.
```

  `start_camera_server(subscription, viewers, format) -> (viewer_id, url)` inserts `Viewer::Proxy(stop)` before spawning. The proxy thread calls `viewers.stop(Some(&id))` when it exits. Delete `CameraWebRtcState`, `WebRtcSessions`, `CameraStreamState` and `CameraProxy`.

- [ ] **Step 3: Media commands.** Add:

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum CameraSource { Printer(String), Camera(String) } // JSON: {"printer":"x"} | {"camera":"y"}

fn subscribe_source(manager, source, quality: Option<StreamQuality>, kind, operation)
    -> Result<CameraSubscription, CommandError>
// Printer -> desktop_printer + subscribe_camera (built-in feed, unchanged)
// Camera  -> cameras::resolve(config_dir, &SystemKeychain, name)
//            then manager.subscribe_external(camera, quality.unwrap_or_default(), kind)
```

  | New command | Returns | Replaces |
  |---|---|---|
  | `camera_webrtc_offer(source, quality?, offer)` | `{viewerId, type, sdp}` | `printer_camera_webrtc_offer` |
  | `camera_h264_stream(source, quality?)` | `{viewerId, url, codec}` | `printer_camera_h264_stream` |
  | `camera_mjpeg_stream(source, quality?)` | `{viewerId, url}` | `printer_camera_stream` |
  | `camera_snapshot(source, quality?, destination?)` | `{dataUrl}` | `printer_camera_snapshot`; uses `fresh_jpeg(2 s)`, else waits up to 10 s for a new frame |
  | `camera_viewer_stop(viewerId?)` | `()` | both stop commands; with no id it stops all |

  Delete the six `printer_camera_*` commands and their handler entries. Add one assertion to the existing `h264_codec` test: `h264_codec(&[0x27, 0x4d, 0x00, 0x34]) == Some("avc1.4d0034")`. These bytes are the C510W stream1 SPS header from `fixtures/tapo/`.

- [ ] **Step 4: Profile commands**, thin wrappers over `cameras::*` with `config_dir()` and `SystemKeychain`:
  - `camera_list`
  - `camera_add(request)`: `create(.., cameras::verify)`
  - `camera_edit(name, request)`: `update(.., cameras::verify)`
  - `camera_remove(name)`: returns `CameraRemoveResult`
  - `camera_link(camera, printer, select)`
  - `camera_unlink(camera, printer)`
  - `camera_select(printer, camera?)`

  Map errors with `camera_error(cameras::CameraError) -> CommandError`, and add a table test over every variant:

  | Variant | Code |
  |---|---|
  | `MissingName` | `profileMissingName` |
  | `InvalidName` | `profileInvalidName` |
  | `InvalidHost` | `profileInvalidHost` |
  | `UnknownDriver`, `InvalidQuality` | `profileInvalid` |
  | `InvalidUsername` | `cameraInvalidUsername` |
  | `InvalidPassword` | `cameraInvalidPassword` |
  | `InvalidPort` | `cameraInvalidPort` |
  | `InvalidTimeout` | `cameraInvalidTimeout` |
  | `NotFound` | `cameraNotFound` |
  | `AlreadyExists` | `cameraExists` |
  | `PrinterNotFound` | `profileNotFound` |
  | `NotLinked` | `cameraNotLinked` |
  | `Authentication` | `cameraAuthFailed` |
  | `DigestRequired` | `cameraDigestRequired` |
  | `Connect`, `Stream` | `cameraUnreachable` (+ `detail`, no secrets) |
  | `Timeout` | `cameraTimeout` |
  | `UnsupportedCodec` | `cameraUnsupportedCodec` |
  | `MissingSecret` | `cameraPasswordUnavailable` |
  | `Secret`, `RollbackFailed` | `keychainFailed` |
  | `Config` | `configUnwritable` |

  Printer rename and remove already carry or drop links in core (`config.rs` `rename_profile`/`remove_profile`), so the desktop needs no change there.

- [ ] **Step 5: Keep the UI working.** In `App.vue`, rename the invokes and pass `{ source: { printer: printerName } }`. `stopCameraSession` becomes `invoke('camera_viewer_stop')` with no id, which stops all, as today. Add the new error codes to `i18n.ts` (en + pt-BR) so `vue-tsc` passes.

- [ ] **Step 6:** Run all the verification commands. Expected: PASS. Commit `feat(desktop): viewer-scoped camera sessions and camera profile commands`.

### Task 3: Pure camera logic (UI)

**Files:**
- Create: `ui/src/cameras.ts`, `ui/src/cameras.test.ts`
- Modify: `ui/src/presentation.ts`, `ui/src/presentation.test.ts` (~33–51), `ui/src/App.vue` (`cameraMayRun` ~953)

- [ ] **Step 1: Tests first**, then implement:

```ts
export type Quality = 'high' | 'low'
export type CameraSource = { printer: string } | { camera: string }
export type CameraSummary = {
  name: string; id: string; driver: string; host: string; username: string
  rtspPort: number; timeout: string; printers: string[]; selectedBy: string[]
}
export function linkedCameras(printer: string, cameras: CameraSummary[]): CameraSummary[]
export function printerSource(printer: string, cameras: CameraSummary[]): CameraSource // selected camera, else built-in
export function linkChanges(before: string[], after: string[]): { link: string[]; unlink: string[] }
export function sourceKey(source: CameraSource, quality: Quality): string // restart a viewer when this changes
```

  Cases:
  - no links gives the built-in feed
  - a linked camera that is not selected gives the built-in feed
  - a selected camera gives that camera
  - `linkChanges` with adds, removals and no-ops
  - `sourceKey` differs by kind, name and quality

- [ ] **Step 2:** Change `shouldRunCamera(activeView, viewerView, visible, supported)` to `activeView === viewerView && …` and update its tests. The control view passes `'control'`.
- [ ] **Step 3:** Run `bun run --cwd ui test` and `bun run --cwd ui build`. Commit `feat(ui): camera source helpers`.

### Task 4: Extract `camera-session.ts` and `CameraViewer.vue` (no behavior change)

**Files:**
- Create: `ui/src/camera-session.ts`, `ui/src/components/CameraViewer.vue`
- Modify: `ui/src/App.vue` (refs ~477–499, computeds ~933–965, functions ~1964–2185, card ~2726–2756, `onUnmounted` ~2428)

- [ ] **Step 1: `useCameraSession(options)`** with these options:

  ```ts
  source: () => CameraSource | undefined
  quality: () => Quality
  mayRun: () => boolean
  canStream: () => boolean
  canSnapshot: () => boolean
  canvas: () => HTMLCanvasElement | undefined
  translate: Translate
  ```

  Move these from `App.vue`:
  - `refreshCamera`, `waitForVideoTrack`, `waitForPeerReady`, `fallbackCamera`, `fallbackMjpegCamera`, `withTimeout`, `waitForIceGathering`, `stopCamera` and `saveSnapshot`'s invoke
  - the request counter
  - the refs, which the composable returns, with `cameraState`/`cameraHasMedia`

  Track `viewerIds: Set<string>`:
  - add each returned id
  - `stopCamera` stops each id, not all
  - when a stale request gets a result, stop that id at once

  Watch `mayRun` and `sourceKey` to start or stop the viewer, and call `stopCamera` from `onScopeDispose`.

- [ ] **Step 2: `CameraViewer.vue`** renders the card body exactly as in `App.vue`: header badge, refresh, snapshot and maximize buttons, `video`/`canvas`/`img`/unavailable states, and a `#controls` slot in the header.
  - Props: `session` (the composable's return value), `title` and `t`.
  - Emits `snapshot-saved`, which App shows as the toast.
  - The fullscreen target is the viewer's own stage.
- [ ] **Step 3:** In `App.vue`, call `invoke('camera_viewer_stop')` with no id once in `onMounted` to clear sessions orphaned by a webview reload. Replace the control card with `<CameraViewer>`, keeping the `source: { printer }` session.
- [ ] **Step 4:** Run `bun run --cwd ui test` and `bun run --cwd ui build`. Then `make run` on a Bambu printer: the feed starts, refresh works, a snapshot saves, and the stream stops when you leave the view. If no Bambu printer is reachable, record that and rely on Task 6's checks. Commit `refactor(ui): extract the camera viewer`.

### Task 5: Cameras view, feed picker, i18n

**Files:**
- Modify: `ui/src/App.vue` (`View` ~105, `views` ~507, `sectionTabs` ~509, settings view ~3390 as the layout model, control card), `ui/src/i18n.ts`

- [ ] **Step 1: Load cameras.** `cameras = ref<CameraSummary[]>([])`, loaded with `camera_list` next to `configured_printers` and reloaded after every camera mutation. Load failure is shown in the Cameras view only.
- [ ] **Step 2: Cameras view** (`'cameras'` added to `View` and the nav).
  - **List** shows name, host and linked printers, plus add, edit and remove actions.
  - **Viewer:** selecting a camera opens one `CameraViewer` with `source: { camera }`. A `high | low` toggle in its `#controls` slot defaults to `high` and is not persisted.
  - **Add/edit `SlideOver`:**
    - Fields: name, host, username, password, RTSP port (placeholder 554) and timeout (default `10s`). The driver is fixed to `tapo`.
    - On edit, the password field is empty and its hint says that an empty password keeps the current one.
    - Printer checkboxes are applied after save with `linkChanges` → `camera_link`/`camera_unlink`.
    - Saving runs a live verification (up to the timeout). Show it as busy, and show `commandMessage` plus `commandDetail` on failure.
    - After saving the open camera, restart its viewer.
  - **Remove** asks with `ConfirmDialog`, stops the viewer first, then shows `warnings` from `CameraRemoveResult` as a toast.
- [ ] **Step 3: Control-view feed picker.** Show a `select` in the camera card's `#controls` only when `linkedCameras(printer)` is non-empty. It lists "Built-in camera" first, then the linked cameras.
  - Changing it calls `camera_select(printer, camera?)` and reloads cameras; the session restarts through `sourceKey`.
  - Feed: `printerSource(printer, cameras)`. `canStream` is true for an external feed, else `capabilities.cameraStream`. `cameraSupported` gets the same external-or-capabilities rule.
  - An external feed runs while the printer is offline.
- [ ] **Step 4: i18n** (en + pt-BR):
  - `nav.cameras` and `camerasView.*`: title, empty state, add/edit/remove, field labels and hints, quality, built-in feed, linked printers, remove confirmation, saved/removed toasts
  - `errors.camera*` from Task 2, if not already added
  - a connection hint pointing to `docs/ip-cameras.md` troubleshooting (Camera Account, Third-Party Compatibility, privacy mode, stream limit)
- [ ] **Step 5:** Run `bun run --cwd ui test` and `bun run --cwd ui build`. Commit `feat(ui): cameras view and printer feed selection`.

### Task 6: Docs and attended desktop check

**Files:** `docs/ip-cameras.md`

- [ ] **Step 1:** Add a "Desktop" subsection to "Everyday use" in `docs/ip-cameras.md`. It covers adding a camera (live verification), linking and the feed picker, the quality toggle, and how viewers share one camera connection. Troubleshooting already covers Third-Party Compatibility, privacy mode and the stream limit; add rows only for new desktop-specific failures.
- [ ] **Step 2 (attended, C510W + `make run`):** record the result of each check below in the commit body.
  1. **Wrong password** on add shows `cameraAuthFailed` and saves nothing (no YAML entry and no keychain entry).
  2. **Add** works with the right password.
  3. **Viewer:** the Cameras viewer goes live. Note the transport from the badge tooltip (`webrtc`, `webcodecs` or `mjpeg`). This answers whether WebKitGTK's WebRTC offer matches the C510W's Main profile (`4D0034`/`4D001F`).
  4. **Quality toggle** switches high and low.
  5. **Link and select** the camera for a printer: the control-view picker appears, switching between built-in and camera works, and only that viewer restarts.
  6. **Fast switching** between Control and Cameras on the same camera: the new viewer keeps playing.
  7. **Snapshot** from each view saves a current frame.
  8. **Unplug the camera:** the viewer shows an error and the printer badge stays online. Plug it back in and refresh: it recovers.
  9. **Soak:** 5 min of live view without freezing (keepalive).
  10. **Closing** the view drops the camera's RTSP session (`ss -tn dst 10.15.15.123:554` is empty within a few seconds).
- [ ] **Step 3:** Commit `docs: desktop IP camera usage`.

## Out of scope

Pan/tilt and ONVIF (Phase 3), discovery (Phase 4), audio, thumbnails, polling, persisted quality, multiple simultaneous viewers on one screen.

## Decided

1. **Control-view quality:** `high`, same as the Cameras view (confirmed 2026-10-08).
2. **Feed picker scope:** only cameras linked to the printer; linking happens in the camera form (confirmed 2026-10-08).
3. **Phase 1 Task 7 leftovers** (Third-Party Compatibility state, the stream limit number, CLI add/edit/remove and wrong-password checks) stay separate. Task 6 checks 1 and 10 above partly cover them through the desktop.
