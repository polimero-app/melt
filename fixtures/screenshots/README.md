# Screenshot samples

Last-updated: 2026-10-12

`state.json` contains synthetic, sanitized data for the current desktop UI's
documentation captures. Printer/camera names, serial labels, paths, module
versions and job states are examples. Hosts use `192.0.2.0/24`; there are no
credentials or raw hardware captures. Tray slots follow the core's zero-based
contract and are displayed as slots 1–4 by the frontend.

[`scripts/capture_screenshots.py`](../../scripts/capture_screenshots.py) renders
the built production frontend with this isolated IPC bridge. It derives
idle/printing scenarios, component update evidence and driver capabilities,
and generates labeled camera scenes and model illustrations. No simulated
response is wired into production code. Unsupported IPC actions fail rather
than reaching real equipment.

The camera sample is encoded with ffmpeg/libx264, framed with the same
13-byte length/timestamp/keyframe header as the desktop bridge, and decoded by
the unchanged frontend's H.264 parser/WebCodecs player. A multipart JPEG
sample is available as fallback. The capture records the playback actually
used; it never sets a successful transport badge directly.

These fixtures are for illustrative screenshots, not hardware qualification,
CLI-contract goldens, or Bambu protocol evidence. The fixed scenario timestamp
keeps visible dates and telemetry freshness stable; the manifest separately
records when captures were made.

See [the gallery and reproduction instructions](../../docs/screenshots.md).
