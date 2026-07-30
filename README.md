# Polimero Tauri migration

This directory is the parallel Rust/Tauri implementation. The Go application
remains the released implementation until this workspace passes its CLI and GUI
compatibility gates.

## Running on Linux Wayland

Use `make run` during development. On a Wayland session that also provides
XWayland, it starts GTK with `GDK_BACKEND=x11` and disables WebKit DMA-BUF
rendering before the process loads, which avoids compositor protocol failures
and GBM buffer warnings. To run a built binary directly, use:

```sh
GDK_BACKEND=x11 WEBKIT_DISABLE_DMABUF_RENDERER=1 ./target/debug/polimero
```

- `crates/polimero-core` contains UI-agnostic behavior.
- `crates/polimero-cli` adapts that behavior to the `polimero` CLI contract.
- `crates/polimero-desktop` selects CLI mode for arguments and starts Tauri for
  a no-argument launch.
- `ui` contains the Vue, TypeScript, Tailwind CSS, and Headless UI frontend.

Run `make ui-install` once, then use `make build`, `make test`, `make lint`,
or `make ci`. `make contract` executes the checked-in JSON CLI fixtures, and
`make release-gate` additionally verifies the installed `polimero` CLI entry
point.

## Current capability boundary

Moonraker profiles support status, gcodes listing/upload/download, jobs,
emergency stop, bounded temperature control, motion, part-cooling fan, and
speed controls through the shared core. State-changing CLI commands require
`--yes` outside a terminal; the desktop requires a confirmation before job
actions and keeps emergency stop immediate.

Bambu LAN camera media uses its authenticated TLS MJPEG endpoint: the desktop
can proxy a real live stream or capture a real snapshot only when that
capability is available. The Linux packages declare the FFmpeg runtime used by
the baseline media policy. Bambu LAN also supports pinned-TLS MQTT status and
controls plus authenticated FTPS SD-card file operations. Bambu discovery
scans its SSDP and UDP announcement channels, while TLS fingerprint refresh
always requires explicit confirmation before replacing a keychain pin. Generic
Klipper light control remains an explicit capability error.

The desktop polls configured profiles locally every five seconds while open;
failed profiles back off independently to one minute. It offers a redacted
diagnostics report. Release tags run native installer builds for the required
OS/architecture matrix and require frozen release evidence. See
[`docs/release-qualification.md`](docs/release-qualification.md) for the
automated gate and the external/manual evidence that it cannot replace.
