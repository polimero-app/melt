# IP cameras

Last-updated: 2026-10-12

Melt can show IP cameras next to, or instead of, a printer's built-in camera.
Camera profiles are independent of printers; a camera can be linked to any
number of printers, and each printer picks one default feed.

## Supported cameras

| Driver | Model | Qualified hardware / firmware | Video | Snapshots | Pan/tilt |
|---|---|---|---|---|---|
| `tapo` | TP-Link Tapo C510W | 1.0 / 1.3.5 (2026-10-08) | H.264 over RTSP | Decoded from video | Planned |

Other models and firmware are unqualified until exercised on hardware.

## Tapo setup

1. In the Tapo app, open the camera → **Settings → Advanced Settings →
   Camera Account** and create a local username and password. This account is
   separate from your TP-Link cloud login; Melt never asks for the cloud
   password. If your firmware shows a **Third-Party Compatibility** option,
   turn it on, and make sure privacy mode is off.
2. Save the password to a file only you can read:

   ```sh
   mkdir -p ~/.config/melt
   chmod 700 ~/.config/melt
   (umask 077; touch ~/.config/melt/porch.pw)
   chmod 600 ~/.config/melt/porch.pw
   ```

   Write the password into that file with an editor so it stays out of shell
   history. Use one line; the file must be regular (not a symlink) and at most
   4 KiB. This is a Unix example; use an appropriate private file on macOS or
   Windows. These files are inputs to setup, not Melt's credential store.

3. Add the camera. Melt connects, authenticates, and decodes one frame before
   saving; it never moves the camera during setup.

   ```sh
   melt camera add porch --driver tapo --host 192.0.2.20 \
     --username melt --password-file ~/.config/melt/porch.pw
   ```

The password is stored in the OS keychain; the profile in `melt.yaml` contains
the immutable ID, driver, username, host, optional port, timeout and timestamps.
It contains no password. After setup, remove the input password file if it is
no longer needed. See [configuration](configuration.md) for locations and backups.

## Everyday use

```sh
melt camera list
melt camera snapshot --camera porch --quality low --to ./porch.jpg
melt camera stream --camera porch            # open http://127.0.0.1:8080/stream
melt camera stream --camera porch --format h264   # ffplay -f h264 http://127.0.0.1:8080/stream
melt camera link porch workshop --select
melt camera select workshop --builtin       # or: melt camera select workshop porch
melt camera edit porch --host 192.0.2.21     # re-verifies the connection
melt camera edit porch --name front-door     # rename only: no connection needed
melt camera remove porch --yes
```

`high` uses the camera's main stream (`/stream1`), `low` its sub-stream
(`/stream2`). `camera snapshot <printer>` and `camera stream <printer>` keep
their meaning: a positional name is always a printer's built-in camera,
regardless of its selected desktop feed. Use `--camera` for an external camera;
do not pass both selectors. Link/unlink take camera then printer, while select
takes printer then camera. Selection requires an existing link.

`--port` changes the loopback server's port; `--duration 30m` bounds its
lifetime. The process stays running while it serves the stream. The default
external quality is `high`. `--timeout` overrides that command's connection
budget. `--insecure` and `--protocol-trace` apply only to printer cameras and
are refused with `--camera`.

`camera stream` serves MJPEG by default, transcoded from the camera's H.264
so any browser can show it. If transcoding cannot keep up with the camera,
Melt skips frames rather than falling behind. `--format h264` instead
forwards the camera's own
H.264, without decoding or re-encoding, as an Annex-B byte stream that starts
at a keyframe. Open it with `ffplay -f h264 <url>` or
`vlc --demux h264 <url>`; browsers cannot play it. The same flag works for
H/X-series Bambu printers (`melt camera stream <printer> --format h264`);
printers whose camera serves only MJPEG reject it with
`capability_unsupported`.

### Desktop

- The **Cameras** tab lists your cameras. **Add camera** asks for the
  same details as `melt camera add`, checks the connection (up to the
  timeout) and saves nothing if that fails. When editing, leave the password
  blank to keep the stored one.
- Selecting a camera opens its live view. **Stream quality** switches between
  High and Low for that view only; it is not saved.
- Under **Linked printers**, tick the printers that may show the camera. Their
  camera card then gets a **Camera feed** picker: **Built-in camera** or a
  linked camera. The choice is saved as the printer's default feed (like
  `melt camera select`), and a camera feed keeps working while the printer is
  offline.
- Views of the same camera at the same quality share one RTSP connection,
  which Melt closes when the last view closes. This helps with the camera's
  simultaneous-stream limit. Different qualities open separate upstreams.
  Leaving the view or hiding the window releases that viewer's subscription;
  returning starts it again. CLI commands run in separate processes and do not
  share the desktop's manager.
- Playback tries WebRTC, then WebCodecs (including on Linux), then MJPEG.
  Native H.264 playback avoids JPEG transcoding; the MJPEG fallback uses
  more CPU, especially at High quality. See the
  [Wayland renderer notes](../README.md#running-on-linux-wayland) if an old
  renderer override is still set.
- The camera badge names the playback in use (`LIVE · WebRTC`, `LIVE · H.264`
  for WebCodecs, or an amber `LIVE · MJPEG`). **Configuration › About ›
  Diagnostics** shows why a faster one was skipped, next to each camera's RTSP
  streams (quality, viewers, reconnect attempts). Cameras appear there as
  `camera-N`; names, hosts, usernames and IDs are masked in the reasons too.
  If playback fails entirely, diagnostics shows **None** with the failure
  reasons. Results remain available after closing the viewer; WebCodecs is
  reported only after its first decoded frame.

The current desktop navigation calls the fleet tab **Printers**. See the
[desktop guide](desktop.md) for the other views. Pan/tilt, ONVIF controls,
WS-Discovery, audio and recording are not implemented; phase 3 and phase 4 in
the [camera design](superpowers/specs/2026-10-08-ip-cameras-design.md) remain
future work.

## Security

Video uses plain RTSP on your LAN; it is not encrypted end to end. Melt only
accepts Digest authentication, so the password is never sent in clear; a
camera that offers only Basic authentication is refused. Credentials are only
sent to the configured host.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `connection_failed`, connection refused on 554 | Camera Account not created yet, Third-Party Compatibility is off (on firmware that has it), or privacy mode is on (it disables RTSP and ONVIF). |
| `connection_failed`, `RTSP DESCRIBE failed: Service Unavailable` | The camera's simultaneous-stream limit is reached; close other viewers (NVRs, apps). |
| `authentication_failed` | Wrong Camera Account username or password (not the cloud login). |
| `authentication_failed` mentioning Basic | The camera offered no Digest challenge; Melt will not send the password. |
| `timeout` | Wrong host, or the camera is busy: Tapo cameras limit simultaneous streams; close other viewers (NVRs, apps). |
| `capability_unsupported` | The stream is not H.264 (H.265 is not supported). |
| `secret_not_found` / `secret_store_failed` | The Camera Account password is missing or the user's OS keychain is unavailable; restore/re-enter credentials or unlock the keychain. |
| Amber `LIVE · MJPEG` badge | Native WebRTC/WebCodecs playback failed or is unavailable in this webview; inspect playback reasons in Diagnostics. |

The scoped physical evidence is documented in [Tapo fixtures](../fixtures/tapo/README.md).
Simulated RTSP tests cover authentication, channel selection, decoding and
bounded failures; they do not qualify every Tapo model or desktop webview.

## Downgrading Melt

Adding a camera writes `melt.yaml` as schema version 2, which version-1-only
Melt builds refuse to read. Removing all cameras and links with the current
Melt writes the remaining printer configuration as version 1. For a manual
downgrade, close Melt and back up the file, then remove the `cameras:` and
`printer_cameras:` sections from `melt.yaml` and set `version: 1`, then
delete the `camera:<id>:password` keychain entries if you no longer need them.
