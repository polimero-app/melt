# CLI reference

Last-updated: 2026-10-12

The packaged `melt` executable enters CLI mode whenever it receives arguments.
With no arguments it starts the desktop app. `melt --help`, group help, and
`melt <command> --help` print the built-in command reference. There is no
standalone CLI binary target or implemented `web` command.

## Output and common options

`--output human|json` defaults to `human`. `--verbose` / `-v` enables extra
human-mode progress and is suppressed in JSON mode. Printer network commands
generally accept `--timeout <duration>`, `--insecure`, and
`--protocol-trace <path>`; consult command help for availability. Timeouts use
units such as `10s` or `1500ms`, and normally inherit the profile's `10s`
default. Traces are redacted JSON Lines; inspect them before sharing.

For printer operations, `--insecure` changes TLS verification for that
invocation. On `printer add` it persists an insecure profile; on
`printer tls refresh` it permanently disables verification. It never supplies
missing credentials. Camera profile commands have no insecure option;
external-camera snapshot/stream reject printer-only `--insecure` and
`--protocol-trace` flags.

JSON operation responses use this envelope:

```json
{"ok":true,"data":{},"error":null,"meta":{"command":"printer list"}}
```

An error has `ok: false`, `data: null`, and an `error` object with `code`,
`message`, and `details`. `meta.command` identifies the operation;
`meta.durationMs` is optional network timing. Help remains human-readable,
including when requested alongside `--output json`. Ordinary non-interactive
JSON results keep stderr empty; interactive confirmations can prompt there.
For predictable automation, supply `--yes` on commands that require it.

| Exit code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Internal/local I/O error or an unclassified printer failure |
| 2 | Usage, validation, configuration, missing profile or invalid printer state |
| 3 | Authentication, TLS identity/pin or keychain failure |
| 4 | Connection, timeout or classified remote operation failure |
| 5 | Unsupported driver operation/capability, including signing-required or conflicting Bambu authorization |

Use `error.code` for the specific recovery action. The stable response and
fixture rules are in [CLI contract fixtures](cli-contract-fixtures.md).

## Printer profiles, status and firmware

| Command | Arguments and main options |
| --- | --- |
| `printer add` | `<name> --driver <bambu-lan|moonraker> --host <host>`; optional `--serial`, `--model`, `--access-code-file`, `--timeout`, `--insecure` |
| `printer list` | List configured profiles |
| `printer remove` | `<name> [--yes]`; removes profile, printer links and stored printer secrets |
| `printer drivers` | List the two registered printer drivers |
| `printer discover` | `[--driver bambu-lan] [--timeout 5s]` |
| `printer capabilities` | `<name>`; refresh observed Bambu identity, module firmware, authorization and transports |
| `printer tls refresh` | `<name> [--yes] [--insecure]`; replace the pin, or disable verification |
| `status` | `<name> [--detailed]`; extended telemetry includes controls, fans, job metadata, HMS and AMS |
| `firmware check` | `<printer> [--refresh] [--include-history] [--include-public-catalogue]` |
| `version` | Version, license and source information |

Secure Bambu setup requires the physical printer serial and an access code.
Initial verification checks the certificate identity and authentication before
saving. `--model` is a fallback hint; authenticated model detection takes
precedence. Only Bambu discovery is implemented; `--driver moonraker` reports
`capability_unsupported`. `printer capabilities` is also Bambu-specific.

The help/inventory currently advertises `--protocol-trace` for add/discover,
but creation rejects that unknown option and discovery explicitly refuses
tracing without a resolved profile. Trace a subsequent supported profile
operation instead. These are existing help/implementation differences.

Printer and camera names are normalized to lowercase, limited to 64 ASCII
characters, and must start with a letter/digit; subsequent characters may
also contain `.`, `_` or `-`. Names are separate namespaces.

For Moonraker, the host may be an HTTP/HTTPS URL or a hostname/address; an HTTP
host without an explicit port defaults to 7125. If an API key is needed,
provide it through `--access-code-file`. A trusted-LAN Moonraker profile can
operate without a key.

```sh
melt printer add workshop --driver bambu-lan --host 192.0.2.10 \
  --serial SANITIZED_SERIAL --access-code-file ./access-code.txt
melt printer add klipper --driver moonraker --host http://192.0.2.30:7125
melt status workshop --detailed --output json
melt firmware check workshop --include-history --include-public-catalogue --output json
```

Credential files must be regular files, not symlinks, at most 4 KiB, with no
group/other permissions on Unix (for example `chmod 600`). A trailing line
ending is removed. Never put the secret in a command argument or YAML profile.
See [configuration](configuration.md) and [firmware evidence](firmware-updates.md).

Version JSON retains migration fields `goVersion: rust`, `commit: unknown`,
and `modes: [gui, headless]`. The reported version comes from Cargo's package
metadata; a nightly installer can have a date-based bundle version while this
CLI still reports the checked-in Cargo version. See
[nightly packaging](release-qualification.md#nightly-prereleases).

## Storage and print jobs

| Command | Arguments and main options |
| --- | --- |
| `files roots` | `<printer>` |
| `files list` | `<printer> [<device-path>...] [--recursive]` |
| `files upload` | `<printer> <local-path> [<device-path>] [--overwrite]` |
| `files download` | `<printer> <device-path>... [--to <path>] [--overwrite]`; multiple files require a destination directory |
| `files delete` | `<printer> <device-path>... [--yes]` |
| `jobs preflight` | `<local-path> [--plate <index>] [--display-name <name>]`; offline sliced-3MF inspection |
| `jobs start` | `<printer> <device-path> [--plate <index>] [--skip-leveling] [--yes]` |
| `jobs pause` | `<printer> [--yes]` |
| `jobs resume` | `<printer> [--yes]` |
| `jobs cancel` | `<printer> [--yes]` |

Device paths use a root prefix: `sdcard:/part.gcode.3mf` for Bambu or
`gcodes:/part.gcode` for Moonraker. Query roots/listings instead of assuming
which physical media a printer uses. Upload's optional destination defaults
to the driver's root plus the local filename; a directory destination appends
that filename. The built-in upload help currently shows the destination as
required, while the implementation accepts its omission.

Preflight does not upload or start a print. CLI `jobs start` uses a file already
on the printer; it does not take a local file. The desktop adds a staged
local-file workflow with package inspection, plate/material reconciliation,
upload and observed print start. See [Bambu compatibility](bambu-compatibility.md).

```sh
melt jobs preflight ./part.gcode.3mf --plate 1 --output json
melt files roots workshop
melt files upload workshop ./part.gcode.3mf sdcard:/part.gcode.3mf
melt jobs start workshop sdcard:/part.gcode.3mf --plate 1 --yes
```

Uploads execute directly and reject existing destinations unless
`--overwrite` is given. Downloads and snapshots similarly need `--overwrite`
to replace an existing local destination; they have no `--yes` flag.

## Controls and confirmation

| Command | Arguments and main options |
| --- | --- |
| `temperature set` | `<printer>` with one or more of `--nozzle`, `--bed`, `--chamber` in °C; zero turns off |
| `motion home` | `<printer> [--axis x,y,z]`; defaults to all axes |
| `motion jog` | `<printer>` with `--x`, `--y`, `--z` in mm, each from -10 to 10; `--feedrate` defaults to 1500 mm/min |
| `fans set` | `<printer> <fan> <percent>` |
| `lights set` | `<printer> <light> <on|off>` |
| `speed set` | `<printer> <silent|standard|sport|ludicrous>` |
| `ams dry start` | `<printer> <ams-id> --temperature <°C> --hours <1-24> [--filament <type>]` |
| `ams dry stop` | `<printer> <ams-id>` |
| `emergency-stop` | `<printer>`; immediate, with no confirmation flag |

Temperature, motion, fan, light, speed and drying commands accept `--yes`.
So do print start/pause/resume/cancel, file deletion, profile removal, camera
removal and TLS refresh. Without it, an interactive terminal requires typing
`yes`; non-interactive use fails with exit 2. Profile creation/editing, camera
link/selection, uploads and immediate emergency stop do not use that prompt.

Confirmation does not bypass state checks, bounds, capability checks or Bambu
authorization. Derive controllable fan/light/heater identifiers from detailed
status; telemetry-only entries cannot be commanded. Moonraker light and AMS
drying operations are unsupported. Bambu drying needs the live remote-drying
capability; AMS IDs 0–3 allow 45–65 °C and 128–135 allow 45–85 °C.

## Cameras

| Command | Arguments and main options |
| --- | --- |
| `camera add` | `<camera> --driver tapo --host <host> --username <user> --password-file <path> [--rtsp-port 554] [--timeout 10s]` |
| `camera edit` | `<camera>` with at least one of `--name`, `--host`, `--username`, `--password-file`, `--rtsp-port`, `--timeout` |
| `camera list` | Profiles, linked printers and selected feeds |
| `camera remove` | `<camera> [--yes]` |
| `camera link` | `<camera> <printer> [--select]` |
| `camera unlink` | `<camera> <printer>` |
| `camera select` | `<printer> <camera>` or `<printer> --builtin` |
| `camera snapshot` | `<printer>` or `--camera <camera>`; `--to`, `--overwrite`, `--timeout`; external cameras also accept `--quality high|low` |
| `camera stream` | `<printer>` or `--camera <camera>`; `--format mjpeg|h264`, `--port 8080`, `--duration`, `--timeout`; external cameras also accept `--quality high|low` |

The positional snapshot/stream target is always a printer's built-in camera.
It does not follow the feed selected for that printer's desktop card. Use
`--camera` explicitly for an external feed. Link/unlink take camera then
printer; select takes printer then camera. The camera must already be linked
before selection.

Stream commands serve `http://127.0.0.1:<port>/stream` while the process stays
running, optionally bounded by `--duration`. The CLI HTTP server permits one
active viewer at a time. MJPEG is browser-compatible;
native H.264 is an Annex-B stream for ffplay/VLC and requires an H.264 source.
These loopback streams are separate from the desktop's viewer-scoped proxies.
See [IP cameras](ip-cameras.md) for setup and playback details. Pan/tilt,
ONVIF and camera discovery commands are not implemented.
