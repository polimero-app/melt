# IP Camera Support — Design

**Status:** Accepted design for phased implementation. Melt has no ADR
directory; like earlier Melt features, this spec is the decision record. Each
phase gets its own implementation plan, written once the previous phase has
landed and its interfaces are real.

**Goal:** Add independent IP camera profiles with optional printer links, live
video, snapshots, and pan/tilt, in the CLI and desktop app. The first external
driver is `tapo`, first qualified on the C510W.

**Qualification unit:** a user-owned Tapo C510W, hardware 1.0, firmware 1.3.5,
on the LAN. Its local Camera Account was not configured as of 2026-10-08, and
RTSP (554) and ONVIF (2020) refused connections then. That is expected until
the Camera Account exists. It is not evidence of a firmware incompatibility.

## Decisions

| Topic | Decision |
|---|---|
| Device model | Independent camera profiles, optionally linked to printers |
| First driver | `tapo`, model-agnostic. The model is observed, never asserted. First qualified on the C510W, hardware 1.0 / firmware 1.3.5 |
| Credentials | Tapo Camera Account only. It enables RTSP (video) and ONVIF on port 2020 (Profile S: device info, pan/tilt). Melt never asks for the TP-Link cloud login |
| Features | Live video, JPEG snapshots, pan/tilt |
| Surfaces | CLI and desktop |
| Excluded | Presets, zoom, audio, recording, timelapses, web interface, cloud login, camera firmware, motion tracking, H.265, transcoding services |
| Links | Many-to-many between cameras and printers; one selected feed per printer |
| Built-in cameras | Remain selectable; a printer with no link record keeps its built-in feed |
| Quality | `high` (`/stream1`) or `low` (`/stream2`), chosen per viewer or per command, default `high`. Not stored in the profile |
| Video transport | Plain RTSP over TCP-interleaved RTP, H.264 only, on the LAN. No claim of encryption, no cloud relay |
| Discovery | Manual host entry. ONVIF WS-Discovery is Phase 4 and optional |
| Dependencies | Phase 1–2 add none. Phase 3 adds `quick-xml` (new to `melt-core`; today it reaches only `melt-desktop` via `plist`) |

## Phases

| Phase | Delivers | Plan |
|---|---|---|
| 1 | Shared RTSP client, config schema, camera profiles + links, Tapo video and snapshots, CLI | `plans/2026-10-08-ip-cameras.md` |
| 2 | Desktop: Cameras view, printer feed selection, shared viewer, per-viewer sessions | `plans/2026-10-08-ip-cameras-desktop.md` |
| 3 | ONVIF client, pan/tilt in CLI and desktop | Written after Phase 2 and ONVIF qualification |
| 4 | WS-Discovery (optional) | Only if manual entry proves painful |

## Architecture

**Shared RTSP client (`melt-core/src/rtsp.rs`).** The existing Bambu RTSP code
moves here and drops its vendor constants. Drivers supply the connected socket
(pinned TLS for Bambu, plain TCP for Tapo), the URI, and credentials. The
module handles DESCRIBE/SETUP/PLAY, Digest auth (realm and `qop=auth` from the
challenge, or a fixed realm for Bambu), the advertised interleaved channel,
relative/absolute/`*` control URIs, keepalive, RFC 6184 depacketization and
optional JPEG decoding. Bambu's TLS, identity, transport selection and quirks
stay in `bambu/`. Bambu's wire behavior must not change: same URI, realm,
header format and channel.

**Camera domain (`melt-core/src/cameras.rs`, `cameras/`).** A closed driver
enum like `drivers.rs`, not a trait. It holds profile workflows (create, update,
remove, link, unlink, select), credential resolution, and the Tapo media
operations. Phase 1 has no shared upstream manager: each CLI command opens its
own stream, as `camera snapshot <printer>` does today.

**Config (`melt.yaml`).**
- `cameras`: map from normalized camera name to profile. A profile holds an
  immutable random 128-bit `id` (hex), `driver`, `host`, `username`, an
  optional `rtsp_port`, `timeout`, `created` and `updated`.
- `printer_cameras`: map from printer profile name to `{cameras: [id…],
  selected: id?}`. A missing `selected` means the built-in feed.
- **Schema version:** a file is written as `version: 2` only when either
  camera map is non-empty, and as `version: 1` otherwise. That keeps
  printer-only installs readable by older binaries. Readers accept 1 and 2,
  reject camera data in a `version: 1` file, and reject dangling references.
- **Downgrade:** delete the two camera sections and set `version: 1`. This is
  documented, not automated.
- Printer rename moves its link record in the same write. Printer removal
  drops its link record. Camera removal drops its ID from every record and
  resets affected selections to built-in. Camera rename touches neither links
  nor secrets, because both use the ID.

**Secrets.** Only the password goes to the keychain, under account
`camera:<id>:password`. The username is non-secret config. Add, update and
remove reuse the printer workflow's secret rollback. An empty password on
update keeps the stored one. A failed keychain delete after removal is
reported as a warning. Add and update verify by decoding one live frame
before saving, and never move the camera.

**Security rules.**
- Accept Digest (MD5) only. Reject a Basic-only challenge with
  `RtspError::DigestRequired`; over plain RTSP, Basic would send the password
  in cleartext.
- Usernames must not contain `"`, `\` or control characters, because they
  are embedded in the Digest header.
- Credentials travel only on the connection Melt opened to the configured
  host and port. RTSP control URIs ride the same connection and never open
  new ones.
- Phase 3 applies to ONVIF: service and stream URIs must match the
  configured host, redirects are disabled (`redirect::Policy::none()`), and
  WS-Discovery data is untrusted.

## CLI contract

New commands. Positional arguments name the objects a command acts on;
flags are options. A command that changes a printer's feed names the printer
first:

```text
melt camera add <camera> --driver tapo --host <host> --username <user>
    --password-file <path> [--rtsp-port <port>] [--timeout <duration>]
melt camera edit <camera> [--name <new>] [--host <host>] [--username <user>]
    [--password-file <path>] [--rtsp-port <port>] [--timeout <duration>]
melt camera list
melt camera remove <camera> [--yes]
melt camera link <camera> <printer> [--select]
melt camera unlink <camera> <printer>
melt camera select <printer> (<camera> | --builtin)
```

Existing media commands gain an external selector. The positional printer
form is unchanged:

```text
melt camera snapshot (<printer> | --camera <camera>) [--quality high|low] [--to <path>] [--overwrite]
melt camera stream   (<printer> | --camera <camera>) [--quality high|low] [existing flags]
```

- `--quality` and `--camera` apply only to external cameras. Passing both a
  printer and `--camera` is a usage error. `--insecure` and
  `--protocol-trace` are rejected with `--camera`.
- **Passwords:** only `--password-file`, with the existing regular-file,
  user-only and 4 KiB rules. No hidden prompt (no TTY dependency) and no
  password argument.
- **Exit codes:** validation and not-found 2, authentication and keychain 3,
  connection and timeout 4, unsupported codec 5, internal 1. Error codes
  reuse the existing set (`config_error`, `authentication_failed`,
  `secret_not_found`, `secret_store_failed`, `connection_failed`, `timeout`,
  `capability_unsupported`, `internal_error`).
- Removal requires confirmation or `--yes`, like `printer remove`. Other
  profile and link commands save directly.
- New commands are listed in `meltNativeCommands`. As with `printer add
  --model`, the frozen Go-baseline entries for `camera snapshot` and
  `camera stream` stay unchanged.

## Requirements carried to Phase 2 (desktop)

- **Manager lock.** `CameraManager::subscribe` currently opens the upstream
  while holding the manager-wide lock (`bambu/camera_manager.rs`). Startup
  must be coordinated per source, so an unreachable camera cannot stall the
  others. Key upstreams by camera ID plus quality; Bambu keeps its serial key.
- **Viewer-scoped sessions.** `printer_camera_stream_stop` stops every MJPEG
  proxy, and WebRTC sessions are keyed by printer. Introduce viewer session
  IDs: stopping one viewer leaves others running on the shared upstream, and
  the last viewer releases it.
- **Snapshots** take a frame no older than 2 s from the active source, else
  wait for a new decodable frame within the timeout. A frame from a previous
  connection generation is never returned.
- **Isolation.** Camera failures never mark a printer offline. An external
  feed works while its printer is offline. Changing feeds releases only that
  viewer's session.
- **UI.** Extract `CameraViewer.vue`, a media-lifecycle composable and a pure
  `cameras.ts`. Keep the WebRTC → WebCodecs → MJPEG fallback. Provide English
  and pt-BR strings. No hidden thumbnail sessions and no periodic camera
  polling; an open viewer's stream state is the status.

## Requirements carried to Phase 3 (pan/tilt)

- **ONVIF with the Camera Account.** Device, Media and PTZ services on port
  2020, authenticated with WS-Security UsernameToken. Device info (model,
  firmware, serial) comes from `GetDeviceInformation`.
- **Capability from observation.** Read the ONVIF profile's PTZ
  configuration and options. Movement controls require observed support and
  a working `Stop`; the model name alone never grants them.
- **Mechanism, preferring bounded motion:**
  1. `RelativeMove` in small steps. Each step is finite, so it cannot run
     away and needs no renewal; holding a control repeats steps, at most one
     request in flight.
  2. Only if the camera lacks relative moves: `ContinuousMove` with a device
     timeout of 2 s or less, and only if the camera demonstrably honors that
     timeout. The UI renews it while the control is held, skipping a renewal
     while the previous request is still in flight.
- **Stop** is sent on release, blur, lost pointer capture, source switch,
  profile edit or removal, and teardown. It is idempotent, and a transport
  failure is reported as "stop not confirmed", never as success.
- **Control serialization is per process only.** CLI and desktop may both
  move the camera; ONVIF `Stop` from either one stops it. Moves are never
  replayed after a reconnect.
- **CLI:** `melt camera pan-tilt <camera> --pan <step> --tilt <step>` and
  `melt camera pan-tilt <camera> --stop`. Steps are finite values in
  [-1, 1] of the advertised range. With the `ContinuousMove` fallback, the
  values are velocities and a `--duration` of 2 s or less is required. No `--yes`: a bounded camera pulse has no physical-safety
  consequence that warrants confirmation.

## Qualification

**Phase 1.** On the qualification unit:
- RTSP DESCRIBE challenge format (realm, qop, whether Basic is offered);
- SDP of `/stream1` and `/stream2` (codec, profile-level-id, control URIs,
  interleaved channels);
- `openh264` decodes both streams;
- add, edit and password change, both qualities, snapshot, and stream
  through `melt`;
- whether a "Third-Party Compatibility" setting exists in the Tapo app, and
  its effect on ports 554 and 2020. Newer Tapo firmware has this option; the
  `tapo` crate's changelog ties it to its TPAP login path;
- ONVIF reachability on port 2020: an unauthenticated
  `GetSystemDateAndTime` succeeds.

**Phase 3.** ONVIF `GetDeviceInformation`, service and PTZ captures, and
whether `RelativeMove` is advertised. With the `ContinuousMove` fallback,
also the timeout it honors and device-side stop after killing the
controlling process. In both cases, the observed movement directions.

**Fixtures.** Store sanitized fixtures under `fixtures/tapo/` with
hardware, firmware and date provenance. Use documentation addresses
(`192.0.2.0/24`). Store no passwords, nonces/responses, serials or images.

**Troubleshooting docs** must cover: Camera Account setup, Third-Party
Compatibility, Tapo privacy mode
(it disables RTSP and ONVIF), and the camera's limit on concurrent streams.

## Alternatives considered

**`tapo` crate (TPAP, previously AES SSL).** Rejected for now. Its camera
handler gives device info, step-wise `pan_tilt`, and presets over TP-Link's
proprietary encrypted API. That login uses the TP-Link cloud account, not the
Camera Account. Its live video is still the Camera Account's RTSP stream
(`get_rtsp_stream_url`), and its snapshots come from the camera's MJPEG
stream, so it cannot replace RTSP. Adopting it would mean storing the cloud
password and adding an async, crypto-heavy dependency that has not been
tested on the C510W. Revisit only if ONVIF pan/tilt fails qualification, or
if presets or privacy control become requirements.
