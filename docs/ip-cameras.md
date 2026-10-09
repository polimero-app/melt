# IP cameras

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
   (umask 077; printf '%s\n' 'camera-account-password' > ~/.config/melt/porch.pw)
   ```

   To keep the password out of your shell history, write the file in an
   editor instead (and `chmod 600` it).

3. Add the camera. Melt connects, authenticates, and decodes one frame before
   saving; it never moves the camera during setup.

   ```sh
   melt camera add porch --driver tapo --host 192.0.2.20 \
     --username melt --password-file ~/.config/melt/porch.pw
   ```

The password is stored in the OS keychain; `melt.yaml` holds only the
username, host, and port.

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
their meaning: a positional name is always a printer.

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
  simultaneous-stream limit.
- On Linux, Melt decodes the camera's H.264 and sends the view to the window
  as MJPEG, because the system webview (WebKitGTK) cannot play H.264. Low
  quality uses much less CPU.

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

## Downgrading Melt

Adding a camera writes `melt.yaml` as schema version 2, which older Melt
releases refuse to read. To downgrade, remove the `cameras:` and
`printer_cameras:` sections from `melt.yaml` and set `version: 1`, then
delete the `camera:<id>:password` keychain entries if you no longer need them.
