# IP Cameras Phase 2 (Desktop) Implementation Plan

> Spec: `docs/superpowers/specs/2026-10-08-ip-cameras-design.md` ("Requirements carried to Phase 2"). Phase 1 (core, config, CLI) is merged; Task 7 qualified the C510W (hw 1.0 / fw 1.3.5).

**Goal:** Desktop Cameras view, per-printer feed selection, one shared viewer, and viewer-scoped sessions, over the Phase 1 core API (`cameras::{resolve, open_stream, snapshot}`, `cameras::profiles::*`).

**Scope cuts (ponytail):** no new dependencies, no thumbnails or polling, no pan/tilt (Phase 3), no audio. External cameras are H.264 only, so MJPEG proxy and WebCodecs reuse the existing converters; WebRTC reuses `webrtc_camera::start`.

**Verification commands:** `cargo test -p melt-core --locked`, `cargo test -p melt-desktop --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `bun run --cwd ui test`, `bun run --cwd ui build`.

## Existing code this plan touches

| Area | Today | Problem |
|---|---|---|
| `melt-core/src/bambu/camera_manager.rs` | `CameraManager::subscribe` takes a Bambu `Profile`, keys owners by `bambu:<SERIAL>`, and opens the upstream while holding the manager-wide lock | An unreachable camera stalls every other camera; no external source |
| `melt-desktop/src/main.rs:3246-3416` | `printer_camera_*` commands; `printer_camera_stream_stop` stops **all** MJPEG/H.264 proxies; WebRTC sessions keyed by printer | Two viewers fight; selection by printer only |
| `ui/src/App.vue:1964-2062` | Camera lifecycle (refresh, fallback, stop) inline in a 4000-line file, bound to `activePrinter` | Not reusable for a camera-only view |
| `ui/src/presentation.ts` | `cameraViewState`, `shouldRunCamera` | Reusable as is |

## Design decisions

1. **Source key.** `CameraSource` = `Printer(name)` | `Camera(name)`. Upstream key: `bambu:<SERIAL>` (unchanged) or `camera:<id>:<high|low>`.
2. **Manager generalization, not a second manager.** `CameraManager::subscribe_external(&ResolvedCamera, quality, kind)` beside the Bambu `subscribe`. Both share `Owner`, `FrameMailbox`, `run_owner`. External uses `cameras::open_stream(camera, quality, decode)`; reconnect reuses the same call. Only `Source` gains a variant that wraps an H264 stream already (`Source::H264`), so no new variant is expected.
3. **Per-source startup, no global lock while connecting.** Replace the held `MutexGuard` with a per-key slot: `owners: BTreeMap<String, Slot>` where `Slot` is `Arc<Mutex<Weak<Owner>>>`. Lookup takes the manager lock briefly to get or create the slot, releases it, then locks only the slot while connecting. Applies to Bambu too.
4. **Viewer sessions.** Each viewer start returns a `viewerId` (random 128-bit hex). `CameraViewerState` maps `viewerId -> ViewerSession { source, kind: Webrtc(session) | Mjpeg(stop flag) | H264(stop flag) }`. `camera_viewer_stop(viewerId)` stops exactly one; the upstream releases when the last subscription drops (already the `Owner::unsubscribe` behavior). Replaces `printer_camera_stream_stop` and `printer_camera_webrtc_stop`.
5. **Snapshot freshness.** `CameraSubscription::latest_jpeg()` returns `(Arc<[u8]>, Instant)`; accept only age <= 2 s and the current owner generation, else wait for a new frame within the timeout. Owner is dropped on reconnect failure, so a stale generation cannot leak across connections.
6. **Isolation.** External commands never touch printer status cache or `invalidate`. A printer's effective feed is resolved at viewer start: linked and selected camera, else built-in. Failure of an external feed returns `cameraPreviewUnavailable` with detail and never marks the printer offline.
7. **Credentials.** Camera passwords reach the desktop only via add/edit forms (typed into a password input, sent once to the command). They are never returned to the UI. Resolved with `cameras::profiles::resolve(dir, &SystemKeychain, name)` inside the command.

## File map

- Modify `crates/melt-core/src/bambu/camera_manager.rs` (move to `crates/melt-core/src/camera_manager.rs`, re-export from `bambu` so existing imports keep compiling).
- Modify `crates/melt-core/src/cameras.rs` (nothing expected; add `generation` accessor only if Task 1 needs it).
- Modify `crates/melt-desktop/src/main.rs` (new commands, viewer state; delete the two stop commands).
- Create `ui/src/cameras.ts` + `ui/src/cameras.test.ts`.
- Create `ui/src/components/CameraViewer.vue`, `ui/src/useCameraSession.ts`.
- Modify `ui/src/App.vue` (use the viewer; Cameras view; feed picker), `ui/src/i18n.ts` (en + pt-BR).
- Modify `docs/ip-cameras.md`, spec phase table.

---

### Task 1: Per-source startup and external subscriptions (melt-core)

**Files:** `camera_manager.rs`, tests in the same file.

- [ ] **Step 1: Failing test, an unreachable source does not block another.** Open a listener that accepts and never answers (source A, `timeout` 3 s) on thread 1; on thread 2 subscribe to a source B backed by a working fake and assert it returns in under 1 s. Use the existing test fakes in `camera_manager.rs` tests; if `open_source` is not injectable, add a `#[cfg(test)]` opener field on `CameraManager`.
- [ ] **Step 2: Slot-per-key locking** (decision 3). Run the new test, expect PASS; run `cargo test -p melt-core --locked camera_manager` for the existing Bambu tests, expect PASS with unchanged wire behavior.
- [ ] **Step 3: `subscribe_external`.** Key `camera:<id>:<quality>`, H264 transport, `parameters` from `H264Stream::parameter_sets`, `repair_rtp_timestamps = false`, revision = SHA-256 of `host|port|username|password|quality`. A changed revision stops the old owner (matches Bambu).
- [ ] **Step 4: Freshness** (decision 5). Test: a frame older than 2 s is not returned by `snapshot_since`; a new frame within the timeout is.
- [ ] **Step 5: Commit** `feat(camera): start camera sources independently and add external subscriptions`.

### Task 2: Viewer-scoped sessions and external commands (melt-desktop)

**Files:** `crates/melt-desktop/src/main.rs`.

- [ ] **Step 1: Failing tests** for pure helpers: `effective_feed(config, printer) -> Feed` (built-in when no link or no `selected`), and `ViewerRegistry` (insert, stop one leaves the other, `stop_all`).
- [ ] **Step 2: `ViewerRegistry`** replacing `CameraStreamState.active` and `CameraWebRtcState.sessions` for stop semantics. Keep the proxy and WebRTC code; change only how they register and stop.
- [ ] **Step 3: Commands.** All take `source: {printer?: string, camera?: string}`, `quality?: "high"|"low"`, and return `viewerId`:
  - `camera_viewer_webrtc_offer(source, offer, quality)` -> `{viewerId, sdp}`
  - `camera_viewer_mjpeg(source, quality)` -> `{viewerId, url}`
  - `camera_viewer_h264(source, quality)` -> `{viewerId, url, codec}`
  - `camera_viewer_snapshot(source, quality, destination?)`
  - `camera_viewer_stop(viewerId)`
  Printer sources resolve through `effective_feed`; built-in goes to the existing `subscribe_camera`. Keep the old `printer_camera_*` command names as thin wrappers only if the UI migration in Task 5 is not done in the same PR; otherwise delete them.
- [ ] **Step 4: Profile commands**, thin over `cameras::profiles`: `camera_list`, `camera_add`, `camera_edit`, `camera_remove`, `camera_link`, `camera_unlink`, `camera_select`. Add and edit call `cameras::verify` before saving through the Phase 1 workflow. Map `CameraError` to existing `CommandError` codes as the CLI does (auth to `authenticationFailed`, connection to `connectionFailed`, etc.; reuse the CLI's mapping function if it is in core, else mirror the table in the spec).
- [ ] **Step 5: Camera removal and printer rename/removal** already update links in core; verify the desktop's printer rename/remove paths call the core workflow and add a test if not.
- [ ] **Step 6: Run** `cargo test -p melt-desktop --locked` and clippy. Commit `feat(desktop): viewer-scoped camera sessions and camera profile commands`.

### Task 3: Pure UI logic (`ui/src/cameras.ts`)

- [ ] **Step 1: Tests first** (`cameras.test.ts`): `feedLabelKey`, `feedOptions(printer, cameras)` (built-in first, then linked cameras), `sourceKey(source, quality)`, `canStartViewer` (visible, supported, source present), `nextTransport(failure)` (webrtc -> webcodecs -> mjpeg).
- [ ] **Step 2: Implement**, reusing `shouldRunCamera` and `cameraViewState` from `presentation.ts` rather than duplicating them.
- [ ] **Step 3:** `bun run --cwd ui test`. Commit.

### Task 4: Extract `CameraViewer.vue` and `useCameraSession.ts`

- [ ] **Step 1:** Move `refreshCamera`, `fallbackCamera`, `stopCamera*`, and their refs out of `App.vue` into `useCameraSession(source, quality)` returning `{state, error, transport, videoRef, canvasRef, restart, stop}`. Keep the request-counter pattern (`cameraRequest`) so superseded starts are discarded.
- [ ] **Step 2:** `CameraViewer.vue` renders the video/canvas/img and state badge, props `source`, `quality`, `active`. Behavior must not change for printers with built-in cameras; verify by running the app (`make` target from the Makefile) against a Bambu profile if one is available, else rely on the unit tests plus code review.
- [ ] **Step 3:** The viewer calls `camera_viewer_stop(viewerId)` on unmount, source change, hidden window, and view switch.
- [ ] **Step 4:** `bun run --cwd ui build` and tests. Commit `refactor(ui): extract the camera viewer`.

### Task 5: Cameras view and printer feed picker

- [ ] **Step 1: i18n** keys (en and pt-BR, parity enforced by `i18n.test.ts`): `cameras.title`, `cameras.add`, `cameras.edit`, `cameras.remove`, `cameras.quality.high|low`, `cameras.feed.builtin`, `cameras.privacyHint`, `cameras.thirdPartyHint`, `cameras.streamLimitHint`, error messages.
- [ ] **Step 2: Cameras view**: list (name, host, linked printers), add/edit `SlideOver` (name, host, username, password, port, timeout), remove with `ConfirmDialog`, link to printers (multi-select), open viewer with quality toggle. Add `'cameras'` to `type View` and the nav.
- [ ] **Step 3: Printer control view**: feed `select` listing `feedOptions`; changing it calls `camera_select` and restarts only that viewer. Show external feed even when the printer is offline.
- [ ] **Step 4:** No polling and no hidden thumbnails (spec). Camera status is the open viewer's state.
- [ ] **Step 5:** Tests and build. Commit `feat(ui): cameras view and printer feed selection`.

### Task 6: Docs and attended desktop check

- [ ] **Step 1:** `docs/ip-cameras.md`: Desktop section, Third-Party Compatibility, privacy mode, and the camera's concurrent stream limit (record the observed number here; Task 7 of Phase 1 left it open).
- [ ] **Step 2:** Update the spec Phase table (Phase 2 plan link).
- [ ] **Step 3 (attended):** with the C510W: add via UI; wrong password shows an auth error and saves nothing; two viewers (view + another window or the CLI) on one camera; stop one, the other keeps playing; stop both, the connection closes (camera session count drops); switch a printer feed built-in <-> camera; unplug the camera, the viewer shows an error and the printer stays online; 5 min soak for keepalive.
- [ ] **Step 4:** Commit `docs: desktop IP camera usage`.

## Open questions

1. **Printer-scoped viewer.** Should a printer's viewer be the effective feed only, or also let the user pick quality for it? (Plan: quality picker only when the effective feed is external.)
2. **WebRTC for external cameras.** The existing `webrtc_camera::start` expects the Bambu H.264 shape. If the C510W's `profile-level-id` (4D0034 / 4D001F, Main profile) is rejected by `compatible_offer_profile`, fall back to WebCodecs. Task 2 Step 3 must test this with the captured SDPs in `fixtures/tapo/`.
3. **Move `camera_manager.rs`?** Plan moves it out of `bambu/` for clarity. Leave in place and only re-export if the diff is too noisy.
