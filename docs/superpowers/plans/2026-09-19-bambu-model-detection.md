# Bambu Printer Model Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve a Bambu printer's canonical model from LAN evidence the printer already gives us, instead of from whatever string the user typed into `--model`. Fix the `canonicalModel: "unknown"` misidentification observed on all three physical printers (filed out of the AMS-drying plan), and record which channel produced the answer so a disagreement is visible rather than silently wrong.

**Architecture:** One `const` table in `crates/melt-core/src/bambu/identity.rs` keys every model by its three LAN-observable spellings — Bambu's `model_id` code (`C12`, `N7`, `O1D`, `BL-P001`), the marketing `product_name` (`"Bambu Lab P1S"`), and the 3-character serial prefix (`01P`). `ModelIdentity::parse` gains the first two; a new `ModelIdentity::from_serial` gains the third. A resolver walks the channels in trust order and returns the identity plus its provenance. It is called from the one place that already merges live status and `info.get_version` into capabilities — `refine_runtime_capabilities` in `bambu/transport.rs` — which then re-derives the model-dependent transport defaults that `RuntimeCapabilities::for_model` currently computes once from the configured string and never revisits. That single re-derivation *is* the bug fix; everything else is the table feeding it.

**Tech Stack:** Rust 2024 (`serde`, `serde_json`, stdlib only). No new dependencies, no build script, no bundled JSON resources: 15 rows scanned linearly is cheaper than any index.

## Spec / evidence

No separate spec. Sources, per `docs/bambu-protocol-evidence.md`:

- `bambulab/BambuStudio` — `resources/printers/*.json` (15 files; each carries `model_id`, `display_name`, `printer_type`, `sn_prefix`, `printer_series`, `subseries`), `src/slic3r/GUI/DeviceCore/DevConfigUtil.h:63-67` + `:215-234` (serial-prefix resolver and the legacy X1 alias map), `DevConfigUtil.cpp:55-148` (prefix index; logs a duplicate warning for the `31B` collision at `:99-105`), `DevManager.cpp:120-146`, `:238`, `:406`, `DeviceManager.cpp:2763-2790` (`get_version` module parser), `:3194-3197` (`print.printer_type`), `:1131-1132`, `:4425-4455` (upgrade-kit flag bits).
- `ClusterM/open-bamboo-networking` — `research/06.01-ssdp.md:5-55` (live 2026-07 P2S NOTIFY, header semantics, the `:2021` vs `Host: …:1990` correction), `research/12.01-status.md:22-35, 76-84` (live `get_version` shape), `research/06-channels.md:41-43` and `research/10.06-lan-tls-and-access-codes.md:8-10` (leaf `CN=<serial>`, issuer `BBL Device CA N7-V2`).
- `torbenconto/bambulabs_api` — `fixtures/{h2dpro,x2d,p1p_no_ams,a1}.json`: real `get_version` module arrays for four distinct firmware generations.
- `greghesp/ha-bambulab` — `pybambu/utils.py:270-366` (module→model resolution incl. the `hw_ver` heuristics), `pybambu/models.py:3196-3208` (accessory module prefixes), `pybambu/const.py:13-38`.
- `Doridian/OpenBambuAPI` — `mqtt.md:79-131` (`get_version` request/response, oldest X1 shape), `cloud-http.md:444-468` (cloud-only `dev_model_name`).
- `codeofaxel/Kiln` — `kiln/src/kiln/printers/bambu.py:1144-1171`, `:1909-1932`, `:1983-2046`: serial-prefix table verified against Bambu's `find-sn` wiki, accessory skipping, and the two-channel conflict policy this plan adopts.
- `Keralots/BambuHelper` — `src/ssdp_discovery.cpp:24-28, 153-154` (printers alternate broadcast ports 1990/2021 on a ~5 s period).

Conflicting community tables were checked and rejected: `maziggy/bambuddy:backend/app/utils/printer_models.py:31-40` maps `C11→X1C`/`C12→X1` (inverted; the same repo's `virtual_printer/manager.py:42-65` is correct), and two other projects invert `00M`/`00W` and `030`/`039`. Bambu's own `sn_prefix` values win every such tie.

## Protocol facts (read before any task)

### The table

| `model_id` | Marketing `product_name` | `sn_prefix` | Melt `CanonicalModel` |
|---|---|---|---|
| `BL-P001` (alias `3DPrinter-X1-Carbon`) | Bambu Lab X1 Carbon | `00M` | `X1Carbon` |
| `BL-P002` (alias `3DPrinter-X1`) | Bambu Lab X1 | `00W` | `X1` |
| `C11` | Bambu Lab P1P | `01S` | `P1P` |
| `C12` | Bambu Lab P1S | `01P` | `P1S` |
| `C13` | Bambu Lab X1E | `03W` | `X1E` |
| `N1` | Bambu Lab A1 mini | `030` | `A1Mini` |
| `N2S` | Bambu Lab A1 | `039` | `A1` |
| `N6` | Bambu Lab X2D | `20P` | `X2D` |
| `N7` | Bambu Lab P2S | `22E` | `P2S` |
| `N9` | Bambu Lab A2L | `26A` | `A2L` |
| `O1C` | Bambu Lab H2C | `31B` | `H2C` |
| `O1C2` | Bambu Lab H2C | `31B` | `H2C` |
| `O1D` | Bambu Lab H2D | `094` | `H2D` |
| `O1E` | Bambu Lab H2D Pro | `239` | `H2DPro` |
| `O1S` | Bambu Lab H2S | `093` | `H2S` |

`O1C`/`O1C2` deliberately collapse to one `CanonicalModel`: they share a serial prefix, share a display name, and Bambu's own loader keeps whichever it indexed first. The variant is recoverable only from `DevModel`/`subseries`, and nothing in Melt routes on it.

### Where model identity appears on the LAN

| Channel | Field | Carries | Notes |
|---|---|---|---|
| MQTT `info.get_version` | `info.module[].product_name` | Marketing name (`"Bambu Lab P2S"`) | Modern firmware, all families. Authenticated. |
| MQTT `info.get_version` | `info.module[].project_name` | `model_id` (`"N2S"`, `"C11"`) | Legacy A1/P1 firmware; `product_name` absent or empty. |
| MQTT `info.get_version` | `info.module[].hw_ver` | Weak hint (`AP02` ⇒ X1E) | Last resort only. X1/X1C firmware carries neither name field. |
| Profile serial / TLS leaf `CN` | first 3 chars | `sn_prefix` | Offline, works with the printer asleep, and the pinned cert binds it to the physical printer. |
| SSDP | `DevModel.bambu.com` | `model_id` | Unauthenticated broadcast. Advisory only. |
| SSDP | `USN` | Serial, **no `uuid:` prefix** on real printers | See Task 6. |
| MQTT `print.push_status` | `print.printer_type` | `model_id` | Parsed by Studio, never observed in any LAN capture or fixture. Out of scope. |

Accessory modules must be skipped before reading a name, or an A1 with an AMS Lite reports whatever its accessory says. Prefixes: `ams/`, `ams_f1/`, `n3f/` (AMS 2 Pro), `n3s/` (AMS HT). Also skip the External Exhaust Fan (`eef`) and buffer (`ahb`) entries, whose `product_name` names the accessory, not the printer.

Observed `get_version` shapes, one per generation (sanitized; the tests below use these verbatim):

```jsonc
// P2S, live capture — modern: product_name on `ota`
{"info":{"command":"get_version","sequence_id":"20002","module":[
  {"name":"ota","product_name":"Bambu Lab P2S","sw_ver":"01.02.00.00","sn":"SANITIZED","visible":true},
  {"name":"n3f/0","product_name":"AMS 2 Pro (1)","sw_ver":"04.00.21.87","sn":"SANITIZED","visible":true}]}}

// A1, device fixture — legacy: no product_name, project_name on every module
{"info":{"command":"get_version","module":[
  {"name":"ota","project_name":"N2S","sw_ver":"01.05.00.00","hw_ver":"OTA","sn":"SANITIZED","flag":3},
  {"name":"esp32","project_name":"N2S","sw_ver":"01.13.33.99","hw_ver":"AP05","sn":"SANITIZED"},
  {"name":"ams_f1/0","project_name":"","hw_ver":"AMS_F102","sn":"SANITIZED"}]}}

// X1C, oldest — neither field; only the serial can separate X1 from X1C
{"info":{"command":"get_version","module":[
  {"hw_ver":"","name":"ota","sn":"","sw_ver":"01.01.01.00"},
  {"hw_ver":"AP05","name":"rv1126","sn":"SANITIZED","sw_ver":"00.00.14.74"},
  {"hw_ver":"TH07","name":"th"},{"hw_ver":"","name":"xm","sn":"","sw_ver":"00.00.00.00"}]}}
```

### Trust order (the resolver's rungs)

1. **`get_version` `product_name`**, first non-accessory module with a non-empty value.
2. **`get_version` `project_name`**, same skip rule (legacy firmware).
3. **Serial prefix** from the configured profile serial. Deliberately above SSDP: the pinned TLS identity already binds that serial to the physical printer, while an SSDP broadcast is unauthenticated and spoofable by anything on the LAN.
4. **User-configured `profile.model`**, parsed through the same table so `C12`, `P1S`, and `Bambu Lab P1S` are all accepted.

SSDP `DevModel` never enters this order. It stays what it is today — advisory input to `printer discover` and `PrinterPresence` — and never re-routes an authenticated session's camera or storage transport.

When two rungs resolve to *different* canonical models, the higher rung still wins the routing decision, and the disagreement is recorded as a `ModelConflict` surfaced by `printer capabilities` and diagnostics. Reporting no model at all (Kiln's choice) would demote a confidently detected printer because of a stale `--model` typo; recording the conflict gets the same visibility without losing the capability gate.

## File map

| File | Change |
|---|---|
| `crates/melt-core/src/bambu/identity.rs` | `MODELS` table, `CanonicalModel` corrections + `display_name`/`code`, `ModelIdentity::{parse, from_serial}`, `ModelSource`, `ModelConflict`, `detect()`. |
| `crates/melt-core/src/bambu/firmware.rs` | `FirmwareModule.product` (from `product_name`), `FirmwareModule::is_accessory()`. |
| `crates/melt-core/src/bambu.rs` | `RuntimeCapabilities::for_identity`, new `source`/`conflicts` fields, `ModelFamily` for the corrected variants. |
| `crates/melt-core/src/bambu/transport.rs` | `refine_runtime_capabilities` takes the serial, detects, and re-derives defaults. |
| `crates/melt-core/src/bambu/public_catalogue.rs` | Catalogue URLs for the renamed/added variants. |
| `crates/melt-core/src/diagnostics.rs` | Report detection source and conflicts. |
| `crates/melt-cli/src/lib.rs` | `printer capabilities` prints the detected model, source, and conflicts. |
| `crates/melt-core/src/bambu/discovery.rs` | `USN` parse accepts a bare serial; display name for the discovered `model_id`. |
| `ui/src/App.vue`, `ui/src/i18n.ts` | Show the display name and a conflict hint. |
| `README.md`, `docs/bambu-compatibility.md` | Reference list and detection contract. |

## Global Constraints

- `make test` and `make lint` green after every task (`cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cd ui && bun run test && bun run build`).
- No new dependencies, Rust or JS. No build script, no `include_str!` of vendored Bambu resources — the table is 15 `const` rows.
- **Detection is a hint, never a gate.** An unresolved model stays `Unknown` and keeps the printer fully operable with conservative defaults, per `README.md` § Current capability boundary and `docs/bambu-compatibility.md` § Compatibility diagnostics. No code path may reject an operation because the model is unknown.
- **Never rewrite the profile.** Detection changes the in-memory `RuntimeCapabilities` only. `config::Profile.model` is user-owned, exactly as discovery never rewrites a host or pin.
- Live observation still outranks the model. Every existing `observed.or(fallback)` in `refine_runtime_capabilities` must keep winning over a freshly re-derived default; the re-derivation therefore happens *before* the observation merge, not after.
- Serials are credentials-adjacent: no serial, serial prefix, or module `sn` may appear in diagnostics output. `ModelSource::SerialPrefix` is serialized as the source name alone.
- Deliberate shortcuts get a `// ponytail:` comment.
- Commit prefixes `feat(bambu):`, `feat(cli):`, `feat(ui):`, `docs(bambu):`. One logical change per commit; every commit builds and passes its tests.
- New user-facing strings get both `en` and `pt-BR` entries in `ui/src/i18n.ts`, listed in the message-key union.

## Out of scope (YAGNI, add when asked)

- **Cloud `dev_model_name`.** Melt never calls a cloud account API. Nothing model-critical is cloud-gated: SSDP + serial prefix + `get_version` cover every model.
- **TLS issuer `BBL Device CA <subseries>` parsing.** It would separate `O1C` from `O1C2`, which nothing routes on, and would mean reaching into the handshake chain from the identity module.
- **`print.printer_type`.** Studio parses it; no LAN capture or fixture has ever contained it, and `docs/bambu-compatibility.md` forbids treating Studio's private routing as fact without evidence. Add it as rung 0 when a sanitized capture shows it.
- **P1P → P1S rename on the enclosure upgrade kit** (`home_flag` bits 26/27, or `cfg` bit 25 + `fun` bit 14). Studio rewrites `printer_type` to `C12` so slicing profiles match. Melt controls the machine rather than slicing for it, and the kit changes the chamber, not the camera or storage transport, so renaming the model would be a lie that buys nothing. Record the bits as an observation if chamber capability ever needs them.
- **Per-model capability matrices** (dual nozzle, laser, chamber heater, airduct, hotend rack). Melt already derives those from live observation, which is the stronger signal; the model table must not start competing with it.
- `printer_series` / `printer_arch` / `use_ams_type` / `ftp_folder` columns. `ModelFamily` and the existing storage probes already cover Melt's routing.

---

### Task 1: Model table and identity parsing

**Files:**
- Modify: `crates/melt-core/src/bambu/identity.rs` (whole file)
- Modify: `crates/melt-core/src/bambu.rs` (`ModelFamily::from_model`, ~line 191)
- Modify: `crates/melt-core/src/bambu/public_catalogue.rs` (`public_catalogue_urls`, ~line 58; `MODELS` test array, ~line 273)
- Modify: `crates/melt-core/src/bambu/quirks.rs` (only if a renamed variant is referenced)

**Interfaces:**

```rust
pub struct ModelEntry {
    pub canonical: CanonicalModel,
    /// Bambu's `model_id`, as broadcast in `DevModel.bambu.com`.
    pub code: &'static str,
    /// Legacy `DevModel` spellings for the same machine.
    pub aliases: &'static [&'static str],
    /// Marketing name as it appears in `get_version` `product_name`.
    pub product_name: &'static str,
    /// First three characters of the physical serial.
    pub sn_prefix: &'static str,
}

pub fn models() -> &'static [ModelEntry];

impl CanonicalModel {
    pub fn display_name(self) -> &'static str;  // "Bambu Lab P1S" / "Unknown"
    pub fn code(self) -> Option<&'static str>;  // "C12"
}

impl ModelIdentity {
    pub fn parse(raw: impl Into<String>) -> Self;          // extended
    pub fn from_serial(serial: &str) -> Self;              // new
}
```

`CanonicalModel` corrections, all three currently speculative and never observed:
`A2` → `A2L`, `X2` → `X2D`, plus a new `H2DPro`. `ModelFamily` keeps its `A2`/`X2` variants — `from_model` matches on the `A2`/`X2` prefix, which still holds.

`parse` normalization stays as it is (trim, uppercase, strip `-`, `_`, space) and gains, in order: exact `code`, exact `alias`, normalized `product_name` (so `"Bambu Lab P1S"`, `"BAMBULABP1S"`, and `"P1S"` all land), then the existing bare marketing spellings for backward compatibility (`X1CARBON`, `A1MINI`). `raw` is still preserved verbatim.

`from_serial` uppercases, takes the first 3 characters, and matches `sn_prefix`. A serial shorter than 3 characters yields `Unknown`. Because `O1C`/`O1C2` share `31B`, the table's first match wins by construction — no warning needed, both rows carry the same `canonical`.

- [ ] **Step 1: Write the failing tests**

In `identity.rs` `mod tests`, one table-driven test per input channel plus the regressions the research turned up:

```rust
#[test]
fn resolves_every_model_id_code() {
    for entry in models() {
        assert_eq!(ModelIdentity::parse(entry.code).canonical, entry.canonical, "{}", entry.code);
        assert_eq!(ModelIdentity::parse(entry.product_name).canonical, entry.canonical, "{}", entry.product_name);
        assert_eq!(ModelIdentity::from_serial(&format!("{}ABC1234567890", entry.sn_prefix)).canonical,
                   entry.canonical, "{}", entry.sn_prefix);
    }
}

#[test]
fn accepts_legacy_x1_ssdp_spellings() {
    assert_eq!(ModelIdentity::parse("3DPrinter-X1-Carbon").canonical, CanonicalModel::X1Carbon);
    assert_eq!(ModelIdentity::parse("3DPrinter-X1").canonical, CanonicalModel::X1);
}

/// Community tables invert these three pairs. Bambu's own `sn_prefix`
/// values are authoritative; this test is the guard against re-importing
/// the wrong table.
#[test]
fn rejects_the_known_inverted_community_mappings() {
    assert_eq!(ModelIdentity::from_serial("00M00000000000").canonical, CanonicalModel::X1Carbon);
    assert_eq!(ModelIdentity::from_serial("00W00000000000").canonical, CanonicalModel::X1);
    assert_eq!(ModelIdentity::from_serial("03000000000000").canonical, CanonicalModel::A1Mini);
    assert_eq!(ModelIdentity::from_serial("03900000000000").canonical, CanonicalModel::A1);
    assert_eq!(ModelIdentity::parse("C11").canonical, CanonicalModel::P1P);
    assert_eq!(ModelIdentity::parse("C12").canonical, CanonicalModel::P1S);
}

#[test]
fn both_h2c_codes_share_one_canonical_model() {
    assert_eq!(ModelIdentity::parse("O1C").canonical, CanonicalModel::H2C);
    assert_eq!(ModelIdentity::parse("O1C2").canonical, CanonicalModel::H2C);
    assert_eq!(ModelIdentity::from_serial("31B00000000000").canonical, CanonicalModel::H2C);
}

#[test]
fn table_is_internally_consistent() {
    for entry in models() {
        assert_eq!(entry.sn_prefix.len(), 3, "{}", entry.code);
        assert!(entry.product_name.starts_with("Bambu Lab "), "{}", entry.code);
        assert_eq!(entry.canonical.display_name(), entry.product_name);
        assert_ne!(entry.canonical, CanonicalModel::Unknown);
    }
    // Only the H2C pair may share a prefix.
    // Every code and alias must be unique across the table.
}
```

Keep the two existing tests (`preserves_unknown_internal_identifiers`, `canonicalizes_only_explicit_aliases`) unchanged — the second is the guard that a future table edit does not start guessing at unknown strings.

- [ ] **Step 2: Add the table and the parsers**

Write `MODELS` with the 15 rows from the Protocol facts table. Add `display_name`/`code` as `match` arms over the table. Extend `parse`, add `from_serial`.

- [ ] **Step 3: Rename the speculative variants**

`A2` → `A2L`, `X2` → `X2D`, add `H2DPro`. `cargo build` names every call site. In `public_catalogue.rs`, map `A2L`, `X2D`, and `H2DPro` to their pages and extend the `MODELS` test array to 17 entries.

`// ponytail:` the catalogue slugs — `bambulab.com` answers 403 to non-browser clients, so the `a2l`/`x2d`/`h2d-pro` slugs cannot be verified from CI. The existing `public_catalogue_family_pages_are_shared` test already proves no model falls back onto another model's page, which is the failure that actually matters; a wrong slug degrades to `publicCatalogueUnavailable`, not to another printer's firmware version. Note the slug as unverified and confirm it during physical qualification.

- [ ] **Step 4: `make test && make lint`**

---

### Task 2: Expose `product_name` and accessory modules in the firmware inventory

**Files:**
- Modify: `crates/melt-core/src/bambu/firmware.rs` (`FirmwareModule` ~line 55, `FirmwareInventory::from_version_info` ~line 74)

**Why:** `product_name` is the strongest LAN signal and it is currently invisible. `from_version_info`'s `known` list is `["name", "sw_ver", "hw_ver", "sn", "project_name"]`, so `product_name` falls into `unknown` — and `docs/bambu-compatibility.md` § Compatibility diagnostics says unknown module values are *never serialized*. The field is parsed into a bucket that is then deliberately withheld.

**Interfaces:**

```rust
// FirmwareModule gains:
pub product: Option<String>,   // from `product_name`; `None` when absent or empty

impl FirmwareModule {
    /// AMS units and add-on hardware report their own `product_name`. Reading
    /// one as the printer's model makes an A1 with an AMS Lite identify as its
    /// accessory.
    pub fn is_accessory(&self) -> bool;  // name starts with ams/, ams_f1/, n3f/, n3s/, or is eef/ahb
}
```

Add `"product_name"` to the `known` list so it stops leaking into `unknown`. Normalize empty strings to `None` — legacy firmware sends `"product_name": ""` on `esp32`.

- [ ] **Step 1: Write the failing tests**

Extend `preserves_all_modules_and_unknown_fields` (~line 639) and add:

```rust
#[test]
fn promotes_product_name_out_of_unknown_fields() {
    let inventory = FirmwareInventory::from_version_info(&json!({"module":[
        {"name":"ota","product_name":"Bambu Lab P2S","sw_ver":"01.02.00.00"},
        {"name":"esp32","product_name":"","hw_ver":"AP04"}]}));
    assert_eq!(inventory.modules[0].product.as_deref(), Some("Bambu Lab P2S"));
    assert_eq!(inventory.modules[1].product, None);
    assert!(!inventory.modules[0].unknown.contains_key("product_name"));
}

#[test]
fn identifies_accessory_modules() {
    // ams/0, ams_f1/0, n3f/0, n3s/128, eef, ahb are accessories; ota, esp32,
    // mc, th, ap, rv1126, xm are not.
}
```

- [ ] **Step 2: Implement, then `make test && make lint`**

---

### Task 3: The detection resolver

**Files:**
- Modify: `crates/melt-core/src/bambu/identity.rs`

**Interfaces:**

```rust
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ModelSource {
    /// `info.get_version` module `product_name`.
    VersionProductName,
    /// `info.get_version` module `project_name` (legacy firmware).
    VersionProjectName,
    /// Leading three characters of the configured, TLS-bound serial.
    SerialPrefix,
    /// The model string stored in the profile.
    Configured,
    #[default]
    None,
}

/// A lower-trust channel that named a different model. Retained for
/// diagnostics; it never changes the resolved model.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConflict {
    pub source: ModelSource,
    pub canonical: CanonicalModel,
}

pub struct ModelDetection {
    pub identity: ModelIdentity,
    pub source: ModelSource,
    pub conflicts: Vec<ModelConflict>,
}

/// Resolves the model from every LAN channel available to this session, in
/// descending trust order. SSDP is deliberately excluded: it is an
/// unauthenticated broadcast, while the serial prefix is bound to the pinned
/// certificate identity.
pub fn detect(
    serial: &str,
    configured: &str,
    firmware: &FirmwareInventory,
) -> ModelDetection;
```

Behavior:
- Build candidates in trust order, skipping accessory modules and empty values. First non-`Unknown` candidate wins `identity` and `source`.
- `identity.raw` is the winning channel's raw string, so `printer capabilities` shows what the printer actually said.
- Every *later* candidate whose canonical differs from the winner becomes a `ModelConflict`. Candidates that agree are dropped; `Unknown` candidates are dropped.
- No candidate at all → `ModelIdentity::default()` with `ModelSource::None` and an empty conflict list.
- `detect` takes `&FirmwareInventory`, not raw JSON — the inventory is already parsed once in `refine_runtime_capabilities`, and parsing it twice would be the kind of thing this codebase gets paged for.

- [ ] **Step 1: Write the failing tests**

```rust
fn inventory(info: Value) -> FirmwareInventory { FirmwareInventory::from_version_info(&info) }

#[test]
fn prefers_product_name_over_every_other_channel() {
    let detection = detect(
        "22E8BJ610801473", "P1S",
        &inventory(json!({"module":[{"name":"ota","product_name":"Bambu Lab P2S"}]})),
    );
    assert_eq!(detection.identity.canonical, CanonicalModel::P2S);
    assert_eq!(detection.source, ModelSource::VersionProductName);
    assert_eq!(detection.identity.raw, "Bambu Lab P2S");
    assert!(detection.conflicts.contains(&ModelConflict {
        source: ModelSource::Configured, canonical: CanonicalModel::P1S }));
    // The serial prefix agrees with the winner, so it is not a conflict.
    assert!(!detection.conflicts.iter().any(|c| c.source == ModelSource::SerialPrefix));
}

#[test]
fn falls_back_to_project_name_on_legacy_firmware() {
    // The A1 fixture: no product_name anywhere, project_name "N2S" on ota.
    // The ams_f1/0 module carries project_name "" and must be skipped.
}

#[test]
fn never_reads_the_model_from_an_accessory() {
    let detection = detect(
        "03912345678901", "",
        &inventory(json!({"module":[
            {"name":"ams_f1/0","product_name":"AMS Lite (1)"},
            {"name":"n3s/128","product_name":"AMS HT (1)"},
            {"name":"ota","product_name":"Bambu Lab A1"}]})),
    );
    assert_eq!(detection.identity.canonical, CanonicalModel::A1);
}

#[test]
fn separates_x1_from_x1_carbon_by_serial_when_firmware_is_silent() {
    // X1-series get_version carries neither product_name nor project_name.
    let silent = inventory(json!({"module":[
        {"name":"ota","sw_ver":"01.01.01.00","hw_ver":"","sn":""},
        {"name":"rv1126","hw_ver":"AP05","sw_ver":"00.00.14.74"}]}));
    assert_eq!(detect("00M00000000000", "", &silent).identity.canonical, CanonicalModel::X1Carbon);
    assert_eq!(detect("00W00000000000", "", &silent).identity.canonical, CanonicalModel::X1);
    assert_eq!(detect("00W00000000000", "", &silent).source, ModelSource::SerialPrefix);
}

#[test]
fn an_unresolvable_printer_stays_unknown_without_conflicts() {
    let detection = detect("", "", &FirmwareInventory::default());
    assert_eq!(detection.identity.canonical, CanonicalModel::Unknown);
    assert_eq!(detection.source, ModelSource::None);
    assert!(detection.conflicts.is_empty());
}

#[test]
fn an_unknown_configured_string_is_not_a_conflict() {
    // configured "future_dev_42" resolves to Unknown and is dropped, not
    // reported as a disagreement with the detected model.
}
```

- [ ] **Step 2: Implement, then `make test && make lint`**

---

### Task 4: Re-derive capabilities from the detected model

This is the fix. Tasks 1–3 are the table it reads.

**Files:**
- Modify: `crates/melt-core/src/bambu.rs`: `RuntimeCapabilities` (~line 283), `for_model` (~line 308)
- Modify: `crates/melt-core/src/bambu/transport.rs`: `refine_runtime_capabilities` (~line 2741) and its two call sites (~line 242, ~line 431)

**Interfaces:**

```rust
// RuntimeCapabilities gains, next to `identity`:
pub model_source: ModelSource,
pub model_conflicts: Vec<ModelConflict>,

impl RuntimeCapabilities {
    /// Derives the model-dependent defaults from an already-resolved identity.
    pub fn for_identity(identity: ModelIdentity) -> Self;
    /// Unchanged public behavior: parses, then delegates.
    pub fn for_model(model: &str) -> Self;
}

fn refine_runtime_capabilities(
    capabilities: RuntimeCapabilities,
    serial: &str,                      // new
    status: Option<&Value>,
    info: Option<&Value>,
) -> RuntimeCapabilities;
```

`for_identity` holds the existing `match model_family { … }` body verbatim, driven by `identity.normalized` and `identity.family` instead of a re-normalized raw string. `for_model` becomes `Self::for_identity(ModelIdentity::parse(model))`, so nothing outside this module changes.

New order inside `refine_runtime_capabilities`, and the order is the whole point:

1. Parse the inventory first (`FirmwareInventory::from_version_info`) — it currently happens last.
2. `detect(serial, &capabilities.identity.raw, &inventory)`.
3. If the detected canonical differs from `capabilities.identity.canonical`, replace `capabilities` with `RuntimeCapabilities::for_identity(detected.identity)`. Carry the just-parsed inventory, and any observations already merged, across the replacement.
4. Record `model_source` and `model_conflicts`.
5. Run the existing observation merge unchanged.

Step 5 stays last so every `observed.or(fallback)` and every explicit override (`has_emmc`, `support_bed_leveling`, `nozzle_temp_range`) still beats the freshly derived default. That preserves `docs/bambu-compatibility.md`'s rule that live observation outranks the model family.

Both call sites pass `self.profile.serial()`. Line 431 (the status-only cached path) passes `None` for `info`, so detection there falls through to the serial prefix — which is exactly the offline rung's purpose and means a printer identifies correctly even before the first `get_version`.

- [ ] **Step 1: Write the failing tests**

In `transport.rs` `mod tests`, next to the existing `refine_runtime_capabilities` test (~line 6327):

```rust
#[test]
fn detects_the_model_from_live_version_info_when_the_profile_says_nothing() {
    // defaults built from "" (the real-world case: `printer add` without --model)
    let capabilities = refine_runtime_capabilities(
        RuntimeCapabilities::for_model(""),
        "094SANITIZED0001",
        None,
        Some(&json!({"command":"get_version","module":[
            {"name":"ota","product_name":"Bambu Lab H2D","sw_ver":"01.02.00.00"}]})),
    );
    assert_eq!(capabilities.identity.canonical, CanonicalModel::H2D);
    assert_eq!(capabilities.model_family, ModelFamily::H2);
    assert_eq!(capabilities.model_source, ModelSource::VersionProductName);
    // The defaults were re-derived, not left at the `for_model("")` values:
    assert_eq!(capabilities.storage_transport, StorageTransport::Tunnel6000);
    assert_eq!(capabilities.camera, CameraTransport::RtspsH264);
    assert_eq!(capabilities.extruder_count, Some(2));
}

#[test]
fn detects_from_the_serial_before_any_version_query() {
    let capabilities = refine_runtime_capabilities(
        RuntimeCapabilities::for_model(""), "01P00000000000", None, None);
    assert_eq!(capabilities.identity.canonical, CanonicalModel::P1S);
    assert_eq!(capabilities.model_source, ModelSource::SerialPrefix);
}

#[test]
fn live_observations_still_outrank_the_re_derived_defaults() {
    // Detected P1S (default extruder_count Some(1), storage Ftps), but the
    // status reports two extruders and eMMC. Observation wins both.
}

#[test]
fn a_stale_configured_model_is_overridden_and_recorded() {
    // configured "X1C", get_version says "Bambu Lab A1 mini" -> canonical
    // A1Mini, source VersionProductName, conflicts contains
    // { Configured, X1Carbon }, and nothing rewrites the profile.
}

#[test]
fn an_unknown_printer_keeps_conservative_defaults() {
    let capabilities = refine_runtime_capabilities(
        RuntimeCapabilities::for_model(""), "ZZZ00000000000", None,
        Some(&json!({"command":"get_version","module":[{"name":"ota","sw_ver":"09.00.00.00"}]})));
    assert_eq!(capabilities.identity.canonical, CanonicalModel::Unknown);
    assert_eq!(capabilities.camera, CameraTransport::Unknown);
    assert_eq!(capabilities.storage_transport, StorageTransport::Unknown);
    assert!(capabilities.storage_volumes.is_empty());
}
```

- [ ] **Step 2: Implement**

- [ ] **Step 3: Check the quirk registry still matches**

`quirks::matching` filters on `identity.canonical`. Detection is what finally makes the P1S and P2S entries reachable on a profile whose `--model` was never set — add one test in `quirks.rs` proving a detected (not configured) `P1S` identity selects both disputed camera entries.

- [ ] **Step 4: `make test && make lint`**

---

### Task 5: Surface detection in diagnostics and the CLI

**Files:**
- Modify: `crates/melt-core/src/diagnostics.rs`
- Modify: `crates/melt-cli/src/lib.rs` (`printer capabilities` human output)
- Modify: `fixtures/cli-contract/inventory.json` + `crates/melt-cli/tests/contract_fixtures.rs` only if a golden file changes

**Interfaces:** the diagnostics report gains `modelSource` and `modelConflicts` beside the existing raw/canonical model names. No serial, prefix, or module `sn` is added — `ModelSource::SerialPrefix` serializes as `"serialPrefix"` and nothing more.

`printer capabilities` human output gains one line:

```
Model: Bambu Lab P1S (P1S, detected from firmware product name)
       conflicts: configured profile says X1 Carbon
```

- [ ] **Step 1: Write the failing test** — a diagnostics test asserting `modelSource` is present and that no serial substring appears anywhere in the serialized report.
- [ ] **Step 2: Implement.**
- [ ] **Step 3:** `make test && make lint && make contract`

---

### Task 6: Discovery display name and the `USN` serial

**Files:**
- Modify: `crates/melt-core/src/bambu/discovery.rs` (`parse_ssdp_response` ~line 174, `DiscoveredPrinter`)
- Modify: `ui/src/App.vue`, `ui/src/i18n.ts`

**Why:** `parse_ssdp_response` requires `USN` to start with `uuid:` and returns an empty serial otherwise. Real printers send the bare serial (`USN: 22E8BJ610801473`); the `uuid:` form appears only in the synthetic test fixture and in some emulators. A discovered printer therefore loses its serial, and `PrinterPresence::observe` drops it entirely because it keys on a non-empty serial. Accept both: strip an optional `uuid:` prefix, then take the portion before the first `::`.

`DevModel.bambu.com` also carries a `model_id`, so `printer discover` currently prints `C12` in its MODEL column. Add `DiscoveredPrinter.display_model` from `ModelIdentity::parse(model).canonical.display_name()`, falling back to the raw string when unknown. The raw `model` field stays exactly as broadcast — it is the evidence, and `printer add --model` consumes it.

`// ponytail:` the SSDP scan window. `BambuHelper` observes printers alternating destination ports 1990/2021 on a ~5 s period, so a single-port listener sees a device roughly every 10 s. Melt binds only 2021. Note the ceiling; widen to both ports if discovery is observed to miss printers, rather than pre-emptively doubling the socket count.

Leave the `LOCATION` handling alone. Real printers send a bare IPv4 with no URL scheme, so `Url::parse` fails and the code falls back to the datagram's source address — which is the same printer, and is the more trustworthy of the two anyway. It looks like a bug and is not one; add a comment saying so while the function is open.

- [ ] **Step 1: Write the failing tests** — `parses_a_bare_usn_serial` and `keeps_the_uuid_prefixed_form_working`, plus `discovery_reports_a_display_name_for_a_model_code` (`DevModel.bambu.com: N7` → `"Bambu Lab P2S"`, raw `model` still `"N7"`). Update the existing SSDP fixture at line 274 to the real bare-serial shape and keep a second case for the prefixed form.
- [ ] **Step 2: Implement, wire the display name into the fleet/settings views with `en` + `pt-BR` strings.**
- [ ] **Step 3:** `make test && make lint`

---

### Task 7: Documentation

**Files:**
- Modify: `README.md` (§ Protocol references, § Current capability boundary)
- Modify: `docs/bambu-compatibility.md` (§ Compatibility diagnostics)

- [ ] **Step 1: Add the new references.** `bambulab/BambuStudio` (the `resources/printers/*.json` tables and `DevConfigUtil`), `greghesp/ha-bambulab`, and `codeofaxel/Kiln` all carried load here and none is currently listed. The existing caveat — undocumented behavior is scoped evidence, not permanent product truth — already covers them.

- [ ] **Step 2: Document the detection contract** in `docs/bambu-compatibility.md`, next to the existing "a reported Bambu model is a capability hint" paragraph: the four-rung trust order, why SSDP is excluded from it, that conflicts are recorded rather than resolved, and that detection never rewrites a profile.

- [ ] **Step 3: Note the boundary in `README.md`** — one sentence: Melt identifies the model from firmware module names and the printer serial, and a configured `--model` is a fallback rather than the source of truth.

- [ ] **Step 4:** `make ci`

---

### Task 8: Physical verification

Run against the three qualification printers (`dakota`, `georgia`, `alaska`) — the same machines that produced the `canonicalModel: "unknown"` report this plan exists to fix.

```sh
melt printer capabilities dakota --output json | jq '{raw: .data.identity.raw, canonical: .data.identity.canonical, family: .data.modelFamily, source: .data.modelSource, conflicts: .data.modelConflicts}'
melt printer discover --output json | jq '.data.printers[] | {model, displayModel, serial: (.serial | length)}'
```

- [ ] Confirm each printer resolves to a non-`unknown` canonical model, and that the source matches the firmware generation expected for it.
- [ ] Confirm `printer discover` now returns a non-empty serial for each (the `USN` fix) and a marketing display name.
- [ ] Record which channel won per printer, and any conflict, as sanitized rows in the `docs/bambu-compatibility.md` physical-printer matrix, following `docs/bambu-physical-qualification.md`.
- [ ] If a printer reports a `product_name` absent from the table, add the row with the capture as its evidence — do not widen the matcher to guess.

---

## Self-review

- **Coverage:** the reported bug (T4) · every LAN channel (T1 table, T2 `product_name`, T3 resolver, T6 SSDP) · wrong-answer visibility (T3 conflicts, T5 diagnostics/CLI) · the inverted community tables (T1 regression test) · accessory misidentification (T2 + T3) · X1 vs X1C, which no firmware field can separate (T3 serial rung) · unknown printers stay operable (T4 test, Global Constraints) · docs (T7) · hardware (T8).
- **Names consistent across tasks:** `ModelEntry`, `models()`, `CanonicalModel::{display_name, code}`, `ModelIdentity::{parse, from_serial}`, `ModelSource`, `ModelConflict`, `ModelDetection`, `detect`, `RuntimeCapabilities::{for_identity, model_source, model_conflicts}`, `FirmwareModule::{product, is_accessory}`, `DiscoveredPrinter.display_model`.
- **Why one table and not Bambu's JSON:** vendoring `resources/printers/*.json` would import ~15 files of slicing data to read 4 fields, and would need a parser, a loader, and a licensing review. 15 `const` rows scanned linearly cost nothing and fail at compile time.
- **Known risks, flagged in-task:**
  - Catalogue slugs for `A2L`/`X2D`/`H2DPro` are unverifiable from CI (T1 Step 3, `ponytail:` comment). Worst case is `publicCatalogueUnavailable`.
  - The `31B` collision is structural, not a Melt bug — Bambu's own loader has it (T1).
  - The single-port SSDP scan may miss printers that broadcast on 1990 (T6, `ponytail:` comment).
  - `A2L`/`X2D`/`H2DPro` rows have no physical capture behind them. They come from Bambu's shipped resource files, which is the strongest available source, but T8's last checkbox is the rule that keeps guesses out of the table.
