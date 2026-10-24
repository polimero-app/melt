#!/usr/bin/env python3
"""Render the production Vue UI with isolated documentation-only IPC fixtures.

Requires Python Playwright + Chromium, Pillow, ffmpeg with libx264, and ui/dist.
Only loopback HTTP is served. No printer, camera, keychain or user config is used.
"""

import argparse
import base64
import copy
import hashlib
import io
import json
import re
import struct
import subprocess
import threading
import time
from datetime import datetime, timezone
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

from PIL import Image, ImageDraw, ImageFont
from playwright.sync_api import expect, sync_playwright

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "fixtures/screenshots/state.json"
WIDTH, HEIGHT, SCALE = 1600, 1100, 2


def font(size):
    return ImageFont.truetype("DejaVuSans.ttf", size)


def model_image(name, large=False):
    """Synthetic model illustrations, supplied as backend preview PNG fixtures."""
    width, height = (1920, 1080) if large else (640, 360)
    image = Image.new("RGB", (width, height), "#132739")
    draw = ImageDraw.Draw(image)
    scale = width / 640

    def point(x, y, z):
        return ((320 + (x - y) * 1.6) * scale, (210 + (x + y) * .6 - z * 1.7) * scale)

    def box(x, y, z, w, d, h, color):
        colors = tuple(int(color[i:i + 2], 16) for i in (1, 3, 5))
        a, b, c, e = point(x, y, z), point(x + w, y, z), point(x + w, y + d, z), point(x, y + d, z)
        top = [point(x, y, z + h), point(x + w, y, z + h), point(x + w, y + d, z + h), point(x, y + d, z + h)]
        draw.polygon([a, b, top[1], top[0]], fill=tuple(int(v * .62) for v in colors))
        draw.polygon([b, c, top[2], top[1]], fill=tuple(int(v * .8) for v in colors))
        draw.polygon([e, c, top[2], top[3]], fill=tuple(int(v * .7) for v in colors))
        draw.polygon(top, fill=colors)

    draw.ellipse((190 * scale, 220 * scale, 460 * scale, 290 * scale), fill="#0e1d2b")
    color = "#35b4ce"
    if "cube" in name:
        box(-35, -35, 0, 70, 70, 70, color)
    elif "tray" in name:
        box(-65, -42, 0, 130, 84, 5, color)
        for values in [(-65, -42, 5, 130, 5, 20), (-65, 37, 5, 130, 5, 20), (-65, -42, 5, 5, 84, 20), (60, -42, 5, 5, 84, 20)]:
            box(*values, color)
    elif "bracket" in name:
        box(-45, -30, 0, 90, 60, 8, color)
        box(-45, 22, 8, 90, 8, 75, color)
    elif "planter" in name:
        box(-42, -42, 0, 84, 84, 65, color)
        draw.polygon([point(-34, -34, 66), point(34, -34, 66), point(34, 34, 66), point(-34, 34, 66)], fill="#153643")
    elif "spool" in name:
        box(-55, -25, 0, 110, 50, 8, color)
        box(-40, -15, 8, 12, 30, 66, color)
        box(28, -15, 8, 12, 30, 66, color)
        box(-40, -15, 65, 80, 30, 10, color)
    else:
        box(-65, -25, 0, 130, 50, 8, color)
        for x in (-56, -20, 16, 52):
            box(x, -25, 8, 8, 50, 26, color)
    out = io.BytesIO()
    image.save(out, "PNG")
    return out.getvalue()


def camera_frame(printing=False):
    """A labeled synthetic camera scene; never derived from a private capture."""
    image = Image.new("RGB", (960, 540), "#10171e")
    draw = ImageDraw.Draw(image)
    draw.rectangle((105, 35, 855, 510), fill="#283540", outline="#5c707d", width=8)
    draw.rectangle((130, 52, 830, 75), fill="#dee9ec")
    draw.polygon([(170, 220), (780, 220), (820, 470), (130, 470)], fill="#535f68", outline="#a8b5bd")
    for row in range(12):
        y = 232 + row * 19
        draw.line((165 - row * 2, y, 782 + row * 2, y), fill="#62707a", width=1)
    for column in range(16):
        x = 170 + column * 39
        draw.line((x, 220, 480 + (x - 480) * 1.12, 470), fill="#62707a", width=1)
    draw.rectangle((145, 155, 810, 180), fill="#a2b0b9")
    draw.rectangle((430, 130, 555, 210), fill="#141c23", outline="#657c89", width=3)
    draw.rectangle((455, 200, 530, 225), fill="#465760")
    draw.polygon([(475, 224), (510, 224), (495, 248)], fill="#e0b35e")
    if printing:
        for x in (315, 400, 485, 570):
            draw.polygon([(x, 337), (x + 70, 337), (x + 85, 372), (x - 10, 372)], fill="#159ab6")
            draw.rectangle((x, 300, x + 70, 337), fill="#2ac2dc")
    else:
        draw.text((335, 337), "BUILD PLATE READY", font=font(23), fill="#d2dde2")
    draw.rounded_rectangle((20, 485, 592, 523), radius=7, fill="#10171e")
    draw.text((32, 494), "SIMULATED CAMERA  /  DOCUMENTATION SAMPLE", font=font(18), fill="#bdd0da")
    out = io.BytesIO()
    image.save(out, "JPEG", quality=90)
    return out.getvalue()


def h264_unit(jpeg):
    process = subprocess.run([
        "ffmpeg", "-loglevel", "error", "-i", "pipe:0", "-frames:v", "1",
        "-c:v", "libx264", "-preset", "ultrafast", "-tune", "zerolatency",
        "-pix_fmt", "yuv420p", "-f", "h264", "pipe:1",
    ], input=jpeg, capture_output=True, check=True)
    nals = [part for part in re.split(b"\x00\x00\x00\x01|\x00\x00\x01", process.stdout) if part]
    sps = next(nal for nal in nals if nal[0] & 31 == 7)
    return b"".join(b"\x00\x00\x00\x01" + nal for nal in nals), "avc1." + sps[1:4].hex()


def screenshot_state(base, printing):
    data = copy.deepcopy(base)
    capabilities = {key: True for key in (
        "status", "temperatureRead", "lightControl", "speedControl", "discovery",
        "cameraStream", "cameraSnapshot", "fileList", "fileDownload", "fileUpload",
        "fileDelete", "jobStart", "jobPause", "jobResume", "jobCancel",
        "emergencyStop", "temperatureWrite", "motionControl", "fanControl",
        "tlsRefresh", "firmwareUpdateCheck", "amsDrying",
    )}
    data["capabilities"] = capabilities
    status = data["idleStatus"]
    active = copy.deepcopy(status)
    active.update({
        "state": "printing", "job": {"name": "Cable organizer"},
        "printMeta": {"fileName": "cable-organizer.gcode.3mf", "fileSize": 17301504, "plateIndex": 1, "plateCount": 2},
        "progress": {"percent": 47, "currentLayer": 83, "totalLayers": 178},
        "timeEstimates": {"elapsedSeconds": 4230, "remainingSeconds": 4770, "totalSeconds": 9000},
        "stage": "printing", "stageCode": 0,
    })
    temperatures = active["controls"]["temperatures"]
    for key, current, target in (("bed", 55, 55), ("chamber", 34, 0), ("leftNozzle", 35, 0), ("rightNozzle", 215, 215)):
        temperatures[key].update(currentCelsius=current, targetCelsius=target)
    for key, speed in (("parts", 100), ("auxiliary", 40), ("exhaust", 30)):
        active["controls"]["fans"][key].update(speedPercent=speed, mode="manual")
    data["monitoring"] = [{
        "name": p["name"], "driver": p["driver"], "connectionState": "live",
        "stale": False, "observedAt": data["capturedAt"],
        "status": copy.deepcopy(active if (p["name"] == "workshop" if printing else p["name"] == "studio") else status),
    } for p in data["printers"]]
    data["firmware"] = []
    for p in data["printers"]:
        if p["driver"] == "moonraker":
            data["firmware"].append({
                "profile": p["name"], "driver": p["driver"], "checkedAt": data["capturedAt"], "stale": False,
                "report": {"source": "moonrakerUpdateManager", "availability": "current", "assessment": "current", "issues": [], "evidenceSources": ["moonrakerUpdateManager"], "components": [{"id": "klipper", "label": "Klipper", "kind": "printerSoftware", "currentVersion": "v0.13.0", "availableVersion": "v0.13.0", "availability": "current", "required": False, "evidence": [{"source": "moonrakerUpdateManager", "role": "installed", "version": "v0.13.0", "required": False}, {"source": "moonrakerUpdateManager", "role": "upstreamCurrent", "version": "v0.13.0", "required": False}]}]},
            })
            continue
        source = "bambuMqtt" if p["driver"] == "bambu-lan" else "moonrakerUpdateManager"
        components = []
        for module, label, current, target in (
            ("ota", "Printer firmware", "01.02.00.00", "01.03.00.00"),
            ("mc", "Motion controller", "00.02.10.06", None),
            ("ams/0", "AMS 2 Pro", "00.01.06.20", None),
            ("ams/128", "AMS HT", "00.01.06.20", None),
        ):
            evidence = [{"source": "bambuLanInventory", "role": "installed", "version": current, "required": False}]
            if target:
                evidence.append({"source": "bambuPublicCatalogue", "role": "publicStable", "version": target, "required": False})
            components.append({"id": module, "label": label, "kind": "printerFirmware" if module == "ota" else "accessoryFirmware", "currentVersion": current, "availability": "unknown", "required": False, "evidence": evidence})
        data["firmware"].append({
            "profile": p["name"], "driver": p["driver"], "checkedAt": data["capturedAt"], "stale": False,
            "sources": [{"source": s, "checkedAt": data["capturedAt"], "outcome": "empty" if s in ("bambuLanHistory", "bambuLanAdvertisement") else "success", "stale": False} for s in ("bambuLanInventory", "bambuLanAdvertisement", "bambuLanHistory", "bambuPublicCatalogue")],
            "report": {"source": source, "availability": "unknown", "assessment": "publicReleaseNewer", "components": components, "issues": [], "evidenceSources": ["bambuLanInventory", "bambuPublicCatalogue"]},
        })
    return data


BRIDGE = r"""
(() => {
  const fixture = __STATE__;
  const base = __BASE__;
  const commands = window.__SCREENSHOT_COMMANDS__ = [];
  const rejected = window.__SCREENSHOT_REJECTED__ = [];
  const callbacks = new Map(); let next = 1;
  const stamp = new Date(fixture.capturedAt).getTime();
  const RealDate = Date;
  window.Date = class extends RealDate {
    constructor(...args) { super(...(args.length ? args : [stamp])); }
    static now() { return stamp; }
  };
  localStorage.setItem('theme', 'dark'); localStorage.setItem('locale', 'en');
  localStorage.setItem('activeView', __VIEW__); localStorage.setItem('activePrinterId', 'workshop');
  // Exercise the app's real WebCodecs -> MJPEG paths without a WebRTC peer.
  window.RTCPeerConnection = undefined;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener() {}};
  window.__TAURI_INTERNALS__ = {
    transformCallback(fn) { const id = next++; callbacks.set(id, fn); return id; },
    unregisterCallback(id) { callbacks.delete(id); },
    async invoke(cmd, args = {}) {
      commands.push(cmd);
      const name = args.name || args.request?.name || 'workshop';
      switch (cmd) {
        case 'plugin:event|listen': return next++;
        case 'plugin:event|unlisten': case 'camera_viewer_stop': case 'set_library_path': return null;
        case 'app_info': return {version: '0.1.0', modes: ['gui', 'headless'], license: 'AGPL-3.0-only', sourceUrl: 'https://github.com/polimero-app/melt'};
        case 'configured_printers': return fixture.printers;
        case 'registered_drivers': return fixture.drivers;
        case 'get_preferences': return fixture.preferences;
        case 'camera_list': return fixture.cameras;
        case 'plug_list': return fixture.plugs;
        case 'plug_observations': return fixture.plugObservations;
        case 'printer_capabilities': return {capabilities: fixture.capabilities};
        case 'cached_monitoring': case 'monitored_printers': return fixture.monitoring;
        case 'printer_status': return fixture.monitoring.find(p => p.name === name);
        case 'cached_firmware_updates': return fixture.firmware;
        case 'printer_firmware_updates': return fixture.firmware.find(p => p.profile === name);
        case 'printer_files': return {entries: fixture.printerFiles};
        case 'inspect_printer_print': return {
          sourceFileName: args.request.devicePath.split('/').pop(), sliced: true, issues: [],
          plates: [{index: 1, gcodePath: 'Metadata/plate_1.gcode', thumbnailPaths: []}]
        };
        case 'library_files': return fixture.library;
        case 'library_file_preview': case 'printer_file_preview': {
          const query = new URLSearchParams({name: args.request.devicePath || args.request.path, large: String(args.request.large || false)});
          return await (await fetch(base + '/__preview?' + query)).json();
        }
        case 'camera_h264_stream': return {viewerId: 'documentation-h264', url: base + '/__h264', codec: __CODEC__};
        case 'camera_mjpeg_stream': return {viewerId: 'documentation-mjpeg', url: base + '/__mjpeg'};
        case 'camera_snapshot': return {dataUrl: base + '/__camera.jpg'};
        case 'camera_select': {
          fixture.cameras.forEach(c => {c.selectedBy = c.selectedBy.filter(p => p !== args.printer); if(c.name === args.camera) c.selectedBy.push(args.printer);});
          return {printer: args.printer, camera: args.camera};
        }
        default: rejected.push(cmd); throw new Error('Screenshot fixture refuses unsupported IPC: ' + cmd);
      }
    }
  };
})();
"""


class CaptureServer(ThreadingHTTPServer):
    daemon_threads = True


def check_view_navigation(page):
    page.get_by_role("button", name="Files", exact=True).click()
    page.get_by_role("button", name="Preview cable-organizer.stl", exact=True).wait_for()
    page.locator('[name="file-search"]').fill("cable")
    page.get_by_role("button", name="Configuration", exact=True).click()
    page.locator("#theme").select_option("light")
    page.wait_for_function("!document.documentElement.classList.contains('dark')")
    page.locator("#theme").select_option("dark")
    page.locator("#language").select_option("pt-BR")
    page.get_by_role("button", name="Arquivos", exact=True).wait_for()
    page.locator("#language").select_option("en")
    page.get_by_role("button", name="Files", exact=True).click()
    assert page.locator('[name="file-search"]').input_value() == "cable"
    page.get_by_role("button", name="Clear search", exact=True).click()
    page.get_by_role("button", name="Cameras", exact=True).click()
    page.get_by_text(re.compile(r"LIVE · (H\.264|MJPEG)$")).wait_for(timeout=20000)
    stops = page.evaluate("window.__SCREENSHOT_COMMANDS__.filter(cmd => cmd === 'camera_viewer_stop').length")
    page.get_by_role("button", name="Smart plugs", exact=True).click()
    page.get_by_role("heading", name="Smart plugs", exact=True).wait_for()
    page.wait_for_function("count => window.__SCREENSHOT_COMMANDS__.filter(cmd => cmd === 'camera_viewer_stop').length > count", arg=stops)
    assert page.get_by_text(re.compile(r"LIVE · (H\.264|MJPEG)$")).count() == 0
    page.get_by_role("button", name=re.compile(r"^workshop\b")).click()
    page.get_by_role("heading", name="workshop", exact=True).wait_for()
    page.get_by_role("button", name="Print cable-organizer.gcode.3mf", exact=True).click()
    expect(page.get_by_role("dialog").get_by_role("button", name="Print", exact=True)).to_be_visible()
    expect(page.locator('[name="print-plate"]')).to_have_value("1")
    assert page.evaluate("!window.__SCREENSHOT_COMMANDS__.includes('printer_job_action')")
    page.get_by_role("dialog").get_by_role("button", name="Cancel", exact=True).click()
    expect(page.get_by_role("dialog")).to_have_count(0)
    page.get_by_role("button", name="Printers", exact=True).click()
    page.get_by_role("heading", name="Printer management", exact=True).wait_for()


def capture(args):
    dist = ROOT / "ui/dist"
    if not (dist / "index.html").is_file():
        raise SystemExit("Build the current frontend with make ui-build first.")
    state = json.loads(FIXTURES.read_text())
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    media = {}
    for printing in (False, True):
        jpeg = camera_frame(printing)
        unit, codec = h264_unit(jpeg)
        media[printing] = jpeg, unit, codec

    class Handler(SimpleHTTPRequestHandler):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, directory=str(dist), **kwargs)

        def log_message(self, *args):
            pass

        def do_GET(self):
            path = urlsplit(self.path)
            if path.path == "/__preview":
                from urllib.parse import parse_qs
                query = parse_qs(path.query)
                image = model_image(query["name"][0], query.get("large") == ["true"])
                body = json.dumps({"kind": "png", "data": base64.b64encode(image).decode()}).encode()
                self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
            elif path.path in ("/__h264", "/__mjpeg", "/__camera.jpg"):
                jpeg, unit, _ = media[server.printing]
                self.send_response(200)
                self.send_header("Cache-Control", "no-store")
                self.send_header("Content-Type", {"/__h264": "application/octet-stream", "/__mjpeg": "multipart/x-mixed-replace; boundary=sample", "/__camera.jpg": "image/jpeg"}[path.path])
                self.end_headers()
                if path.path == "/__camera.jpg":
                    self.wfile.write(jpeg); return
                try:
                    for index in range(1200):
                        chunk = struct.pack(">IQB", len(unit), index * 100000, 1) + unit if path.path == "/__h264" else b"--sample\r\nContent-Type: image/jpeg\r\nContent-Length: " + str(len(jpeg)).encode() + b"\r\n\r\n" + jpeg + b"\r\n"
                        self.wfile.write(chunk); self.wfile.flush(); time.sleep(.1)
                except (BrokenPipeError, ConnectionResetError):
                    pass
            else:
                super().do_GET()

    server = CaptureServer(("127.0.0.1", 0), Handler)
    server.printing = False
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base = f"http://127.0.0.1:{server.server_port}"
    captures = []
    problems = []
    definitions = [
        ("manage-printers.png", "printers", False, "fleet"),
        ("configuration.png", "settings", False, "settings"),
        ("file-library.png", "files", False, "library"),
        ("firmware-software.png", "printers", False, "firmware"),
        ("printer-control-idle.png", "control", False, "idle"),
        ("printer-control-printing.png", "control", True, "printing"),
        ("printer-control-compact.png", "control", False, "compact"),
        ("cameras.png", "cameras", False, "cameras"),
        ("camera-settings.png", "cameras", False, "camera-settings"),
        ("file-preview.png", "files", False, "preview"),
        ("printer-control-offline.png", "control", False, "offline"),
        ("smart-plugs.png", "plugs", False, "plugs"),
        ("smart-plug-settings.png", "plugs", False, "plug-settings"),
        ("smart-plug-power-off.png", "plugs", False, "plug-off"),
    ]
    app_types = (ROOT / "ui/src/app-types.ts").read_text()
    views = set(re.findall(r"'(\w+)'", re.search(r"export type View = (.+)", app_types).group(1)))
    uncovered = views - {view for _, view, _, _ in definitions}
    if uncovered:
        raise SystemExit(f"Add screenshot scenarios for views: {', '.join(sorted(uncovered))}")
    try:
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch(headless=True)
            for filename, view, printing, kind in definitions:
                server.printing = printing
                context = browser.new_context(viewport={"width": 1280 if kind == "compact" else WIDTH, "height": HEIGHT}, device_scale_factor=SCALE, locale="en-US", timezone_id="UTC", color_scheme="dark", reduced_motion="reduce")
                scenario = screenshot_state(state, printing)
                if kind == "compact":
                    # A1 mini-style printer: the chamber is a read-only reading with no target.
                    scenario["monitoring"][0]["status"]["controls"]["temperatures"]["chamber"] = {"kind": "chamber", "currentCelsius": 5, "controllable": False}
                if kind == "offline":
                    scenario["monitoring"][0] = {"name": "workshop", "driver": "bambu-lan", "connectionState": "offline", "stale": True}
                script = BRIDGE.replace("__STATE__", json.dumps(scenario)).replace("__BASE__", json.dumps(base)).replace("__VIEW__", json.dumps(view)).replace("__CODEC__", json.dumps(media[printing][2]))
                context.add_init_script(script)
                page = context.new_page()
                page.on("pageerror", lambda error: problems.append(str(error)))
                page.route("**/*", lambda route: route.continue_() if urlsplit(route.request.url).hostname in ("127.0.0.1", None) else route.abort())
                page.goto(base, wait_until="load")
                page.get_by_role("navigation", name="Workspace navigation", exact=True).wait_for()
                page.get_by_role("button", name="Printers", exact=True).wait_for()
                if kind == "fleet":
                    summary = page.locator("dl").filter(has=page.get_by_text("Total printers", exact=True))
                    expect(summary.locator("dd")).to_have_text(["3", "3", "1"])
                    assert summary.bounding_box()["height"] <= 24, "Fleet counts must fit on one line"
                    cards = page.get_by_role("article")
                    expect(cards.get_by_role("button", name="Open control", exact=True)).to_have_count(len(state["printers"]))
                    expect(cards.get_by_role("button", name=re.compile("Firmware"))).to_have_count(0)
                    check_view_navigation(page)
                if kind == "firmware":
                    page.get_by_role("button", name="Printer actions", exact=True).first.click()
                    expect(page.get_by_role("menuitem", name="Firmware", exact=True)).to_have_count(1)
                    page.get_by_role("menuitem", name="Firmware", exact=True).click()
                    expect(page.get_by_role("heading", name="workshop", exact=True)).to_be_visible()
                    page.get_by_text("Public stable release is newer", exact=True).wait_for()
                    page.locator("summary").click()
                if kind in ("idle", "printing", "cameras", "camera-settings"):
                    page.get_by_text(re.compile(r"LIVE · (H\.264|MJPEG)$")).wait_for(timeout=20000)
                if kind == "camera-settings":
                    page.get_by_role("button", name="Camera actions", exact=True).first.click()
                    page.get_by_role("menuitem", name="Edit camera", exact=True).click()
                    page.locator("#camera-form").wait_for()
                if kind == "compact":
                    expect(page.get_by_role("button", name=re.compile("^workshop"))).to_be_visible()
                    expect(page.get_by_role("button", name="Select printer")).to_have_count(0)
                    expect(page.get_by_text("Chamber", exact=True)).to_have_count(0)
                if kind in ("idle", "cameras"):
                    expect(page.get_by_role("button", name="Turn off bench-power", exact=True)).to_be_visible()
                if kind == "offline":
                    page.get_by_role("heading", name="Printer unavailable", exact=True).wait_for()
                    expect(page.get_by_role("button", name="Turn off bench-power", exact=True)).to_be_visible()
                if kind in ("plugs", "plug-settings", "plug-off"):
                    expect(page.get_by_role("button", name="Smart plug actions", exact=True)).to_have_count(len(state["plugs"]))
                    expect(page.get_by_role("button", name="Edit credentials", exact=True)).to_be_visible()
                    for text in ("215 W", "0 W", "Last known: Relay on (stale) · Unreachable · Observed 11:55 AM"):
                        expect(page.get_by_text(text, exact=True)).to_be_visible()
                if kind == "plug-settings":
                    page.get_by_role("button", name="Smart plug actions", exact=True).first.click()
                    page.get_by_role("menuitem", name="Edit smart plug", exact=True).click()
                    page.locator("#plug-form").wait_for()
                    expect(page.locator("#plug-password")).to_have_value("")
                if kind == "plug-off":
                    page.get_by_role("button", name="Turn off bench-power", exact=True).click()
                    dialog = page.get_by_role("alertdialog")
                    expect(dialog).to_contain_text("Printers: workshop (drying), studio (printing). Cameras: workbench.")
                    assert page.evaluate("!window.__SCREENSHOT_COMMANDS__.includes('plug_set_power')")
                if kind in ("library", "preview"):
                    page.get_by_role("button", name="Preview cable-organizer.stl", exact=True).wait_for()
                    page.wait_for_function("Array.from(document.querySelectorAll('[aria-busy]')).every(e => e.getAttribute('aria-busy') !== 'true')")
                if kind == "preview":
                    page.get_by_role("button", name="Preview cable-organizer.stl", exact=True).click()
                    page.locator('[role="dialog"] img').wait_for()
                    page.wait_for_function("document.querySelector('[role=dialog] img')?.complete && document.querySelector('[role=dialog] img')?.naturalWidth > 0")
                page.evaluate("document.fonts.ready")
                page.mouse.move(1590, 1090)
                problems.extend(f"Unsupported IPC in {filename}: {cmd}" for cmd in page.evaluate("window.__SCREENSHOT_REJECTED__"))
                if page.get_by_role("alert").count():
                    problems.extend(page.get_by_role("alert").all_text_contents())
                if kind in ("camera-settings", "plug-settings", "plug-off", "preview"):
                    # Modals fade/slide in; capture only once settled.
                    page.wait_for_function("""() => Array.from(document.querySelectorAll('[id^="headlessui-dialog-panel-"]')).some(e => {
                        const r = e.getBoundingClientRect();
                        return r.width > 0 && r.right <= innerWidth + .5 && getComputedStyle(e).opacity === '1';
                    })""")
                page.screenshot(path=str(output / filename), full_page=True, animations="disabled")
                with Image.open(output / filename) as png:
                    size = list(png.size)
                playback = page.get_by_text(re.compile(r"LIVE · (H\.264|MJPEG)$")).all_text_contents()
                captures.append({"file": filename, "view": kind, "size": size, "sha256": hashlib.sha256((output / filename).read_bytes()).hexdigest(), "playback": playback})
                print(f"Captured {filename}: {size[0]}×{size[1]}" + (f" ({', '.join(playback)})" if playback else ""), flush=True)
                context.close()
            browser.close()
    finally:
        server.shutdown(); server.server_close(); thread.join()
    if problems:
        raise SystemExit("Screenshot capture errors:\n" + "\n".join(sorted(set(problems))))
    source = hashlib.sha256()
    for file in sorted((ROOT / "ui/src").rglob("*")):
        if file.is_file():
            source.update(str(file.relative_to(ROOT)).encode()); source.update(file.read_bytes())
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    manifest = {"capturedAt": datetime.now(timezone.utc).isoformat(timespec="seconds"), "scenarioTime": state["capturedAt"], "sourceCommit": commit, "frontendSourceSha256": source.hexdigest(), "fixtureSha256": hashlib.sha256(FIXTURES.read_bytes()).hexdigest(), "viewport": {"width": WIDTH, "height": HEIGHT, "deviceScaleFactor": SCALE}, "locale": "en-US", "timezone": "UTC", "theme": "dark", "renderer": "Playwright Chromium", "data": "Sanitized synthetic IPC and camera/model media; no live hardware or user configuration", "captures": captures}
    (output / "capture-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Saved {len(captures)} screenshots and capture-manifest.json to {output}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "screenshots")
    capture(parser.parse_args())
