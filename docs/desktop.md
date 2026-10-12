# Desktop guide

Last-updated: 2026-10-12

Start the packaged `melt` executable without arguments. For a source checkout,
use `make run` after `make ui-install`; it starts the frontend server before
the Tauri process. The main navigation is **Printers**, **Control**,
**Cameras**, **Configuration** and **Files** (the File library view). Historical
plans call Printers “Manage printers”; the [screenshot gallery](screenshots.md)
now captures the current labels with sanitized sample data.

## Printers and Control

**Printers** lists the configured fleet, online/printing counts and per-printer
actions. Add a profile manually or discover Bambu printers, then verify and
save it. Editing can update the connection and profile name. Bambu credentials
and TLS pins are verified and saved through the keychain. Moonraker discovery
is not implemented. New address observations are suggestions requiring an
explicit edit, never automatic endpoint changes.

Open **Control** for a printer's job, camera, temperatures, fans, lights,
motion, storage files and AMS inventory. The current job exposes progress,
preparation stage, layers, remaining time, plate metadata and available
thumbnail. Informational HMS events remain diagnostic; actionable errors and
warnings appear as printer alerts.

The app distinguishes connecting, synchronizing, live, recovering and offline
connections from the printer's own idle/printing/paused/error state. Retained
telemetry keeps its observation timestamp while reconnecting; it is not a
fresh online sample. Local status polls target five-second cycles, with one
worker per configured printer, a three-second background status budget and independent retry
backoff up to one minute. Discovery scans recur every 15 seconds and retain
presence for 60 seconds. Closing the app stops these background workers.

Control availability follows observed inventory, printer state and driver
support. A visible temperature or fan may be telemetry-only. Busy-state notices
explain locked motion/temperature controls. Firmware authorization and safety
checks run again in Rust; a disabled control cannot be enabled by changing the
frontend. AMS drying controls appear only for a heater-equipped unit whose
printer advertises remote drying. See [Bambu compatibility](bambu-compatibility.md).

Print cancellation, destructive actions and print dispatch use confirmation
flows. The dedicated emergency-stop button also asks for confirmation in the
desktop and is disabled without usable status. Once confirmed, its backend
path avoids the ordinary pooled command lock. The CLI's `emergency-stop`
command has no confirmation prompt; the two surfaces differ here.

## Cameras and feed selection

**Cameras** manages independent Tapo profiles, lists linked printers and opens
the selected camera's live view. Add/edit verifies the stream before saving
connection changes; a rename alone can succeed offline. On edit, a blank
password preserves the stored password. Linked printers can select the camera
through **Camera feed** in Control. An external feed remains usable when that
printer is offline.

High/Low quality is per viewer and is not saved. Views of the same external
camera at the same quality share an RTSP upstream, while different qualities
use separate streams. Built-in Bambu views share an owner per physical serial.
Each desktop viewer has its own backend session ID, so stopping one releases
only that subscription. Leaving the view or hiding the window releases camera
resources; returning opens them again. There are no background camera
thumbnail sessions.

H.264 playback tries WebRTC, then WebCodecs, then MJPEG. The badge reports the
active transport: **LIVE · WebRTC**, **LIVE · H.264** or amber **LIVE · MJPEG**.
Classic Bambu JPEG sources use MJPEG. Webview/codec support determines whether
native H.264 is available; Linux can use WebCodecs. See [IP cameras](ip-cameras.md)
for setup and [Wayland notes](../README.md#running-on-linux-wayland).

## File library and slicers

The local library starts at the last browsed folder, or Documents/Melt on first
use. Browse directories, search/sort files by name/size/date, add files to the
current folder, and view model cards with file metadata. A directory watcher
refreshes changes made outside Melt. The library is local storage, separate
from the printer's storage file table in Control.

Model previews support STL, OBJ, 3MF and supported ZIP contents. Clicking a
model thumbnail opens a larger static preview with metadata and available
print, slicer and delete actions. A loading indicator covers the larger render;
preview failure does not make the source file unprintable or undownloadable.
Embedded previews are preferred, and bounded software rendering handles models
without them. Preview queues and memory/disk caches avoid repeated work.

Configure enabled slicers under Configuration and open a model with the
selected executable. Melt launches local slicer processes; it is not a slicer.
An unsliced model must be sliced before print dispatch. For a sliced 3MF,
the print flow inspects the package, chooses the printer and plate, checks
materials/nozzles and destination, uploads with verification and waits for
observed print start. Printer file downloads expose progress and cancellation
and are finalized only after transfer integrity checks.

## Configuration, firmware and diagnostics

**Configuration** contains completion/failure/disconnection notifications,
firmware source opt-in, enabled slicers, temperature presets, theme and language,
plus About/version/license information and diagnostics. Slicer and preset
editing opens a side panel. English and Brazilian Portuguese are supported.
See [configuration storage](configuration.md) for which choices persist where.

**Firmware & software** opens from the printer's actions. It separates installed,
printer-advertised, device-catalogue and public-stable evidence and leads with
the assessment. Refresh follows the saved source preference; **Also check
Bambu.com** is an explicit one-shot opt-in. No action installs firmware.
See [firmware evidence](firmware-updates.md) for interpretation and caches.

**Configuration → About → Diagnostics** exports printer compatibility,
firmware/capability/transport provenance, authorization, camera-owner and
external-camera stream state, and the most recent playback result and failure
reasons. Reports use local `printer-N` / `camera-N` labels and redact endpoints,
credentials, module serials and full fingerprints. Playback observations remain
available after a viewer closes, for the current app session. Unknown module
values are summarized by type rather than copied. Inspect exported reports
before sharing and keep raw configuration/protocol captures private.
