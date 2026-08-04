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
their original RTP packets for WebRTC and may produce a bounded JPEG preview
from the same access unit; classic camera sources expose complete JPEG frames.
This boundary permits snapshots and multiple viewers to share one upstream
without transcoding the WebRTC path.

The shared camera manager is keyed by the normalized printer serial and a
credential-free connection revision. Subscriber queues hold at most two media
units and drop new frames under backpressure. The most recent decoded JPEG is
retained for snapshots. The first subscriber opens the upstream; dropping the
last subscriber interrupts its socket, and connection changes invalidate the
previous generation.

Desktop MJPEG previews, WebRTC offers, and snapshots all subscribe through
this manager. WebRTC subscriptions receive only H.264 access units, while
preview subscriptions receive only JPEG frames, so one slow consumer cannot
fill another media type's bounded queue.

## Authorization policy

Polimero resolves LAN authorization only from explicit live security fields or
future evidence-qualified rules. A proven `signingRequired` mode blocks file
writes, print start/control, motion, thermal, fan, light, speed, and emergency
commands before any mutating transport work. Read-only status and diagnostics
remain available. Unknown authorization retains attempt-and-observe behavior.

The community-reported meaning of `print.fun` bit 29 is retained as an
inactive, provisional observation. It is visible for qualification but cannot
block a command until independent physical evidence scopes it to the affected
model and module firmware. Conflicting explicit fields block mutations
conservatively and are surfaced rather than guessed.

## Camera transport selection

Fresh, host-validated `print.ipcam` advertisements outrank qualified quirks,
exact-model defaults, and family hints. Advertised URLs may target only the
configured printer and may not embed credentials. Stale or rejected
advertisements fall through to a non-mutating probe. TLS pin and certificate
identity failures are terminal and never cause a fallback.

P1S deliberately has no unconditional camera default because current community
implementations disagree between RTSPS and the classic TLS-MJPEG transport.
Polimero probes RTSPS and then classic MJPEG within the caller's safety
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

## Compatibility diagnostics

Diagnostics identify printers only as report-local `printer-N` labels. They
include raw/canonical model names, module software and hardware versions,
capability field names with provenance and age, authorization resolution,
camera/storage selection, active and inactive quirks, TLS-pin presence, and
redacted camera-owner state. Module serials, unknown module values, observed
URLs, hosts, complete fingerprints, credentials, and raw capability objects are
never serialized. Unknown values are represented only by their JSON kind.

Polimero treats a reported Bambu model as a capability hint, not a complete
contract. Runtime observations and safe protocol probes take precedence over
the model family. Unknown models and unknown fields remain distinct from an
explicitly unsupported feature, so new firmware can degrade conservatively
without being misclassified.

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
- Execute local prints as explicit inspect, capability, material, destination,
  storage, upload, verify, send, acceptance, and observed-start stages.

Bambu Studio's exact private routing rules are not treated as facts merely
because a model name resembles a known family. Add or change a route only with
sanitized protocol evidence plus physical-printer qualification.

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
`docs/release-qualification.md`. Until a row has real-device evidence, its
behavior is implemented and automatically tested, but not physically
qualified for that model/firmware pair.
