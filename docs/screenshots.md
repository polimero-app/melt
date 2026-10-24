# Screenshots

Last-updated: 2026-10-12

Nine fresh captures of the current production Vue interface, including the
**Printers** label, **Cameras** view, **Camera feed** selector, playback badges,
camera settings and large file preview. The six original image paths are
replaced with new captures; three additional views cover current features.

These screenshots use sanitized sample printer/camera status, filenames,
documentation-network addresses and illustrated media. The camera scene is
explicitly labeled as simulated; its H.264 frames pass through the real
WebCodecs player before capture. The images illustrate the UI and do not
establish live hardware, firmware availability or native-OS qualification.
See [desktop usage](desktop.md) and [IP cameras](ip-cameras.md) for behavior.

Capture settings: dark theme, English, UTC, 1600×1100 CSS viewport at 2× pixel
scale, full-page PNG. Native title bars and browser chrome are excluded.
[The capture manifest](../screenshots/capture-manifest.json) records the source
commit/hash, sample fixture hash, actual capture time, fixed scenario time,
image dimensions and checksums.

## Printers

Fleet overview: total, online, and printing counts, plus per-printer status,
model, host, and quick actions.

![Current Printers view with three sample profiles, fleet counts and Open control actions](../screenshots/manage-printers.png)

## Configuration

Notifications, firmware sources, registered slicer applications, material
presets, appearance, and version/license information.

![Configuration view with notifications, firmware sources, and slicers](../screenshots/configuration.png)

## File library

Browsing a directory of model files with size and modified date, ready to
organize, slice, or print.

![Current File library view with sample STL thumbnails, folder navigation, sorting and search](../screenshots/file-library.png)

## File preview

The large preview dialog keeps the file's metadata and print/slicer/delete
actions together. This capture uses an illustrated sample model preview.

![File preview dialog showing the cable organizer sample and file actions](../screenshots/file-preview.png)

## Firmware & software

Per-printer comparison of installed module versions against printer-advertised,
device-catalogue, and public-stable evidence.

The assessment distinguishes a newer public stable release from an update
confirmed by the printer; a public release is not an actual release offer.

![Firmware and software comparison view for a single printer](../screenshots/firmware-software.png)

## Printer control — idle

Camera, current job, temperatures, fans, lights, motion controls, printer
files, and AMS/material systems for an idle printer.

The camera card includes the built-in/external feed picker and the H.264
playback badge. The sample also shows active AMS HT drying telemetry.

![Printer control view for an idle printer](../screenshots/printer-control-idle.png)

## Printer control — printing

The same control view mid-print, with job progress, layer count, time
remaining, and live AMS slot assignments.

The job is the sample Cable organizer, plate 1 of 2. Busy-state explanations
remain visible above locked temperatures and motion controls.

![Printer control view for a printer actively printing](../screenshots/printer-control-printing.png)

## Cameras

Independent camera profiles, linked printers, per-viewer High/Low quality,
and the active H.264 playback badge.

![Cameras view showing two sample Tapo profiles and a decoded H.264 sample feed](../screenshots/cameras.png)

## Camera settings

Editing an existing profile shows the connection details, a blank password
field that retains the saved password, and the linked-printer checkboxes.
The sample contains no camera password or real credentials.

![Edit camera side panel with sample connection settings and linked printers](../screenshots/camera-settings.png)

## Reproducing the captures

Install the UI dependencies as described in [development](development.md).
The screenshot helper additionally needs Python with `playwright` and `Pillow`,
the Playwright Chromium browser, and `ffmpeg` with the `libx264` encoder.

```sh
python3 -m pip install playwright Pillow
python3 -m playwright install chromium
make screenshots
```

`make screenshots` builds the current UI and runs
[`scripts/capture_screenshots.py`](../scripts/capture_screenshots.py).
The helper serves `ui/dist` on an ephemeral loopback port, supplies only the
checked-in [sample IPC data](../fixtures/screenshots/state.json), blocks
external browser requests and refuses unsupported backend actions. It never
opens a printer connection, reads user configuration or accesses a keychain.
WebRTC is unavailable in this capture environment; the actual WebCodecs path
is attempted and MJPEG remains its fallback.

To review files before replacing the repository images:

```sh
make ui-build
python3 scripts/capture_screenshots.py --output /tmp/melt-screenshots
```

Visually inspect every view, check the recorded playback, and retain the
manifest alongside the images. Rebuild and recapture whenever UI source or
dependencies change.
