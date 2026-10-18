# Development and architecture

Last-updated: 2026-10-12

## Prerequisites and first build

The Rust workspace uses edition 2024 and declares MSRV **1.95**. Install
Rust/Cargo with rustfmt and Clippy, Bun (CI pins **1.3.14**), and native Tauri 2
dependencies. The frontend uses Vue 3, TypeScript 6, Vite 8, Tailwind CSS 4,
Headless UI, Phosphor icons and Vitest. Cargo.lock and ui/bun.lock govern
reproducible dependency resolution; use the locked Make targets.

Linux CI installs `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`,
`libayatana-appindicator3-dev` and `librsvg2-dev`; installer builds also install
`rpm`. OpenSSL headers/libraries are needed by the Rust transport/keychain
dependencies. Windows jobs point `OPENSSL_DIR`, `OPENSSL_LIB_DIR` and
`OPENSSL_INCLUDE_DIR` at the runner's OpenSSL installation. Authenticated local
printer/camera tests also need that user's OS keychain session.

From the repository root:

```sh
make ui-install
make build
./target/debug/melt --help
make run
```

`make build` compiles the debug Rust workspace and type-checks/builds the UI; it
does not create native installers. `make run` starts Vite at
`http://127.0.0.1:1420`, waits for it, then runs `melt-desktop`'s `melt` binary.
The development app uses this server, while packaged builds use `ui/dist`.
CLI commands enter before GTK/Tauri startup and do not need a display server.
Serving the UI alone does not provide the Tauri backend or printer controls.

For a native bundle, install the Tauri CLI and build the frontend first;
the workflow uses:

```sh
make ui-install ui-build
cargo install tauri-cli --locked
cargo tauri build --bundles deb,rpm
```

Select the bundle type for the host platform (`dmg` on macOS, `msi` on Windows).
Bundles are written under `target/release/bundle`. The checked-in Tauri config
has no frontend build hook, so an explicit UI build is required. See
[release qualification](release-qualification.md) before calling a bundle qualified.

## Workspace boundaries

| Location | Responsibility |
| --- | --- |
| `crates/melt-core/src/config.rs`, `profiles.rs`, `keychain.rs`, `preferences.rs` | Versioned local state, profile workflows, credentials and preferences |
| `crates/melt-core/src/drivers.rs`, `pool.rs`, `monitor.rs` | Closed printer-driver registry, shared desktop sessions and bounded monitoring |
| `crates/melt-core/src/bambu/` | MQTT, pinned TLS, discovery, storage, package/print workflow, capability and firmware evidence |
| `crates/melt-core/src/moonraker.rs` | HTTP status, storage, jobs, controls and Klipper Update Manager |
| `crates/melt-core/src/rtsp.rs`, `cameras.rs`, `cameras/` | Shared RTSP/Digest/H.264 client and independent Tapo workflows |
| `crates/melt-core/src/bambu/camera_manager.rs` | Shared Bambu/external camera upstreams and bounded subscriptions |
| `crates/melt-core/src/diagnostics.rs`, `trace.rs` | Redacted compatibility reports and protocol tracing |
| `crates/melt-cli` | Public help, parsing, confirmation, JSON envelopes and exit codes |
| `crates/melt-desktop` | The `melt` entry point, Tauri commands/events, local library/previews and camera playback bridges |
| `ui/src/App.vue`, `components/` | Desktop views and reusable UI, including viewer-scoped `CameraViewer.vue` |
| `ui/src/*.ts`, `*.test.ts` | Pure presentation/validation/playback helpers and Vitest coverage |
| `fixtures/` | CLI goldens, Bambu evidence manifests/transcript, sanitized Tapo SDP |

Both surfaces call the Rust core directly. Desktop actions do not spawn a
separate CLI process. A profile-aware connection pool shares Moonraker HTTP
pools and one authenticated Bambu MQTT session per printer. Changing an
endpoint, TLS mode, timeout or credential replaces its session; removal evicts
it. One-shot CLI operations use short-lived connections.

Status, firmware and discovery use separate workers/caches. Desktop status
polling targets five-second cycles with one thread per configured printer and
per-profile backoff; it currently does not apply the core's eight-worker cap.
Firmware checks use batches of up to eight workers, start after ten seconds and recur on a 30-minute
cadence. Discovery uses a four-second scan every 15 seconds and 60-second
presence retention. A local status snapshot accelerates initial display and
is accepted for at most 30 seconds; live monitoring still establishes state.

Each desktop camera viewer owns a subscription ID. Shared upstreams are keyed
by physical printer serial or camera ID plus quality and connection revision.
WebRTC forwards RTP; WebCodecs uses an authenticated loopback binary bridge
and AVC decoder configuration; MJPEG/snapshots attach JPEG consumers only when
needed. Last-viewer teardown closes the upstream. Camera errors are isolated
from printer health. See [camera transport details](bambu-compatibility.md#camera-transport-selection)
and [IP cameras](ip-cameras.md).

The desktop also owns bounded model-preview rendering, local file watching,
transfer progress/cancellation and slicer subprocess management. Its blocking
commands run on Tauri workers. The workspace dev profile disables reqwest's
debug assertion for its blocking client so development firmware/Moonraker
commands can run on those workers; this is an implementation constraint, not
permission to run blocking I/O in the UI.

## Build and verification targets

| Target | Checks/actions |
| --- | --- |
| `make ui-install` | `bun install --cwd ui --frozen-lockfile` |
| `make cargo-build` | `cargo build --workspace --locked` |
| `make ui-build` | `vue-tsc --noEmit` then Vite production build |
| `make build` | Rust debug build and UI build |
| `make cargo-test` | All Rust workspace tests with `--locked` |
| `make ui-test` | `bun run --cwd ui test`, which runs Vitest |
| `make screenshots` | Build the production UI and capture documentation views using isolated sample IPC/media; see [screenshot prerequisites](screenshots.md#reproducing-the-captures) |
| `make test` | Rust and UI tests |
| `make cargo-lint` / `make lint` | rustfmt check and Clippy, all targets, warnings denied |
| `make contract` | Checked-in CLI JSON/exit fixtures |
| `make bambu-evidence-check` | Bambu manifest/redaction integration test; transcript execution is covered by core unit tests |
| `make license-check` | AGPL metadata, license file and bundled license declaration |
| `make ci` | License/evidence checks, locked UI install, lint, tests, builds and CLI fixtures |
| `make release-gate` | Evidence, CLI fixtures, Rust build and built executable version smoke test |
| `make release-qualification` | Full `ci` plus executable version smoke test |
| `make release-cli-check` | `target/debug/melt version --output json` against empty fixture config; requires a prior build |
| `make release-evidence RELEASE_TAG=<tag>` | Structural validation of frozen release documents |
| `make install-desktop` / `make uninstall-desktop` | Add/remove the development desktop entry and hicolor icons |
| `make help` | Full target list |

Use `bun run --cwd ui test`, not bare `bun test`: the latter runs Bun's own
runner instead of the configured Vitest suite. UI tests exercise helper logic
in Node; Vue templates are checked by `vue-tsc`, and visual/native playback
still needs an attended desktop check. Rust network tests use local simulated
servers; they do not require a physical printer. Targeted camera coverage can
be run with `cargo test -p melt-core --test tapo_simulated --locked`.

## CI, dependency updates and contribution checks

The CI workflow has Linux Rust, Linux UI, and native desktop build jobs for
Intel/Apple Silicon macOS and Windows. The Linux Rust job runs license,
format/Clippy, workspace tests/build and contract targets. The evidence
integration suite is included in workspace tests. The UI job installs locked
dependencies and runs Vitest/type-check/build. Native jobs compile the desktop;
CI does not install or launch every packaged artifact.

Dependabot groups weekly Cargo, Bun and GitHub Actions updates. It defers
WebRTC minor/major upgrades (pre-1.0 API breaks), vek minor/major upgrades (euc
compatibility), and the TypeScript next major (vue-tsc compatibility).

Keep driver behavior in the core and surface adapters thin. Add user-facing
strings to both `en` and `pt-BR` catalogs. Preserve the public CLI contract,
register new paths in the command inventory, and cover accepted response
changes with fixtures. Changes to undocumented Bambu behavior need scoped
evidence and qualification rather than broad model assumptions. Update the
relevant current guide alongside a behavior change; historical plans should
identify departures and point to it.

Use [the interface system](../.interface-design/system.md) for shared UI state
rules and [Wayland notes](../README.md#running-on-linux-wayland) for native H.264
renderer troubleshooting. The development desktop entry is for shell icon
resolution and is independent of packaged installers.
