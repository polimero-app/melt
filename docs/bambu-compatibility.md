# Bambu compatibility and qualification

Last-updated: 2026-10-23

Undocumented behavior and physical-printer captures follow the
[protocol evidence policy](bambu-protocol-evidence.md). Live observations and
safe probes outrank defaults; unresolved conflicts never become unconditional
model rules.

Secure first contact binds the self-signed leaf certificate's common name to
the configured printer serial before storing its SHA-256 pin. Every later TLS
transport requires that serial identity and the stored pin; a mismatch never
falls back to insecure mode or silently rewrites a profile.

Camera fan-out operates on complete encoded media units. RTSPS sources retain
their original RTP packets for WebRTC and Annex-B access units for WebCodecs;
they enable the software JPEG decoder only while a snapshot or MJPEG consumer
is attached. Classic camera sources expose complete JPEG frames. This boundary
permits snapshots and multiple viewers to share one upstream without
transcoding native H.264 viewers.

The shared camera manager keys Bambu sources by the normalized printer serial
and a credential-free connection revision. External Tapo sources use the
immutable camera ID, quality and connection revision in the same manager;
see [IP cameras](ip-cameras.md). Each subscriber mailbox retains only its
newest complete media unit under backpressure. The most recent decoded JPEG is
also retained for snapshots. The first subscriber opens the upstream; dropping
the last subscriber interrupts its socket, and connection changes invalidate
the previous generation.

Desktop MJPEG previews, WebRTC offers, WebCodecs streams, and snapshots all
subscribe through this manager. Native-video subscriptions receive only H.264
access units, while preview subscriptions receive only JPEG frames, so one slow
consumer cannot fill another media type's bounded queue. The WebCodecs bridge
uses an authenticated loopback binary stream rather than base64 IPC messages.
The browser waits for a keyframe, converts its normalized Annex-B NAL units to
length-prefixed AVC, and supplies an AVC decoder configuration record from the
SPS/PPS. This avoids WebKitGTK's rejection of Annex-B input without decoder
configuration. Later keyframe parameter-set changes reconfigure the decoder;
unchanged keyframes do not. This changes framing only, not video encoding.
WebRTC remains the first choice, with WebCodecs next and MJPEG as the final
fallback when native H.264 playback fails.
Each desktop viewer has an independent backend `viewerId`. Stopping a viewer
releases only its subscription, so other viewers of the source keep running.
The desktop releases subscriptions when their Control/Cameras view or the
application window is hidden and reconnects when visible again.

On Linux, WebRTC availability depends on the installed WebKitGTK build;
WebCodecs can render H.264 even when `RTCPeerConnection` is unavailable.
The webview never opens the printer's RTSPS URL directly, so
`WEBKIT_GST_ALLOWED_URI_PROTOCOLS` is not required for Melt's camera viewer.
Authentication and TLS pinning remain in Rust, and the player leaves decoder
hardware selection to the browser.

## Authorization policy

Melt resolves LAN authorization only from explicit live security fields or
future evidence-qualified rules. A proven `signingRequired` mode blocks file
writes, print start/control, motion, thermal, fan, light, and speed commands
before any mutating transport work, resolving live status first when the
client has not yet observed it. Emergency stop is gated only by an already
resolved mode: it must never block on a status exchange or the reusable
session lock, and the printer's own rejection remains its backstop. Read-only
status and diagnostics remain available. Unknown authorization retains
attempt-and-observe behavior.

Signed mutations are deliberately outside Melt's supported security model.
Melt does not accept a signing certificate, private key, CRL, or extracted
vendor application credentials, and it does not fall back to a signing helper.
When live evidence reports `signingRequired`, Developer Mode/LAN-only mode is
the supported path for control; otherwise the printer remains read-only in
Melt. This boundary applies even when another client could sign the same
command.

The community-reported meaning of `print.fun` bit 29 is retained as an
inactive, provisional observation. It is visible for qualification but cannot
block a command until independent physical evidence scopes it to the affected
model and module firmware. Conflicting explicit fields block mutations
conservatively and are surfaced rather than guessed.

The CLI and desktop surface these as distinct actionable errors. Use
`melt printer capabilities <name>` to refresh and inspect the live model,
module firmware, authorization resolution, selected transports, observation
provenance, and active/inactive quirks. Add `--output json` when capturing sanitized
qualification evidence; observed values that could contain endpoints or
credentials remain summarized by type rather than serialized.

## Camera transport selection

Fresh, host-validated `print.ipcam` advertisements outrank qualified quirks,
exact-model defaults, and family hints. Advertised URLs may target only the
configured printer and may not embed credentials. Stale or rejected
advertisements fall through to a non-mutating probe. TLS pin and certificate
identity failures are terminal and never cause a fallback.

P1S deliberately has no unconditional camera default because current community
implementations disagree between RTSPS and the classic TLS-MJPEG transport.
Melt probes RTSPS and then classic MJPEG within the caller's safety
boundary, caches the successful transport in the physical camera owner, and
keeps both disputed registry entries provisional.

The WebRTC adapter contains an arrival-clock RTP repair path, but it is enabled
only by an applicable qualified firmware quirk. It assigns one 90 kHz timestamp
to every packet in an access unit, preserves valid source deltas and wraparound,
and repairs frozen or regressing values from monotonic arrival time. The current
P2S report remains provisional, so production streams do not activate this path.

## MQTT command correlation

Command acknowledgements are matched by MQTT document section, command name,
and sequence ID together. Unsolicited full or delta status remains eligible to
update telemetry and to prove a requested state transition, but cannot
acknowledge an unrelated command. Full `pushall` status owns an independent
sequence namespace. Locally generated sequence IDs remain positive and within
the signed 32-bit range, including after wrap, to avoid exposing future
AMS-facing commands to known unsafe oversized identifiers.

Detailed status also preserves every nonzero HMS tuple with its raw
`AAAA-BBBB-SSSS-CCCC` identifier, firmware severity, normalized description,
and alert disposition. Error and warning events remain printer alerts; unknown
notification-severity events remain diagnostic-only. The known MQTT command
verification event and slot-scoped AMS auto-refill progress/completion events
are diagnostic-only regardless of their encoded severity, so they cannot
create false print-fault notifications. A printer whose primary state is
explicitly `error` still receives a generic error when no actionable event
explains it.

## Print preparation stages

Bambu stage codes 0–66, 74, and 77 are mapped to stable identifiers and
localized in the desktop. The raw `stageCode` is retained in detailed status
for future firmware values. A stage is displayed only while printing or
paused, because some idle firmware continues to report stage 0. Unknown active
codes render as the conservative `Preparing` label rather than an invented
operation; idle sentinels -1 and 255 are omitted.

## Temperature, fan, and light inventory

Detailed status exposes a `controls` inventory alongside the legacy portable
`temperatures`, `fans`, and `lights` fields. Inventory keys are stable command
identifiers; each entry carries its observed value, semantic kind, operating
mode where applicable, bounds when reported, and a per-item `controllable`
flag. The desktop renders every observed entry, but only shows an actuator when
that item is controllable. Read-only chamber temperature, automatic fans,
hotend/mainboard fan telemetry, and unknown lights therefore remain visible
without implying that Melt can command them. On H2-class printers, an
`off` airduct entry for Parts, Aux, or Exhaust is a user-adjustable switch state,
so those controls remain commandable just as they are in Bambu Studio.
Likewise, the H2 device-tree chamber controller (`device.ctc`) establishes a
writable chamber-temperature path when the legacy edit-support flag is absent;
an explicitly false support flag is still honored.

Names follow Bambu Studio terminology: Nozzle (or Left Nozzle and Right
Nozzle), Bed, Chamber; Parts, Aux, Hotend, Exhaust, MC Board, and Heat; and one
logical Lamp. `chamber_light` and `chamber_light2` are combined into Lamp and
are switched together. Other reported nodes, including Work Light, remain
separate telemetry unless live data establishes a supported control path.

Legacy fan commands retain their M106 routing. Printers reporting the device
airduct schema use `set_fan` with the reported fan index and tenths-of-percent
speed. Dual-nozzle printers use `set_nozzle_temp` with Bambu's right/left
extruder indices. Before any thermal, fan, or light mutation, Melt refreshes
status and rejects missing or telemetry-only inventory entries.

Legacy firmware may publish zero-valued Aux and Exhaust speed slots on models
without that hardware. Matching Bambu Studio, Melt includes those items in
the control inventory only when `support_aux_fan` or `support_chamber_fan`
confirms them. Heatbreak/Hotend speed remains visible as telemetry because it is
a real reported fan even though the legacy protocol does not expose a control.

Toolhead controls follow Bambu Studio. `select_extruder` switches between the
right (0) and left (1) extruder; the active one is `device.extruder.state`
bits 4-7. Extrude and retract move ±10 mm only when the active nozzle reads at
least 170 °C: printers reporting `cfg`, `fun`, `aux` and `stat` use
`set_extrusion_length`, others receive `M83` / `G0 E±10 F900`. `set_airduct`
switches mode (submode -1) or toggles the filter (submode 1/0, shown when
`fun` bit 46 is set); Exhaust is never offered and O-series models list only
Cooling and Heating. The H2C rack (`fun` bit 60 or `device.holder`) uses
`nozzle_holder_ctrl` actions 0/1/2 for Home/Row A/Row B and
`holder_nozzle_refresh` with id 255 to read every hotend. Hotend serial and
firmware details come from `get_version` and are not shown.

## Offline HMS and print-error descriptions

The bundled ha-bambulab catalogue supplies English and Brazilian Portuguese HMS
and print-error text without a network lookup. Model-specific variants take
priority over generic descriptions regardless of source iteration order.
Missing translations fall back to the English message; unknown codes and
published empty placeholders retain Melt's explicit generic explanation and
raw identifier. Existing named explanations, severity and alert/diagnostic
classification remain unchanged, including command-verification and AMS-refill
notifications. The desktop accepts the API's existing snake-case error
identifiers as well as retained camel-case payloads.

Official model-specific wiki paths are preferred to the existing HMS search
link when supplied by the pinned catalogue. These links open only when the
operator follows them; status parsing remains offline.

To update, review the pinned ha-bambulab revision in
`scripts/update_bambu_errors.py`, run `python scripts/update_bambu_errors.py`,
and review the generated model variants and accompanying
`crates/melt-core/data/bambu-errors.LICENSE`.
`python scripts/update_bambu_errors.py --check` verifies a byte-identical regeneration.
The notice is also embedded in the generated catalogue shipped in the binary.
Neither flattened Bambu Bar tables nor vendor application code are copied.

## Exact-model capability defaults

The verified P2S baseline (`resources/printers/N7.json`, Bambu Studio revision
`79df7ae309be398ed30c9f397c13c9911c2edc57`, profile `00.00.00.00`) supplies
otherwise absent nozzle/bed bounds, auxiliary/exhaust fan support, AMS humidity,
bed-leveling mode, flow calibration, timelapse and send-to-storage capabilities.
The baseline is exact-model source evidence, not a claim that these are MQTT
wire fields. Other models remain unseeded rather than inheriting family-wide
assumptions. Additional firmware-versioned profiles need their own review.

`modelDefaults` in runtime capabilities and exported diagnostics records the
source, revision, profile version and values separately from timestamped live
observations. Live values, including false, and retained live observations
outrank defaults even after identity is re-resolved. Status controls and command
confirmation use the same effective inventory; raw MQTT snapshots,
authorization, transport negotiation and delta merging are unchanged.

## AMS humidity

AMS humidity has two independent quantities. `humidityPercent` comes only from
the reported `humidity_raw` relative-humidity percentage; finite values from
0 through 100 are accepted, including fractional readings and zero.
`humidityIndex` is a presentation scale from 1 (dry) to 5 (wet), reversing the
printer's ordinal `humidity` field. The CLI and desktop prefer the measured
percentage and otherwise label the ordinal scale explicitly. An ordinal index
is never converted to an invented percentage or percentage range; the legacy
optional `humidityRange` field is no longer populated.

The source-derived P2S fixture identifies OTA `01.01.00.00` and is provisional
evidence, not a new physical-printer qualification.

## Physical AMS presence and print preflight

`present` is an optional per-tray physical-presence flag read from
`tray_exist_bits`, independently of retained filament identity, colour,
remaining quantity and firmware-variant `state`. Missing or malformed masks
and unsupported layouts remain unknown, not empty. The evidenced layouts
are ordinary units 0–3 with slots 0–3, HT units 128–152 with slot 0, and
explicit type-5 mixed AMS Lite reports. HT occupancy starts at bit 16 rather
than bit 128; mixed Lite slots start at bit 24. External holders have no
internal occupancy bit.

Explicitly absent trays cannot satisfy preflight filament requirements even
when their cached identity and colour match. CLI/desktop material cards show
them as empty without stale quantities; unknown presence retains the reported
metadata without claiming an empty slot. No universal tray-state heuristic is
used. These source-scoped rules remain provisional hardware evidence.

## Remaining filament quantities

Reported `remainingGrams` always wins, including zero. When it is unavailable,
an internal spool with a positive nominal weight and a remaining percentage
from 0 through 100 may expose a separately labelled `remainingGramsEstimate`,
rounded to the nearest gram. No default spool weight is invented. Invalid
percentages, missing weights, explicitly absent trays and external holders
produce no estimate. The desktop and detailed CLI mark estimates with `~`;
the desktop also supplies a localized screen-reader approximation label.

## Active filament path

`activeFilament` distinguishes a reported internal tray, an external holder,
and an explicitly unassigned path. An omitted field means unknown. Modern
reports select `device.extruder.info` by the extruder ID in `state` bits 4–7,
not by array position; its `snow` supplies the unit/slot pair. Ordinary,
HT and explicit mixed-Lite paths reuse the inventory's canonical tray indices.
Unknown/duplicate active IDs and invalid assignments do not fall back to a
different nozzle or a retained legacy value.

Legacy reports fall back to `ams.tray_now`: 254 means external and 255 means
unassigned. A legacy external selection does not identify one holder when
multiple external holders exist; modern reports preserve unit 254 versus 255.
Manufacturer code clears `snow == 0xffff` only when the reported extruder count
is one, so ambiguous multi-extruder `0xffff` remains unknown rather than applying
Bambu Bar's single-printer interpretation universally.

The CLI names unknown and unassigned separately. The desktop supplies a
selection ring, check mark and localized screen-reader label on the matching
slot, without a separate text summary. It never marks an explicitly absent
slot active.
The reported path does not by itself prove physical spool or filament presence.

## AMS drying

Heater-equipped AMS units (AMS 2 Pro, AMS HT) report `dry_time` (minutes
remaining), and only those units expose a `drying` block in detailed status.
The phase comes from bits 4–7 of the unit's `info` word (Off, Checking,
Drying, Cooling, Stopping, Error, HeatOutOfControl). A cycle is active while
time remains or the phase is Checking, Drying, or Cooling, because `dry_time`
reads 0 during the closing cooling phase. HeatOutOfControl is never treated as
an active cycle. H2-class firmware also reports the active `dry_setting` and
`dry_sf_reason` codes, which Melt surfaces as stable identifiers
(`already_drying`, `insufficient_power`, …).

A unit is `controllable` only when the printer advertises
`is_support_remote_dry` (`fun2` bit 5). P1-class firmware omits the bit and
acknowledges `ams_filament_drying` without acting on it, so Melt shows its
drying state but never sends it the command. Start (`mode` 1) and stop
(`mode` 0) pass the mutation authorization gate and complete only once the
targeted unit's reported phase confirms the transition. Start refuses
outright when the unit is already active, and otherwise refuses only on a
blocking reason other than the stale `already_drying` code an idle unit can
still report. Stop is a no-op only when the unit is fully `Off` with no time
remaining; every other state, including the `Error` and `HeatOutOfControl`
fault states, still sends the stop command. Start falls back to the first
loaded tray's filament type, then PLA, because the firmware rejects an empty
type.
Temperature bounds are 45–65 °C for units 0–3 and 45–85 °C for units 128–135;
duration is 1–24 h. Humidity-target mode, tray rotation, and scheduled drying
are not implemented.

## Port 6000 transport

Control replies correlate by typed operation and sequence, and no more than 64
unrelated replies may be skipped for one request. Control JSON, binary frames,
paths, listings, uploads, downloads, and `SUB_FILE` output have independent
bounds. Upload `frag_id` remains at the proven top level. The checked-in
executable transcript under `fixtures/bambu/tunnel-6000` is the regression
authority when community prose differs from executable protocol behavior.

Printer-generated 3MF thumbnails are requested read-only through bounded
`SUB_FILE` candidates, including the firmware-observed `#thumbnail` form.
Responses must be PNG, at most 16 MiB, no larger than 4096 pixels on either
axis, and no more than 16 million pixels before they enter memory/disk caches.
The cache key includes physical printer identity, remote path, size, and
modification marker; preview failure never affects download or printing.

Large uploads, downloads, and printer-thumbnail reads share a bounded
per-serial transfer gate across all in-process clients. Transfers to different
printers remain concurrent, and MQTT control—including emergency stop—does not
acquire the storage gate. FTPS upload completion closes the data socket without
waiting for a TLS `close_notify`, which can stall on affected firmware, then
requires the final 226/250 control reply before reporting success.

## Compatibility diagnostics

Diagnostics identify printers only as report-local `printer-N` labels. They
include raw/canonical model names, the channel that identified the model and
any channel that disagreed, module software and hardware versions,
capability field names with provenance and age, authorization resolution,
camera/storage selection, active and inactive quirks, TLS-pin presence, and
redacted camera-owner state. Module serials, unknown module values, observed
URLs, hosts, complete fingerprints, credentials, and raw capability objects are
never serialized. Unknown values are represented only by their JSON kind.

The desktop additionally includes external cameras as `camera-N` entries with
per-quality RTSP stream state, subscriber counts and reconnect attempts. Both
printer and external camera reports retain the latest frontend playback
result (`webrtc`, `webcodecs`, `mjpeg`, or no successful transport) and bounded,
redacted failure reasons after a viewer closes. Playback state is session-local;
WebCodecs success is recorded only after a decoded frame. The camera badge
shows WebRTC, H.264 (WebCodecs), or MJPEG rather than implying all live feeds
use the same transport.

An active camera owner reconnects the same negotiated transport while it still
has subscribers, using interruptible exponential backoff from 250 ms through a
4-second cap. TLS pin or certificate-identity failures terminate immediately.
RTSPS recovery also requires unchanged SPS/PPS parameter sets; otherwise
subscribers are disconnected so a new WebRTC negotiation can establish the new
codec description. Diagnostics report the reconnect-attempt count.

Melt treats a reported Bambu model as a capability hint, not a complete
contract. Runtime observations and safe protocol probes take precedence over
the model family. Unknown models and unknown fields remain distinct from an
explicitly unsupported feature, so new firmware can degrade conservatively
without being misclassified.

### Model detection

Every LAN channel reports the model differently — `info.get_version` modules
carry a marketing `product_name` on current firmware, a `model_id` in
`project_name` on legacy A1/P1 firmware, and neither on the oldest X1/X1C
firmware, where only the serial prefix separates an X1 from an X1 Carbon. The
model is therefore resolved from four channels in a fixed trust order:

1. `info.get_version` module `product_name`
2. `info.get_version` module `project_name`
3. The leading three characters of the configured serial
4. The model string stored in the profile

The serial outranks the profile because it is bound to the pinned TLS
certificate, which carries it as the subject common name: a wrong serial makes
the connection fail rather than misreport. A typed `--model` is the only
channel a stale or mistaken profile can affect, so it ranks last.

SSDP is deliberately excluded from the order. `DevModel.bambu.com` is an
unauthenticated broadcast any host on the segment can send, so it stays
advisory input to `printer discover` and to presence, and never decides
capabilities on an authenticated connection.

The highest-ranked channel that resolves a known model wins, except that the
configured string decides only when the printer itself named nothing. A model
the table has never seen still outranks a recognised one typed by hand:
routing an unreleased printer as a P1S is a misclassification, and it would
erase the only evidence a new table row could be added from. Where both
firmware names are present, an unrecognised `product_name` still yields to a
`project_name` the table knows, because both are the printer's own
authenticated report.

Lower-ranked channels that named a *different* model are recorded as conflicts
and reported by diagnostics and `printer capabilities`; they never change the
resolved model. A conflict names the channel only, never the value it read, so
a serial prefix is never serialized. A raw value that matches nothing is
preserved verbatim with an unknown canonical model, so unreleased firmware
stays inventoried instead of being silently discarded.

Detection is observational. It re-derives the model-keyed defaults — camera and
storage transport, storage volumes, extruder count, quirk matches — before live
observations are merged, so observation still wins over every default. It never
rewrites the configured profile, in the same way discovery never rewrites an
endpoint.

Desktop discovery also runs continuously on a bounded cadence. Presence is
keyed by the normalized physical serial, retained for 60 seconds, and enriches
configured profiles with last-seen model/firmware observations. A newly
advertised address is displayed as a suggestion only: discovery never rewrites
the configured endpoint, TLS identity, certificate pin, or credentials.
Fresh authenticated status can provide the same advisory address from
`wifi_ip` or the packed little-endian `net.info[].ip` field. Invalid
unspecified, loopback, multicast, and broadcast values are discarded. Status
observations use the same explicit suggestion UI and likewise never rewrite a
profile.

## Implemented compatibility behavior

- Inspect sliced 3MF archives before upload, with limits on source size, entry
  count, expanded size, metadata size, duplicate names, and unsafe paths.
- Discover printable plates and their G-code, previews, time, weight,
  filament requirements, and nozzle assignments.
- Keep source, project, plate, display, and remote storage names separate;
  sanitize and UTF-8 truncate only the remote name.
- Reconcile sliced filament requirements deterministically against live AMS
  trays and apply mappings only when every requirement has one unambiguous
  compatible tray.
- Derive nozzle mappings from package metadata and omit incomplete mappings.
- Cache observed printer capabilities and successful storage transports. Safe
  connection/protocol failures may fall back; authentication, TLS identity,
  and destination-collision failures never do.
- Preserve storage capacity, writable state, timestamps, media classification,
  and transfer byte counts. Desktop downloads are finalized atomically only
  after integrity checks.
- Preserve preparation progress, plate/queue metadata, raw job states, and raw
  printer error/image identifiers alongside normalized status.
- Preserve raw HMS and stage codes while suppressing informational events from
  the actionable printer-alert list.
- Serialize heavy storage transfers per physical printer without blocking
  MQTT control or transfers to other printers.
- Execute local prints as explicit inspect, capability, material, destination,
  storage, upload, verify, send, acceptance, and observed-start stages.

Bambu Studio's exact private routing rules are not treated as facts merely
because a model name resembles a known family. Add or change a route only with
sanitized protocol evidence plus physical-printer qualification.

Qualified quirks may use inclusive numeric minimum/maximum bounds, but every
bounded rule names the exact firmware module it constrains. Suffixes such as
`-beta` are retained for diagnostics while range matching compares numeric
components with missing trailing components treated as zero. Registry tests
reject malformed/reversed ranges, duplicate IDs, missing evidence ownership or
removal criteria, and overlapping active MJPEG/RTSPS routes. Provisional,
contradicted, and expired entries remain visible but never alter runtime behavior.

## Read-only firmware availability

Firmware checks reuse the authenticated LAN MQTT session. They combine the
installed module inventory with explicit printer-advertised targets, optional
read-only `upgrade.get_history`, and opt-in official public stable pages.
They never send an upgrade/install command, download a package, or log in to
the Bambu cloud. Public HTTPS is separate from the default LAN-only CLI check;
desktop history checks are automatic, and public checks require a saved or
one-shot opt-in. See [firmware evidence](firmware-updates.md) for source
provenance, cache behavior and assessment fields.
Unknown modules with a qualified advertised target remain visible as accessory
firmware rather than being discarded.

An advertised target can be reported as available. Silence cannot prove that a
printer is current, so missing or empty target data remains unknown until the
physical-evidence workflow qualifies a no-update state for an exact firmware
line. Forced-update flags are retained as a required-update signal, while
malformed or lower targets remain unknown and produce a bounded issue.
Public/history versions retain their own evidence rails and do not become
printer-advertised `availableVersion` values. A newer public release is not
proof of that printer's eligibility or rollout.

## Automated qualification

The Rust tests cover legacy and structured status shapes, unknown fields,
numeric/string variants, archive safety limits, multi-plate selection, naming,
AMS exact/ambiguous/unavailable matches, nozzle assignment, storage fallback
classification, and partial-transfer rejection. CLI contract tests cover the
public preflight command. Run the complete gate with:

```sh
make ci
```

## Physical-printer matrix

Model identity parsing currently recognizes A1, A1 mini, A2L, P1P, P1S, P2S,
X1, X1 Carbon, X1E, X2D, H2D, H2D Pro, H2S and H2C; this is an identity table,
not a physical qualification claim. H2C has two protocol model-code rows for
the same canonical model. All three checked-in runtime quirks (two disputed
P1S camera routes and P2S RTP timestamps) remain provisional and inactive.

Each supported firmware line needs at least one completed row. Record the
exact marketed model, firmware, hardware/AMS configuration, transport actually
observed, and evidence location; do not infer one row from another.
No completed physical Bambu row is checked in yet. The current fixture is an
executable-test port-6000 transcript, not a physical printer capture.

| Family | Variants to qualify | Storage/protocol focus | Material-system focus |
| --- | --- | --- | --- |
| A1/A2 | mini and full-size | SD/FTPS and observed alternatives | none and AMS Lite |
| P1/P2 | base and upgraded variants | SD/FTPS and observed alternatives | none and classic AMS |
| X1/X2 | base and premium variants | SD, eMMC, USB, and observed alternatives | classic and multi-AMS |
| H2 | single-nozzle, dual-nozzle, and toolchanger variants | eMMC/USB and tunnel behavior | external spool, HT/Pro, and multi-AMS |
| Unknown/new | first observed firmware/model pair | conservative discovery and feature gating | inventory only until mapped |

For every row, verify and retain evidence for:

1. Discovery identity, certificate pinning, authentication, and reconnect.
2. Full and delta status streams, including unknown fields and printer errors.
3. File listing, capacity/writable metadata, upload, overwrite collision,
   verified download, delete, and interrupted/partial transfer behavior.
4. Single- and multi-plate sliced 3MF inspection, remote naming, preview,
   filament/nozzle metadata, and the selected plate sent to the printer.
5. Empty, unavailable, duplicated, and mixed AMS tray inventories; ambiguous
   matches must require correction instead of silently choosing a tray.
6. Capability gating for bed leveling, flow calibration, timelapse, temperature
   bounds, nozzle count, and any model-specific command options.
7. Command acknowledgement and one observed transition into printing. Confirm
   retry/reconnect behavior cannot create a duplicate start.
8. Authentication failure, TLS mismatch, storage exhaustion, destination
   collision, printer rejection, cancellation, and network loss at each stage.

Attach completed matrix evidence to the frozen release records described in
[`bambu-physical-qualification.md`](bambu-physical-qualification.md) and
`docs/release-qualification.md`. Until a row has real-device evidence, its
behavior is implemented and automatically tested, but not physically
qualified for that model/firmware pair.
