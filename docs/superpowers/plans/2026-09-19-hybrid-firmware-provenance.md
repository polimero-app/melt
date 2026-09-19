# Hybrid Firmware Catalogue and Provenance Implementation Plan

**Goal:** Extend Melt's read-only firmware checks so a Bambu printer that is
not signed in to Bambu Cloud can still be compared with Bambu's public stable
firmware catalogue, while also attempting the printer-local
`upgrade.get_history` query and preserving exactly where every version claim
came from.

**Architecture:** Keep `info.get_version` and `print.upgrade_state` as the
authoritative LAN inventory and printer-advertisement sources. Add a bounded,
read-only `upgrade.get_history` query as optional device-catalogue evidence and
an opt-in HTTPS client for Bambu's public model pages as public-release
evidence. Merge observations without overwriting them, derive a separate
assessment for confirmed, device-catalogue, and public-only updates, and expose
per-field provenance through the core, CLI, desktop IPC, and dedicated
firmware-management screen.

**Primary references:**

- Installed Bambu module inventory, `info.get_version`:
  <https://github.com/Doridian/OpenBambuAPI/blob/main/mqtt.md>
- Printer status advertisements, `print.upgrade_state`:
  <https://github.com/ClusterM/open-bamboo-networking/blob/master/research/12.01-status.md>
- Read-only firmware history, `upgrade.get_history`:
  <https://github.com/Doridian/OpenBambuAPI/blob/main/mqtt.md>
- Bambu public firmware pages, for example the P1 family:
  <https://bambulab.com/en/support/firmware-download/p1>
- Independent public-page implementation and its documented Cloudflare
  failure modes:
  <https://github.com/maziggy/bambuddy/blob/main/backend/app/services/firmware_check.py>
- Existing Melt evidence boundary:
  `docs/bambu-protocol-evidence.md` and `docs/bambu-compatibility.md`

## Product and safety decisions

### Included

- Existing installed-version inventory from `info.get_version`.
- Existing explicit update targets from `print.upgrade_state`.
- A read-only `upgrade.get_history` request over the already authenticated LAN
  MQTT session.
- A public-catalogue check against an allowlisted official Bambu HTTPS page for
  a physically identified, supported printer model.
- Main-printer `ota` comparison against the latest public stable release.
- Device-catalogue accessory evidence only when a qualified response can be
  matched safely to an installed component.
- Independent source status, timestamps, cache freshness, errors, and UI copy.
- A persistent user preference controlling public-catalogue network access.
- CLI flags that make public-catalogue access explicit.

### Explicitly excluded

- Bambu account login, access tokens, cloud MQTT, or the authenticated
  `/user/device/version` endpoint.
- Downloading firmware, following firmware package URLs, or publishing any
  update, confirmation, downgrade, consistency, or start command.
- Browser/TLS impersonation or attempts to bypass Cloudflare challenges.
- Treating a public release as proof that a staged, regional, beta, or
  device-specific rollout is available to this printer.
- Copying a public `ota` version onto internal modules such as `mc`, `th`,
  `esp32`, or unqualified AMS controller identifiers.
- Scraping community-maintained version lists as authoritative data.
- A Melt-operated firmware catalogue service in the first implementation.

## User-visible semantics

Melt must distinguish these claims:

| Claim | Evidence | UI wording | Alert priority |
| --- | --- | --- | --- |
| Required update | Qualified printer flag | **Required update** | Highest |
| Confirmed available | Newer `upgrade_state` target | **Update offered by printer** | High |
| Device catalogue newer | Newer qualified `get_history` target | **Newer compatible version in printer catalogue** | Medium |
| Public stable newer | Newer official public-page target | **Newer public release** | Medium, visually distinct |
| Current | Authoritative comparable source equals installed | **Current for this source** | Neutral |
| Unknown | No comparable target | **Availability unknown** | Neutral |
| Conflict | Sources disagree or a target is older/uncomparable | **Sources disagree** | Warning, never actionable |

Rules:

- `availableVersion` retains its existing contract: it is populated only by a
  printer-confirmed advertisement or required-update signal. Public-page data
  never silently fills it.
- A public release newer than installed does not make legacy `availability`
  equal `available`; it produces a new public-release assessment.
- `upgrade.get_history` is stronger than a generic public page because the
  response came through the printer, but it is still not equivalent to an
  explicit `upgrade_state` advertisement until physical evidence qualifies
  that interpretation.
- Never choose the numerically greatest value across sources and discard the
  rest. Preserve all observations and show disagreements.
- `required` can only come from printer-specific evidence, never from a public
  page.
- Version strings remain exact for display. Numeric comparison keeps the
  existing conservative Bambu comparison rules; suffixes or malformed values
  cannot produce a false-newer result.
- Availability remains informational and must not block printing or other
  controls.

## Additive data-contract design

The existing stable fields remain serialized for CLI and desktop compatibility:

```text
FirmwareUpdateReport.source
FirmwareUpdateReport.availability
FirmwareUpdateComponent.currentVersion
FirmwareUpdateComponent.availableVersion
FirmwareUpdateComponent.required
```

Add driver-neutral evidence types in
`crates/melt-core/src/firmware_updates.rs`:

```rust
pub enum FirmwareEvidenceSource {
    BambuLanInventory,
    BambuLanAdvertisement,
    BambuLanHistory,
    BambuPublicCatalogue,
    MoonrakerUpdateManager,
}

pub enum FirmwareEvidenceRole {
    Installed,
    PrinterAdvertised,
    DeviceCatalogue,
    PublicStable,
    UpstreamCurrent,
}

pub struct FirmwareVersionEvidence {
    pub version: String,
    pub source: FirmwareEvidenceSource,
    pub role: FirmwareEvidenceRole,
    pub required: bool,
}

pub enum FirmwareUpdateAssessment {
    Required,
    ConfirmedAvailable,
    DeviceCatalogueNewer,
    PublicReleaseNewer,
    Current,
    Conflict,
    Unknown,
    Unsupported,
}
```

Extend each component with a bounded, deterministically sorted `evidence`
collection and an `assessment`. Extend the report with `sources` and an overall
`assessment`. Keep the legacy report `source` as the primary driver/provider
for compatibility; it is no longer presented as complete provenance.

Construct components through helpers rather than independent struct literals.
The helper derives legacy fields from evidence:

- `currentVersion` from `Installed` evidence;
- `availableVersion` only from `PrinterAdvertised` evidence;
- `required` only from qualified printer evidence;
- legacy `availability=available` only for `Required` or
  `ConfirmedAvailable`;
- legacy `availability=current` only where the existing provider can make that
  authoritative claim;
- otherwise preserve `unknown`/`unsupported`.

The desktop wrapper adds per-source freshness because core parsers should not
own a clock:

```text
FirmwareSourceCheck
  source
  checkedAt
  outcome: success | empty | failed | disabled | unsupported
  stale
  error?: stable bounded CommandError
```

Source check metadata is additive to `FirmwareUpdateEntry`. The existing
entry-level `checkedAt`, `stale`, and `error` remain as compatibility summaries
derived from the source checks.

Apply the existing bounds to evidence strings and add explicit limits:

- at most 4 version observations per component;
- at most 8 source checks per report;
- existing 64-byte component ID/version and 128-byte label bounds;
- deterministic deduplication by source, role, and exact version;
- no URLs, release descriptions, serials, device IDs, raw JSON, or page HTML
  in public reports, diagnostics, events, or disk caches.

## Hybrid collection and merge flow

For a Bambu check:

1. Query `info.get_version` and create `Installed` evidence for every bounded
   module.
2. Refresh or reuse the accumulated full status and parse
   `print.upgrade_state` into `PrinterAdvertised` evidence.
3. If device-catalogue checks are enabled for this invocation, send one
   read-only `upgrade.get_history` request and parse its response into
   `DeviceCatalogue` evidence. Empty, unsupported, unauthenticated, and timeout
   outcomes are recorded independently and do not fail the LAN inventory.
4. Resolve the configured Bambu model to a qualified public-page slug. If
   public checks are enabled and the model is known, query the shared public
   catalogue cache and create `PublicStable` evidence for `ota` only.
5. Merge by normalized component identity without overwriting observations.
6. Derive component and overall assessments with priority:
   `Required` > `ConfirmedAvailable` > `Conflict` >
   `DeviceCatalogueNewer` > `PublicReleaseNewer` > `Current` > `Unknown`.
7. Return partial success when one source fails. Only failure to obtain the
   installed LAN inventory remains a command-level Bambu check failure.

Moonraker keeps its current behavior but is migrated to the evidence model:
current and remote Klipper versions become `Installed` and `UpstreamCurrent`
evidence from `MoonrakerUpdateManager`.

## Bambu `upgrade.get_history`

### Transport

Add a dedicated query method to `MqttConnection` in
`crates/melt-core/src/bambu/transport.rs` rather than routing the request
through a mutating-command helper:

```json
{"upgrade":{"sequence_id":"…","command":"get_history"}}
```

Requirements:

- Correlate the response by command and sequence ID.
- Reuse the pooled authenticated MQTT connection and existing packet-size and
  deadline protections.
- Do not publish `upgrade_confirm`, `consistency_confirm`, `start`, or any URL.
- Continue accumulating unrelated status reports while waiting.
- Treat a missing/failed/empty response as a source-local outcome, not a
  printer connectivity failure.
- Do not retry more aggressively than existing bounded MQTT queries.

### Pure parser

Add a parser in `crates/melt-core/src/bambu/firmware.rs` for only:

```text
upgrade.command
upgrade.result
upgrade.reason (mapped to bounded stable issue codes, not exposed verbatim)
upgrade.firmware_optional[].firmware.version/force_update
upgrade.firmware_optional[].ams[].address/dev_model_name
upgrade.firmware_optional[].ams[].firmware[].version/force_update
```

Ignore URLs and descriptions at deserialization time. Do not retain them and
do not scan arbitrary nested strings for versions.

Selection rules:

- Accept only successful, correctly correlated responses.
- Preserve multiple catalogue observations when their roles differ; never use
  a blind numeric maximum across unrelated models/channels.
- Match printer firmware to `ota` only.
- Match AMS targets only through physically qualified aliases/model/address
  rules. Until qualified, show an unmatched bounded accessory catalogue entry
  instead of attaching it to `n3s/l28`, `esp32`, or another installed module.
- A catalogue entry equal to installed is evidence of equality for that
  source, not proof that every accessory is current.
- Empty history on a LAN-only printer is expected and neutral.

## Public Bambu catalogue provider

Create a provider isolated from the LAN driver, for example
`crates/melt-core/src/bambu/public_catalogue.rs`, with a pure parser and a
small blocking HTTPS client. The CLI, connection pool, and desktop scheduler
reuse this provider instead of duplicating HTTP logic.

### Model resolution

- Resolve through the existing `ModelIdentity`/`CanonicalModel` contract and
  the persisted profile model.
- Maintain an explicit allowlist from canonical models/families to verified
  official page slugs.
- Record the official reference URL and last qualification date for every map
  entry in the compatibility documentation.
- Do not query for `Unknown`, infer a model from the serial, or fall back to a
  family page unless that family mapping is explicitly qualified.
- Public pages initially produce evidence for `ota` only.

### HTTP policy

- HTTPS only; allowlist Bambu-owned catalogue hosts.
- No cookies, authorization headers, account tokens, printer serials, device
  IDs, LAN addresses, or access codes.
- Honest `User-Agent` identifying Melt; no browser impersonation.
- Ten-second timeout, bounded redirects restricted to allowlisted HTTPS hosts,
  maximum 2 MiB response, and bounded decompression.
- Fetch metadata pages only. Reject firmware/CDN package redirects and never
  download a `.sig`, archive, or binary.
- Treat HTTP 403/429, Cloudflare challenge pages, layout changes, and parsing
  failures as a source-local error with cached last-good retention.
- Cache only extracted model/version/source/timestamp data, never raw HTML.

### Parser and evidence

- Prefer structured embedded page/Next.js JSON over text scraping.
- Use semantic HTML parsing only as a bounded fallback; do not use a regex over
  arbitrary scripts.
- Select the newest stable release explicitly marked for the qualified model
  page. Exclude beta/public-beta entries unless a later separately scoped
  feature adds an opt-in beta channel.
- Keep minimal sanitized fixture fragments for parser tests, including current
  layout, prior releases, missing version, model mismatch, localized page, and
  Cloudflare challenge response.
- If the official layout cannot be parsed without impersonation, return
  unavailable. Do not weaken transport policy to keep the feature green.

## Preferences and privacy boundary

Preserve LAN-only behavior by default. Add a desktop preference such as
`bambuPublicFirmwareChecks`, default `false`, in
`crates/melt-core/src/preferences.rs` and the Settings UI.

Behavior:

- The firmware screen explains that enabling the option contacts an official
  Bambu public page with the computer's IP address and requested model-family
  path, but sends no account or printer identifier.
- A one-time explicit **Check public catalogue** action may perform the check
  without enabling background checks; confirmation is part of the action
  itself, not an install/update confirmation.
- `upgrade.get_history` is separately described as a LAN request that may
  cause a cloud-connected printer to consult Bambu services. It remains safe
  and empty on printers without cloud access.
- Preference writes use the existing optimistic-concurrency and permissions
  behavior. The additive field uses `serde(default)` and does not require a
  preferences schema bump.
- Update README language that currently guarantees firmware checks never
  contact Bambu services. Preserve the stronger guarantee when the preference
  is disabled.

## Cache and scheduling design

Refactor `FirmwareUpdateState` in `crates/melt-desktop/src/main.rs` so source
freshness is independent:

- LAN inventory/advertisement: keep the existing 30-minute TTL and 24-hour
  stale threshold.
- `upgrade.get_history`: cache per profile/connection generation for six hours;
  run on manual refresh and at most once per TTL when enabled.
- Public catalogue: cache per canonical model, not per printer, for 24 hours;
  deduplicate concurrent requests across profiles of the same model.
- Retain the last successful observation for each source after a later source
  failure and mark only that source stale.
- Profile edit/removal, credential revision, TLS refresh, model change, and
  pool lifecycle changes invalidate LAN/device-history evidence. A model
  change invalidates the public merge for that profile but does not discard a
  still-valid shared cache entry for the old model.
- Public failures never mark a printer offline and never remove installed
  versions.
- Emit `firmware-updates-updated` only when the merged serialized entry changes.
- Keep public work outside the five-second monitor loop and use existing
  bounded worker/concurrency patterns.

## CLI contract

Keep `melt firmware check <printer> --refresh` LAN-only for backward
compatibility. Add explicit options:

```text
--include-history
--include-public-catalogue
```

`--include-public-catalogue` is the CLI user's opt-in for that invocation; it
does not read or change the desktop preference. Human output lists observations
by role and source. JSON adds evidence, source checks, and assessment while
retaining existing fields and exit codes.

An available/public/catalogue finding remains exit code `0`. Transport errors
for optional sources remain data when the installed inventory succeeded.
Unsupported public model resolution is not a command failure.

## Desktop UI

Use the dedicated firmware screen under **Manage printers**.

- Keep the installed-to-advertised rail for printer-confirmed targets.
- Add source-labelled rows/columns only when evidence exists:
  **Offered by printer**, **Printer catalogue**, and **Public stable**.
- For a public-only newer version, show:
  “Newer public release found; availability for this printer is unconfirmed.”
- Show source-specific last-checked/stale/error state rather than one ambiguous
  timestamp.
- Add distinct fleet badges for **Update offered** and **New public release**;
  required/confirmed always outrank public-only evidence.
- Show conflicts instead of choosing one source silently.
- Leave internal module public targets empty unless explicit component evidence
  exists.
- Keep **Check again** for LAN evidence and add an explicit public-catalogue
  action when the preference is disabled.
- Never add Install, Download, Upgrade, Downgrade, or package-link controls.
- Add complete English and Brazilian Portuguese copy, text alternatives, and
  polite live-region announcements.

## Diagnostics and logging

- Extend sanitized firmware diagnostics with source/role/version/assessment and
  per-source outcome/age.
- Never include raw history/page payloads, URLs, descriptions, device IDs,
  serials, access codes, cookies, headers, or HTML.
- Protocol traces may record the MQTT command name and bounded outcome, but not
  response URLs/descriptions.
- HTTP diagnostics record allowlisted host class, status class, parser outcome,
  byte count, and cache use; never log query data or page bodies.
- Bound error detail and map provider text to stable error codes.

## Implementation tasks and atomic commits

### Task 1: Add additive evidence and assessment types

**Files:**

- `crates/melt-core/src/firmware_updates.rs`
- `crates/melt-core/src/bambu/firmware.rs`
- `crates/melt-core/src/moonraker.rs`
- `crates/melt-cli/src/lib.rs` tests/fixtures that construct reports
- `ui/src/firmware-updates.ts`

- [ ] Add evidence source/role, assessment, source-summary, and bounded
      deterministic merge types.
- [ ] Refactor component construction through derivation helpers.
- [ ] Preserve every legacy serialized field and semantic.
- [ ] Migrate Bambu and Moonraker results to evidence without behavior changes.
- [ ] Add table-driven tests for deduplication, priority, conflict, bounds,
      suffixes, legacy field derivation, and stable camelCase JSON.
- [ ] Commit: `refactor(core): add firmware evidence provenance`.

### Task 2: Add pure `upgrade.get_history` parsing and evidence fixtures

**Files:**

- `crates/melt-core/src/bambu/firmware.rs`
- `fixtures/bambu/mqtt/firmware-updates/**`
- `docs/bambu-protocol-evidence.md`
- fixture manifest/schema only if a new evidence classification is required

- [ ] Add sanitized physical fixtures for success, empty history, printer OTA,
      AMS entries, multiple options, failure, malformed versions, oversized
      collections, and unknown future fields.
- [ ] Parse only the allowlisted response fields and discard URLs/descriptions.
- [ ] Preserve unmatched accessory evidence without unsafe module aliasing.
- [ ] Test exact model/address matching and conservative fallback.
- [ ] Commit: `feat(bambu): parse read-only firmware history`.

### Task 3: Add the read-only MQTT history query

**Files:**

- `crates/melt-core/src/bambu/transport.rs`
- `crates/melt-core/src/bambu.rs` exports if required

- [ ] Add a correlated, bounded `query_firmware_history` method on the pooled
      session.
- [ ] Add mock TLS/MQTT tests proving the only upgrade command is
      `get_history` and that unrelated status packets are accumulated.
- [ ] Test success, empty, rejection, timeout, reconnect, packet limit, and
      response-correlation behavior.
- [ ] Ensure optional failure does not poison the shared connection or status.
- [ ] Commit: `feat(bambu): query firmware history over LAN`.

### Task 4: Add the official public-catalogue provider

**Files:**

- Add `crates/melt-core/src/bambu/public_catalogue.rs`
- Modify `crates/melt-core/src/bambu.rs`
- Modify workspace/core dependency manifests only if an HTML parser is needed
- Add `fixtures/bambu/public-firmware/**`
- Modify `docs/bambu-compatibility.md`

- [ ] Add the qualified canonical-model-to-official-page allowlist.
- [ ] Implement the restricted HTTPS client and pure structured-data parser.
- [ ] Add minimal sanitized page/challenge/layout-change fixtures.
- [ ] Test allowlisted redirects, HTTP failures, size/time limits, stable-only
      selection, unknown models, and absence of identifiers/credentials.
- [ ] Prove tests never access the live network.
- [ ] Commit: `feat(bambu): read public firmware catalogue`.

### Task 5: Merge hybrid evidence in drivers and the pool

**Files:**

- `crates/melt-core/src/drivers.rs`
- `crates/melt-core/src/pool.rs`
- `crates/melt-core/src/bambu/transport.rs`
- `crates/melt-core/src/firmware_updates.rs`

- [ ] Add check options controlling history/public sources.
- [ ] Collect partial results and derive assessments with the documented
      precedence.
- [ ] Share public catalogue results by canonical model.
- [ ] Test confirmed/public/history disagreement, lower public target, missing
      installed OTA, public-only OTA, accessory isolation, and partial failure.
- [ ] Commit: `feat(core): merge hybrid firmware evidence`.

### Task 6: Extend CLI without breaking its stable JSON

**Files:**

- `crates/melt-cli/src/lib.rs`
- `fixtures/cli-contract/**`
- `README.md`

- [ ] Add explicit history/public flags, help, completion metadata, and human
      source labels.
- [ ] Add evidence/source-check/assessment fields additively to JSON.
- [ ] Test optional-source failures remain exit `0` with installed evidence.
- [ ] Test terminal sanitization and absence of URLs/descriptions/raw payloads.
- [ ] Commit: `feat(cli): expose firmware evidence sources`.

### Task 7: Add preference and source-aware desktop caching

**Files:**

- `crates/melt-core/src/preferences.rs`
- `crates/melt-desktop/src/main.rs`

- [ ] Add the default-off public-catalogue preference and update command.
- [ ] Split cache/freshness/error state by source and add shared model cache.
- [ ] Keep entry-level compatibility fields derived from source checks.
- [ ] Add manual one-shot public check support without changing the preference.
- [ ] Test preference migration, concurrent writes, per-source TTL, stale
      retention, request deduplication, lifecycle/model invalidation, and event
      suppression.
- [ ] Commit: `feat(desktop): cache hybrid firmware checks`.

### Task 8: Present provenance on the dedicated firmware screen

**Files:**

- `ui/src/firmware-updates.ts`
- `ui/src/firmware-updates.test.ts`
- `ui/src/App.vue`
- `ui/src/i18n.ts`

- [ ] Add typed evidence/source-check/assessment contracts and pure selectors.
- [ ] Render confirmed, device-catalogue, public, conflict, disabled, stale,
      and partial-failure states with distinct copy.
- [ ] Add the Settings preference and one-shot public check disclosure.
- [ ] Keep fleet priority required > confirmed > device catalogue > public.
- [ ] Preserve the 68 px shared card-header rule and responsive version rails.
- [ ] Test selection/priority, source labels, timestamp behavior, partial error,
      preference state, accessibility text, and i18n completeness.
- [ ] Commit: `feat(ui): show firmware evidence provenance`.

### Task 9: Update diagnostics, evidence policy, and documentation

**Files:**

- `crates/melt-core/src/diagnostics.rs`
- `crates/melt-cli/src/lib.rs`
- `README.md`
- `docs/bambu-protocol-evidence.md`
- `docs/bambu-compatibility.md`
- `docs/bambu-physical-qualification.md`
- `.interface-design/system.md` only if a reusable source-provenance visual
  pattern is finalized

- [ ] Document local, printer-mediated, and public network boundaries.
- [ ] Document why public-newer is not printer-confirmed availability.
- [ ] Add sanitized source diagnostics and prove forbidden data is absent.
- [ ] Record supported model-page mappings and their qualification dates.
- [ ] Update the previous firmware plan's exclusions/status to point here.
- [ ] Commit: `docs: document hybrid firmware checks`.

### Task 10: Verification and physical qualification

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
- [ ] Run `cargo test --workspace --locked`.
- [ ] Run `bun test --cwd ui` and `bun run --cwd ui build`.
- [ ] Run `make bambu-evidence-check`, `make contract`, and `make ci`.
- [ ] Qualify `get_history` on a cloud-connected printer and a LAN-only printer.
- [ ] Capture a sanitized empty history from the LAN-only printer in scope.
- [ ] Qualify at least one OTA and one AMS response before attaching AMS
      catalogue evidence to an installed module.
- [ ] Compare a recognized LAN-only printer with its official public stable
      page and confirm only `ota` receives public evidence.
- [ ] Exercise public-page 403/429/layout-change/offline cases and confirm the
      last LAN result remains visible.
- [ ] Verify with packet/protocol traces that no update/start/download command
      or firmware package request occurs.
- [ ] Verify no request contains a serial, access code, account token, host IP,
      or other printer identifier.

## Acceptance criteria

- A configured, recognized Bambu printer that is not logged into Bambu Cloud
  can, after explicit opt-in, show its installed OTA beside the latest public
  stable OTA with the public source and timestamp.
- A newer public stable version is labelled **Newer public release**, never
  **Update offered by printer**.
- A qualified newer `upgrade_state` target retains priority and the existing
  confirmed update semantics.
- `upgrade.get_history` success, emptiness, and failure are independently
  visible and never affect printer connectivity.
- Public failure or Cloudflare blocking never removes installed versions,
  marks a printer offline, or produces a false current/available result.
- Internal modules receive no public target without explicit component-level
  evidence.
- Source conflicts are preserved and displayed rather than resolved by a blind
  maximum-version rule.
- Existing CLI/desktop fields and exit behavior remain backward compatible;
  new provenance fields are additive.
- No account authentication, firmware download, update mutation, raw provider
  payload, package URL, or sensitive printer identifier enters the feature.
- Default behavior remains LAN-only until the user enables or explicitly
  invokes the public-catalogue check.
- All automated, contract, evidence, and physical-qualification checks pass.

