#!/usr/bin/env python3
"""Check E-stop keyboard behavior with isolated IPC; run after make ui-build.

Uses the same Python Playwright, Chromium and Pillow setup as screenshots.
"""

import json
import threading
from functools import partial
from http.server import SimpleHTTPRequestHandler
from urllib.parse import urlsplit

from playwright.sync_api import expect, sync_playwright

from capture_screenshots import BRIDGE, FIXTURES, ROOT, CaptureServer, screenshot_state


def check():
    state = screenshot_state(json.loads(FIXTURES.read_text()), printing=True)
    state["cameras"] = []
    state["capabilities"].update(cameraStream=False, cameraSnapshot=False)
    handler = partial(SimpleHTTPRequestHandler, directory=str(ROOT / "ui/dist"))
    with CaptureServer(("127.0.0.1", 0), handler) as server:
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        base = f"http://127.0.0.1:{server.server_port}"
        script = BRIDGE.replace("__STATE__", json.dumps(state)).replace("__BASE__", json.dumps(base)).replace("__VIEW__", '"control"').replace("__CODEC__", '""')
        script += """
        window.stopCommands = [];
        const invoke = window.__TAURI_INTERNALS__.invoke;
        window.__TAURI_INTERNALS__.invoke = async (cmd, args) => {
          if (cmd === 'plug_list') return [];
          if (cmd === 'printer_emergency_stop' || cmd === 'printer_job_action') {
            window.stopCommands.push({cmd, args});
            return null;
          }
          return invoke(cmd, args);
        };
        """
        try:
            with sync_playwright() as playwright:
                browser = playwright.chromium.launch(headless=True)
                for theme in ("light", "dark"):
                    page = browser.new_page(viewport={"width": 1280, "height": 800})
                    page.add_init_script(script + f"localStorage.setItem('theme', '{theme}');")
                    page.route("**/*", lambda route: route.continue_() if urlsplit(route.request.url).hostname == "127.0.0.1" else route.abort())
                    page.goto(base)
                    emergency = page.get_by_role("button", name="Emergency stop", exact=True, include_hidden=True)
                    expect(emergency).to_be_enabled()
                    assert ("dark" in (page.locator("html").get_attribute("class") or "").split()) == (theme == "dark")
                    job_cancel = page.locator("#main-content").get_by_role("button", name="Cancel", exact=True)
                    assert emergency.locator("svg").inner_html() != job_cancel.locator("svg").inner_html()
                    emergency.click()
                    dialog = page.get_by_role("alertdialog")
                    confirm = dialog.get_by_role("button", name="Stop now", exact=True)
                    expect(confirm).to_be_focused()
                    for button in (emergency, confirm):
                        color = button.evaluate("""el => {
                          const canvas = document.createElement('canvas');
                          const ctx = canvas.getContext('2d');
                          ctx.fillStyle = getComputedStyle(el).backgroundColor;
                          ctx.fillRect(0, 0, 1, 1);
                          return [...ctx.getImageData(0, 0, 1, 1).data];
                        }""")
                        assert color[3] == 255 and color[0] > 2 * max(color[1:3]), color
                    page.keyboard.press("Enter")
                    expect(dialog).to_have_count(0)
                    assert page.evaluate("window.stopCommands") == [
                        {"cmd": "printer_emergency_stop", "args": {"name": "workshop"}}
                    ]
                    emergency.click()
                    expect(confirm).to_be_focused()
                    page.keyboard.press("Escape")
                    expect(dialog).to_have_count(0)
                    emergency.click()
                    dialog.get_by_role("button", name="Cancel", exact=True).click()
                    expect(dialog).to_have_count(0)
                    job_cancel.click()
                    cancel = dialog.get_by_role("button", name="Cancel", exact=True)
                    expect(cancel).to_be_focused()
                    page.keyboard.press("Enter")
                    expect(dialog).to_have_count(0)
                    assert page.evaluate("window.stopCommands.length") == 1
                    page.close()
                browser.close()
        finally:
            server.shutdown()
            thread.join()
    print("E-stop focus, Enter, Escape, solid red themes and ordinary Cancel behavior passed.")


if __name__ == "__main__":
    check()
