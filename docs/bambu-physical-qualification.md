# Bambu physical-printer qualification

Last-updated: 2026-10-12

Physical qualification proves one exact marketed model, hardware configuration,
and module-firmware inventory. It does not qualify a family, a later firmware,
or a printer that merely reports a similar model string.

No completed Bambu physical qualification row is currently checked in. The
port-6000 fixture is executable-test evidence. This document is the procedure
for adding hardware evidence, not a claim that every step has been performed.
For independent Tapo camera evidence, see [the camera fixture notes](../fixtures/tapo/README.md).

## Preconditions and safety boundary

1. Use a printer you own, or record the informed consent of its owner. Never
   capture a printer, network, or account outside that permission.
2. Start from a clean, frozen Melt commit and record its full SHA. Run
   `make release-qualification` before physical tests.
3. Use a disposable sliced object, clear build volume, known material, attended
   printer, and immediate physical access to its power and emergency controls.
4. Back up any printer files that a destructive test may replace. Keep cloud
   credentials, Wi-Fi credentials, access codes, private keys, serials, MACs,
   user paths, and local addresses out of committed artifacts.
5. Do not disable TLS verification to turn a failed qualification into a pass.
   Test insecure mode only as an explicitly labeled negative/security case.

## Capture identity before behavior

Run the live compatibility command before and after the test session:

```sh
melt printer capabilities <profile> --output json
```

Retain the raw output only in a private scratch location. The committed copy
must pass `make bambu-evidence-check`. Record the marketed model separately from
the raw/canonical model Melt observed, plus every returned module software
and hardware version. If a needed module version is absent, the row cannot
qualify a firmware-bounded quirk.

Record certificate identity/pin success as a boolean and failure category, not
the fingerprint. Record advertised or selected transports by scheme/type, not
the printer address or full URL. Record capability observation field names,
source, report kind, age, and value type; do not copy raw unknown values.

## Required test sequence

Use a new evidence directory under the matching transport in `fixtures/bambu`.
Every assertion gets a `manifest.json`; split unrelated assertions so a future
firmware contradiction can invalidate one fact without discarding the session.

1. **Discovery and identity:** observe SSDP/UDP identity, continuous last-seen
   refresh, configured-host mismatch suggestion, serial certificate identity,
   authentication, and pinned reconnect. Confirm discovery never edits the
   profile, credentials, or pin.
2. **Status and authorization:** retain sanitized full and delta field shapes,
   module inventory, explicit authorization fields, unknown fields, job/error
   transitions, and a reconnect. Verify a proven signing requirement blocks a
   mutation before the file or command transport is touched.
3. **Camera:** record live advertisement fields (redacted), negotiated transport,
   TLS identity result, codec parameters, timestamp behavior, reconnect with
   unchanged parameters, and behavior after changed SPS/PPS. Open concurrent
   snapshot, MJPEG, WebRTC and WebCodecs viewers as applicable and verify only
   one upstream owner exists for the serial; let one viewer stall and verify
   the others continue. Record the OS/webview, negotiated playback badge and
   redacted fallback reasons. Stop one viewer without disrupting another, then
   hide/restore the view and close the last viewer to verify resource release.
4. **Storage and port 6000:** list every root with capacity/writable metadata;
   upload a unique file, verify size/content, exercise collision handling,
   download, preview, and delete it. For tunnel operations retain sanitized
   command type, sequence, bounds, fragment placement, and correlated reply.
   Interrupt one transfer and confirm a partial destination is not accepted.
5. **Package and material mapping:** inspect a sliced single-plate and
   multi-plate 3MF, select a non-default plate, validate preview bounds, remote
   name, filament/nozzle metadata, and empty/ambiguous/unavailable AMS cases.
6. **Printing:** with an attended printer, exercise preflight through observed
   transition into printing exactly once. Verify pause/resume/cancel and the
   applicable leveling, calibration, timelapse, nozzle, and material options.
   A retry or reconnect must not create a duplicate print start.
7. **Controls and failures:** within safe temperatures/motion, check supported
   thermal, fan, light, speed, home/jog, and emergency paths. Record
   authentication rejection, TLS mismatch, timeout/network loss, storage full,
   destination collision, printer rejection, and unsupported-capability errors.
8. **Firmware availability:** capture `info.get_version` and the matching full
   `print.upgrade_state`, then capture a delta that advertises, changes, or
   clears a target. Cover printer and accessory modules plus forced updates.
   Confirm passive and explicit-refresh checks send no upgrade/install command.
   Verify a default CLI/LAN-only desktop setup makes no public HTTPS request;
   test history and public catalogue separately with their explicit source
   configuration, ensuring version evidence never becomes a printer offer.
   Confirm no cloud account credentials or download requests are used.
   A “current” qualification additionally needs
   an exact no-update capture and repeatable proof that silence has that meaning
   for the scoped model and module versions.
9. **AMS drying, when changed:** capture per-unit phase, remaining time,
   settings and blocking reasons. Verify a telemetry-only unit never receives
   a remote command, and a controllable unit's start/stop completes only on
   an observed transition. Cover already-active, cooling, stopping and fault
   states within the unit's safe temperature/duration bounds; mark absent
   hardware explicitly not applicable.

## Evidence layout and sanitization

Use an ID that states the narrow fact, for example:

```text
fixtures/bambu/camera/p2s-ota-01.02.03.04-rtp-timestamps/
  manifest.json
  access-units.json
```

The manifest's printer identity must use stable synthetic labels such as
`SANITIZED_P2S_DEVICE`; do not encode a real serial in filenames, hashes, or
replacement labels. `sourceRevision` identifies the Melt commit used for
an executable or source-derived assertion. In `qualification`, name the exact
marketed model, hardware configuration, module/range being qualified, test
steps, result, operator/date, and owner-consent record location. Do not commit
the consent document if it contains personal information.

Run:

```sh
make bambu-evidence-check
git diff --check
```

Then have a second reviewer compare decoded and raw private captures, confirm
that replacements are consistent, and approve the narrow assertion. Automated
redaction is a floor and cannot recognize every semantic identifier.

## Promotion rule for quirks

A provisional registry entry becomes `Qualified` only when one physical row:

- names an exact canonical model and firmware module with inclusive bounds;
- contains sanitized evidence that directly demonstrates the effect;
- has a reproducer or executable regression test;
- identifies conflicts, owner, and an objective removal condition; and
- passes registry, evidence, CI, and review gates.

Use `Confirmed` only after an independent capture reproduces the same scoped
fact on another physical printer or after equivalent vendor-confirmed evidence.
A contradictory later firmware gets a separate scoped entry; it does not erase
the older evidence. Expired and contradicted entries stay diagnostic-only.

## Qualification record

Copy [the row template](templates/bambu-qualification-row.md) into the release
evidence or evidence directory and fill every field. Mark an item not applicable
with a reason; never turn an unperformed item into a pass. Link the completed row
from the release packaging checklist when the release changes Bambu behavior.
