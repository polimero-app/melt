# AMS Drying Display and Control Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show every heater-equipped AMS unit's drying state (phase, time left, setting, blocking reasons) in CLI and desktop status, and let Melt start/stop a drying cycle on printers whose firmware advertises remote drying.

**Architecture:** Drying telemetry is parsed per AMS unit in `bambu/transport.rs` into a new `AmsUnit.drying` block carrying a per-unit `controllable` flag, following the existing "observed inventory + controllable flag" rule for temperatures/fans/lights (`docs/bambu-compatibility.md` § Temperature, fan, and light inventory). One mutation, `ams_drying_set(ams_id, DryingRequest::{Start, Stop})`, goes through the existing authorization gate, the correlated MQTT exchange, and a state predicate. Each surface (drivers → pool → CLI → Tauri → Vue) adds one thin adapter that mirrors `light_set`.

**Tech Stack:** Rust 2024 (`serde_json`, stdlib), Tauri 2, Vue 3 + TypeScript + Tailwind, Vitest.

**Spec / evidence:** No separate spec. This plan is based on:
- Raw `pushall` captures taken 2026-09-19 from dakota (H2-class, fw 01.02.00.00, AMS 2 Pro ×2 + AMS HT, drying), georgia (fw 01.08.01.00, AMS HT, drying) and alaska (fw 01.08.01.00, AMS HT, printing, not drying). The sanitized excerpts are inlined in the tests below.
- The README protocol references:
  - Bambuddy `backend/app/utils/ams_drying.py`, `services/drying_preflight.py`, `services/bambu_mqtt.py::send_drying_command`, `services/printer_manager.py` (`_DRYING_SCREEN_ONLY_MODELS`).
  - open-bamboo-networking `research/12.07-ams-filament.md` (`ams_filament_drying`), `research/12.01-status.md` (`fun2` bit 5 `is_support_remote_dry`), `research/12.01.03-fields-ams.md`.
  - torbenconto `internal/protocol/report.go` (`DrySetting`, `dry_sf_reason`).

## Protocol facts (read before any task)

| Field (per `print.ams.ams[]` unit) | Meaning | Source |
|---|---|---|
| `dry_time` (int) | Minutes remaining. Present only on heater-equipped units (AMS 2 Pro, AMS HT). 0 while idle **and** during the closing cooling phase. | captures, Bambuddy |
| `info` (hex string) | Bits 4–7 = `dry_status`: 0 Off, 1 Checking, 2 Drying, 3 Cooling, 4 Stopping, 5 Error, 6 HeatOutOfControl, 7 PrdTesting. | Bambuddy (from Bambu Studio `DevFilaSystem.cpp`) |
| `dry_setting` `{dry_duration, dry_filament, dry_temperature}` | Active cycle's settings (hours / type / °C); `-1` = n/a. Only H2-class firmware sends it. | OBN, captures |
| `dry_sf_reason` (int[]) | Reasons a start is refused: 0 printer busy, 1 insufficient power, 2 AMS busy, 3 filament at outlet, 4 already starting, 5 not in 2D mode, 6 already drying, 7 AMS firmware upgrading, 8 external power required. | Bambuddy `DRY_SF_REASON_MESSAGES` |
| `print.fun2` bit 5 | `is_support_remote_dry`. dakota `2011FF` → set. alaska/georgia send no `fun2`. | OBN, captures |

A drying cycle is **active** when `dry_time > 0` or `dry_status ∈ {1, 2, 3}` (Bambuddy `is_drying_active`).

Start/stop command (`print` section, correlated by `command` + `sequence_id` like every other Melt mutation):

```json
{"print":{"sequence_id":"N","command":"ams_filament_drying","ams_id":0,"mode":1,
  "filament":"PLA","temp":45,"duration":12,"humidity":0,"rotate_tray":false,
  "cooling_temp":20,"close_power_conflict":false}}
```

Stop uses the same keys with `mode: 0`, `temp: 0`, `duration: 0`, `cooling_temp: 0`, `filament: ""`. The printer rejects an empty `filament` on start, so start falls back to the first loaded tray's type, then `PLA`.

P1P/P1S firmware answers `result: success` and then does nothing (Bambuddy #2533). The `fun2` bit 5 gate is what keeps Melt from ever sending that command to those printers.

## Global Constraints

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, and `cd ui && bun run test && bun run build` must be green after every task (`make test`, `make lint`).
- **Do not rename or remove any existing serialized field.** New Rust status fields are `#[serde(skip_serializing_if = ...)]` so existing JSON consumers and `ui/src/presentation.test.ts` snapshots don't shift.
- No new dependencies (Rust or JS).
- A capability decision comes from live observation (`fun2` bit 5 + the unit's own `dry_time` key), never from the model name (README § Current capability boundary).
- Every mutation calls `authorize_mutation` first (a new `MutationClass::Drying`), refreshes status, and rejects a missing or telemetry-only target before it publishes anything. This matches `fan_set`/`light_set`.
- State-changing CLI commands require `--yes` or interactive confirmation (`require_confirmation`). The desktop confirms through `askConfirmation`.
- Deliberate shortcuts get a `// ponytail:` comment, following the module convention.
- Commit prefixes: `feat(bambu):`, `feat(cli):`, `feat(desktop):`, `feat(ui):`, `docs(bambu):`. One logical change per commit. Every commit must build and pass its tests.
- Every new user-facing string in `ui/src/i18n.ts` gets both an `en` and a `pt-BR` entry. The message-key union type there must list it.

## Out of scope (YAGNI, add when asked)

- Humidity-target mode (`mode: 2`), `rotate_tray`, scheduled/auto drying, and the `auto_stop_ams_dry` emergency command. A timed stop (`mode: 0`) covers stopping.
- Drying while printing as a separate feature. Melt sends the command and the firmware arbitrates through `dry_sf_reason` (code 0 "printer busy"), which Melt surfaces as-is.
- `dry_sub_status` (bits 22–25).
- The `canonicalModel: "unknown"` misidentification seen on all three printers. File it separately.

## File map

| File | Change |
|---|---|
| `crates/melt-core/src/moonraker.rs` | New types `AmsDrying`, `DryingStatus`, `DryingSetting`, `DryingRequest`, `DryingResult`; new field `AmsUnit.drying`. |
| `crates/melt-core/src/bambu/transport.rs` | Telemetry parse (`ams_drying`, `fun2_bit`), `MutationClass::Drying`, `Error::{InvalidDryingRequest, DryingBlocked}`, `Client::ams_drying_set`, pure helpers + tests. |
| `crates/melt-core/src/drivers.rs` | `Operation::AmsDrying`, `Capabilities.ams_drying`, `drivers::ams_drying_set`. |
| `crates/melt-core/src/pool.rs` | `ConnectionPool::ams_drying_set`. |
| `crates/melt-cli/src/lib.rs` | Status "Drying:" line; `ams dry start|stop` commands; error mapping. |
| `fixtures/cli-contract/inventory.json`, `crates/melt-cli/tests/contract_fixtures.rs` | 4 new inventory rows; count 37 → 41. |
| `crates/melt-desktop/src/main.rs` | `printer_ams_drying_set` Tauri command. |
| `ui/src/drying.ts` (+ `drying.test.ts`) | Pure presentation helpers: presets, bounds, labels. |
| `ui/src/App.vue`, `ui/src/i18n.ts` | Types, AMS card badge, start/stop controls, strings. |
| `docs/bambu-compatibility.md`, `README.md` | Behavior docs. |

---

### Task 1: Parse AMS drying telemetry into status

**Files:**
- Modify: `crates/melt-core/src/moonraker.rs` (after `AmsUnit`, ~line 1276)
- Modify: `crates/melt-core/src/bambu/transport.rs`: `ams_data` / `ams_unit` (~3474–3536), `virtual_tray_units` (~3563), `has_emmc` (~3631)
- Test: `crates/melt-core/src/bambu/transport.rs` `mod tests`, next to `external_spool_holders_become_a_virtual_ams_unit` (~4770)

**Interfaces:**
- Produces (in `crate::moonraker`, re-exported like the other AMS types):
  ```rust
  pub enum DryingStatus { Off, Checking, Drying, Cooling, Stopping, Error, HeatOutOfControl, Unknown }
  pub struct DryingSetting { pub filament: Option<String>, pub temperature_c: u16, pub hours: u16 }
  pub struct AmsDrying {
      pub status: DryingStatus,
      pub active: bool,
      pub minutes_remaining: Option<u32>,
      pub setting: Option<DryingSetting>,
      pub blocked_reasons: Vec<&'static str>,
      pub controllable: bool,
  }
  // AmsUnit gains: pub drying: Option<AmsDrying>
  ```
- Produces (private, in `transport.rs`): `fn ams_drying(unit: &Value, remote_dry: bool) -> Option<AmsDrying>`, `fn fun2_bit(print: &Map<String, Value>, bit: u32) -> Option<bool>`, `const DRY_BLOCK_REASONS: [(&str, &str); 9]` (stable code, human message), `fn unit_drying_active(minutes: Option<u32>, status: DryingStatus) -> bool`.
- JSON shape (camelCase): `"drying": {"status":"drying","active":true,"minutesRemaining":661,"setting":{"filament":"PLA","temperatureC":45,"hours":12},"blockedReasons":["already_drying"],"controllable":true}`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `transport.rs`:

```rust
    // Sanitized from a 2026-09-19 dakota capture (H2-class, fw 01.02.00.00).
    fn h2_drying_print() -> Value {
        json!({
            "fun2": "2011FF",
            "ams": {"ams": [
                {"id": "0", "info": "10942023", "dry_time": 661, "humidity": "1", "temp": "44.7",
                 "dry_setting": {"dry_duration": 12, "dry_filament": "PLA", "dry_temperature": 45},
                 "dry_sf_reason": [6], "tray": []},
                {"id": "128", "info": "11942124", "dry_time": 661, "temp": "45.0",
                 "dry_setting": {"dry_duration": -1, "dry_filament": "", "dry_temperature": -1},
                 "dry_sf_reason": [], "tray": []}
            ]}
        })
    }

    #[test]
    fn decodes_an_active_remote_capable_drying_cycle() {
        let print = h2_drying_print();
        let units = ams_data(print.as_object().unwrap()).unwrap().units;

        let drying = units[0].drying.as_ref().unwrap();
        assert_eq!(drying.status, DryingStatus::Drying);
        assert!(drying.active);
        assert_eq!(drying.minutes_remaining, Some(661));
        assert_eq!(
            drying.setting,
            Some(DryingSetting { filament: Some("PLA".into()), temperature_c: 45, hours: 12 })
        );
        assert_eq!(drying.blocked_reasons, vec!["already_drying"]);
        assert!(drying.controllable);
        // `-1` sentinels mean "no setting reported", not a 65535 °C target.
        assert_eq!(units[1].drying.as_ref().unwrap().setting, None);
    }

    #[test]
    fn drying_state_table() {
        // (unit json, fun2, expected status, active, controllable)
        let cases = [
            // georgia: P1-class firmware, drying started on screen, no fun2.
            (json!({"id": "128", "info": "142024", "dry_time": 661}), None, DryingStatus::Drying, true, false),
            // alaska: heater-equipped HT unit, idle.
            (json!({"id": "128", "info": "2004", "dry_time": 0}), None, DryingStatus::Off, false, false),
            // Cooling phase: dry_time already 0 but the cycle is still live.
            (json!({"id": "0", "info": "30", "dry_time": 0}), Some("20"), DryingStatus::Cooling, true, true),
            // Unparseable info with time left: still active, phase unknown.
            (json!({"id": "0", "info": "zz", "dry_time": 5}), Some("20"), DryingStatus::Unknown, true, true),
            // HeatOutOfControl is a fault, never an "expected heat" live cycle.
            (json!({"id": "0", "info": "60", "dry_time": 0}), Some("20"), DryingStatus::HeatOutOfControl, false, true),
        ];
        for (unit, fun2, status, active, controllable) in cases {
            let mut print = json!({"ams": {"ams": [unit.clone()]}});
            if let Some(fun2) = fun2 {
                print["fun2"] = json!(fun2);
            }
            let units = ams_data(print.as_object().unwrap()).unwrap().units;
            let drying = units[0].drying.as_ref().unwrap_or_else(|| panic!("{unit}"));
            assert_eq!(drying.status, status, "{unit}");
            assert_eq!(drying.active, active, "{unit}");
            assert_eq!(drying.controllable, controllable, "{unit}");
        }
    }

    #[test]
    fn units_without_a_heater_report_no_drying_block() {
        // Classic AMS and AMS Lite never send dry_time.
        let print = json!({"fun2": "20", "ams": {"ams": [{"id": "0", "info": "1001", "tray": []}]}});
        let units = ams_data(print.as_object().unwrap()).unwrap().units;
        assert_eq!(units[0].drying, None);
    }

    #[test]
    fn unknown_block_reason_codes_are_kept_as_unknown() {
        let print = json!({"fun2": "20", "ams": {"ams": [
            {"id": "0", "info": "0", "dry_time": 0, "dry_sf_reason": [3, 99]}
        ]}});
        let units = ams_data(print.as_object().unwrap()).unwrap().units;
        assert_eq!(
            units[0].drying.as_ref().unwrap().blocked_reasons,
            vec!["filament_at_outlet", "unknown"]
        );
    }
```

Add `DryingSetting, DryingStatus` to the test module's `use` of `crate::moonraker` items (look at the existing `use super::*;` and the import list at `transport.rs:17`).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p melt-core drying`
Expected: compile error, `DryingStatus` / field `drying` not found.

- [ ] **Step 3: Add the types to `moonraker.rs`**

Insert directly after `pub struct AmsUnit { … }` and add `drying` to `AmsUnit`:

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmsUnit {
    pub id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub humidity_range: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub humidity_level: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drying: Option<AmsDrying>,
    pub trays: Vec<AmsTray>,
}

/// Drying phase from bits 4-7 of the AMS `info` word (Bambu Studio
/// `DevFilaSystem`). `PrdTesting` (7) and future values read as `Unknown`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DryingStatus {
    Off,
    Checking,
    Drying,
    Cooling,
    Stopping,
    Error,
    HeatOutOfControl,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DryingSetting {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filament: Option<String>,
    pub temperature_c: u16,
    pub hours: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AmsDrying {
    pub status: DryingStatus,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes_remaining: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setting: Option<DryingSetting>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked_reasons: Vec<&'static str>,
    pub controllable: bool,
}
```

Find every `AmsUnit { … }` literal with `rg -n "AmsUnit \{" crates` (at least `ams_unit` and `virtual_tray_units` in `transport.rs`) and add `drying: None` to each one except `ams_unit`, which Step 4 handles. External spools have no heater, so `drying: None` is correct there.

- [ ] **Step 4: Implement the parse in `transport.rs`**

Replace `has_emmc` with a shared bit reader and keep its behavior:

```rust
/// `fun2` is a hex capability mask of arbitrary length.
fn fun2_bit(print: &Map<String, Value>, bit: u32) -> Option<bool> {
    let raw = string(print.get("fun2")).filter(|value| !value.is_empty())?;
    let bits = u128::from_str_radix(raw.trim(), 16).ok()?;
    Some(bits & (1 << bit) != 0)
}

/// eMMC support is advertised by bit 17 of the hex `fun2` capability mask.
fn has_emmc(print: &Map<String, Value>) -> Option<bool> {
    fun2_bit(print, 17).filter(|supported| *supported)
}
```

(`has_emmc` returned `Some(true)` or `None`, never `Some(false)`. The `.filter` keeps that contract.)

Thread the remote-dry bit into `ams_unit`:

```rust
fn ams_data(print: &Map<String, Value>) -> Option<AmsData> {
    // Bit 5 is Bambu Studio's `is_support_remote_dry`. P1-class firmware omits
    // `fun2` and silently ignores the command, so absence means screen-only.
    let remote_dry = fun2_bit(print, 5).unwrap_or(false);
    let mut units: Vec<AmsUnit> = print
        .get("ams")
        .and_then(|ams| ams.get("ams"))
        .and_then(Value::as_array)
        .map(|entries| entries.iter().map(|unit| ams_unit(unit, remote_dry)).collect())
        .unwrap_or_default();
    units.extend(virtual_tray_units(print));
    (!units.is_empty()).then_some(AmsData { units })
}
```

Change `fn ams_unit(unit: &Value) -> AmsUnit` to `fn ams_unit(unit: &Value, remote_dry: bool) -> AmsUnit` and add `drying: ams_drying(unit, remote_dry),` to its literal. Then add, directly below `ams_humidity`:

```rust
/// `dry_sf_reason` codes: (stable identifier, message). Index = firmware code.
const DRY_BLOCK_REASONS: [(&str, &str); 9] = [
    ("printer_busy", "the printer is busy"),
    ("insufficient_power", "insufficient power: too many AMS units drying or an external PSU is required"),
    ("ams_busy", "the AMS is busy"),
    ("filament_at_outlet", "filament is at the AMS outlet; retract it first"),
    ("already_starting", "the AMS is already starting a drying cycle"),
    ("unsupported_in_2d_mode", "drying is not available in 2D mode"),
    ("already_drying", "the AMS is already drying"),
    ("firmware_upgrading", "the AMS firmware is upgrading"),
    ("external_power_required", "plug in the external AMS power adapter"),
];

fn drying_status(code: u64) -> DryingStatus {
    match code {
        0 => DryingStatus::Off,
        1 => DryingStatus::Checking,
        2 => DryingStatus::Drying,
        3 => DryingStatus::Cooling,
        4 => DryingStatus::Stopping,
        5 => DryingStatus::Error,
        6 => DryingStatus::HeatOutOfControl,
        _ => DryingStatus::Unknown,
    }
}

/// Bambuddy's rule: `dry_time` reads 0 through the closing cooling phase, so
/// the phase also counts. HeatOutOfControl is never "expected heat".
fn unit_drying_active(minutes: Option<u32>, status: DryingStatus) -> bool {
    minutes.is_some()
        || matches!(status, DryingStatus::Checking | DryingStatus::Drying | DryingStatus::Cooling)
}

/// Only heater-equipped units (AMS 2 Pro, AMS HT) report `dry_time`.
fn ams_drying(unit: &Value, remote_dry: bool) -> Option<AmsDrying> {
    let minutes = integer(unit.get("dry_time"))?;
    let minutes_remaining = u32::try_from(minutes).ok().filter(|value| *value > 0);
    let status = string(unit.get("info"))
        .and_then(|info| u64::from_str_radix(info.trim(), 16).ok())
        .map_or(DryingStatus::Unknown, |info| drying_status((info >> 4) & 0xF));
    let setting = unit.get("dry_setting").and_then(|setting| {
        Some(DryingSetting {
            filament: string(setting.get("dry_filament")).filter(|value| !value.is_empty()),
            temperature_c: integer(setting.get("dry_temperature"))
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| *value > 0)?,
            hours: integer(setting.get("dry_duration"))
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| *value > 0)?,
        })
    });
    let blocked_reasons = unit
        .get("dry_sf_reason")
        .and_then(Value::as_array)
        .map(|codes| {
            codes
                .iter()
                .map(|code| {
                    integer(Some(code))
                        .and_then(|code| usize::try_from(code).ok())
                        .and_then(|code| DRY_BLOCK_REASONS.get(code))
                        .map_or("unknown", |(identifier, _)| *identifier)
                })
                .collect()
        })
        .unwrap_or_default();
    Some(AmsDrying {
        status,
        active: unit_drying_active(minutes_remaining, status),
        minutes_remaining,
        setting,
        blocked_reasons,
        controllable: remote_dry,
    })
}
```

Import `AmsDrying, DryingSetting, DryingStatus` in the `use crate::moonraker::{…}` list at `transport.rs:17`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p melt-core` (the whole crate, because `has_emmc` changed)
Expected: PASS, including the existing eMMC tests (`rg -n "fun2" crates/melt-core/src/bambu/transport.rs` lists them).

- [ ] **Step 6: Lint and commit**

```bash
cargo fmt && cargo clippy --workspace --all-targets -- -D warnings
git add crates/melt-core/src/moonraker.rs crates/melt-core/src/bambu/transport.rs
git commit -m "feat(bambu): report AMS drying state per unit"
```

---

### Task 2: `ams_drying_set` mutation in the Bambu client

**Files:**
- Modify: `crates/melt-core/src/moonraker.rs` (next to `LightResult`, ~line 1529)
- Modify: `crates/melt-core/src/bambu/transport.rs`: `MutationClass` (~120), `Error` (~63), new `Client::ams_drying_set` after `speed_set` (~960), pure helpers next to `ams_drying`
- Test: `crates/melt-core/src/bambu/transport.rs` `mod tests`, next to `a_cold_client_resolves_authorization_before_it_mutates` (~6060)

**Interfaces:**
- Consumes: `AmsUnit.drying`, `unit_drying_active`, `drying_status`, `DRY_BLOCK_REASONS` (Task 1).
- Produces:
  ```rust
  // moonraker.rs
  #[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
  #[serde(tag = "action", rename_all = "camelCase")]
  pub enum DryingRequest {
      #[serde(rename_all = "camelCase")]
      Start { temperature_c: u16, hours: u16, filament: Option<String> },
      Stop,
  }
  #[derive(Clone, Debug, Eq, PartialEq, Serialize)]
  #[serde(rename_all = "camelCase")]
  pub struct DryingResult { pub ams_id: u32, pub active: bool }

  // transport.rs
  MutationClass::Drying
  Error::InvalidDryingRequest(&'static str)   // "invalid AMS drying request: {0}"
  Error::DryingBlocked(&'static str)          // "AMS drying is blocked: {0}"
  impl Client { pub fn ams_drying_set(&self, access_code: Option<&str>, fingerprint: Option<&str>, ams_id: u32, request: &DryingRequest) -> Result<DryingResult, Error> }
  ```

- [ ] **Step 1: Write the failing pure-function tests**

```rust
    #[test]
    fn drying_start_validation_table() {
        let start = |temperature_c, hours| DryingRequest::Start { temperature_c, hours, filament: None };
        // (ams_id, request, ok)
        let cases = [
            (0, start(45, 12), true),
            (0, start(65, 1), true),
            (0, start(66, 12), false),  // AMS 2 Pro max 65 °C
            (128, start(85, 24), true), // AMS HT max 85 °C
            (128, start(44, 12), false),
            (128, start(60, 0), false),
            (128, start(60, 25), false),
            (254, start(45, 1), false), // external spool has no heater
            (0, DryingRequest::Stop, true),
        ];
        for (ams_id, request, ok) in cases {
            assert_eq!(validate_drying_request(ams_id, &request).is_ok(), ok, "{ams_id} {request:?}");
        }
    }

    #[test]
    fn drying_filament_falls_back_to_the_loaded_tray_then_pla() {
        let loaded = AmsUnit {
            trays: vec![AmsTray { filament_type: Some("PETG".into()), ..AmsTray::default() }],
            ..AmsUnit::default()
        };
        assert_eq!(drying_filament(None, &loaded).unwrap(), "PETG");
        assert_eq!(drying_filament(Some("abs"), &loaded).unwrap(), "ABS");
        assert_eq!(drying_filament(None, &AmsUnit::default()).unwrap(), "PLA");
        assert!(drying_filament(Some("PLA\"}"), &loaded).is_err());
        assert!(drying_filament(Some(&"X".repeat(17)), &loaded).is_err());
    }

    #[test]
    fn drying_payloads_match_the_reference_wire_shape() {
        let start: Value = serde_json::from_str(&drying_payload(128, Some((60, 8, "PETG")))).unwrap();
        let print = &start["print"];
        assert_eq!(print["command"], "ams_filament_drying");
        assert_eq!(print["ams_id"], 128);
        assert_eq!(print["mode"], 1);
        assert_eq!(print["temp"], 60);
        assert_eq!(print["duration"], 8);
        assert_eq!(print["filament"], "PETG");
        assert_eq!(print["cooling_temp"], 20);
        assert_eq!(print["humidity"], 0);
        assert_eq!(print["rotate_tray"], false);
        assert_eq!(print["close_power_conflict"], false);
        assert!(print["sequence_id"].is_string());

        let stop: Value = serde_json::from_str(&drying_payload(0, None)).unwrap();
        assert_eq!(stop["print"]["mode"], 0);
        assert_eq!(stop["print"]["temp"], 0);
        assert_eq!(stop["print"]["duration"], 0);
        assert_eq!(stop["print"]["cooling_temp"], 0);
        assert_eq!(stop["print"]["filament"], "");
    }

    #[test]
    fn drying_predicate_reads_the_targeted_unit() {
        let report = |id: &str, info: &str, minutes: i64| {
            json!({"print": {"gcode_state": "IDLE", "ams": {"ams": [
                {"id": "1", "info": "20", "dry_time": 300},
                {"id": id, "info": info, "dry_time": minutes}
            ]}}})
        };
        assert!(drying_reached(&report("0", "10", 0), 0, true)); // Checking counts as started
        assert!(drying_reached(&report("0", "0", 720), 0, true));
        assert!(!drying_reached(&report("0", "0", 0), 0, true));
        assert!(drying_reached(&report("0", "40", 0), 0, false)); // Stopping
        assert!(drying_reached(&report("0", "0", 0), 0, false));
        assert!(!drying_reached(&report("0", "20", 100), 0, false));
        // Unit 1 is drying, but unit 0 is the target.
        assert!(!drying_reached(&report("0", "0", 0), 0, true));
        assert!(!drying_reached(&json!({"print": {}}), 0, true));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p melt-core drying_`
Expected: compile error, `validate_drying_request` / `DryingRequest` not found.

- [ ] **Step 3: Add types, error variants, and pure helpers**

In `moonraker.rs` next to `LightResult`, add `DryingRequest` and `DryingResult` exactly as in **Interfaces**. Check that `Deserialize` is already imported there (`rg -n "^use serde" crates/melt-core/src/moonraker.rs`).

In `transport.rs`:
- add `Drying,` to `MutationClass`
- add to `Error`:
  ```rust
    #[error("invalid AMS drying request: {0}")]
    InvalidDryingRequest(&'static str),
    #[error("AMS drying is blocked: {0}")]
    DryingBlocked(&'static str),
  ```
- then add the helpers below `ams_drying`:

```rust
// ponytail: bounds by unit id range (0-3 AMS 2 Pro, 128-135 AMS HT, per
// Bambuddy). The firmware still rejects out-of-range targets; read module
// names (n3f/n3s) from the firmware inventory if a new unit type breaks this.
fn drying_temperature_bounds(ams_id: u32) -> Option<std::ops::RangeInclusive<u16>> {
    match ams_id {
        0..=3 => Some(45..=65),
        128..=135 => Some(45..=85),
        _ => None,
    }
}

fn validate_drying_request(ams_id: u32, request: &DryingRequest) -> Result<(), Error> {
    let bounds = drying_temperature_bounds(ams_id)
        .ok_or(Error::Unsupported("drying on this AMS unit"))?;
    let DryingRequest::Start { temperature_c, hours, .. } = request else {
        return Ok(());
    };
    if !bounds.contains(temperature_c) {
        return Err(Error::InvalidDryingRequest("temperature outside this AMS unit's range"));
    }
    if !(1..=24).contains(hours) {
        return Err(Error::InvalidDryingRequest("duration must be 1-24 hours"));
    }
    Ok(())
}

/// The printer rejects a start with no filament type.
fn drying_filament(requested: Option<&str>, unit: &AmsUnit) -> Result<String, Error> {
    let filament = requested
        .map(str::to_owned)
        .or_else(|| unit.trays.iter().find_map(|tray| tray.filament_type.clone()))
        .unwrap_or_else(|| "PLA".into())
        .to_ascii_uppercase();
    if filament.is_empty()
        || filament.len() > 16
        || !filament.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'+' | b' '))
    {
        return Err(Error::InvalidDryingRequest("invalid filament type"));
    }
    Ok(filament)
}

/// `start` = `(temperature °C, hours, filament)`; `None` stops the cycle.
fn drying_payload(ams_id: u32, start: Option<(u16, u16, &str)>) -> String {
    let (mode, temp, duration, cooling_temp, filament) = match start {
        Some((temp, hours, filament)) => (1, temp, hours, 20, filament),
        None => (0, 0, 0, 0, ""),
    };
    json!({
        "print": {
            "sequence_id": next_sequence_id(),
            "command": "ams_filament_drying",
            "ams_id": ams_id,
            "mode": mode,
            "filament": filament,
            "temp": temp,
            "duration": duration,
            "humidity": 0,
            "rotate_tray": false,
            "cooling_temp": cooling_temp,
            "close_power_conflict": false,
        }
    })
    .to_string()
}

/// Start is proven once the unit leaves Off (Checking counts); stop once it is
/// neither checking nor drying and has no time left.
fn drying_reached(report: &Value, ams_id: u32, start: bool) -> bool {
    let Some(unit) = report
        .pointer("/print/ams/ams")
        .and_then(Value::as_array)
        .and_then(|units| {
            units.iter().find(|unit| integer(unit.get("id")) == Some(i64::from(ams_id)))
        })
    else {
        return false;
    };
    let Some(drying) = ams_drying(unit, false) else {
        return false;
    };
    let running = matches!(drying.status, DryingStatus::Checking | DryingStatus::Drying);
    if start {
        running || drying.minutes_remaining.is_some()
    } else {
        !running && drying.minutes_remaining.is_none()
    }
}
```

Check that `integer()` parses the string `"0"`. `ams_unit` already calls `integer(unit.get("id"))` on string ids, so it does.

- [ ] **Step 4: Run the pure tests**

Run: `cargo test -p melt-core drying_`
Expected: PASS.

- [ ] **Step 5: Write the failing client tests (fake broker)**

Add next to `a_cold_client_resolves_authorization_before_it_mutates`. Reuse `test_acceptor`, `read_test_packet`, `write_test_report`, `mqtt_publish_payload`, `is_pushall_payload`, `Profile::with_timeout`, `open_tls`, `MqttConnection`, `CachedConnection` exactly as that test does:

```rust
    fn drying_test_client(port: u16, host: String) -> Client {
        let profile = Profile::with_timeout(host, "SN001", true, Duration::from_secs(5)).unwrap();
        let connector = tls_connector().unwrap();
        let deadline = deadline_after(profile.timeout()).unwrap();
        let (stream, _) = open_tls(&connector, &profile, port, None, false, deadline).unwrap();
        let mut mqtt = MqttConnection::new(stream, profile.mqtt_topics(), deadline);
        mqtt.connect("access-code").unwrap();
        mqtt.subscribe().unwrap();
        let identity = profile.connection_identity(Some("access-code"), None);
        let client = Client::new(profile);
        *client.mqtt.lock().unwrap() = Some(CachedConnection { identity, mqtt });
        client
    }

    /// Answers every pushall with `idle` until the drying command arrives,
    /// acks it, then answers the next pushall with `after`. Returns the command.
    fn drying_broker(fun2: &'static str, after: Value) -> (String, u16, thread::JoinHandle<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let acceptor = test_acceptor();
        let server = thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let mut stream = acceptor.accept(socket).unwrap();
            let connect = read_test_packet(&mut stream);
            assert_eq!(connect.kind, 0x10);
            stream.write_all(&[0x20, 0x02, 0, 0]).unwrap();
            let subscribe = read_test_packet(&mut stream);
            let id = &subscribe.payload[..2];
            stream.write_all(&[0x90, 0x03, id[0], id[1], 0]).unwrap();
            stream.flush().unwrap();
            let idle = json!({"print": {"gcode_state": "IDLE", "mc_percent": 0, "fun2": fun2,
                "ams": {"ams": [{"id": "0", "info": "3", "dry_time": 0, "dry_sf_reason": [],
                    "tray": [{"id": "0", "tray_type": "PETG"}]}]}}});
            let mut command: Option<Value> = None;
            loop {
                let packet = read_test_packet(&mut stream);
                if packet.kind >> 4 != 3 {
                    continue;
                }
                let payload = mqtt_publish_payload(packet.kind, &packet.payload).unwrap();
                if is_pushall_payload(&payload) {
                    let done = command.is_some();
                    write_test_report(&mut stream, if done { &after } else { &idle });
                    if done {
                        return command.unwrap();
                    }
                    continue;
                }
                let value: Value = serde_json::from_slice(&payload).unwrap();
                write_test_report(&mut stream, &json!({"print": {
                    "sequence_id": value["print"]["sequence_id"],
                    "command": "ams_filament_drying", "result": "success"}}));
                command = Some(value);
            }
        });
        (address.ip().to_string(), address.port(), server)
    }

    #[test]
    fn starting_a_drying_cycle_publishes_the_command_and_waits_for_the_phase() {
        let drying = json!({"print": {"gcode_state": "IDLE", "mc_percent": 0, "fun2": "20",
            "ams": {"ams": [{"id": "0", "info": "23", "dry_time": 480, "tray": []}]}}});
        let (host, port, server) = drying_broker("20", drying);
        let client = drying_test_client(port, host);

        let result = client
            .ams_drying_set(Some("access-code"), None, 0,
                &DryingRequest::Start { temperature_c: 60, hours: 8, filament: None })
            .unwrap();

        assert_eq!(result, DryingResult { ams_id: 0, active: true });
        let command = server.join().unwrap();
        assert_eq!(command["print"]["filament"], "PETG"); // loaded tray fallback
        assert_eq!(command["print"]["temp"], 60);
        assert_eq!(command["print"]["duration"], 8);
    }

    #[test]
    fn screen_only_printers_are_never_sent_a_drying_command() {
        // No fun2 bit 5: P1-class firmware acks and ignores the command.
        let listener_result = {
            let (host, port, _server) = drying_broker("0", json!({}));
            let client = drying_test_client(port, host);
            client.ams_drying_set(Some("access-code"), None, 0,
                &DryingRequest::Start { temperature_c: 45, hours: 4, filament: None })
        };
        assert!(matches!(listener_result, Err(Error::Unsupported(_))));
        // The broker thread is left blocked on read. It is dropped when the
        // test process exits, the same as the other tests' unjoined servers.
    }
```

The second test leaves a thread blocked. If the harness complains, restructure `drying_broker` so the server exits when `read_test_packet` sees EOF: wrap the read in a `take_mqtt_packet`/`read` loop that returns `None` on `read == 0`, then `drop(client)` before joining. Keep that change local to this helper.

Also add the authorization case, mirroring the existing cold-client test's report with `"security": {"signing_required": true}`:

```rust
    #[test]
    fn drying_respects_the_signing_gate() {
        let profile =
            Profile::with_timeout("203.0.113.1", "SN001", true, Duration::from_millis(50)).unwrap();
        let client = Client::new(profile);
        let mut capabilities = RuntimeCapabilities::for_model("H2D");
        capabilities.authorization = AuthorizationMode::SigningRequired;
        *client.capabilities.lock().unwrap() = Some(capabilities);

        assert!(matches!(
            client.ams_drying_set(Some("code"), None, 0, &DryingRequest::Stop),
            Err(Error::AuthorizationRequired(MutationClass::Drying))
        ));
    }
```

- [ ] **Step 6: Run to verify failure**

Run: `cargo test -p melt-core drying`
Expected: compile error, `ams_drying_set` not found.

- [ ] **Step 7: Implement `Client::ams_drying_set`**

Place after `speed_set`:

```rust
    pub fn ams_drying_set(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        ams_id: u32,
        request: &DryingRequest,
    ) -> Result<DryingResult, Error> {
        self.authorize_mutation(access_code, fingerprint, MutationClass::Drying)?;
        validate_drying_request(ams_id, request)?;
        let status = self.status(access_code, fingerprint)?;
        let unit = status
            .extensions
            .bambu_lan
            .as_ref()
            .and_then(|extension| extension.ams.as_ref())
            .and_then(|ams| ams.units.iter().find(|unit| unit.id == ams_id))
            .ok_or(Error::Unsupported("requested AMS unit on this printer"))?;
        let drying = unit
            .drying
            .as_ref()
            .ok_or(Error::Unsupported("drying on this AMS unit"))?;
        if !drying.controllable {
            return Err(Error::Unsupported(
                "remote AMS drying on this printer; start it from the printer screen",
            ));
        }
        let start = match request {
            DryingRequest::Stop if !drying.active => {
                return Ok(DryingResult { ams_id, active: false });
            }
            DryingRequest::Stop => None,
            DryingRequest::Start { temperature_c, hours, filament } => {
                if let Some(code) = drying.blocked_reasons.first() {
                    let message = DRY_BLOCK_REASONS
                        .iter()
                        .find(|(identifier, _)| identifier == code)
                        .map_or("the printer refused to start drying", |(_, message)| *message);
                    return Err(Error::DryingBlocked(message));
                }
                Some((*temperature_c, *hours, drying_filament(filament.as_deref(), unit)?))
            }
        };
        let payload = drying_payload(
            ams_id,
            start.as_ref().map(|(temp, hours, filament)| (*temp, *hours, filament.as_str())),
        );
        let is_start = start.is_some();
        self.exchange(access_code, fingerprint, payload, |report| {
            drying_reached(report, ams_id, is_start)
        })?;
        Ok(DryingResult { ams_id, active: is_start })
    }
```

Import `DryingRequest, DryingResult` in the `crate::moonraker` import list.

`validate_drying_request` runs before the status fetch, so invalid input never touches the network. `authorize_mutation` runs first so the signing test fails fast, the same as `fan_set`.

- [ ] **Step 8: Run all core tests**

Run: `cargo test -p melt-core`
Expected: PASS.

- [ ] **Step 9: Lint and commit**

```bash
cargo fmt && cargo clippy --workspace --all-targets -- -D warnings
git add crates/melt-core/src/moonraker.rs crates/melt-core/src/bambu/transport.rs
git commit -m "feat(bambu): start and stop AMS drying when firmware allows remote drying"
```

---

### Task 3: Driver, capability, and pool plumbing

**Files:**
- Modify: `crates/melt-core/src/drivers.rs`: `Capabilities` (~31), `supports` (~55), `Driver::capabilities` (~104), `Operation` (~157), new `ams_drying_set` after `light_set` (~653), test next to `rejects_light_control_for_moonraker_without_connecting` (~822)
- Modify: `crates/melt-core/src/pool.rs`: new method after `light_set` (~372)

**Interfaces:**
- Consumes: `bambu::Client::ams_drying_set`, `moonraker::{DryingRequest, DryingResult}`.
- Produces:
  ```rust
  Operation::AmsDrying
  Capabilities { …, pub ams_drying: bool }            // JSON "amsDrying"
  pub fn drivers::ams_drying_set(profile: &Profile, access_code: Option<&str>, tls_fingerprint: Option<&str>, ams_id: u32, request: &moonraker::DryingRequest) -> Result<moonraker::DryingResult, DriverError>
  impl ConnectionPool { pub fn ams_drying_set(&self, name: &str, profile: &drivers::Profile, access_code: Option<&str>, tls_fingerprint: Option<&str>, ams_id: u32, request: &moonraker::DryingRequest) -> Result<moonraker::DryingResult, drivers::DriverError> }
  ```

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn rejects_ams_drying_for_moonraker_without_connecting() {
        let profile = Profile::Moonraker(
            moonraker::Profile::new("printer.local", false, Duration::from_secs(1)).unwrap(),
        );
        assert!(!Driver::Moonraker.supports(Operation::AmsDrying));
        assert!(Driver::BambuLan.supports(Operation::AmsDrying));
        assert!(matches!(
            ams_drying_set(&profile, None, None, 0, &moonraker::DryingRequest::Stop),
            Err(DriverError::UnsupportedOperation(Driver::Moonraker, Operation::AmsDrying))
        ));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p melt-core rejects_ams_drying`
Expected: compile error.

- [ ] **Step 3: Implement**

- `Capabilities`: add `pub ams_drying: bool,` after `speed_control`.
- `supports`: add `Operation::AmsDrying => self.ams_drying,`.
- `Driver::capabilities`: `ams_drying: true` for `BambuLan`. Moonraker already fills the rest with `..Default::default()` or explicit fields; check which and add `ams_drying: false` if its fields are explicit.
- `Operation`: add `AmsDrying,` after `SpeedSet`. Fix any exhaustive `match` on `Operation` that the compiler flags (e.g. a display/name function). For a name string, use `"ams drying"`.
- Function after `light_set`:

```rust
pub fn ams_drying_set(
    profile: &Profile,
    access_code: Option<&str>,
    tls_fingerprint: Option<&str>,
    ams_id: u32,
    request: &moonraker::DryingRequest,
) -> Result<moonraker::DryingResult, DriverError> {
    match profile {
        Profile::Bambu(profile) => bambu::Client::new(profile.clone())
            .ams_drying_set(access_code, tls_fingerprint, ams_id, request)
            .map_err(DriverError::Bambu),
        Profile::Moonraker(_) => Err(DriverError::UnsupportedOperation(
            Driver::Moonraker,
            Operation::AmsDrying,
        )),
    }
}
```

- `pool.rs`, after `light_set`:

```rust
    pub fn ams_drying_set(
        &self,
        name: &str,
        profile: &drivers::Profile,
        access_code: Option<&str>,
        tls_fingerprint: Option<&str>,
        ams_id: u32,
        request: &moonraker::DryingRequest,
    ) -> Result<moonraker::DryingResult, drivers::DriverError> {
        let drivers::Profile::Bambu(profile) = profile else {
            return Err(drivers::DriverError::UnsupportedOperation(
                drivers::Driver::Moonraker,
                drivers::Operation::AmsDrying,
            ));
        };
        self.with_bambu(name, profile, access_code, tls_fingerprint, |client| {
            client.ams_drying_set(access_code, tls_fingerprint, ams_id, request)
        })
    }
```

- [ ] **Step 4: Run all tests**

Run: `cargo test --workspace`
Expected: PASS. If a CLI test snapshot prints `capabilities`, it now includes `"amsDrying"`. Update only that expected value.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --workspace --all-targets -- -D warnings
git add crates/melt-core/src/drivers.rs crates/melt-core/src/pool.rs crates/melt-cli
git commit -m "feat(bambu): expose AMS drying through drivers and the connection pool"
```

---

### Task 4: CLI shows drying in detailed status

**Files:**
- Modify: `crates/melt-cli/src/lib.rs`: AMS block in the detailed status renderer (~1171–1186)
- Test: `crates/melt-cli/src/lib.rs` tests, next to `detailed_status_lists_airduct_fans_from_the_control_inventory` (~4860). Read that test first and copy how it builds a `Status` and calls the renderer.

**Interfaces:**
- Consumes: `AmsUnit.drying` (Task 1).
- Produces: a human line per unit, e.g. `  Unit 0 (humidity: < 10% [very dry], temp: 44.7 C, drying: 11h 1m left, PLA 45 C / 12h):`, plus `, drying blocked: …` only when not active. JSON output needs no change because it serializes `Status`.

- [ ] **Step 1: Write the failing test**

Build a `Status` whose `extensions.bambu_lan.ams` has one unit with `drying: Some(AmsDrying { status: DryingStatus::Drying, active: true, minutes_remaining: Some(661), setting: Some(DryingSetting { filament: Some("PLA".into()), temperature_c: 45, hours: 12 }), blocked_reasons: vec!["already_drying"], controllable: false })`. Build it the way the airduct test builds its status. Then:

```rust
    assert!(
        detailed.contains("  Unit 0 (drying: 11h 1m left, PLA 45 C / 12h):"),
        "{detailed}"
    );
    assert!(!detailed.contains("drying blocked"), "an active cycle hides its own already_drying reason");
```

Add a second unit with `active: false, status: DryingStatus::Off, minutes_remaining: None, setting: None, blocked_reasons: vec!["insufficient_power"]` and assert it contains `"(drying: off, drying blocked: insufficient_power)"`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p melt-cli detailed_status`
Expected: FAIL (line missing).

- [ ] **Step 3: Implement**

In the AMS loop, after the `temperature` part is pushed and before `suffix` is built:

```rust
            if let Some(drying) = &unit.drying {
                let mut text = match drying.minutes_remaining {
                    Some(minutes) => format!("drying: {}h {}m left", minutes / 60, minutes % 60),
                    None if drying.active => "drying: cooling".into(),
                    None => format!("drying: {}", drying_status_label(drying.status)),
                };
                if let Some(setting) = &drying.setting {
                    text.push_str(", ");
                    if let Some(filament) = &setting.filament {
                        text.push_str(&format!("{} ", sanitize(filament)));
                    }
                    text.push_str(&format!("{} C / {}h", setting.temperature_c, setting.hours));
                }
                parts.push(text);
                if !drying.active && !drying.blocked_reasons.is_empty() {
                    parts.push(format!("drying blocked: {}", drying.blocked_reasons.join(", ")));
                }
            }
```

and a helper next to `fan_display_name`:

```rust
fn drying_status_label(status: melt_core::moonraker::DryingStatus) -> &'static str {
    use melt_core::moonraker::DryingStatus;
    match status {
        DryingStatus::Off => "off",
        DryingStatus::Checking => "checking",
        DryingStatus::Drying => "drying",
        DryingStatus::Cooling => "cooling",
        DryingStatus::Stopping => "stopping",
        DryingStatus::Error => "error",
        DryingStatus::HeatOutOfControl => "heat out of control",
        DryingStatus::Unknown => "unknown",
    }
}
```

`sanitize` returns a `String`; adjust the `map` call to whatever its signature is (`rg -n "^fn sanitize" crates/melt-cli/src/lib.rs`).

- [ ] **Step 4: Run tests, commit**

```bash
cargo test -p melt-cli && cargo fmt && cargo clippy --workspace --all-targets -- -D warnings
git add crates/melt-cli/src/lib.rs
git commit -m "feat(cli): show AMS drying state in detailed status"
```

---

### Task 5: CLI `ams dry start|stop`

**Files:**
- Modify: `crates/melt-cli/src/lib.rs`: `LeafCommand` table (alphabetical; put before `camera`), root `CommandGroup` list (~220), new `CommandGroup`s for `["ams"]` and `["ams", "dry"]` (model them on `["printer", "tls"]` at ~316), dispatcher (~572), new `fn ams_dry`, `driver_error` (~4229)
- Modify: `fixtures/cli-contract/inventory.json`
- Modify: `crates/melt-cli/tests/contract_fixtures.rs:35` (`37` → `41`)
- Test: CLI unit tests near the existing `lights set` argument tests (`rg -n "lights set requires" crates/melt-cli/src/lib.rs`)

**Interfaces:**
- Consumes: `drivers::ams_drying_set`, `drivers::Operation::AmsDrying`, `Error::{InvalidDryingRequest, DryingBlocked}`.
- Produces:
  ```
  melt ams dry start <printer> <ams-id> --temperature <°C> --hours <h> [--filament <type>] [--yes --insecure --protocol-trace <path> --timeout <duration>]
  melt ams dry stop  <printer> <ams-id> [--yes --insecure --protocol-trace <path> --timeout <duration>]
  JSON data: {"profile","driver","amsId","active","warnings","capabilities"}
  ```

- [ ] **Step 1: Write the failing tests**

Model them on the existing argument-validation tests (read one first to copy the harness call, e.g. how it runs `run(&["lights","set",...])` against an empty config dir):

```rust
    #[test]
    fn ams_dry_start_requires_temperature_and_hours() {
        // (args, expected usage fragment)
        let cases: [(&[&str], &str); 4] = [
            (&["ams", "dry", "start", "dakota", "0"], "--temperature"),
            (&["ams", "dry", "start", "dakota", "0", "--temperature", "45"], "--hours"),
            (&["ams", "dry", "start", "dakota", "x", "--temperature", "45", "--hours", "4"], "AMS id"),
            (&["ams", "dry", "stop", "dakota"], "printer profile and AMS id"),
        ];
        for (args, fragment) in cases {
            let (code, stdout) = run_json(args); // use the existing JSON-run helper
            assert_eq!(code, 2, "{args:?}");
            assert!(stdout.contains(fragment), "{args:?}: {stdout}");
        }
    }
```

If no `run_json`-style helper exists, use the pattern those tests use (they call the public `run` entry with `Vec<String>` and in-memory `out`/`err` buffers).

Update `contract_fixtures.rs` `assert_eq!(commands.len(), 37)` → `41` and add these inventory rows (keep alphabetical order; `ams` sorts first):

```json
    {"path": "ams", "arguments": "", "flags": [], "implementation": "implemented"},
    {"path": "ams dry", "arguments": "", "flags": [], "implementation": "implemented"},
    {"path": "ams dry start", "arguments": "<printer> <ams-id>", "flags": ["--temperature <celsius>", "--hours <hours>", "--filament <type>", "--yes", "--insecure", "--protocol-trace <path>", "--timeout <duration>"], "implementation": "implemented"},
    {"path": "ams dry stop", "arguments": "<printer> <ams-id>", "flags": ["--yes", "--insecure", "--protocol-trace <path>", "--timeout <duration>"], "implementation": "implemented"},
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p melt-cli`
Expected: FAIL (unknown command, inventory count).

- [ ] **Step 3: Implement the command**

Add the leaf/group table entries. Copy the `flags` help string format from `lights set` and add `--temperature int`, `--hours int` and `--filament string` lines. Dispatcher arm:

```rust
        [ams, dry, action, rest @ ..]
            if ams.as_str() == "ams"
                && dry.as_str() == "dry"
                && matches!(action.as_str(), "start" | "stop") =>
        {
            ams_dry(invocation.format, action.as_str() == "start", rest, out, err)
        }
```

Handler (mirrors `lights_set` step for step):

```rust
fn ams_dry(
    format: OutputFormat,
    start: bool,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let command = if start { "ams dry start" } else { "ams dry stop" };
    let value_options: &[&str] = if start {
        &["timeout", "protocol-trace", "temperature", "hours", "filament"]
    } else {
        &["timeout", "protocol-trace"]
    };
    let (positionals, options) = match parse_options(args, &["yes", "insecure"], value_options) {
        Ok(value) => value,
        Err(error) => return write_error(command, format, AppError::usage(error), out, err),
    };
    let (name, ams_id) = match positionals.as_slice() {
        [name, ams_id] => (name.as_str(), ams_id.as_str()),
        _ => {
            return write_error(command, format,
                AppError::usage(format!("{command} requires a printer profile and AMS id")), out, err);
        }
    };
    let Ok(ams_id) = ams_id.parse::<u32>() else {
        return write_error(command, format,
            AppError::usage("AMS id must be a number (see `melt status <printer> --detailed`)"), out, err);
    };
    let request = if start {
        let number = |flag: &str| -> Result<u16, AppError> {
            options
                .value(flag)
                .ok_or_else(|| AppError::usage(format!("{command} requires --{flag}")))?
                .parse::<u16>()
                .map_err(|_| AppError::usage(format!("--{flag} must be a whole number")))
        };
        let (temperature_c, hours) = match (number("temperature"), number("hours")) {
            (Ok(temperature), Ok(hours)) => (temperature, hours),
            (Err(error), _) | (_, Err(error)) => return write_error(command, format, error, out, err),
        };
        melt_core::moonraker::DryingRequest::Start {
            temperature_c,
            hours,
            filament: options.value("filament").map(str::to_owned),
        }
    } else {
        melt_core::moonraker::DryingRequest::Stop
    };
    let connection = match connection_options(&options) {
        Ok(connection) => connection,
        Err(error) => return write_error(command, format, error, out, err),
    };
    let printer = match resolve_printer(name, connection, drivers::Operation::AmsDrying) {
        Ok(printer) => printer,
        Err(error) => return write_error(command, format, error, out, err),
    };
    let prompt = match &request {
        melt_core::moonraker::DryingRequest::Start { temperature_c, hours, .. } => format!(
            "Start drying AMS {ams_id} at {temperature_c} C for {hours}h on {}? Type 'yes' to continue: ",
            printer.name
        ),
        melt_core::moonraker::DryingRequest::Stop => format!(
            "Stop drying AMS {ams_id} on {}? Type 'yes' to continue: ",
            printer.name
        ),
    };
    if let Err(code) = require_confirmation(command, options.enabled("yes"), &prompt, format, out, err) {
        return code;
    }
    match drivers::ams_drying_set(
        &printer.driver,
        printer.access_code.as_deref(),
        printer.tls_fingerprint.as_deref(),
        ams_id,
        &request,
    ) {
        Ok(result) => write_success(
            command,
            format,
            AmsDryData {
                profile: printer.name,
                driver: printer.driver_kind.name(),
                ams_id: result.ams_id,
                active: result.active,
                warnings: Vec::new(),
                capabilities: printer.driver_kind.capabilities(),
            },
            |out| {
                if result.active {
                    writeln!(out, "AMS {} drying started.", result.ams_id)
                } else {
                    writeln!(out, "AMS {} drying stopped.", result.ams_id)
                }
            },
            out,
        ),
        Err(error) => write_error(command, format, driver_error(error), out, err),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AmsDryData {
    profile: String,
    driver: &'static str,
    ams_id: u32,
    active: bool,
    warnings: Vec<String>,
    capabilities: drivers::Capabilities,
}
```

No `require_state` call: the firmware arbitrates via `dry_sf_reason`, and the core preflight surfaces that reason.

In `driver_error`:
- add `| melt_core::bambu::TransportError::InvalidDryingRequest(_)` to the `AppError::usage(error.to_string())` arm
- add a new arm before the `DriverError::Bambu(_)` catch-all:

```rust
        DriverError::Bambu(melt_core::bambu::TransportError::DryingBlocked(_)) => AppError {
            exit_code: 2,
            code: "invalid_printer_state",
            message: error.to_string(),
        },
```

(`invalid_printer_state`/exit 2 is the same code `require_state` uses; check at `lib.rs:1506`.)

- [ ] **Step 4: Run tests, commit**

```bash
cargo test --workspace && cargo fmt && cargo clippy --workspace --all-targets -- -D warnings
./target/debug/melt ams dry start --help   # help text renders
git add crates/melt-cli fixtures/cli-contract/inventory.json
git commit -m "feat(cli): add ams dry start and stop commands"
```

---

### Task 6: Desktop Tauri command

**Files:**
- Modify: `crates/melt-desktop/src/main.rs`: request struct next to `LightRequest` (~454), command after `printer_light_set` (~2203), registration list next to `printer_light_set` (~3558)

**Interfaces:**
- Consumes: `ConnectionPool::ams_drying_set`, `Operation::AmsDrying`.
- Produces: Tauri command `printer_ams_drying_set` taking `{ request: { name: string, amsId: number, drying: { action: 'start', temperatureC, hours, filament? } | { action: 'stop' } } }` and returning `DryingResult` (`{ amsId, active }`).

- [ ] **Step 1: Implement**

(Tauri command glue has no unit-test seam in this crate. `printer_light_set` is untested the same way, so verification is the build plus Task 8.)

```rust
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AmsDryingRequest {
    name: String,
    ams_id: u32,
    drying: moonraker::DryingRequest,
}
```

```rust
#[tauri::command(async)]
fn printer_ams_drying_set(
    request: AmsDryingRequest,
    state: tauri::State<'_, MonitorState>,
) -> Result<moonraker::DryingResult, CommandError> {
    let (generation, printer) =
        pooled_desktop_printer(&state, &request.name, Operation::AmsDrying)?;
    let result = current_pool_operation(&state, generation, Operation::AmsDrying, |pool| {
        pool.ams_drying_set(
            &request.name,
            &printer.driver,
            printer.access_code.as_deref(),
            printer.tls_fingerprint.as_deref(),
            request.ams_id,
            &request.drying,
        )
    });
    invalidate(&state, &request.name);
    result
}
```

Add `printer_ams_drying_set,` to `generate_handler![…]` after `printer_light_set`. There's no `ensure_state`, same reason as the CLI (Task 5). If the compiler requires `Operation::AmsDrying` in a desktop-side `match` (e.g. error wording), add an arm for it.

- [ ] **Step 2: Build and commit**

```bash
cargo build -p melt-desktop && cargo clippy --workspace --all-targets -- -D warnings
git add crates/melt-desktop/src/main.rs
git commit -m "feat(desktop): add AMS drying command"
```

---

### Task 7: Desktop UI: drying badge and start/stop controls

**Files:**
- Create: `ui/src/drying.ts`, `ui/src/drying.test.ts`
- Modify: `ui/src/App.vue`: `Capabilities` type (~122), `AmsUnitStatus` (~197), `MaterialSystemView` (~351), `materialSystems` (~767), AMS card template (~2757), a new action next to `toggleLight` (~1537), the Phosphor import list
- Modify: `ui/src/i18n.ts`: message-key union + `en` + `pt-BR` blocks

**Interfaces:**
- Consumes: status JSON `extensions['bambu-lan'].ams.units[].drying` (Task 1), `capabilities.amsDrying` (Task 3), command `printer_ams_drying_set` (Task 6).
- Produces (`ui/src/drying.ts`):
  ```ts
  export type DryingStatus = 'off' | 'checking' | 'drying' | 'cooling' | 'stopping' | 'error' | 'heatOutOfControl' | 'unknown'
  export type AmsDrying = { status: DryingStatus; active: boolean; minutesRemaining?: number; setting?: { filament?: string; temperatureC: number; hours: number }; blockedReasons?: string[]; controllable: boolean }
  export function dryingBounds(unitId: number): { min: number; max: number } | undefined
  export function dryingDefaults(unitId: number, filament?: string): { temperatureC: number; hours: number; filament: string } | undefined
  ```

- [ ] **Step 1: Write the failing UI tests**

`ui/src/drying.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import { dryingBounds, dryingDefaults } from './drying'

describe('drying presets', () => {
  it('uses AMS 2 Pro limits for units 0-3 and AMS HT limits for 128-135', () => {
    expect(dryingBounds(0)).toEqual({ min: 45, max: 65 })
    expect(dryingBounds(128)).toEqual({ min: 45, max: 85 })
    expect(dryingBounds(254)).toBeUndefined()
  })

  it('prefills from the loaded filament per unit type', () => {
    expect(dryingDefaults(0, 'PLA')).toEqual({ temperatureC: 45, hours: 12, filament: 'PLA' })
    expect(dryingDefaults(128, 'ABS')).toEqual({ temperatureC: 80, hours: 8, filament: 'ABS' })
    expect(dryingDefaults(0, 'ABS')).toEqual({ temperatureC: 65, hours: 12, filament: 'ABS' })
  })

  it('normalizes variants and falls back to PLA', () => {
    expect(dryingDefaults(128, 'PETG HF')?.temperatureC).toBe(65)
    expect(dryingDefaults(128, 'PA6-CF')?.filament).toBe('PA')
    expect(dryingDefaults(0, undefined)?.filament).toBe('PLA')
    expect(dryingDefaults(0, 'Mystery')?.filament).toBe('PLA')
  })
})
```

- [ ] **Step 2: Run to verify failure**

Run: `cd ui && bun run test drying`
Expected: FAIL (module not found).

- [ ] **Step 3: Implement `ui/src/drying.ts`**

```ts
// Defaults from Bambuddy's print_scheduler presets: [°C, hours] for AMS 2 Pro
// (n3f, units 0-3) and AMS HT (n3s, units 128-135). The firmware owns the
// real limits; these only prefill the form.
export type DryingStatus = 'off' | 'checking' | 'drying' | 'cooling' | 'stopping' | 'error' | 'heatOutOfControl' | 'unknown'

export type AmsDrying = {
  status: DryingStatus
  active: boolean
  minutesRemaining?: number
  setting?: { filament?: string; temperatureC: number; hours: number }
  blockedReasons?: string[]
  controllable: boolean
}

const PRESETS: Record<string, { pro: [number, number]; ht: [number, number] }> = {
  PLA: { pro: [45, 12], ht: [45, 12] },
  PETG: { pro: [65, 12], ht: [65, 12] },
  TPU: { pro: [65, 12], ht: [75, 18] },
  ABS: { pro: [65, 12], ht: [80, 8] },
  ASA: { pro: [65, 12], ht: [80, 8] },
  PA: { pro: [65, 12], ht: [85, 12] },
  PC: { pro: [65, 12], ht: [80, 8] },
  PVA: { pro: [65, 12], ht: [85, 18] },
}
const ALIASES: Record<string, string> = { NYLON: 'PA', PA6: 'PA', PAHT: 'PA' }

export function dryingBounds(unitId: number): { min: number; max: number } | undefined {
  if (unitId >= 0 && unitId <= 3) return { min: 45, max: 65 }
  if (unitId >= 128 && unitId <= 135) return { min: 45, max: 85 }
  return undefined
}

export function dryingDefaults(unitId: number, filament?: string) {
  if (!dryingBounds(unitId)) return undefined
  const raw = (filament ?? '').split(' ')[0].toUpperCase()
  const key = [raw, raw.split('-')[0]].map((candidate) => ALIASES[candidate] ?? candidate).find((candidate) => PRESETS[candidate]) ?? 'PLA'
  const [temperatureC, hours] = PRESETS[key][unitId >= 128 ? 'ht' : 'pro']
  return { temperatureC, hours, filament: key }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cd ui && bun run test drying`
Expected: PASS.

- [ ] **Step 5: Wire App.vue**

1. `import { dryingBounds, dryingDefaults, type AmsDrying } from './drying'` and `import { formatDuration } from './formatting'` (skip the second import if it's already there).
2. `Capabilities` type: add `amsDrying?: boolean`.
3. `AmsUnitStatus`: add `drying?: AmsDrying`.
4. `MaterialSystemView`: add `unitId: number; drying?: AmsDrying; firstFilament?: string`. In `materialSystems`, set `unitId: unit.id, drying: unit.drying, firstFilament: unit.trays.find((tray) => tray.filamentType)?.filamentType`.
5. State and actions next to `toggleLight`:

```ts
const dryingForm = ref<{ unitId: number; temperatureC: number; hours: number; filament: string } | undefined>()
const dryingBusy = ref(false)

function openDryingForm(system: MaterialSystemView) {
  const defaults = dryingDefaults(system.unitId, system.firstFilament)
  if (defaults) dryingForm.value = { unitId: system.unitId, ...defaults }
}

function setDrying(system: MaterialSystemView, start: boolean) {
  if (!activePrinter.value) return
  const name = activePrinter.value.name
  const form = dryingForm.value
  const drying = start && form
    ? { action: 'start', temperatureC: form.temperatureC, hours: form.hours, filament: form.filament }
    : { action: 'stop' }
  askConfirmation(
    start
      ? { title: 'drying.startTitle', description: 'drying.startDescription', confirm: 'drying.startConfirm' }
      : { title: 'drying.stopTitle', description: 'drying.stopDescription', confirm: 'drying.stopConfirm' },
    { unit: system.name, temperature: String(form?.temperatureC ?? ''), hours: String(form?.hours ?? '') },
    async () => {
      dryingBusy.value = true
      try {
        await invoke('printer_ams_drying_set', { request: { name, amsId: system.unitId, drying } })
        dryingForm.value = undefined
        showToast(t(start ? 'drying.started' : 'drying.stopped', { unit: system.name }))
      } catch (reason) {
        showToast(message(reason), 'error')
      } finally {
        dryingBusy.value = false
        void refreshMonitoring()
      }
    },
  )
}
```

6. Template, inside the per-system header row of the AMS card (next to the humidity/temperature spans at ~2761), add a badge when `system.drying?.active`:

```vue
<span v-if="system.drying?.active" class="flex items-center gap-1 text-orange-600 dark:text-orange-400">
  <PhSun class="size-4" aria-hidden="true" />
  <span>{{ system.drying.minutesRemaining ? t('drying.remaining', { time: formatDuration(system.drying.minutesRemaining * 60) }) : t(`drying.status.${system.drying.status}` as MessageKey) }}</span>
  <span v-if="system.drying.setting" class="font-mono">{{ system.drying.setting.temperatureC }} °C</span>
</span>
```

   Below the slots `div`, add the control row. It's only shown when the driver and the unit both allow control:

```vue
<div v-if="capabilities?.amsDrying && system.drying" class="mt-3 flex flex-wrap items-center gap-2 text-xs">
  <template v-if="system.drying.controllable">
    <Button v-if="system.drying.active" :disabled="dryingBusy" @click="setDrying(system, false)">{{ t('drying.stop') }}</Button>
    <template v-else-if="dryingForm?.unitId === system.unitId">
      <label class="flex items-center gap-1">{{ t('drying.temperature') }}
        <input v-model.number="dryingForm.temperatureC" type="number" :min="dryingBounds(system.unitId)?.min" :max="dryingBounds(system.unitId)?.max" class="w-16 rounded border px-1 font-mono" /> °C</label>
      <label class="flex items-center gap-1">{{ t('drying.hours') }}
        <input v-model.number="dryingForm.hours" type="number" min="1" max="24" class="w-14 rounded border px-1 font-mono" /> h</label>
      <Button variant="primary" :disabled="dryingBusy" @click="setDrying(system, true)">{{ t('drying.start') }}</Button>
      <Button @click="dryingForm = undefined">{{ t('common.cancel') }}</Button>
    </template>
    <Button v-else :disabled="dryingBusy || Boolean(system.drying.blockedReasons?.length)" @click="openDryingForm(system)">{{ t('drying.dry') }}</Button>
    <span v-if="!system.drying.active && system.drying.blockedReasons?.length" class="text-gray-500 dark:text-gray-400">{{ t(`drying.blocked.${system.drying.blockedReasons[0]}` as MessageKey) }}</span>
  </template>
  <span v-else class="text-gray-500 dark:text-gray-400">{{ t('drying.screenOnly') }}</span>
</div>
```

   Match the input classes to the existing numeric inputs in the controls panel (`rg -n 'type="number"' ui/src/App.vue`) instead of the placeholder classes above. Add `PhSun` to the Phosphor import. Check that `common.cancel` exists (`rg -n '"common.cancel"' ui/src/i18n.ts`) and use its actual key.

7. `i18n.ts`: add these keys to the union and to both locales:

| Key | en | pt-BR |
|---|---|---|
| `drying.dry` | Dry… | Secar… |
| `drying.start` | Start drying | Iniciar secagem |
| `drying.stop` | Stop drying | Parar secagem |
| `drying.temperature` | Temperature | Temperatura |
| `drying.hours` | Duration | Duração |
| `drying.remaining` | Drying · {time} left | Secando · faltam {time} |
| `drying.started` | {unit}: drying started | {unit}: secagem iniciada |
| `drying.stopped` | {unit}: drying stopped | {unit}: secagem interrompida |
| `drying.startTitle` | Start drying? | Iniciar secagem? |
| `drying.startDescription` | {unit} will heat to {temperature} °C for {hours} h. | {unit} aquecerá a {temperature} °C por {hours} h. |
| `drying.startConfirm` | Start | Iniciar |
| `drying.stopTitle` | Stop drying? | Parar secagem? |
| `drying.stopDescription` | {unit} will stop its drying cycle. | {unit} interromperá o ciclo de secagem. |
| `drying.stopConfirm` | Stop | Parar |
| `drying.screenOnly` | Drying can only be started from the printer screen. | A secagem só pode ser iniciada pela tela da impressora. |
| `drying.status.off` / `checking` / `drying` / `cooling` / `stopping` / `error` / `heatOutOfControl` / `unknown` | Off / Checking / Drying / Cooling / Stopping / Error / Heat out of control / Unknown | Desligado / Verificando / Secando / Resfriando / Parando / Erro / Aquecimento fora de controle / Desconhecido |
| `drying.blocked.printer_busy` … `external_power_required`, `unknown` | Printer is busy · Insufficient power · AMS is busy · Retract filament from the AMS outlet first · Already starting · Not available in 2D mode · Already drying · AMS firmware is upgrading · Plug in the external AMS power adapter · Printer refused to start drying | Impressora ocupada · Energia insuficiente · AMS ocupado · Retraia o filamento da saída do AMS · Já iniciando · Indisponível no modo 2D · Já secando · Atualizando firmware do AMS · Conecte o adaptador de energia externo do AMS · A impressora recusou iniciar a secagem |

   If `i18n.test.ts` checks locale parity, it will catch a missing pt-BR entry.

- [ ] **Step 6: Verify and commit**

```bash
cd ui && bun run test && bun run build
cd .. && git add ui/src/drying.ts ui/src/drying.test.ts ui/src/App.vue ui/src/i18n.ts
git commit -m "feat(ui): show AMS drying state and start/stop controls"
```

---

### Task 8: Docs, then physical qualification

**Files:**
- Modify: `docs/bambu-compatibility.md`: new `## AMS drying` section after `## Temperature, fan, and light inventory`; bump any `Last-updated` line if the file has one.
- Modify: `README.md`: Features "Printer control" bullet and the CLI command list (`ams dry start|stop`).

- [ ] **Step 1: Write the docs section**

````markdown
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
(`mode` 0) pass the mutation authorization gate, refuse to start while the unit
reports a blocking reason, and complete only once the targeted unit's reported
phase confirms the transition. Start falls back to the first loaded tray's
filament type, then PLA, because the firmware rejects an empty type.
Temperature bounds are 45–65 °C for units 0–3 and 45–85 °C for units 128–135;
duration is 1–24 h. Humidity-target mode, tray rotation, and scheduled drying
are not implemented.
````

- [ ] **Step 2: Commit docs**

```bash
git add docs/bambu-compatibility.md README.md
git commit -m "docs(bambu): document AMS drying display and control"
```

- [ ] **Step 3: Physical qualification (manual; needs the user at the printers)**

Run against the real fleet, keeping the machines attended (README § Safety disclaimer):

```bash
make build
./target/debug/melt status dakota --detailed    # 3 units, "drying: …h …m left, PLA 45 C / 12h"
./target/debug/melt status georgia --detailed   # HT unit drying, no controls implied
./target/debug/melt status alaska --detailed    # HT unit "drying: off"
./target/debug/melt ams dry stop georgia 128 --yes --output json   # expect capability_unsupported, nothing sent
./target/debug/melt ams dry stop dakota 128 --yes --protocol-trace /tmp/dry-stop.jsonl
./target/debug/melt ams dry start dakota 128 --temperature 45 --hours 1 --yes --protocol-trace /tmp/dry-start.jsonl
./target/debug/melt ams dry stop dakota 128 --yes
```

Record what actually happened:
- Does the stop predicate complete, or time out because the firmware parks in Cooling/Stopping? If it times out, widen `drying_reached(…, start = false)` to accept `DryingStatus::Cooling` and add a test row.
- Does `dry_sf_reason` clear after a stop?
- Try `ams dry start dakota 0 …` while the HT unit is drying and note whether reason 1 (power) appears.

Add sanitized captures and results to the physical-printer matrix in `docs/bambu-compatibility.md`, following `docs/bambu-physical-qualification.md`.

---

## Self-review

- **Coverage:** display (T1 core, T4 CLI, T7 UI) · control (T2 core, T3 plumbing, T5 CLI, T6 desktop, T7 UI) · screen-only printers (T1 `controllable`, T2 gate + test, T7 hint) · blocked reasons (T1 parse, T2 preflight, T7 message) · authorization (T2 test) · docs (T8) · real-hardware check (T8).
- **Names are consistent across tasks:** `AmsDrying`, `DryingStatus`, `DryingSetting`, `DryingRequest::{Start{temperature_c,hours,filament},Stop}`, `DryingResult{ams_id,active}`, `ams_drying_set`, `Operation::AmsDrying`, `Capabilities.ams_drying`/`amsDrying`, `printer_ams_drying_set`, `drying_reached`, `validate_drying_request`, `drying_filament`, `drying_payload`, `DRY_BLOCK_REASONS`.
- **Known risks, flagged in-task:**
  - The stop predicate (T8 Step 3).
  - The id-range temperature bounds (`ponytail:` comment in T2).
  - The unjoined broker thread in the screen-only test (T2 Step 5).
