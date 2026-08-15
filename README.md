# Melt

<https://melt.polimero.app>

Melt is a local-first 3D printer control application with two surfaces from
one executable:

- a headless CLI for automation and scripting;
- a Tauri desktop application for day-to-day printer control.

The CLI, GUI, desktop application, and shared source are licensed under the GNU
Affero General Public License v3.0 only (`AGPL-3.0-only`). See
[`LICENSE`](LICENSE). Corresponding source is available from
<https://github.com/polimero-app/melt>.

This Rust/Tauri workspace unifies and supersedes the legacy standalone CLI,
Fyne GUI, and Go application implementations.

## Features

- **Printer profiles and discovery** — configure Bambu LAN and Moonraker
  printers, store secrets in the OS keychain, discover compatible printers on
  the local network, and refresh pinned TLS identities explicitly.
- **Fleet and dashboard views** — inspect online/idle/busy state, temperatures,
  job progress, layers, time estimates, fans, lights, Wi-Fi signal, and AMS
  inventory. Background monitors retain the last useful status during bounded
  reconnect attempts.
- **Camera** — share one authenticated upstream per physical Bambu printer
  across snapshots, browser MJPEG previews, and WebRTC viewers, with bounded
  fan-out and reconnect behavior.
- **File and print workflow** — browse printer storage, upload and download
  files, preview supported models, inspect sliced 3MF packages, reconcile
  filament/nozzle assignments, select a plate, and start a print only after
  preflight and confirmation.
- **Printer control** — pause, resume, or cancel jobs; set safe temperature,
  fan, light, and speed targets; home or jog axes; and issue an immediate
  emergency stop where the driver supports it.
- **Diagnostics** — export redacted compatibility information covering model
  identity, module firmware, capability provenance, authorization, negotiated
  transports, TLS-pin presence, camera-owner state, and active/inactive quirks.
- **Automation contract** — request stable JSON envelopes and exit codes from
  the same executable used by the desktop application.

## Quick start

Install the UI dependencies once, then build the complete workspace:

```sh
make ui-install
make build
./target/debug/melt --help
```

Launch `./target/debug/melt` without arguments to open the desktop
application. Passing command arguments selects headless CLI mode instead.

Source builds require Rust 1.85 or newer, Bun, and the platform dependencies
required by Tauri 2. Linux packages declare FFmpeg as a runtime dependency for
the baseline camera media policy. Printer credentials are stored through the
platform keychain, so a usable Secret Service or desktop keychain must be
available for authenticated profiles.

## CLI

The command groups cover:

- `printer add|list|remove|drivers|discover|capabilities|tls refresh`;
- `status`;
- `camera snapshot|stream`;
- `files roots|list|download|upload`;
- `jobs preflight|start|pause|resume|cancel`;
- `temperature set`, `motion home|jog`, `fans set`, `lights set`, and
  `speed set`;
- `emergency-stop` and `version`.

Run any command with `--help` for its current arguments and flags. Common
global options are:

- `--output human|json` selects human-readable output or the stable
  `{ok, data, error, meta}` JSON envelope;
- `--verbose` or `-v` enables additional human-mode progress while remaining
  suppressed in JSON mode.

Examples:

```sh
melt printer discover
melt printer add workshop --driver bambu-lan --host 192.0.2.10 \
  --serial SANITIZED_SERIAL --access-code-file ./access-code.txt
melt status workshop --detailed
melt printer capabilities workshop --output json
melt jobs preflight ./part.gcode.3mf --output json
```

On Unix, credential files supplied to `--access-code-file` must be regular
files with user-only permissions (for example, mode `0600`).

State-changing commands require explicit confirmation. In non-interactive CLI
use, pass `--yes` only after independently validating the target printer and
operation.

## Project layout

- `crates/melt-core` contains UI-independent drivers, profiles, protocol
  transports, validation, monitoring, and printer workflows.
- `crates/melt-cli` adapts shared behavior to the public command, output,
  and exit-code contract.
- `crates/melt-desktop` selects CLI mode when arguments are present and
  starts Tauri for a no-argument launch.
- `ui` contains the Vue, TypeScript, Tailwind CSS, and Headless UI frontend.
- `fixtures` contains CLI contract fixtures and sanitized, manifest-governed
  Bambu protocol evidence.
- `docs` contains compatibility, qualification, evidence, and release policy.

Unlike the retired Fyne GUI, the desktop does not shell out to a separate CLI
process for each action. Both surfaces call the same Rust core. The desktop
also reuses one profile-aware connection session per printer: Moonraker shares
its HTTP connection pool, while Bambu serializes status and commands over one
authenticated MQTT session. Endpoint, TLS, credential, or timeout changes
replace the session immediately, and removing a profile evicts it. One-shot CLI
operations retain short-lived connection behavior.

## Configuration and security

Profiles are stored as versioned configuration under the operating system's
user configuration directory. Access codes, API keys, and Bambu TLS
fingerprints are stored separately through the OS keychain. Secure Bambu first
contact binds the certificate identity to the configured physical printer
serial before retaining its pin; a mismatch never silently rewrites the
profile or falls back to insecure transport.

Continuous desktop discovery is advisory. It keys presence by normalized
physical serial and may show a newly advertised address as a suggestion, but it
never changes a configured endpoint, TLS identity, certificate pin, or
credential.

Mutating Bambu commands are gated by explicit live authorization evidence.
When firmware reports that signing is required—or supplies conflicting
security evidence—Melt blocks the mutation and keeps read-only status and
diagnostics available.

## Current capability boundary

Moonraker profiles support status, G-code listing/upload/download, jobs,
emergency stop, bounded temperature control, motion, part-cooling fan, and
speed controls through the shared core. Generic Klipper light control remains
an explicit capability error.

Bambu LAN supports pinned-TLS MQTT status and controls, authenticated storage
operations, SSDP/UDP discovery, camera snapshots/streams, and sliced 3MF
preflight. Live observations and safe protocol probes take precedence over
model-family assumptions. Unknown models and fields remain operable but are
not misclassified as supported or unsupported without evidence.

The print workflow preserves project, plate, display, and storage names
separately; reconciles available AMS/nozzle data; probes storage protocols
conservatively; and waits for an observed print start. See
[`docs/bambu-compatibility.md`](docs/bambu-compatibility.md) for the
compatibility matrix and
[`docs/bambu-physical-qualification.md`](docs/bambu-physical-qualification.md)
for the hardware-capture and quirk-promotion procedure.

The desktop polls configured profiles locally every five seconds while open;
failed profiles back off independently to one minute. Release tags build the
native installer matrix and require frozen release evidence. Automated tests
do not constitute physical-printer qualification.

## Development and verification

The primary targets are:

```sh
make build
make test
make lint
make contract
make ci
make release-qualification
```

- `make build` builds the Rust workspace and production UI.
- `make test` runs the Rust and UI tests.
- `make lint` checks Rust formatting and runs Clippy with warnings denied.
- `make contract` executes the checked-in CLI JSON/exit-code fixtures.
- `make ci` additionally checks AGPL metadata and sanitized Bambu evidence.
- `make release-qualification` runs the complete reproducible release gate and
  installed CLI smoke test.

See [`docs/cli-contract-fixtures.md`](docs/cli-contract-fixtures.md),
[`docs/bambu-protocol-evidence.md`](docs/bambu-protocol-evidence.md), and
[`docs/release-qualification.md`](docs/release-qualification.md) for the
contracts and evidence that these targets enforce.

## Running on Linux Wayland

Use `make run` during development. It starts the Vue development server before
the desktop process. On a Wayland session that also provides XWayland, it
starts GTK with `GDK_BACKEND=x11` and disables WebKit DMA-BUF rendering before
the process loads, avoiding compositor protocol failures and GBM buffer
warnings. To run a built binary directly, use:

```sh
GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 ./target/debug/melt
```

A development build shows a placeholder icon in the window switcher because
desktop shells do not read the icon from the running process. They resolve the
window's application id against an installed desktop entry and use that
entry's `Icon` key, and a `cargo run` build installs no entry. Run
`make install-desktop` once to install a development entry and the `melt`
hicolor icons under `XDG_DATA_HOME`; `make uninstall-desktop` removes them.

Package builds carry their own desktop entry, so they do not need this. The
entry Tauri generates sets `StartupWMClass` to the binary name, which matches
the GTK application id only while `app.enableGTKAppId` stays unset — setting
it switches the application id to the bundle identifier and breaks the match.

## Protocol references

### Bambu LAN

- [bambulabs_api](https://github.com/BambuTools/bambulabs_api)
- [Open Bambu Networking](https://github.com/ClusterM/open-bamboo-networking)
- [OpenBambuAPI](https://github.com/Doridian/OpenBambuAPI)
- [BambuHelper](https://github.com/Keralots/BambuHelper)
- [Bambuddy](https://github.com/maziggy/bambuddy)

Undocumented behavior from these projects is treated as scoped evidence, not
permanent product truth. Conflicting reports stay provisional until sanitized
physical captures qualify an exact model and module-firmware range.

### Moonraker

- [Moonraker documentation](https://moonraker.readthedocs.io/)

## Credits

The application icon is
[melt](https://thenounproject.com/icon/melt-8239158/) by Paphada from
[Noun Project](https://thenounproject.com/), used under
[CC BY 3.0](https://creativecommons.org/licenses/by/3.0/) and modified: the
embedded credit line was removed and the artboard was squared for use as an
application icon. The icon is separately licensed from the rest of this
project and is not covered by `AGPL-3.0-only`.

`crates/melt-desktop/icons/icon.svg` is the master; the PNG, ICO, and ICNS
variants beside it are generated from it.

## Safety disclaimer

Melt controls physical equipment and can move axes, heat components,
transfer or delete files, and start prints. Review the selected printer and
command, keep the machine attended during qualification and first use, and
maintain immediate access to its physical safety controls. The software is
provided without warranty under the AGPL-3.0-only license; use it at your own
risk.

Legacy implementations reported testing on Bambu Lab A1 Mini and H2C hardware.
That historical statement does not qualify this implementation or any current
model/firmware pair. Melt records physical qualification only through the
evidence workflow linked above.

## License

Melt is licensed under the
[GNU Affero General Public License v3.0 only](LICENSE).
