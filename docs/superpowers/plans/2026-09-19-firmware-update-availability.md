# Firmware-Update Availability Implementation Plan

**Goal:** Add a read-only, local-first way to determine whether a configured
printer has an advertised firmware or printer-software update, expose the
result consistently through the core, CLI, desktop IPC, and UI, and preserve
Melt's conservative handling of undocumented Bambu protocol fields.

**Architecture:** Introduce one driver-neutral update report in `melt-core`.
Bambu derives positive availability from the authenticated LAN MQTT status
snapshot plus the existing `info.get_version` module inventory. Moonraker
reads the documented Update Manager status and reports only the `klipper`
entry, explicitly categorized as printer software rather than MCU firmware.
Update checks have their own slow cache and error state; they do not run in or
affect the five-second printer-status health loop.

**Primary references:**

- Bambu installed module inventory: `info.get_version` in
  <https://github.com/Doridian/OpenBambuAPI/blob/main/mqtt.md>
- Bambu advertised targets: `print.push_status.upgrade_state`, documented in
  <https://github.com/ClusterM/open-bamboo-networking/blob/master/research/12.01.06-fields-net-upgrade-errors.md>
- Confirmed Bambu Studio behavior using `info.module[]` plus
  `upgrade_state.new_ver_list`: <https://github.com/ClusterM/open-bamboo-networking>
- Optional authenticated Bambu cloud catalogue, deliberately excluded from
  this implementation: <https://github.com/Doridian/OpenBambuAPI/blob/main/cloud-http.md>
- Moonraker Update Manager status and refresh endpoints:
  <https://moonraker.readthedocs.io/en/latest/external_api/update_manager/>

## Scope and product decisions

### Included

- Bambu printer firmware (`ota`) and accessory/module firmware advertised by
  the printer over its already authenticated LAN MQTT connection.
- Moonraker's configured `klipper` updater, labeled **Klipper software**. This
  is useful update availability but is not proof that any attached MCU has a
  flashable firmware update.
- A manual refresh action and a cached passive check.
- Human and stable JSON CLI output.
- A desktop update badge/banner and a detail surface listing current and
  advertised versions.
- Sanitized diagnostics and protocol-evidence coverage.

### Explicitly excluded

- Installing, confirming, downgrading, rolling back, or scheduling updates.
- Publishing Bambu `upgrade_confirm`, `upgrade.start`, `get_history`, or
  user-supplied firmware URLs.
- Bambu account authentication, cloud tokens, release-note retrieval, package
  URLs, or downloads from Bambu's CDN.
- Moonraker, Mainsail/Fluidd, system-package, and arbitrary custom updater
  entries. A later generalized "system updates" feature can expose those
  without mislabeling them as printer firmware.
- Claiming that Moonraker can determine MCU-board firmware availability. It
  cannot do so from the documented Update Manager response.
- Treating an absent Bambu advertisement as proof that the printer is current.

## User-visible semantics

Use four states throughout the Rust and TypeScript boundaries:

| State | Meaning |
| --- | --- |
| `available` | At least one provider-advertised target is newer than its current component, or the provider explicitly requires an update. |
| `current` | The provider authoritatively returned current and remote versions and they match. Initially this is available for Moonraker only. |
| `unknown` | The check succeeded, but the provider did not give enough qualified evidence to say available or current. A Bambu printer with no advertised target is `unknown`, rendered as “No update advertised,” not “Up to date.” |
| `unsupported` | The driver or its optional service cannot provide update availability, such as Moonraker without a configured `klipper` updater. |

Rules:

- Overall state is `available` if any component is available.
- Otherwise it is `current` only if every in-scope component is authoritatively
  current and at least one component was observed.
- Otherwise it is `unknown`, unless the provider itself is unsupported.
- `force_upgrade` makes the report `available` and `required`, even when the
  target version is absent.
- Unknown fields, unknown module names, and unrecognized numeric Bambu states
  remain visible in diagnostics but never become unsupported/current guesses.
- Version strings are retained exactly for display. Comparisons use the
  existing numeric components in `FirmwareVersion`; suffixes and malformed
  versions cannot produce a false “newer” result.
- Availability is informational. It must not block printing or any other
  operation.

## Public data contract

### Driver-neutral core types

Add `crates/melt-core/src/firmware_updates.rs` and re-export it from
`crates/melt-core/src/lib.rs`:

```rust
pub enum FirmwareUpdateAvailability {
    Available,
    Current,
    Unknown,
    Unsupported,
}

pub enum FirmwareUpdateComponentKind {
    PrinterFirmware,
    AccessoryFirmware,
    PrinterSoftware,
}

pub enum FirmwareUpdateSource {
    BambuMqtt,
    MoonrakerUpdateManager,
}

pub struct FirmwareUpdateComponent {
    pub id: String,
    pub label: String,
    pub kind: FirmwareUpdateComponentKind,
    pub current_version: Option<String>,
    pub available_version: Option<String>,
    pub availability: FirmwareUpdateAvailability,
    pub required: bool,
}

pub struct FirmwareUpdateIssue {
    pub code: &'static str,
    pub message: &'static str,
}

pub struct FirmwareUpdateReport {
    pub availability: FirmwareUpdateAvailability,
    pub source: FirmwareUpdateSource,
    pub components: Vec<FirmwareUpdateComponent>,
    pub issues: Vec<FirmwareUpdateIssue>,
}
```

All serialized names use `camelCase`; enum values use `camelCase`. Components
sort deterministically by kind and then normalized ID. Provider payloads,
download URLs, release descriptions, commit bodies, serials, and opaque raw
objects are never included.

Bound all printer/provider-controlled public strings before storing them:
component IDs and versions to 64 bytes, labels to 128 bytes, issue count to 16,
and components to 64. Truncate on a UTF-8 boundary and add a stable
`providerDataTruncated` issue when a limit is reached. These limits supplement,
not replace, the transport-level JSON response limit.

The caller supplies freshness metadata because a one-shot CLI check and a
desktop cache have different lifetimes. Desktop IPC wraps the report with:

```text
profile, driver, report, checkedAt, stale
```

### Capability contract

Add `firmware_update_check: bool` to `drivers::Capabilities` and
`Operation::FirmwareUpdateCheck`. Both current drivers set the static
capability to true because each can attempt the operation. Runtime absence of
Moonraker's optional Update Manager is returned as `unsupported`, not as a
printer connectivity failure.

## Bambu parsing contract

Extend `crates/melt-core/src/bambu/firmware.rs` with a pure parser accepting a
full accumulated `print.push_status` document and a `FirmwareInventory`.

Evidence precedence:

1. Non-empty, evidence-qualified `upgrade_state.new_ver_list[]` entries and
   their `sw_new_ver` target.
2. Legacy named targets: `ota_new_version_number`,
   `ams_new_version_number`, `ahb_new_version_number`, and
   `ext_new_version_number`.
3. Top-level or nested `force_upgrade` as a required-update signal.
4. Opaque `new_version_state` is retained only as redacted diagnostic metadata
   until physical fixtures qualify its numeric mapping.

Important parser behavior:

- Parse only the accumulated snapshot. Delta reports already merge through
  `merge_status_delta`, so omitted fields retain their previous value and a
  present empty array explicitly replaces the previous array.
- Never scan arbitrary nested strings for version-looking values.
- Ignore blank targets and sentinel zero versions.
- Create an `unknown` component for every installed `info.module[]` entry even
  when it has no advertised target, then merge advertised-only components into
  that inventory. This lets users see the installed module set without
  interpreting silence as current.
- Prefer an exact module ID match from `info.module[]`; use documented aliases
  only where the evidence manifest names the alias.
- When a target is advertised but the current version is missing, report that
  component as available because availability came from the printer; leave
  `currentVersion` absent.
- When current and target parse numerically, require target > current. Equal
  versions are not available; a lower target is an `unknown` component with a
  `targetNotNewer` issue rather than being presented as an update.
- Preserve unknown module names as accessory firmware instead of dropping
  them, provided the entry shape is qualified and contains an advertised
  target.
- An empty/missing target list is `unknown`, not `current`.

Add `Client::firmware_update_status(access_code, fingerprint, refresh)` in
`crates/melt-core/src/bambu/transport.rs`:

- Reuse the pooled authenticated MQTT session.
- With `refresh == false`, use a valid accumulated snapshot when available;
  otherwise request one `pushing.pushall` snapshot.
- With `refresh == true`, cross the existing request freshness boundary and
  require a new full snapshot.
- Obtain current module versions through the existing cached
  `MqttConnection::query_version()` implementation.
- Do not send any `upgrade.*` command and do not contact the Bambu cloud.

## Moonraker parsing contract

Add `Client::firmware_update_status(access_code, refresh)` in
`crates/melt-core/src/moonraker.rs`.

- Passive check: `GET /machine/update/status` with no deprecated `refresh`
  query parameter.
- Manual refresh: `POST /machine/update/refresh` with `{"name":"klipper"}`;
  parse its returned status directly because the documented response matches
  the status response.
- Apply the existing response-size limit, redirect prohibition, authentication
  handling, timeout handling, and protocol tracing.
- Inspect only `result.version_info.klipper`.
- Return `unsupported` if the endpoint is missing or `klipper` is not a
  configured updater.
- Return `unknown` when the updater is invalid, corrupt, missing current or
  remote identity, or carries warnings that disable updates.
- Return `available` when a valid updater has differing current/remote hashes,
  a positive `commits_behind_count`, a non-empty `commits_behind`, or—when the
  hash/count fields are absent—different `version` and `remote_version`.
- Return `current` only when a valid response authoritatively shows matching
  hashes or matching current/remote versions with no behind count.
- Serialize only stable summary fields. Do not expose commit authors, messages,
  repository URLs, recovery URLs, or GitHub rate-limit details.
- Map a refresh rejection while printing or while another update is busy to a
  specific non-destructive error. The desktop retains the last successful
  result; the CLI reports the failure normally.

## Desktop caching and scheduling

Create a state object alongside `MonitorState` in
`crates/melt-desktop/src/main.rs`:

```text
FirmwareUpdateState
  pool: shared ConnectionPool
  entries: HashMap<normalized profile name, cached result>
  generation/connection identity
  next passive check
```

Behavior:

- Keep update failures independent from `MonitorEntry.error`; a failed update
  check must never mark a reachable printer offline.
- Cache successful reports for 30 minutes. Retain the last success after a
  later failure and mark it stale.
- Consider cached data stale after 24 hours; stale `available` data remains
  visible with its timestamp but cannot be worded as newly confirmed.
- Invalidate the entry on profile edit/removal, TLS refresh, credential
  revision, endpoint change, or connection-pool lifecycle change.
- Start a low-priority worker after the normal monitor initializes. It performs
  passive checks only, caps update-check concurrency at
  `monitor::DEFAULT_WORKERS`, prevents overlapping checks for the same profile,
  and repeats every 30 minutes. This bound is explicit because the existing
  status collector currently fans out one thread per configured printer.
- Never call Moonraker's refresh endpoint from a background worker. That API is
  CPU-intensive and can reject checks during a print.
- Bambu passive checks reuse the session/snapshot and cached version reply;
  they must not create a second permanent MQTT owner.
- Emit `firmware-updates-updated` only when the serialized entry changes.

Add Tauri commands:

- `printer_firmware_updates(name, refresh: bool)` — return a cached passive
  report or perform the requested refresh.
- `cached_firmware_updates()` — return all current cache entries for immediate
  UI hydration.

Register both commands in the existing `generate_handler!` list. Unit-test
cache freshness, failure retention, lifecycle invalidation, and event-change
suppression using extracted pure helpers rather than starting a Tauri runtime.

## CLI surface

Add:

```text
melt firmware check <printer> [--refresh] [connection flags]
```

Update `LEAF_COMMANDS`, `COMMAND_GROUPS`, dispatch, completions, and README CLI
documentation. `--refresh` means a fresh Bambu full snapshot or Moonraker's
explicit refresh endpoint; it never installs anything.

Human output example:

```text
Printer: workshop
Availability: update available
Printer firmware (ota): 01.08.00.00 -> 01.09.00.00
AMS firmware (n3f/0): 00.00.06.40 (no update advertised)
Required: no
```

JSON data uses the shared report plus `profile`, `driver`, `checkedAt`, and the
existing driver capabilities. Exit behavior:

- `0`: the provider check completed (`available`, `current`, or `unknown`).
- `5 / capability_unsupported`: the runtime provider is unavailable.
- Existing `3` and `4` authentication/transport/timeout mappings remain.
- An available update is not an error and does not change the exit code.

Sanitize every printer-provided label/version before human terminal output.
JSON preserves the bounded strings as data and never includes package URLs.

## Desktop UI

Create `ui/src/firmware-updates.ts` for pure presentation logic and
`ui/src/firmware-updates.test.ts` for deterministic tests. Keep protocol
interpretation in Rust.

UI behavior:

- Hydrate from `cached_firmware_updates`, then subscribe to
  `firmware-updates-updated`.
- Show a compact amber update badge in the configured-printer/fleet row only
  for `available`; add “Required” when applicable.
- On the selected printer's Control view, add a **Firmware & software** card:
  - Bambu rows are labeled printer/accessory firmware.
  - Moonraker's row is labeled “Klipper software,” with helper text stating
    that controller-board firmware is not checked.
  - Show current and available versions, last-checked time, stale state, and a
    “Check again” button.
  - `unknown` renders as “No update advertised” for Bambu and “Could not
    determine” otherwise. It never uses “Up to date.”
  - `unsupported` is neutral informational copy, not an error banner.
  - A refresh failure leaves the previous rows visible and adds an inline
    error with retry.
- Do not show an Install/Update button.
- Do not generate OS notifications in the first release; the persistent badge
  avoids duplicate/spam state. Notification policy can be added separately
  once version acknowledgement is persisted.
- Use `aria-live="polite"` for refreshed status, accessible button labels, and
  text in addition to color/icon differences.
- Add complete English and Brazilian Portuguese strings in `ui/src/i18n.ts`;
  extend the existing key-completeness tests.

## Diagnostics and privacy

Extend `BambuCompatibilityReport` in `crates/melt-core/src/diagnostics.rs`
with a sanitized update summary:

- overall availability and source;
- component ID/kind/current/available/required;
- the presence and JSON kind of unknown upgrade fields;
- whether a full or delta report supplied the latest observation;
- observation age.

Never serialize module serials, URLs, descriptions, raw `new_ver_list`
objects, cloud identifiers, or access credentials. Update
`human_capabilities` so it reports only the aggregate state and count. Keep the
existing report-local `printer-N` identifiers.

Moonraker diagnostics remain outside the Bambu compatibility report. The
driver-neutral CLI JSON report is sufficient for its initial support.

## Protocol evidence and compatibility policy

Before claiming Bambu `current`, add sanitized physical fixtures under
`fixtures/bambu/mqtt/firmware-updates/` for each qualified payload shape:

1. `info.get_version` plus a full status advertising an OTA update.
2. A legacy full status using named `*_new_version_number` fields.
3. A newer full status using `new_ver_list[]`/`sw_new_ver`.
4. Multiple accessory modules, including at least one AMS module.
5. A full status with no advertised update.
6. A delta that changes or clears the advertised target.
7. A forced update with and without a target version.

Each directory gets a `manifest.json` conforming to
`fixtures/bambu/manifest.schema.json`, pinned source revision, exact
model/module firmware scope, sanitization record, conflicts, and removal
condition. Until the no-update fixture qualifies an exact state mapping, the
implementation ships positive detection plus `unknown`/“No update advertised”
for absence.

Document this behavior and its evidence boundary in:

- `docs/bambu-protocol-evidence.md`
- `docs/bambu-compatibility.md`
- `docs/bambu-physical-qualification.md`
- `README.md`

## Implementation tasks

### Task 1: Add the driver-neutral model

**Files:**

- Add `crates/melt-core/src/firmware_updates.rs`
- Modify `crates/melt-core/src/lib.rs`
- Test in the new module

- [ ] Define the four-state availability enum, component kinds, sources,
      components, issues, and report.
- [ ] Implement deterministic sorting and overall-state aggregation.
- [ ] Test mixed available/current/unknown components, unsupported providers,
      malformed versions, suffixes, and stable JSON names.
- [ ] Verify `cargo test -p melt-core firmware_updates` and Clippy.

### Task 2: Add Bambu evidence fixtures and pure parsing

**Files:**

- Modify `crates/melt-core/src/bambu/firmware.rs`
- Add `fixtures/bambu/mqtt/firmware-updates/**`
- Modify fixture/evidence tests only if the new evidence class needs routing

- [ ] Add sanitized fixtures for every payload shape currently obtainable.
- [ ] Implement parsing for `new_ver_list` and legacy named targets without
      assigning semantics to unqualified numeric states.
- [ ] Enforce the component-count and UTF-8 string bounds at the parser edge.
- [ ] Match advertised targets to installed modules and preserve unknown
      module names conservatively.
- [ ] Test positive, forced, equal/lower, empty, unknown, multiple-module, and
      delta-clear cases.
- [ ] Verify `make bambu-evidence-check`.

### Task 3: Add the Bambu client operation

**Files:**

- Modify `crates/melt-core/src/bambu/transport.rs`
- Modify `crates/melt-core/src/bambu.rs` exports if needed

- [ ] Add `Client::firmware_update_status` using the existing pooled MQTT
      session, accumulated full status, and `query_version` cache.
- [ ] Make manual refresh require a post-request full report.
- [ ] Track internal update-observation time and full/delta provenance when the
      `upgrade_state` subtree is merged so diagnostics do not invent it later.
- [ ] Add an in-process TLS/MQTT test proving the request sequence is
      `pushall` plus `info.get_version` and contains no `upgrade.*` publish.
- [ ] Test session reuse and behavior after reconnect/version-cache reset.

### Task 4: Add Moonraker Update Manager support

**Files:**

- Modify `crates/melt-core/src/moonraker.rs`

- [ ] Add bounded response structs for only the status fields Melt consumes.
- [ ] Implement passive GET and explicit-refresh POST.
- [ ] Map absent endpoint/entry to unsupported and invalid/corrupt/incomplete
      entries to unknown.
- [ ] Test git hashes, versions, commit counts, exact-current state, missing
      manager, invalid updater, authentication, timeout, oversized response,
      and refresh rejection with local mock HTTP servers.
- [ ] Assert protocol traces contain method/endpoint/outcome but no response
      body or repository details.

### Task 5: Wire drivers and the connection pool

**Files:**

- Modify `crates/melt-core/src/drivers.rs`
- Modify `crates/melt-core/src/pool.rs`
- Update exhaustive capability fixtures in CLI/desktop tests

- [ ] Add `Operation::FirmwareUpdateCheck` and
      `Capabilities::firmware_update_check`.
- [ ] Add one driver-dispatch function returning the shared report.
- [ ] Add pooled execution that uses the existing per-printer client/session.
- [ ] Test static capabilities, driver dispatch, pooled reuse, lifecycle
      invalidation, and credential-free connection identity.
- [ ] Update the `printer-drivers` CLI golden because the serialized capability
      object gains the additive `firmwareUpdateCheck` field.

### Task 6: Add the CLI command and stable contract

**Files:**

- Modify `crates/melt-cli/src/lib.rs`
- Add `fixtures/cli-contract/golden/firmware-check-profile-not-found.json`
- Modify `fixtures/cli-contract/manifest.json`
- Modify `README.md`

- [ ] Add command metadata, group help, dispatch, option parsing, and human
      rendering for `firmware check`.
- [ ] Add JSON serialization and the defined exit-code mapping.
- [ ] Test terminal sanitization for component IDs, labels, and versions.
- [ ] Add unit tests for human output across every availability state.
- [ ] Add a no-network profile-not-found contract fixture. Do not add the new
      command to the legacy Go migration inventory unless that inventory's
      schema is first extended to distinguish Melt-native commands.
- [ ] Verify `make contract`.

### Task 7: Add desktop cache, command, and event

**Files:**

- Modify `crates/melt-desktop/src/main.rs`

- [ ] Add `FirmwareUpdateState`, cache-entry types, invalidation, and the
      passive scheduler.
- [ ] Add `printer_firmware_updates` and `cached_firmware_updates` commands.
- [ ] Emit change-only update events and retain stale success after failures.
- [ ] Ensure update errors never alter `MonitorEntry.connection_state`.
- [ ] Test TTL, stale transition, last-good retention, profile-generation
      invalidation, and bounded concurrent checks.

### Task 8: Add UI presentation and accessibility

**Files:**

- Add `ui/src/firmware-updates.ts`
- Add `ui/src/firmware-updates.test.ts`
- Modify `ui/src/App.vue`
- Modify `ui/src/i18n.ts`

- [ ] Add typed IPC/event models and pure summary/ordering helpers.
- [ ] Add fleet badge, selected-printer card, stale timestamp, and refresh
      action without any install action.
- [ ] Label Moonraker's result as Klipper software and explain the MCU limit.
- [ ] Preserve the previous result when a refresh fails.
- [ ] Add English/Portuguese copy and accessibility semantics.
- [ ] Test state labels, required-update priority, stale rendering decisions,
      module sorting, driver-specific copy, and i18n completeness.

### Task 9: Extend diagnostics and documentation

**Files:**

- Modify `crates/melt-core/src/diagnostics.rs`
- Modify `crates/melt-cli/src/lib.rs`
- Modify `ui/src/App.vue` diagnostic types if the summary is rendered there
- Modify compatibility/evidence/qualification docs and `README.md`

- [ ] Add only the sanitized update summary to Bambu compatibility reports.
- [ ] Prove through tests that URLs, serials, descriptions, and raw payloads
      are absent.
- [ ] Document availability semantics, manual refresh cost, supported scope,
      privacy boundary, and the no-install limitation.
- [ ] Update the README feature list and current capability boundary.

### Task 10: Verification and physical qualification

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [ ] Run `cargo test --workspace --locked`.
- [ ] Run `bun test --cwd ui` and `bun run --cwd ui build`.
- [ ] Run `make bambu-evidence-check` and `make contract`.
- [ ] Run `make ci`.
- [ ] On a Bambu printer with an advertised update, confirm module/current/
      target mapping and capture sanitized evidence under the physical
      qualification procedure.
- [ ] On a Bambu printer with no advertisement, confirm the UI says “No update
      advertised,” not “Up to date.”
- [ ] On Moonraker with `[update_manager]`, verify current and behind states;
      repeat without the component and verify neutral unsupported behavior.
- [ ] Verify manual Moonraker refresh failure during a print retains the last
      result and never affects printer connectivity status.
- [ ] Verify no network request goes to Bambu cloud and no command capable of
      starting an update appears in protocol traces.

## Acceptance criteria

- A Bambu-advertised newer OTA/module version produces `available` in core,
  CLI JSON/human output, desktop IPC, and UI without any Bambu cloud login.
- Absence of a Bambu target never produces a false “Up to date” claim.
- Moonraker reports the configured Klipper updater accurately and never labels
  it as attached-controller firmware.
- Manual refresh is explicit; background checks never invoke Moonraker's
  expensive refresh endpoint.
- Update-check failure cannot mark a printer offline or hide the last
  successful result.
- No update-installation command, firmware URL, token, serial, or raw provider
  payload enters public JSON, diagnostics, cache files, or logs.
- Existing status/CLI JSON remains backward compatible because all new fields
  are additive and omitted when absent.
- All automated checks pass, and Bambu claims remain bounded by checked-in
  evidence and physical qualification.
