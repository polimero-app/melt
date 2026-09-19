# Bambu compatibility and qualification

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

The shared camera manager is keyed by the normalized printer serial and a
credential-free connection revision. Each subscriber mailbox retains only its
newest complete media unit under backpressure. The most recent decoded JPEG is
also retained for snapshots. The first subscriber opens the upstream; dropping
the last subscriber interrupts its socket, and connection changes invalidate
the previous generation.

Desktop MJPEG previews, WebRTC offers, WebCodecs streams, and snapshots all
subscribe through this manager. Native-video subscriptions receive only H.264
access units, while preview subscriptions receive only JPEG frames, so one slow
consumer cannot fill another media type's bounded queue. The WebCodecs bridge
uses an authenticated loopback binary stream rather than base64 IPC messages.
The desktop releases its camera subscription whenever the Control view or the
application window is hidden and reconnects when that view becomes visible.

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
include raw/canonical model names, module software and hardware versions,
capability field names with provenance and age, authorization resolution,
camera/storage selection, active and inactive quirks, TLS-pin presence, and
redacted camera-owner state. Module serials, unknown module values, observed
URLs, hosts, complete fingerprints, credentials, and raw capability objects are
never serialized. Unknown values are represented only by their JSON kind.

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
installed module inventory with explicit printer-advertised targets and never
send an `upgrade` command, download a package, or contact the Bambu cloud.
Unknown modules with a qualified advertised target remain visible as accessory
firmware rather than being discarded.

An advertised target can be reported as available. Silence cannot prove that a
printer is current, so missing or empty target data remains unknown until the
physical-evidence workflow qualifies a no-update state for an exact firmware
line. Forced-update flags are retained as a required-update signal, while
malformed or lower targets remain unknown and produce a bounded issue.

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

Each supported firmware line needs at least one completed row. Record the
exact marketed model, firmware, hardware/AMS configuration, transport actually
observed, and evidence location; do not infer one row from another.

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
