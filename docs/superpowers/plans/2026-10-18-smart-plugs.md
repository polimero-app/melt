# Smart Plugs Implementation Plan

Last-updated: 2026-10-18

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** Ready for development. Covers core, CLI and desktop; no change to
existing printer or camera behavior.

**Goal:** Switch printer and camera mains power on and off from the CLI and
desktop through independent, named smart plug profiles, starting with the
Tapo P110, and keep plug controls working while the attached devices are offline.

**Architecture:** A closed-set `plugs` domain in `melt-core` mirrors
`cameras`: profiles in `melt.yaml` (schema v3), passwords in the keychain,
and vendor code behind a `Session` type. Network operations are async
(the `tapo` client is async). Config and keychain work stays synchronous.
Profile workflows split into `prepare_*` → async verify → `commit_*` so neither
surface nests a runtime. A core `PlugManager` serializes per-plug I/O and
owns observations for the desktop. The CLI uses one-shot sessions on a
current-thread runtime.

**Tech Stack:** Rust 2024 (MSRV 1.95), `tapo = "=0.11.1"` (new), tokio (moves
into `melt-core`/`melt-cli`), Tauri, Vue 3 + TypeScript, vitest.

**Spec:** this document. The design sections below are the spec, and the
tasks implement them. Patterns to copy are
[`cameras.rs`](../../../crates/melt-core/src/cameras.rs),
[`cameras/profiles.rs`](../../../crates/melt-core/src/cameras/profiles.rs),
[`config.rs`](../../../crates/melt-core/src/config.rs) and
[`melt-cli/src/cameras.rs`](../../../crates/melt-cli/src/cameras.rs).

## Global Constraints

- First driver `tapo`. The only qualified model is `P110`, an exact match on the model the device reports.
- Plug IDs: random 128-bit, 32 lowercase hex digits, immutable, same format as camera IDs.
- Keychain: service `melt`, account `plug:<id>:password`. Never reuse or copy a camera password.
- Default plug timeout `10s`. This is one overall deadline per operation, including waiting for the per-plug lock.
- Config schema version 3 only when plug data exists; otherwise 2 with camera data; otherwise 1.
- No relay change during verification, add, edit, link, unlink, remove or downgrade.
- No queued, replayed or automatically retried power writes.
- Exit codes: config/not-found/confirmation 2; credentials/keychain 3; connection/timeout/unconfirmed/mismatch 4; unsupported device/protocol 5; internal 1.
- Secret-bearing types (`PlugRequest`, `ResolvedPlug`) implement neither `Debug` nor `Serialize`.
- Never expose raw `tapo` errors, response bodies or authentication payloads. Map them to `PlugError`.
- CLI noun `plug`. Desktop label **Smart plugs**. Locales `en` and `pt-BR`.
- No new dependency beyond `tapo` (and tokio in crates that lacked it).

## Review Focus

1. **The plug's IP now answers as a different Tapo device** (DHCP reassignment): every session rejects it with `UnsupportedDevice` before any read or write. This is tested in Task 4.
2. **`melt plug off` with no TTY and no `--yes`** while a print is running: exit 2, and no network request is made. Tested in Task 7.
3. **The relay is switched from the Tapo app or the physical button:** the next poll reports the device's value, and the manager never answers from a cache. Tested in Task 5.
4. **A printer linked to plugs is renamed and later removed:** links follow the rename, disappear on removal, and the config still validates. Tested in Task 2.
5. **Wrong password at add:** nothing is saved to config or keychain, and the CLI makes no second login attempt. During desktop polling, an auth failure stops automatic attempts until the profile changes or the user retries. Tested in Tasks 3 and 5.

---

## Design

| Topic | Behavior |
| --- | --- |
| Device model | Independent named plug profiles with immutable IDs, like external cameras |
| Drivers | Closed set in `melt-core`; vendor types stay private to the adapter |
| Attachments | Many-to-many: a plug lists the printers and cameras it powers; both kinds can share a plug |
| Control | Explicit on/off for one plug; no toggle, group, default plug or fan-out |
| Authentication | TP-Link account email + password, separate from the Tapo Camera Account |
| Transport | Melt talks directly to the configured LAN host; no cloud relay |
| Initial scope | Profiles, attachments, relay status, on/off with readback, connection health |
| Deferred | Energy data, schedules, auto-off after print, power cycling, bulk switching, cloud control, discovery, multi-outlet devices |

### Attachment semantics

A printer or camera may have several plugs, and a plug may power several
printers and cameras. Every control names one plug and lists every device
attached to it. Links between cameras and printers describe video only. A plug
linked to a printer is not linked to that printer's cameras, and the reverse
also holds. A printer's built-in camera needs no plug attachment.

### Power state and command results

Relay state (`on`, `off`, `unknown`) is separate from connection health and
observation time. When a plug is unreachable, its current relay state is
unknown and the last observation is kept as stale. Printer reachability and
camera playback never decide relay state.

`set_power` reads status first (which renews the session), sends the command,
then reads status again, all within one deadline. The result is one of:

- **Confirmed:** the readback equals the requested state → `PowerChange`.
- **Mismatch:** the device answered, but with the other state → `PlugError::PowerMismatch`.
- **Unconfirmed:** a transport error, a timeout or a malformed response
  during or after the send → `PlugError::PowerUnconfirmed`, which carries the
  requested state and the last observation. It never claims the relay stayed
  as it was.
- **Rejected:** a definite device rejection of the send (authentication,
  lockout, a device error code → `DeviceRejected`) passes through as that error. The device
  answered, so the command was not applied.

Writes are never retried. A read that fails with session expiry reconnects
once within its deadline. Authentication failure and lockout stop
*automatic* logins until the profile revision changes or the user
explicitly asks for status or power.

Turning a plug off cuts power immediately. It is not a print cancel, an
emergency stop or a graceful shutdown. Both surfaces ask for confirmation
first and name the plug and every attached printer and camera. The desktop
also shows known printing, paused, heating or drying activity from the
monitoring it already has. The CLI makes no extra printer requests for the
prompt. Turning a plug on needs no confirmation.

### Configuration

```yaml
version: 3
plugs:
  bench-power:
    id: '11111111111111111111111111111111'
    driver: tapo
    host: 192.0.2.30
    username: operator@example.invalid
    timeout: 10s
    printers: [workshop]
    cameras: ['0123456789abcdef0123456789abcdef']
    created: '2026-10-18T12:00:00Z'
    updated: '2026-10-18T12:00:00Z'
```

`printers` holds printer names, because printers have no stable ID.
`cameras` holds immutable camera IDs. The username's case is kept exactly as
entered. Model, hardware and firmware are observations, never stored.

Lifecycle, applied in the same config write as the change:

- Renaming a plug keeps its ID, links and keychain account.
- Renaming a printer rewrites its name in every plug's `printers`.
- Renaming a camera touches nothing, because links use the camera ID.
- Removing a printer or camera drops it from every plug; plug profiles stay.
- Removing a plug drops its links; printer and camera profiles stay.
- Removing, unlinking or editing never switches a relay.

Downgrading to a v2 binary: remove the plugs with the new binary, or back up
the file, delete `plugs:`, and set `version` to 2 (cameras present) or 1. Then
delete the orphaned `plug:<id>:password` keychain entries.

### Accepted `tapo` 0.11.1 limitations

`tapo` builds its own `reqwest::Client` (`api_client.rs:1078`), and Melt
cannot replace it. Melt accepts this and documents it in
`docs/smart-plugs.md`:

- `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` apply to plug traffic. Users with a
  proxy should add the plug hosts to `NO_PROXY`.
- reqwest's default redirect policy applies. Melt still rejects hosts that
  contain a scheme, port, path or user info.
- TPAP-over-TLS accepts any certificate, because the device certificate is
  self-signed. There is no fingerprint pinning like the Bambu driver has.
- KLAP runs over plain HTTP with its own handshake cryptography.
- The plug password is the user's full TP-Link cloud account password.

Pin with `=0.11.1` (released 2026-10-04, TPAP support only weeks old) and
upgrade deliberately. Keep the `debug`, `python` and `json-schema` features off.

### Qualification units

Three P110s are available for physical testing: units A, B and C. Their LAN
addresses stay out of the repository. Fixtures and docs use `192.0.2.30`–`.32`.

| Unit | Coverage |
| --- | --- |
| A | Authentication, device information, relay status, on/off readback |
| B | Independent control, printer/camera attachment scenarios |
| C | Concurrent sessions, shared attachments, isolation from another plug's failure |

Before any relay test, record each unit's connected load, region,
hardware/firmware and compatibility setting, and which loads may be
interrupted. Start read-only on all three. Record results per unit and per
firmware.

---

## File Structure

| File | Responsibility |
| --- | --- |
| `crates/melt-core/src/config.rs` (modify) | `PlugProfile`, `PlugTarget`, v3 read/write, validation, link ops, lifecycle sweeps |
| `crates/melt-core/src/profiles.rs` (modify) | Gains shared `new_id()`, `now()` and `validate_endpoint_host()`, moved out of camera profiles |
| `crates/melt-core/src/plugs.rs` (create) | Domain types, `PlugError`, constants, `Session` dispatch, `verify`/`status`/`set_power`/`switch` |
| `crates/melt-core/src/plugs/profiles.rs` (create) | Validated CRUD with keychain rollback, link/unlink, resolve |
| `crates/melt-core/src/plugs/tapo.rs` (create) | `tapo` adapter and error mapping |
| `crates/melt-core/src/plugs/manager.rs` (create) | `PlugManager`: per-plug slots, sessions, auth block, observations |
| `crates/melt-cli/src/plugs.rs` (create) | `melt plug …` parsing, runtime, output, error mapping |
| `crates/melt-cli/src/lib.rs` (modify) | Help entries, dispatch, `ErrorDetail.details` as JSON values, `write_error_with_details`, `confirm` |
| `crates/melt-desktop/src/plugs.rs` (create) | Tauri commands, error codes, poller |
| `crates/melt-desktop/src/main.rs` (modify) | Register commands/state, start the poller, evict on printer/camera changes |
| `ui/src/plugs.ts` + `plugs.test.ts` (create) | Types and pure helpers |
| `ui/src/components/PlugControl.vue` (create) | Shared control |
| `ui/src/App.vue`, `ui/src/i18n.ts` (modify) | Smart plugs view, forms, embedded controls, strings |
| `docs/smart-plugs.md` (create) + existing docs (modify) | User documentation |
| `fixtures/tapo/plugs/README.md` (create) | Sanitized physical evidence |

---

### Task 0: Protocol spike (manual, no production code)

**Files:**
- Create: `fixtures/tapo/plugs/README.md`

- [ ] **Step 1:** On units A–C, record load, region, model/hw/fw and the Third-Party Compatibility setting. Mark which loads may be interrupted.
- [ ] **Step 2:** In a scratch crate outside the repo, use `tapo = "=0.11.1"`: `ApiClient::new(user, pass).with_timeout(10s).p110(host)`, then `get_device_info()` on all three units. Record the protocol negotiated (KLAP/TPAP) and the reported `model`, `hw_ver` and `fw_ver`.
- [ ] **Step 3:** On interruptible loads only: `on()`/`off()` + readback; leave a session idle for an hour and record which `TapoResponseError` signals expiry (expected: `Unauthorized` with kind `SESSION_TIMEOUT`, `SESSION_EXPIRED` or `TPAP_SESSION_INVALID`); cold-boot a unit and note how long until login works (and whether a TDP probe on UDP 20002 is needed); confirm LAN control works with WAN disconnected.
- [ ] **Step 4:** Write `fixtures/tapo/plugs/README.md` with model/hw/fw/date/protocol per unit, using unit letters and documentation addresses only. If a cold-booted unit needs a TDP probe, add a follow-up task before Task 4. Otherwise record "no probe needed".
- [ ] **Step 5:** Commit `docs: record P110 protocol spike results`.

**Exit:** the expiry error kinds and boot behavior are known. If expiry looks different from the kinds above, update the mapping table in Task 4 first.

---

### Task 1: Config schema v3 and plug profiles

**Files:**
- Modify: `crates/melt-core/src/config.rs`, `crates/melt-core/src/profiles.rs`, `crates/melt-core/src/cameras/profiles.rs`

**Interfaces:**
- Produces:
  - `pub struct PlugProfile { pub id: String, pub driver: String, pub host: String, pub username: String, pub timeout: String, #[serde(default)] pub printers: Vec<String>, #[serde(default)] pub cameras: Vec<String>, pub created: String, pub updated: String }` (`Clone, Debug, Deserialize, Eq, PartialEq, Serialize`, `deny_unknown_fields`)
  - `ConfigFile.plugs: BTreeMap<String, PlugProfile>` (`skip_serializing_if = "BTreeMap::is_empty"`) and `Config.plugs`
  - `ConfigError::InvalidPlugs(&'static str)`
  - `pub fn is_profile_id(id: &str) -> bool`, renamed from `is_camera_id` (3 call sites)
  - `pub(crate) fn new_id() -> String` and `pub(crate) fn now() -> String` in `crate::profiles`, moved from `cameras/profiles.rs`

- [ ] **Step 1: Write failing tests** in `config.rs` tests:

```rust
#[test]
fn plug_data_round_trips_as_version_3() { /* write plug, reopen, equal; file starts "version: 3" */ }
#[test]
fn writer_picks_the_minimum_version() {
    // no cameras, no plugs → 1; cameras only → 2; plugs only → 3; both → 3
}
#[test]
fn plug_data_in_older_schemas_is_rejected() {
    // version 1 and 2 files with `plugs:` → ConfigError::InvalidPlugs("plug data requires config version 3")
}
#[test]
fn invalid_plug_data_is_rejected() {
    // table: malformed id, duplicate id across plugs, repeated printer, repeated camera,
    // unknown printer, unknown camera id → each Err(InvalidPlugs(_))
}
#[test]
fn version_2_files_open_unchanged() { /* existing v2 fixture reads and rewrites byte-identical */ }
```

- [ ] **Step 2:** Run `cargo test -p melt-core config::` → the new tests fail to compile or fail.
- [ ] **Step 3: Implement.** `MAX_VERSION = 3`. In `open`, reject non-empty `plugs` when `version < 3`. In `validate`, check that plug IDs are valid and unique, that `printers` ⊆ profiles with no repeats, and that `cameras` ⊆ camera IDs with no repeats. In `save`, choose the version as `3 if !plugs.is_empty() else 2 if has_cameras else 1`. Update the `MAX_VERSION` doc comment and the `UnsupportedVersion` message ("want 1 to 3").
- [ ] **Step 4:** Run `cargo test -p melt-core` → PASS (camera tests still pass after the moves and rename).
- [ ] **Step 5:** Commit `feat(core): add config schema v3 with plug profiles`.

---

### Task 2: Plug config operations and lifecycle sweeps

**Files:**
- Modify: `crates/melt-core/src/config.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces (on `Config`):
  - `pub enum PlugTarget { Printer(String), Camera(String) }`. `Camera` holds a camera **name**, resolved to its ID inside `Config`.
  - `pub fn get_plug(&self, name: &str) -> Option<&PlugProfile>`, `pub fn plugs(&self) -> &BTreeMap<String, PlugProfile>`
  - `pub fn plug_by_id(&self, id: &str) -> Option<(&str, &PlugProfile)>`
  - `pub fn add_plug(&mut self, name: &str, plug: PlugProfile) -> Result<(), ConfigError>`
  - `pub fn replace_plug(&mut self, current: &str, name: &str, plug: PlugProfile) -> Result<(), ConfigError>`: the caller passes the links it wants kept
  - `pub fn remove_plug(&mut self, name: &str) -> Result<PlugProfile, ConfigError>`
  - `pub fn link_plug(&mut self, plug: &str, target: &PlugTarget) -> Result<(), ConfigError>`: idempotent
  - `pub fn unlink_plug(&mut self, plug: &str, target: &PlugTarget) -> Result<(), ConfigError>`: `PlugNotLinked` when absent, same as cameras
  - `pub fn camera_name(&self, id: &str) -> Option<&str>`
  - `ConfigError::{PlugAlreadyExists, PlugNotFound, PlugNotLinked}`

- [ ] **Step 1: Write failing tests:**

```rust
#[test]
fn printer_rename_rewrites_plug_links() { /* plug printers [old] → [new]; save validates */ }
#[test]
fn printer_removal_drops_it_from_plugs_and_keeps_plugs() {}
#[test]
fn camera_rename_keeps_plug_links_and_removal_drops_them() {}
#[test]
fn plug_links_are_idempotent_and_mixed() {
    // link printer twice → one entry; link camera by name → its id stored; both kinds coexist
}
#[test]
fn unlinking_an_absent_target_is_an_error() { /* Err(PlugNotLinked) */ }
#[test]
fn linking_unknown_targets_fails_without_mutation() {
    // Printer("nope") → ProfileNotFound; Camera("nope") → CameraNotFound; plug unchanged
}
#[test]
fn renamed_then_removed_printer_leaves_a_valid_config() { /* Review Focus 4: rename, remove, save Ok, reopen Ok */ }
```

- [ ] **Step 2:** Run `cargo test -p melt-core config::` → FAIL.
- [ ] **Step 3: Implement.** Extend `rename_profile` and `remove_profile` to update every plug's `printers`, and `remove_camera` to drop the camera ID from every plug's `cameras`. Update their doc comments.
- [ ] **Step 4:** Run `cargo test -p melt-core` → PASS.
- [ ] **Step 5:** Commit `feat(core): add plug links and lifecycle rules to config`.

---

### Task 3: Plug domain types and profile workflows

**Files:**
- Create: `crates/melt-core/src/plugs.rs`, `crates/melt-core/src/plugs/profiles.rs`
- Modify: `crates/melt-core/src/lib.rs` (`pub mod plugs;`), `crates/melt-core/src/profiles.rs` (gains `pub(crate) fn validate_endpoint_host(host: &str) -> Result<(), ProfileError>`, moved from the camera wrapper; cameras call it)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces in `plugs.rs`:
  - `pub const TAPO: &str = "tapo"; pub const DRIVERS: &[&str] = &[TAPO]; pub const QUALIFIED_MODELS: &[&str] = &["P110"]; pub const DEFAULT_TIMEOUT: &str = "10s";`
  - `#[serde(rename_all = "lowercase")] pub enum PowerState { On, Off }` (`Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize`)
  - `#[serde(rename_all = "lowercase")] pub enum RelayState { On, Off, Unknown }`
  - `pub struct DeviceInfo { pub model: String, pub hardware: String, pub firmware: String }` (Serialize, camelCase)
  - `pub struct PlugStatus { pub device: DeviceInfo, pub relay: PowerState, pub observed_at: String }`
  - `pub struct Observation { pub state: PowerState, pub observed_at: String }`
  - `pub struct PowerChange { pub requested: PowerState, pub status: PlugStatus }`
  - `pub struct ResolvedPlug { pub name: String, pub profile: PlugProfile, pub password: String, pub timeout: Duration }` (not `Debug`)
  - `PlugError` (thiserror): `MissingName, InvalidName, UnknownDriver(String), InvalidHost, InvalidUsername, InvalidPassword, InvalidTimeout, NotFound(String), AlreadyExists(String), PrinterNotFound(String), CameraNotFound(String), NotLinked { plug: String, target: String }, Authentication, CompatibilityDisabled, Lockout, UnsupportedDevice(String), UnsupportedProtocol, DeviceRejected, Connect, Timeout, MalformedResponse, PowerUnconfirmed { requested: PowerState, last_observed: Option<Observation> }, PowerMismatch { requested: PowerState, observed: PowerState }, ProfileChanged, MissingSecret, Secret(#[source] SecretError), Config(#[source] ConfigError), RollbackFailed`. No variant carries upstream text.
- Produces in `plugs/profiles.rs` (re-exported from `plugs`):
  - `#[derive(Deserialize)] pub struct PlugRequest { pub name, pub driver, pub host, pub username, pub password, pub timeout }` (all `String`; empty password on edit keeps the stored one)
  - `pub struct PlugSummary { name, id, driver, host, username, timeout, printers: Vec<String>, cameras: Vec<String> /* camera names */, updated: String }` (Serialize camelCase; `updated` is the revision)
  - `pub struct PlugRemoveResult { name, id, password_removed: bool, unlinked_printers: Vec<String>, unlinked_cameras: Vec<String>, warnings: Vec<RemoveWarning> }`
  - `pub fn prepare_create(dir, request: PlugRequest) -> Result<ResolvedPlug, PlugError>`: validates, checks the name is free, mints the ID and timestamps; saves nothing
  - `pub fn commit_create(dir, store: &dyn SecretStore, plug: ResolvedPlug) -> Result<PlugSummary, PlugError>`: re-checks the name, stores the secret, saves, and rolls the secret back on config failure
  - `pub struct PlugUpdate { pub plug: ResolvedPlug, current: String, password_changed: bool, pub needs_verify: bool }`
  - `pub fn prepare_update(dir, store, current: &str, request: PlugRequest) -> Result<PlugUpdate, PlugError>`: loads the stored password only when `needs_verify` (driver/host/username/password changed); keeps the existing links
  - `pub fn commit_update(dir, store, update: PlugUpdate) -> Result<PlugSummary, PlugError>`
  - `pub fn remove(dir, store, name) -> Result<PlugRemoveResult, PlugError>`: config first, then the keychain; a failed delete gives the warning `password_delete_failed`
  - `pub fn list(dir) -> Result<Vec<PlugSummary>, PlugError>`
  - `pub fn list_for(dir, target: &PlugTarget) -> Result<Vec<PlugSummary>, PlugError>`
  - `pub fn resolve(dir, store, name) -> Result<ResolvedPlug, PlugError>` and `pub fn resolve_by_id(dir, store, id) -> Result<ResolvedPlug, PlugError>`
  - `pub fn revision(dir, id: &str) -> Result<Option<String>, PlugError>`: config only; `None` when removed
  - `pub fn link(dir, plug, target: &PlugTarget) -> Result<PlugSummary, PlugError>`, `pub fn unlink(...)`

Validation: names use `profiles::normalize_name`; host uses `validate_endpoint_host` (bare IP or hostname: no scheme, port, path or `@`); username is 1–254 chars with no control characters and keeps its case; password has no control characters and is required on create; timeout uses `drivers::parse_timeout`.

- [ ] **Step 1: Write failing tests** (with `keychain::test_support::MemoryStore`, temp dirs):

```rust
#[test] fn create_saves_profile_and_secret_after_commit() {}
#[test] fn failed_verification_saves_nothing() {
    // Review Focus 5: prepare_create Ok, (verify fails → caller never commits) → list empty, store empty
}
#[test] fn config_failure_rolls_back_the_secret() {}
#[test] fn edit_metadata_works_without_secret() { /* rename + timeout change, empty store → Ok, needs_verify false */ }
#[test] fn edit_connection_needs_verify_and_stored_secret() { /* host change, empty store → MissingSecret */ }
#[test] fn rename_keeps_id_links_and_keychain_account() {}
#[test] fn username_case_is_preserved() { /* "Operator@Example.invalid" round-trips */ }
#[test] fn invalid_requests_are_rejected() {
    // table: "" name, "a/b" name, unknown driver, "http://h", "h:80", "u@h", empty/long username,
    // control char password, empty create password, bad timeout → matching PlugError
}
#[test] fn remove_reports_unlinked_targets_and_keychain_warning() {}
#[test] fn list_for_filters_by_printer_and_camera_name() {}
#[test] fn unknown_targets_fail_before_mutation() {}
```

- [ ] **Step 2:** Run `cargo test -p melt-core plugs::` → FAIL.
- [ ] **Step 3: Implement** by copying the structure of `cameras/profiles.rs` (`StoredSecret::replace`, `profiles::restore`, `RemoveWarning`).
- [ ] **Step 4:** Run `cargo test -p melt-core` → PASS.
- [ ] **Step 5:** Commit `feat(core): add smart plug profile workflows`.

---

### Task 4: Tapo adapter and power semantics

**Files:**
- Create: `crates/melt-core/src/plugs/tapo.rs`
- Modify: `crates/melt-core/src/plugs.rs`, `Cargo.toml` (`tapo = { version = "=0.11.1", default-features = false }` in workspace deps), `crates/melt-core/Cargo.toml` (`tapo.workspace = true`, `tokio.workspace = true`), `Cargo.lock`

**Interfaces:**
- Consumes: Task 3 types.
- Produces in `plugs.rs`:
  - `pub struct Session` (opaque; holds `enum Inner { Tapo(tapo::TapoSession), #[cfg(test)] Fake(FakeSession) }`)
  - `impl Session { pub async fn connect(plug: &ResolvedPlug) -> Result<Session, PlugError>; pub async fn status(&mut self) -> Result<PlugStatus, PlugError>; async fn send(&mut self, state: PowerState) -> Result<(), PlugError> }`
  - `pub async fn verify(plug: &ResolvedPlug) -> Result<PlugStatus, PlugError>`: connect + status, never `send`
  - `pub async fn status(plug: &ResolvedPlug) -> Result<PlugStatus, PlugError>`: one-shot, deadline `plug.timeout`
  - `pub async fn set_power(plug: &ResolvedPlug, state: PowerState) -> Result<PowerChange, PlugError>`: one-shot
  - `pub(crate) async fn switch(session: &mut Session, state: PowerState, deadline: tokio::time::Instant, last: Option<Observation>) -> Result<PowerChange, PlugError>`: shared by `set_power` and `PlugManager`
- `plugs/tapo.rs` (private): `TapoSession` wraps `tapo::PlugEnergyMonitoringHandler` + the `ResolvedPlug` data needed to reconnect; `fn map_error(error: tapo::Error) -> PlugError`.

`connect` builds `ApiClient::new(username, password).with_timeout(plug.timeout)`, calls `.p110(host)`, then `get_device_info()`. It returns `UnsupportedDevice(model)` unless `QUALIFIED_MODELS.contains(model)`. `status` applies the same check (Review Focus 1). `status` reconnects once on a session-expiry error. `send` never reconnects.

Error mapping (pinned by a table test):

| `tapo` error | `PlugError` |
| --- | --- |
| `Tapo(Unauthorized { kind: "TPAP_AUTH_ATTEMPTS_LIMIT", .. })` | `Lockout` |
| `Tapo(Unauthorized { kind: "FORBIDDEN", .. })` | `CompatibilityDisabled` |
| `Tapo(Unauthorized { kind: "HASH_MISMATCH" \| "TPAP_CREDENTIALS" \| "TPAP_HASH_MISMATCH", .. })`, `Tapo(DeviceError { kind: "LOGIN", .. })` | `Authentication` |
| `Tapo(Unauthorized { kind: "SESSION_TIMEOUT" \| "SESSION_EXPIRED" \| "TPAP_SESSION_INVALID", .. })`, `Tapo(DeviceError { kind: "SESSION_EXPIRED", .. })` | internal `SessionExpired` (adapter only; a read reconnects once; otherwise surfaced as `DeviceRejected`) |
| `UnsupportedProtocol { .. }` | `UnsupportedProtocol` |
| `Http(e)` with `e.is_timeout()` | `Timeout` |
| `Http(_)`, `DeviceNotFound`, `Other(_)` | `Connect` |
| any other `Tapo(DeviceError { .. })` | `DeviceRejected` (the device answered with an error code) |
| `Serde(_)`, `Validation { .. }`, `Tapo(EmptyResult \| ResponseError { .. } \| HttpError { .. })` | `MalformedResponse` |

`switch` algorithm (deadline-bounded via `tokio::time::timeout_at` on each await):

```text
before = status()            // error → return it unchanged (nothing sent)
                             // update `last` with before
send(state):
  Ok                         → go to readback
  Err(Authentication|Lockout|CompatibilityDisabled|DeviceRejected)
                             → return it (device refused; not applied)
  Err(other) or deadline     → PowerUnconfirmed { requested, last }
readback = status():
  Ok(s) if s.relay == state  → Ok(PowerChange)
  Ok(s)                      → PowerMismatch { requested, observed: s.relay }
  Err(_) or deadline         → PowerUnconfirmed { requested, last }
```

`FakeSession` (`#[cfg(test)]`): a scripted queue of `FakeStep::{Status(PowerState), Ok, Err(PlugError), Hang}` plus a shared `Arc<Mutex<PowerState>>` relay, so tests can change the relay "externally". A `fake` driver (`#[cfg(test)]`) looks up `host` in a test-only `static FAKES: Mutex<HashMap<String, FakeSession>>`, so `PlugManager` tests can go through `Session::connect`.

- [ ] **Step 1: Write failing tests:**

```rust
#[test] fn tapo_errors_map_to_plug_errors() { /* table above, constructing tapo::Error values */ }
#[test] fn plug_error_display_never_contains_upstream_text() {
    // map an Unauthorized with description "secret-pw" → to_string() lacks "secret-pw"
}
#[tokio::test] async fn confirmed_when_readback_matches() {}
#[tokio::test] async fn already_on_is_confirmed() {}
#[tokio::test] async fn mismatch_when_readback_differs() {}
#[tokio::test] async fn lost_response_after_send_is_unconfirmed_and_not_replayed() {
    // send → Err(Connect); assert PowerUnconfirmed with last = before; fake saw exactly one send
}
#[tokio::test] async fn hang_during_send_hits_deadline_as_unconfirmed() { /* tokio::time::pause */ }
#[tokio::test] async fn failed_pre_read_sends_nothing() {}
#[tokio::test] async fn device_refusal_passes_through() { /* send → Err(Authentication) and Err(DeviceRejected) */ }
#[tokio::test] async fn malformed_reply_to_send_is_unconfirmed() { /* send → Err(MalformedResponse) */ }
#[tokio::test] async fn unqualified_model_is_rejected_on_connect_and_status() { /* model "P100" */ }
#[tokio::test] async fn expired_session_read_reconnects_once() {}
```

`#[tokio::test]` needs tokio's `macros` feature as a melt-core **dev**-dependency: `tokio = { workspace = true, features = ["macros", "test-util"] }`.

- [ ] **Step 2:** Run `cargo test -p melt-core plugs::` → FAIL.
- [ ] **Step 3: Implement** the adapter, `Session`, `verify`/`status`/`set_power`/`switch`.
- [ ] **Step 4:** Run `cargo test -p melt-core` → PASS. Then run `make cargo-deny`, `cargo build --workspace --locked` and the MSRV check from CI → clean. Record the binary size change in the commit body.
- [ ] **Step 5:** Commit `feat(core): add Tapo P110 plug driver`.

---

### Task 5: Plug manager

**Files:**
- Create: `crates/melt-core/src/plugs/manager.rs`

**Interfaces:**
- Consumes: `Session`, `switch`, Task 3 types.
- Produces:
  - `pub struct PlugKey { pub id: String, pub revision: String, pub timeout: Duration }`, built from `PlugSummary` (`updated`, parsed `timeout`)
  - `#[serde(rename_all = "camelCase")] pub struct PlugObservation { pub relay: RelayState, pub connection: Connection, pub last: Option<Observation>, pub device: Option<DeviceInfo> }`
  - `#[serde(rename_all = "camelCase")] pub enum Connection { Unknown, Online, Unreachable, AuthFailed, Locked, Unsupported }`
  - `#[derive(Default)] pub struct PlugManager`
  - `pub async fn status<R>(&self, key: &PlugKey, automatic: bool, resolve: R) -> Result<PlugStatus, PlugError>`
  - `pub async fn set_power<R, C>(&self, key: &PlugKey, state: PowerState, resolve: R, current_revision: C) -> Result<PowerChange, PlugError>`
    - `R: FnOnce() -> Result<ResolvedPlug, PlugError> + Send + 'static` runs on `spawn_blocking` only when a session must be created
    - `C: FnOnce() -> Result<Option<String>, PlugError> + Send + 'static` runs on `spawn_blocking` after the slot lock, before dispatch. A changed revision gives `ProfileChanged`, and `None` gives `NotFound`.
  - `pub fn observations(&self) -> HashMap<String, PlugObservation>` (sync snapshot)
  - `pub fn evict(&self, id: &str)` and `pub fn retain(&self, ids: &HashSet<String>)`

Design: a `std::sync::Mutex<HashMap<id, Arc<tokio::sync::Mutex<Slot>>>>` holds the slots. The outer lock is never held across `.await`. A `Slot` holds `{ revision, session: Option<Session>, auth_blocked: bool }`. Observations live in a separate `std::sync::Mutex<HashMap<id, PlugObservation>>` that is written **while the slot lock is held**, so results are recorded in completion order and a slower poll cannot overwrite a newer write. The deadline is `Instant::now() + key.timeout` at entry and also covers the wait for the slot lock. A revision mismatch drops the session and clears `auth_blocked`. `automatic && auth_blocked` returns the cached error without touching the network. An explicit call clears the block. On error, `relay = Unknown` and `last` is kept.

- [ ] **Step 1: Write failing tests** (fake driver, `tokio::time::pause`):

```rust
#[tokio::test] async fn external_relay_change_shows_on_next_status() { /* Review Focus 3 */ }
#[tokio::test] async fn session_is_reused_until_revision_changes() { /* connect count */ }
#[tokio::test] async fn queued_write_after_edit_returns_profile_changed_and_sends_nothing() {}
#[tokio::test] async fn queued_write_after_removal_returns_not_found() {}
#[tokio::test] async fn auth_failure_blocks_automatic_status_until_explicit_or_revision_change() { /* Review Focus 5 */ }
#[tokio::test] async fn slow_plug_does_not_delay_another_plug() {}
#[tokio::test] async fn poll_finishing_after_write_cannot_overwrite_newer_observation() {}
#[tokio::test] async fn lock_wait_counts_against_the_deadline() {}
#[tokio::test] async fn failure_keeps_last_observation_as_stale() { /* relay Unknown, last Some */ }
```

- [ ] **Step 2:** Run `cargo test -p melt-core plugs::manager` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p melt-core` → PASS.
- [ ] **Step 5:** Commit `feat(core): add per-plug session manager`.

---

### Task 6: CLI profile and attachment commands

**Files:**
- Create: `crates/melt-cli/src/plugs.rs`
- Modify: `crates/melt-cli/src/lib.rs` (help entries next to `camera`, dispatch for `plug`), `crates/melt-cli/Cargo.toml` (`tokio.workspace = true`), `fixtures/cli-contract/inventory.json` (`meltNativeCommands`: `plug`, `plug drivers|add|edit|list|remove|link|unlink|status|on|off`), `fixtures/cli-contract/manifest.json`, new goldens, `crates/melt-cli/tests/contract_fixtures.rs` only if the counting needs it

**Interfaces:**
- Consumes: Task 3 workflows, `plugs::verify`.
- Produces:
  - `pub(crate) fn run_plug_command(format, args, out, err) -> i32`
  - `pub(crate) fn plug_error(error: PlugError) -> AppError`
  - `fn confirm(err: &mut dyn Write, prompt: &str, declined: &str) -> Result<(), AppError>` in `lib.rs`, extracted from camera `remove`, which now calls it

Command contract:

```text
melt plug drivers
melt plug add <plug> --driver tapo --host <host> --username <email> --password-file <path> [--timeout <duration>]
melt plug edit <plug> [--name <new>] [--host <host>] [--username <email>] [--password-file <path>] [--timeout <duration>]
melt plug list [--printer <printer> | --camera <camera>]
melt plug remove <plug> [--yes]
melt plug link <plug> (--printer <printer> | --camera <camera>)
melt plug unlink <plug> (--printer <printer> | --camera <camera>)
```

`link`/`unlink` take named target options rather than camera-style positionals (`camera link <camera> <printer>`), because a plug target can be either kind and the names may collide. `add`/`edit` run `prepare_*`. When verification is needed, they build a current-thread runtime (`Builder::new_current_thread().enable_all()`), call `mark_network_call()` and `block_on(plugs::verify(&plug))`, then `commit_*`. `--password-file` reuses `read_secret_file`.

`plug_error` mapping: validation/`NotFound`/`AlreadyExists`/`PrinterNotFound`/`CameraNotFound`/`NotLinked`/`ProfileChanged` → `AppError::usage` (2, `config_error`); `Authentication`/`CompatibilityDisabled` → 3 `authentication_failed`; `Lockout` → 3 `authentication_locked`; `MissingSecret` → 3 `secret_not_found`; `Secret` → 3 `secret_store_failed`; `Timeout` → 4 `timeout`; `Connect`/`MalformedResponse` → 4 `connection_failed`; `DeviceRejected` → 4 `device_rejected`; `PowerUnconfirmed` → 4 `power_change_unconfirmed`; `PowerMismatch` → 4 `power_change_mismatch`; `UnsupportedDevice`/`UnsupportedProtocol` → 5 `capability_unsupported`; `RollbackFailed` → 1 `internal_error`; `Config(e)` → `config_error(e)`.

- [ ] **Step 1: Write failing tests:** `plug_errors_follow_the_exit_contract` (table over every variant above), `link_requires_exactly_one_target`, `list_rejects_both_filters`, `add_requires_password_file`, `remove_without_yes_in_noninteractive_mode_fails`. Goldens: `plug-list-empty`, `plug-drivers`, `plug-remove-unknown`, `plug-link-no-target`, `plug-link-both-targets`, `plug-link-unknown-printer` (each in human and `--output json` form, like the camera goldens).
- [ ] **Step 2:** Run `cargo test -p melt-cli` and `make contract` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p melt-cli` and `make contract` → PASS.
- [ ] **Step 5:** Commit `feat(cli): add plug profile and attachment commands`.

---

### Task 7: CLI status and power commands

**Files:**
- Modify: `crates/melt-cli/src/plugs.rs`, `crates/melt-cli/src/lib.rs`, contract fixtures

**Interfaces:**
- Consumes: Task 6, `plugs::status`, `plugs::set_power`.
- Produces in `lib.rs`:
  - `ErrorDetail.details: BTreeMap<String, serde_json::Value>`. Existing envelopes still serialize `{}`, and the goldens stay unchanged.
  - `fn write_error_with_details(command, format, error: AppError, details: BTreeMap<String, Value>, out, err) -> i32`. `write_error` delegates with an empty map.

```text
melt plug status <plug>
melt plug on <plug>
melt plug off <plug> [--yes]
```

`off` confirms **before** resolving the secret or opening a runtime, with this prompt:
`Turn off plug {name}? Power is cut immediately to printers: {list or "none"}; cameras: {list or "none"}. Type 'yes' to continue: `.
Declined → `confirmation declined; no power command was sent`.

JSON details on power errors: `PowerUnconfirmed` →
`{"requestedState": "on", "lastObservedState": "off"|null, "observedAt": "<rfc3339>"|null, "deliveryUncertain": true}`;
`PowerMismatch` → `{"requestedState": "on", "lastObservedState": "<observed>", "observedAt": "<readback time>", "deliveryUncertain": false}`.
Human output: `Plug {name} is {on|off} (confirmed {observedAt})`. For unconfirmed: `Sent "{state}" to plug {name} but could not confirm the result. Last known state: {state|unknown}.`

- [ ] **Step 1: Write failing tests:** `off_without_tty_or_yes_sends_nothing` (Review Focus 2: run with a plug whose host is `192.0.2.30` and no keychain; expect exit 2 and `config_error`, and assert no network attempt by checking that `meta.durationMs` is absent), `unconfirmed_error_carries_details` (serialize the envelope for a `PowerUnconfirmed` error and match the JSON exactly), `mismatch_error_carries_details`, `existing_error_details_stay_empty` (the current goldens pass unchanged). Goldens: `plug-off-noninteractive`, `plug-status-unknown`.
- [ ] **Step 2:** Run `cargo test -p melt-cli` and `make contract` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p melt-cli` and `make contract` → PASS.
- [ ] **Step 5:** Commit `feat(cli): add plug status and power commands`.

---

### Task 8: Desktop commands and poller

**Files:**
- Create: `crates/melt-desktop/src/plugs.rs`
- Modify: `crates/melt-desktop/src/main.rs` (`mod plugs;`, `.manage(Arc<PlugManager>)`, register commands, start the poller). Printer/camera rename and removal need no hook: the next poll cycle's `retain` drops removed plugs, and the UI reloads `plug_list` after any printer or camera save, as it already does for `camera_list`.

**Interfaces:**
- Consumes: Tasks 3–5.
- Produces (all `#[tauri::command] async fn`; config/keychain work runs inside `tauri::async_runtime::spawn_blocking`):
  - `plug_list() -> Result<Vec<PlugSummary>, CommandError>`
  - `plug_add(request: PlugRequest) -> Result<PlugSummary, CommandError>`: prepare → `plugs::verify` → commit
  - `plug_edit(name: String, request: PlugRequest) -> Result<PlugSummary, CommandError>`: verifies only when `needs_verify`, then evicts the session
  - `plug_remove(name: String) -> Result<PlugRemoveResult, CommandError>`: evicts the session
  - `plug_link(plug: String, printer: Option<String>, camera: Option<String>) -> Result<PlugSummary, CommandError>` and `plug_unlink(...)`: exactly one target
  - `plug_status(id: String) -> Result<PlugStatus, CommandError>`: explicit, `automatic = false`
  - `plug_set_power(id: String, state: PowerState) -> Result<PowerChange, CommandError>`
  - `plug_observations() -> HashMap<String, PlugObservation>`
  - Event `plugs-updated` (payload: `plug_observations()`), emitted whenever an observation changes
  - `fn plug_error(error: PlugError) -> CommandError` with codes: `profileMissingName`, `profileInvalidName`, `profileInvalidHost`, `profileInvalid` (driver), `plugInvalidUsername`, `plugInvalidPassword`, `plugInvalidTimeout`, `plugNotFound`, `plugExists`, `profileNotFound`, `cameraNotFound`, `plugNotLinked`, `plugAuthFailed`, `plugCompatibilityDisabled`, `plugLocked`, `plugUnsupportedDevice`, `plugUnsupportedProtocol`, `plugUnreachable` (`Connect`/`MalformedResponse`), `plugDeviceRejected`, `plugTimeout`, `plugPowerUnconfirmed`, `plugPowerMismatch`, `plugProfileChanged`, `plugPasswordUnavailable`, `keychainFailed`, `configUnwritable`

Poller: a `tauri::async_runtime::spawn` loop that waits `monitor::DEFAULT_INTERVAL` (5 s) between cycles. Each cycle lists the summaries, calls `manager.retain(ids)`, and for each plug that is due runs `manager.status(key, automatic = true, resolve_by_id)` under a `tokio::sync::Semaphore` of 4. Each plug has its own `monitor::Backoff`, so failures space out up to 60 s. The poller is independent of `start_monitor_worker` and the camera manager.

- [ ] **Step 1: Write failing tests:** `plug_errors_map_to_stable_codes` (table over every `PlugError` variant, like `camera_errors_map_to_stable_codes`), `link_requires_exactly_one_target`.
- [ ] **Step 2:** Run `cargo test -p melt-desktop` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `cargo test -p melt-desktop` and `cargo clippy --workspace --all-targets -- -D warnings` → PASS.
- [ ] **Step 5:** Commit `feat(desktop): add smart plug commands and polling`.

---

### Task 9: Smart plugs view and forms

**Files:**
- Create: `ui/src/plugs.ts`, `ui/src/plugs.test.ts`
- Modify: `ui/src/App.vue` (`View` gains `'plugs'`, nav entry `t('nav.plugs')` after Cameras, list cards, add/edit `SlideOver`, remove via `ConfirmDialog`), `ui/src/i18n.ts` (`en` + `pt-BR`: nav, form labels, a hint about Third-Party Compatibility and the TP-Link account, an `errors.<code>` for every Task 8 code)

**Interfaces:**
- Consumes: Task 8 commands.
- Produces in `plugs.ts`:
  - `type PowerState = 'on' | 'off'`, `type RelayState = PowerState | 'unknown'`, `type Connection = 'unknown' | 'online' | 'unreachable' | 'authFailed' | 'locked' | 'unsupported'`
  - `type PlugSummary` and `type PlugObservation` mirroring Rust (camelCase)
  - `plugsForPrinter(printer: string, plugs: PlugSummary[]): PlugSummary[]` (lowercased match, like `linkedCameras`)
  - `plugsForCamera(camera: string, plugs: PlugSummary[]): PlugSummary[]` (by camera name)
  - `isStale(observation: PlugObservation | undefined): boolean` → true when `relay === 'unknown' && last` exists
  - Link deltas reuse `linkChanges` from `cameras.ts`

Form fields: driver (fixed `tapo`), name, host, TP-Link email, password (on edit, empty keeps the stored one), timeout (default `10s`), a printer checklist and a camera checklist. Save runs `plug_add`/`plug_edit`, then applies the link deltas. If a link call fails after the profile was saved, keep the profile, reload `plug_list`, and show "Plug saved, but some attachments could not be updated" with the error.

Add `loadPlugs()` (calls `plug_list`) next to every existing `void loadCameras()` call in `App.vue` (printer save/remove, camera save/remove), so printer renames and removals show up in plug links.

- [ ] **Step 1: Write failing tests** in `plugs.test.ts`: `plugsForPrinter matches case-insensitively`, `plugsForCamera matches by name`, `isStale only when unknown with a previous observation`, `link deltas cover printers and cameras independently`.
- [ ] **Step 2:** Run `bun run --cwd ui test` → FAIL.
- [ ] **Step 3: Implement** the helpers, view and forms using the existing `Card`, `SlideOver`, `ConfirmDialog` and `commandMessage`.
- [ ] **Step 4:** Run `bun run --cwd ui test`, `bun run --cwd ui build` and the `i18n.test.ts` key-parity test → PASS.
- [ ] **Step 5:** Commit `feat(ui): add smart plugs view`.

---

### Task 10: Shared plug control in every context

**Files:**
- Create: `ui/src/components/PlugControl.vue`
- Modify: `ui/src/App.vue` (plug cards, printer Control including its offline/unavailable screen, and external camera details including the unavailable state), `ui/src/plugs.ts` (+ tests), `ui/src/i18n.ts`

**Interfaces:**
- Consumes: Task 9 helpers, `plug_set_power`, `plug_status`, `plugs-updated`.
- Produces:
  - `PlugControl.vue` props: `{ plug: PlugSummary, observation?: PlugObservation, activity: string[] }`. Emits nothing; it calls the commands itself.
  - `offConfirmation(plug: PlugSummary, activity: Record<string, string>): { printers: string[]; cameras: string[]; active: string[] }` in `plugs.ts`. `activity` maps printer name to a known state (`printing`, `paused`, `heating`, `drying`). Unknown printers are listed without a state.

The control shows the plug name, relay state (or "unknown", with the last observation as stale), connection health, observation time, attached printers and cameras, and On/Off buttons. Off opens `ConfirmDialog` with `offConfirmation`. While a command is in flight, both buttons for **that** plug are disabled and nothing changes optimistically. Each attachment renders its own control, and clicking one never affects another. After a confirmed change, call `plug_status`. After a confirmed **on**, let printer monitoring re-establish the printer on its own schedule and restart the affected visible camera viewers through their existing subscription. Never open a viewer automatically or treat relay state as printer availability. Power buttons stay enabled whatever the target's connectivity. `plugPowerUnconfirmed` shows the error message plus the last observation.

- [ ] **Step 1: Write failing tests:** `offConfirmation lists both kinds and known activity`, `offConfirmation keeps unknown printers without blocking`.
- [ ] **Step 2:** Run `bun run --cwd ui test` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Run `bun run --cwd ui test` and `bun run --cwd ui build` → PASS. Attended check with `make run`: confirmation flow, keyboard access, light/dark, `en`/`pt-BR`, minimum window size, controls on the offline printer and camera screens, an external camera that keeps streaming while a printer's own plug is off.
- [ ] **Step 5:** Commit `feat(ui): add plug controls to printer and camera views`.

---

### Task 11: Documentation

**Files:**
- Create: `docs/smart-plugs.md`
- Modify: `docs/cli.md` (plug commands, exit codes, JSON details), `docs/desktop.md`, `docs/configuration.md` (schema v3, downgrade), `docs/development.md`, `docs/README.md`, `.interface-design/system.md` (plug control), `fixtures/tapo/README.md` (link to `plugs/`)

- [ ] **Step 1:** Write `docs/smart-plugs.md`: setup in the Tapo app, the TP-Link account (and that it is the cloud account password), Third-Party Compatibility, LAN reachability, qualified hardware/firmware from Task 0, attachments, immediate power cut semantics, unconfirmed and mismatch results, keychain recovery, downgrade, and the **Accepted `tapo` limitations** list verbatim.
- [ ] **Step 2:** Update the other files and check every example against the implemented commands (`cargo run -p melt-cli -- plug --help`).
- [ ] **Step 3:** Commit `docs: document smart plugs`.

---

### Task 12: Physical qualification (release gate)

**Files:**
- Modify: `fixtures/tapo/plugs/README.md`, `docs/smart-plugs.md` (qualified firmware list)

- [ ] **Step 1:** Units A–C, interruptible loads only: on/off/readback via CLI and desktop; one plug attached to a printer and a camera; a camera with its own power that keeps streaming; boot delay after power returns; switching from the Tapo app or the button shows up on the next poll; network unplugged during a write gives an unconfirmed result.
- [ ] **Step 2:** All three units at once: independent relay states, no change on the other two plugs, healthy plugs progress while one session fails (wrong password on one profile).
- [ ] **Step 3:** Record results per unit and firmware, separately from automated coverage. Name only qualified hardware/firmware. Make no broad Tapo compatibility claim.
- [ ] **Step 4:** Run `make ci` → PASS. Refresh the screenshot samples if `docs/desktop.md` shows the new view.
- [ ] **Step 5:** Commit `docs: record P110 qualification results`.

---

## Acceptance criteria

- A user can add one P110, attach it to printers, cameras or both, and switch it from either surface while the attached devices are offline.
- Several plugs on one target and several targets on one plug stay explicit. Switching one plug sends exactly one relay command.
- Renames keep attachments and secrets. Removals clean up links without switching power or deleting other profiles.
- Setup and edit verification are read-only. Passwords stay in the keychain. Logs, fixtures and JSON never contain credentials or authentication material.
- Relay state comes only from the plug. Network loss gives unknown/stale state or an unconfirmed result, and reconnecting never replays a power change.
- Existing printer and camera configuration, commands, feed selection and health stay compatible, including an external camera that stays live while its printer is off.
