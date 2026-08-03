# Bambu compatibility and qualification

Undocumented behavior and physical-printer captures follow the
[protocol evidence policy](bambu-protocol-evidence.md). Live observations and
safe probes outrank defaults; unresolved conflicts never become unconditional
model rules.

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
