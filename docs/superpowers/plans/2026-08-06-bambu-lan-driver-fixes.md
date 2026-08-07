# Bambu LAN Driver Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix four correctness defects and three cleanups found in a full review of the `bambu-lan` driver, so H2-class fan control works, the documented signing-required gate actually fires, and long-lived streams stop inheriting their connect budget as a permanent idle timeout.

**Architecture:** All changes are local to `crates/polimero-core/src/bambu/` plus one field on `BambuExtension` in `moonraker.rs` and one workspace dependency. The recurring theme is *dispatch on observed protocol generation, not on a string that both generations can produce* — the fan fixes make that explicit, and the timeout fixes apply the re-arm pattern that MQTT and FTPS already use to the three transports that skipped it.

**Tech Stack:** Rust 2024, `openssl` (TLS), `serde_json`, `rtsp-types`, `socket2` (new direct dependency, already in the lockfile at 0.6.5 via the tokio/reqwest tree).

## Global Constraints

- Rust edition and toolchain: unchanged. Do not bump any existing dependency version.
- `cargo test --workspace` must be green after every task. The baseline is **160 passing tests in the `polimero-core` lib target, plus 1 in `tests/bambu_evidence.rs`**; every task adds tests and none may be removed.
- `cargo clippy --workspace --all-targets -- -D warnings` must be clean after every task.
- Never widen a `pub` surface beyond what the task states. `set_socket_idle_timeout` becomes `pub(super)`, not `pub`.
- Comments explaining a deliberate shortcut use the existing `// ponytail:` prefix convention already present in this module (`transport.rs:2734`, `transport.rs:3282`).
- Do not change any existing serialized JSON field name. New serialized fields must be `skip_serializing_if` so existing UI snapshots in `ui/src/presentation.test.ts` do not shift.
- Commit after each task with a `fix(bambu):` or `refactor(bambu):` prefix, matching the existing log style (`git log --oneline -5`).

---

### Task 1: Fan acceptance predicate reads the map the command targeted

**Problem:** `fan_set` picks the fan from `status.controls.fans` (airduct keys: `parts`, `exhaust`, `hotend`, `heat`, …) but verifies against `status.fans`, which `status_fans()` only ever fills with four legacy keys (`partCooling`, `heatbreak`, `auxiliary`, `chamber`). On an H2, `fan_set("parts", 50)` sends a correct `set_fan`, the printer applies it, and the exchange then loops to `Error::Timeout` because `status.fans.get("parts")` is always `None`.

**Files:**
- Modify: `crates/polimero-core/src/bambu/transport.rs:847-853` (the predicate inside `fan_set`)
- Modify: `crates/polimero-core/src/bambu/transport.rs` (add `fan_speed_matches` next to `light_state_is`, around line 2558)
- Test: `crates/polimero-core/src/bambu/transport.rs` `mod tests` (add next to `filters_unsupported_legacy_slots_but_keeps_real_fan_telemetry`, around line 4459)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `fn fan_speed_matches(report: &serde_json::Value, fan: &str, speed_percent: u8) -> bool` — a private free function in `transport.rs`. Task 2 does not use it, but leaves it in place.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/polimero-core/src/bambu/transport.rs`:

```rust
    #[test]
    fn fan_acceptance_reads_the_control_map_the_command_targets() {
        // H2-class: fans live under device.airduct and never appear in
        // `Status::fans`, so the legacy map cannot confirm the command.
        let airduct: Value = serde_json::from_str(
            r#"{"print":{"gcode_state":"IDLE","device":{"airduct":{"modeCur":0,"modeList":[{"modeId":0,"ctrl":[16,32],"off":[48]}],"parts":[{"id":16,"state":750,"range":65536000},{"id":48,"state":0,"range":65536000}]}}}}"#,
        )
        .unwrap();
        assert!(fan_speed_matches(&airduct, "parts", 75));
        assert!(!fan_speed_matches(&airduct, "parts", 40));
        assert!(!fan_speed_matches(&airduct, "partCooling", 75));

        // Legacy firmware still reports a 0-15 scale, so the tolerance holds.
        let legacy: Value = serde_json::from_str(
            r#"{"print":{"gcode_state":"IDLE","support_aux_fan":true,"cooling_fan_speed":"9","big_fan1_speed":"3"}}"#,
        )
        .unwrap();
        assert!(fan_speed_matches(&legacy, "partCooling", 60));
        assert!(fan_speed_matches(&legacy, "auxiliary", 20));
        assert!(!fan_speed_matches(&legacy, "partCooling", 10));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p polimero-core fan_acceptance_reads_the_control_map`

Expected: FAIL to compile with `cannot find function 'fan_speed_matches' in this scope`.

- [ ] **Step 3: Add the predicate**

Insert immediately before `fn light_state_is` in `crates/polimero-core/src/bambu/transport.rs`:

```rust
/// Confirms a fan reached the requested speed using the same control map the
/// command was dispatched from. `Status::fans` only ever carries the four
/// legacy slot names, so an airduct key looked up there is never found and the
/// exchange would spin until it timed out on a command the printer accepted.
fn fan_speed_matches(report: &Value, fan: &str, speed_percent: u8) -> bool {
    parse_status_value(report)
        .ok()
        .and_then(|status| {
            status
                .controls
                .fans
                .get(fan)
                .and_then(|control| control.speed_percent)
        })
        // Legacy reports use a 0-15 scale, so one step is about 7 points.
        .is_some_and(|reported| reported.abs_diff(speed_percent) <= 7)
}
```

- [ ] **Step 4: Use it in `fan_set`**

In `crates/polimero-core/src/bambu/transport.rs`, replace the closure at lines 847-853:

```rust
        let report = self.exchange(access_code, fingerprint, payload, |report| {
            parse_status_value(report)
                .ok()
                .and_then(|status| status.fans.get(fan).copied())
                // Reports use a 0-15 scale, so one step is about 7 points.
                .is_some_and(|reported| reported.abs_diff(speed_percent) <= 7)
        })?;
```

with:

```rust
        let report = self.exchange(access_code, fingerprint, payload, |report| {
            fan_speed_matches(report, fan, speed_percent)
        })?;
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS, 161 tests in the `polimero-core` lib target (160 baseline + 1 new).

- [ ] **Step 6: Commit**

```bash
git add crates/polimero-core/src/bambu/transport.rs
git commit -m "fix(bambu): verify fan speed against the targeted control map"
```

---

### Task 2: Dispatch fan commands on the reported protocol generation

**Problem:** `modern_fan_index("auxiliary")` returns `Some(2)`, so the modern branch in `fan_set` always wins and the `"auxiliary" => "M106 P2"` legacy arm is unreachable. A P1S with an aux fan receives `set_fan{fan_index:2}` instead of `M106 P2 S128`. The key name cannot decide the protocol because `auxiliary` exists in both key spaces; the *reported* generation must.

**Files:**
- Modify: `crates/polimero-core/src/moonraker.rs:1268-1295` (`BambuExtension` field + `is_empty`)
- Modify: `crates/polimero-core/src/bambu/transport.rs:2828-2839` (`control_inventory` returns the generation flag)
- Modify: `crates/polimero-core/src/bambu/transport.rs:2660-2675` (`parse_status_value` ordering)
- Modify: `crates/polimero-core/src/bambu/transport.rs:835-846` (`fan_set` dispatch)
- Modify: `crates/polimero-cli/src/lib.rs:4525` (exhaustive fixture literal)
- Test: `crates/polimero-core/src/bambu/transport.rs` `mod tests`

**Interfaces:**
- Consumes: Task 1's `fan_speed_matches` stays untouched.
- Produces:
  - `moonraker::BambuExtension.airduct_fans: bool` — public field, serialized as `airductFans`, omitted when false.
  - `fn control_inventory(print: &Map<String, Value>, temperatures: &Option<Temperatures>, fans: &BTreeMap<String, u8>, lights: &BTreeMap<String, String>) -> (ControlInventory, bool)` — the `bool` is `true` when the airduct fan map was used.
  - `fn fan_command_payload(airduct: bool, fan: &str, speed_percent: u8) -> Result<String, Error>`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/polimero-core/src/bambu/transport.rs`:

```rust
    #[test]
    fn fan_commands_follow_the_reported_protocol_generation_not_the_key_name() {
        // `auxiliary` exists in both key spaces; only the generation decides.
        let modern: Value =
            serde_json::from_str(&fan_command_payload(true, "auxiliary", 50).unwrap()).unwrap();
        assert_eq!(modern["print"]["command"], "set_fan");
        assert_eq!(modern["print"]["fan_index"], 2);
        assert_eq!(modern["print"]["speed"], 500);

        let legacy: Value =
            serde_json::from_str(&fan_command_payload(false, "auxiliary", 50).unwrap()).unwrap();
        assert_eq!(legacy["print"]["command"], "gcode_line");
        assert_eq!(legacy["print"]["param"], "M106 P2 S128");

        assert!(matches!(
            fan_command_payload(false, "parts", 50),
            Err(Error::Unsupported("requested fan"))
        ));
        assert!(matches!(
            fan_command_payload(true, "nonsense", 50),
            Err(Error::Unsupported("requested fan"))
        ));
    }

    #[test]
    fn airduct_reports_flag_the_modern_fan_protocol() {
        let h2 = parse_status(
            br#"{"print":{"gcode_state":"IDLE","device":{"airduct":{"modeCur":0,"modeList":[],"parts":[{"id":16,"state":750,"range":65536000}]}}}}"#,
        )
        .unwrap();
        assert!(h2.extensions.bambu_lan.as_ref().unwrap().airduct_fans);

        let p1s = parse_status(
            br#"{"print":{"gcode_state":"IDLE","support_aux_fan":true,"cooling_fan_speed":"9"}}"#,
        )
        .unwrap();
        assert!(
            p1s.extensions
                .bambu_lan
                .is_none_or(|extension| !extension.airduct_fans)
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p polimero-core fan_commands_follow airduct_reports_flag`

Expected: FAIL to compile with `cannot find function 'fan_command_payload'` and `no field 'airduct_fans' on type 'BambuExtension'`.

- [ ] **Step 3: Add the `BambuExtension` field**

In `crates/polimero-core/src/moonraker.rs`, add to `pub struct BambuExtension` (after `reported_ip`, line 1282):

```rust
    /// True when this firmware reports fans under `device.airduct`, which
    /// selects the `set_fan` command family over legacy `M106` g-code.
    #[serde(skip_serializing_if = "is_false")]
    pub airduct_fans: bool,
```

Add the helper immediately before `impl BambuExtension` (line 1285):

```rust
fn is_false(value: &bool) -> bool {
    !*value
}
```

Extend `is_empty` (line 1286) to end with:

```rust
            && self.reported_ip.is_none()
            && !self.airduct_fans
```

- [ ] **Step 4: Return the generation from `control_inventory`**

In `crates/polimero-core/src/bambu/transport.rs`, replace `fn control_inventory` (lines 2828-2839) with:

```rust
/// Builds the control inventory and reports which fan protocol produced it.
/// The flag matters because `auxiliary` is a valid key in both generations.
fn control_inventory(
    print: &Map<String, Value>,
    temperatures: &Option<Temperatures>,
    fans: &BTreeMap<String, u8>,
    lights: &BTreeMap<String, String>,
) -> (ControlInventory, bool) {
    let airduct = airduct_fan_controls(print);
    let airduct_fans = airduct.is_some();
    (
        ControlInventory {
            temperatures: temperature_controls(print, temperatures),
            fans: airduct.unwrap_or_else(|| legacy_fan_controls(print, fans)),
            lights: light_controls(lights),
        },
        airduct_fans,
    )
}
```

- [ ] **Step 5: Reorder `parse_status_value` so the extension can carry the flag**

In `crates/polimero-core/src/bambu/transport.rs`, the current block at lines 2660-2675 builds `extension` before `controls`. Replace it so `controls` comes first:

```rust
    let fans = status_fans(print);
    let lights = lights(report);
    let (controls, airduct_fans) = control_inventory(print, &temperatures, &fans, &lights);
    let extension = BambuExtension {
        ams: ams_data(print),
        sd_card_state: sd_card_state(print),
        emmc_storage: has_emmc(print),
        extruder_count: observed_extruder_count(print),
        mqtt_alive_supported: print.get("support_mqtt_alive").and_then(Value::as_bool),
        status_transport: report
            .get("_polimero")
            .and_then(|metadata| metadata.get("status_transport"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        reported_ip: string(print.get("wifi_ip")).filter(|value| !value.is_empty()),
        airduct_fans,
    };
```

The `Ok(Status { ... })` literal below is unchanged — it already refers to `fans`, `lights`, `controls`, and `extension` by name.

- [ ] **Step 6: Add `fan_command_payload` and use it**

Insert immediately after `fn set_fan_payload` in `crates/polimero-core/src/bambu/transport.rs`:

```rust
/// Chooses the command for the fan protocol this firmware actually reports.
/// `auxiliary` exists in both key spaces, so the observed generation — never
/// the key name — decides whether `set_fan` or legacy `M106` g-code is sent.
fn fan_command_payload(airduct: bool, fan: &str, speed_percent: u8) -> Result<String, Error> {
    if airduct {
        let index = modern_fan_index(fan).ok_or(Error::Unsupported("requested fan"))?;
        return Ok(set_fan_payload(index, speed_percent));
    }
    let prefix = match fan {
        "partCooling" => "M106",
        "auxiliary" => "M106 P2",
        "chamber" => "M106 P3",
        _ => return Err(Error::Unsupported("requested fan")),
    };
    let pwm = (u16::from(speed_percent) * 255 + 50) / 100;
    Ok(gcode_payload(&format!("{prefix} S{pwm}")))
}
```

Then in `fan_set`, replace lines 835-846:

```rust
        let payload = if let Some(index) = modern_fan_index(fan) {
            set_fan_payload(index, speed_percent)
        } else {
            let prefix = match fan {
                "partCooling" => "M106",
                "auxiliary" => "M106 P2",
                "chamber" => "M106 P3",
                _ => return Err(Error::Unsupported("requested fan")),
            };
            let pwm = (u16::from(speed_percent) * 255 + 50) / 100;
            gcode_payload(&format!("{prefix} S{pwm}"))
        };
```

with:

```rust
        let airduct = status
            .extensions
            .bambu_lan
            .as_ref()
            .is_some_and(|extension| extension.airduct_fans);
        let payload = fan_command_payload(airduct, fan, speed_percent)?;
```

- [ ] **Step 7: Fix the exhaustive fixture literal in the CLI**

`crates/polimero-cli/src/lib.rs:4490` builds a `moonraker::BambuExtension` with every field named rather than `..Default::default()`, so the new field breaks it. After `reported_ip: None,` on line 4525, add:

```rust
                    airduct_fans: false,
```

This fixture models a legacy printer, so `false` is the correct value and its expected JSON output does not change.

- [ ] **Step 8: Run the tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS, 163 tests in the `polimero-core` lib target (160 baseline + 3). If the compiler reports any other exhaustive `BambuExtension` literal, add `airduct_fans: false` there too — it names each site.

- [ ] **Step 9: Check the UI contract is unaffected**

Run: `cd ui && npm test`

Expected: PASS with no fixture changes. `airductFans` is omitted when false, so nothing shifts. If a fixture does shift, the `skip_serializing_if` was not applied correctly — fix that rather than updating the fixture.

- [ ] **Step 10: Commit**

```bash
git add crates/polimero-core/src/moonraker.rs crates/polimero-core/src/bambu/transport.rs crates/polimero-cli/src/lib.rs
git commit -m "fix(bambu): dispatch fan commands on the reported protocol generation"
```

---

### Task 3: Make the signing-required gate actually resolve authorization

**Problem:** `docs/bambu-compatibility.md:31` promises that a proven `signingRequired` mode blocks mutations "before any mutating transport work." `authorize_mutation` reads only `self.capabilities`, which is `None` on a cold client, yielding `AuthorizationMode::Unknown` → allowed. Every `drivers::*` free function builds a fresh `bambu::Client` per call, so the gate is dead on the entire CLI path; on the pooled desktop path the *first* mutation is also ungated because `authorize_mutation` runs before the `self.status()` call that would populate the cache. The unused `_access_code` / `_fingerprint` parameters show the signature was designed to fetch.

`MutationClass::EmergencyStop` is deliberately excluded from the refresh: `emergency_stop` uses `exchange_fresh` specifically so it never waits on the reusable-session lock (see the existing test `emergency_stop_does_not_wait_for_the_reusable_session_lock`), and blocking a stop behind a status exchange would be worse than the gate it enforces.

**Files:**
- Modify: `crates/polimero-core/src/bambu/transport.rs:249-269` (`authorize_mutation`)
- Modify: `crates/polimero-core/src/bambu/transport.rs:291-300` (`remember_storage_transport`)
- Modify: `docs/bambu-compatibility.md:31-37` (record the emergency-stop exception)
- Test: `crates/polimero-core/src/bambu/transport.rs` `mod tests`

**Interfaces:**
- Consumes: nothing from Tasks 1-2.
- Produces: `fn cached_authorization(&self) -> Option<AuthorizationMode>` — private method on `Client`.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/polimero-core/src/bambu/transport.rs`:

```rust
    #[test]
    fn a_cold_client_resolves_authorization_before_it_mutates() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let host = address.ip().to_string();
        let port = address.port();
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

            let poll = read_test_packet(&mut stream);
            assert!(is_pushall_payload(
                &mqtt_publish_payload(poll.kind, &poll.payload).unwrap()
            ));
            let report = json!({"print": {
                "gcode_state": "IDLE",
                "mc_percent": 0,
                "security": {"signing_required": true}
            }});
            let mut payload = Vec::new();
            mqtt_string(&mut payload, "device/SN001/report").unwrap();
            payload.extend_from_slice(report.to_string().as_bytes());
            let mut packet = vec![0x30];
            mqtt_remaining_length(&mut packet, payload.len()).unwrap();
            packet.extend_from_slice(&payload);
            stream.write_all(&packet).unwrap();
            stream.flush().unwrap();
        });

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

        // The client has never resolved capabilities, so the gate must fetch
        // status itself rather than falling through as Unknown.
        assert!(matches!(
            client.fan_set(Some("access-code"), None, "partCooling", 50),
            Err(Error::AuthorizationRequired(MutationClass::Fan))
        ));
        server.join().unwrap();
    }

    #[test]
    fn emergency_stop_uses_cached_authorization_without_fetching_status() {
        let profile =
            Profile::with_timeout("203.0.113.1", "SN001", true, Duration::from_millis(50))
                .unwrap();
        let client = Client::new(profile);
        let mut capabilities = RuntimeCapabilities::for_model("X1C");
        capabilities.authorization = AuthorizationMode::SigningRequired;
        *client.capabilities.lock().unwrap() = Some(capabilities);

        // Unroutable host: reaching the network at all would time out instead.
        assert!(matches!(
            client.authorize_mutation(Some("code"), None, MutationClass::EmergencyStop),
            Err(Error::AuthorizationRequired(MutationClass::EmergencyStop))
        ));

        // A cold emergency stop stays permitted rather than blocking on status.
        let cold = Client::new(
            Profile::with_timeout("203.0.113.1", "SN001", true, Duration::from_millis(50))
                .unwrap(),
        );
        assert!(
            cold.authorize_mutation(Some("code"), None, MutationClass::EmergencyStop)
                .is_ok()
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p polimero-core a_cold_client_resolves emergency_stop_uses_cached`

Expected: `a_cold_client_resolves_authorization_before_it_mutates` FAILS — `fan_set` returns `Err(Error::Timeout)` or `Ok(..)` rather than `AuthorizationRequired`, because the cold cache resolves to `Unknown` and the mutation proceeds. The second test may already pass; it is a regression guard for Step 3's exception.

- [ ] **Step 3: Resolve authorization before mutating**

In `crates/polimero-core/src/bambu/transport.rs`, replace `fn authorize_mutation` (lines 249-269) with:

```rust
    /// Resolves authorization before mutating. A cold client has no cached
    /// capabilities, so without this refresh the documented `signingRequired`
    /// block would silently fall through as `Unknown` on every one-shot client
    /// the CLI builds, and on the first mutation of a pooled one.
    ///
    /// `EmergencyStop` deliberately opts out: it runs on a fresh session
    /// precisely so it never waits on the reusable-session lock, and blocking a
    /// stop behind a status exchange is worse than the gate it would enforce.
    fn authorize_mutation(
        &self,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        class: MutationClass,
    ) -> Result<(), Error> {
        if class != MutationClass::EmergencyStop && self.cached_authorization().is_none() {
            self.status(access_code, fingerprint)?;
        }
        match self
            .cached_authorization()
            .unwrap_or(AuthorizationMode::Unknown)
        {
            AuthorizationMode::SigningRequired => Err(Error::AuthorizationRequired(class)),
            AuthorizationMode::ConflictingEvidence => Err(Error::AuthorizationConflict(class)),
            AuthorizationMode::Unknown | AuthorizationMode::DeveloperMode => Ok(()),
        }
    }

    fn cached_authorization(&self) -> Option<AuthorizationMode> {
        self.capabilities
            .lock()
            .ok()
            .and_then(|capabilities| capabilities.clone())
            .map(|capabilities| capabilities.authorization)
    }
```

- [ ] **Step 4: Stop `remember_storage_transport` from faking a resolved cache**

`cached_authorization()` treats "capabilities present" as "authorization observed". `remember_storage_transport` currently conjures a default capability set out of nothing, which would make a later mutation skip the refresh with a bogus `Unknown`. It is unreachable today (every mutating caller authorizes first), but the coupling is load-bearing now, so remove it. `storage_transport()` already consults `observed_storage` first, so nothing is lost.

In `crates/polimero-core/src/bambu/transport.rs`, replace `fn remember_storage_transport` (lines 291-300) with:

```rust
    fn remember_storage_transport(&self, transport: StorageTransport) {
        if let Ok(mut observed) = self.observed_storage.lock() {
            *observed = Some(transport);
        }
        // Only refine an existing observation. Creating one here would make
        // `cached_authorization` report a resolved-but-unobserved Unknown and
        // silently disable the mutation gate.
        if let Ok(mut capabilities) = self.capabilities.lock()
            && let Some(capabilities) = capabilities.as_mut()
        {
            capabilities.storage_transport = transport;
        }
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS, 165 tests in the `polimero-core` lib target. `emergency_stop_does_not_wait_for_the_reusable_session_lock` and `emergency_stop_succeeds_while_the_reusable_session_is_locked` must both still pass — they are the guards on the exception in Step 3.

- [ ] **Step 6: Record the emergency-stop exception in the compatibility doc**

In `docs/bambu-compatibility.md`, in the `## Authorization policy` section, replace:

```
A proven `signingRequired` mode blocks file
writes, print start/control, motion, thermal, fan, light, speed, and emergency
commands before any mutating transport work.
```

with:

```
A proven `signingRequired` mode blocks file
writes, print start/control, motion, thermal, fan, light, and speed commands
before any mutating transport work, resolving live status first when the
client has not yet observed it. Emergency stop is gated only by an already
resolved mode: it must never block on a status exchange or the reusable
session lock, and the printer's own rejection remains its backstop.
```

- [ ] **Step 7: Commit**

```bash
git add crates/polimero-core/src/bambu/transport.rs docs/bambu-compatibility.md
git commit -m "fix(bambu): resolve authorization before gating mutations"
```

---

### Task 4: Re-arm stream sockets with a steady-state idle timeout

**Problem:** `open_tls_socket` sets `SO_RCVTIMEO`/`SO_SNDTIMEO` to `remaining(deadline)` at handshake time. MQTT (`transport.rs:1970`) and FTPS (`transport.rs:3916`, `:3993`, `:4064`) re-arm it before every read. RTSPS, the classic MJPEG camera, and the `:6000` tunnel never do, so they stream for minutes on whatever fraction of the connect budget was left. Worst case is `open_mjpeg_stream` (`bambu.rs:640`): RTSPS is probed first against a shared deadline, so if it burns 9 of 10 seconds failing, the classic fallback gets both a 1s connect budget *and* a 1s permanent per-frame read deadline, killing the stream at the first inter-frame gap.

`profile.timeout()` is the right steady-state value: it is the configured per-printer budget and is stable regardless of how much of it the handshake consumed.

**Files:**
- Modify: `crates/polimero-core/src/bambu/transport.rs:1489` (visibility)
- Modify: `crates/polimero-core/src/bambu/rtsp.rs:260-263`
- Modify: `crates/polimero-core/src/bambu/tunnel.rs:74-80`
- Modify: `crates/polimero-core/src/bambu.rs:733-759`
- Test: `crates/polimero-core/src/bambu/transport.rs` `mod tests`

**Interfaces:**
- Consumes: nothing from Tasks 1-3.
- Produces: `pub(super) fn set_socket_idle_timeout(socket: &TcpStream, timeout: Duration) -> Result<(), Error>` — visible to sibling modules under `bambu`.

- [ ] **Step 1: Write the test that pins the helper contract**

Add to `mod tests` in `crates/polimero-core/src/bambu/transport.rs`:

```rust
    #[test]
    fn idle_timeouts_can_be_rearmed_past_the_connect_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();

        let deadline = Instant::now() + Duration::from_millis(50);
        set_socket_timeout(&socket, deadline).unwrap();
        assert!(socket.read_timeout().unwrap().unwrap() <= Duration::from_millis(50));

        set_socket_idle_timeout(&socket, Duration::from_secs(10)).unwrap();
        assert!(socket.read_timeout().unwrap().unwrap() > Duration::from_secs(9));
        assert!(socket.write_timeout().unwrap().unwrap() > Duration::from_secs(9));
    }
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p polimero-core idle_timeouts_can_be_rearmed`

Expected: PASS. This test pins the helper's contract; the defect is at the three call sites that never invoke it, which is what Steps 3-6 fix. Call-site coverage is not automatable without a live printer — Step 7 is the structural check.

- [ ] **Step 3: Widen the helper's visibility**

In `crates/polimero-core/src/bambu/transport.rs:1489`, change:

```rust
fn set_socket_idle_timeout(socket: &TcpStream, timeout: Duration) -> Result<(), Error> {
```

to:

```rust
pub(super) fn set_socket_idle_timeout(socket: &TcpStream, timeout: Duration) -> Result<(), Error> {
```

- [ ] **Step 4: Re-arm the RTSPS camera socket**

In `crates/polimero-core/src/bambu/rtsp.rs`, replace lines 260-263:

```rust
    let (connection, _) =
        transport::open_tls(&connector, profile, RTSP_PORT, fingerprint, true, deadline)
            .map_err(map_transport_error)?;
    let mut connection = FramedConnection::new(connection);
```

with:

```rust
    let (connection, _) =
        transport::open_tls(&connector, profile, RTSP_PORT, fingerprint, true, deadline)
            .map_err(map_transport_error)?;
    // The handshake budget must not become the live stream's read deadline: a
    // slow connect (or a failed RTSPS probe ahead of this one) would otherwise
    // leave the camera with a sub-second inter-frame timeout.
    transport::set_socket_idle_timeout(connection.get_ref(), profile.timeout())
        .map_err(map_transport_error)?;
    let mut connection = FramedConnection::new(connection);
```

- [ ] **Step 5: Re-arm the `:6000` tunnel socket**

In `crates/polimero-core/src/bambu/tunnel.rs`, replace lines 74-80:

```rust
        let connector = transport::tls_connector()?;
        let (stream, _) =
            transport::open_tls(&connector, profile, PORT, fingerprint, true, deadline)?;
        let mut connection = Self {
            stream,
            sequence: 1,
        };
```

with:

```rust
        let connector = transport::tls_connector()?;
        let (stream, _) =
            transport::open_tls(&connector, profile, PORT, fingerprint, true, deadline)?;
        // Bulk transfers are bounded by an idle timeout, not a wall clock, so
        // the socket must not keep whatever fraction of the connect budget the
        // handshake happened to leave behind.
        transport::set_socket_idle_timeout(stream.get_ref(), profile.timeout())?;
        let mut connection = Self {
            stream,
            sequence: 1,
        };
```

- [ ] **Step 6: Re-arm the classic MJPEG camera socket**

In `crates/polimero-core/src/bambu.rs`, replace the tail of `fn open_camera_connection` (lines 742-758):

```rust
    transport::open_tls(
        &connector,
        profile,
        CAMERA_PORT,
        fingerprint,
        true,
        deadline,
    )
    .map(|(stream, _)| stream)
    .map_err(|error| match error {
        TransportError::Pin(error) => CameraError::Pin(error),
        TransportError::MissingCertificate => CameraError::MissingCertificate,
        TransportError::CertificateIdentity => CameraError::Identity,
        TransportError::Tls => CameraError::Tls,
        TransportError::Timeout => CameraError::Connect(io::Error::from(io::ErrorKind::TimedOut)),
        _ => CameraError::Connect(io::Error::from(io::ErrorKind::ConnectionRefused)),
    })
```

with:

```rust
    let (stream, _) = transport::open_tls(
        &connector,
        profile,
        CAMERA_PORT,
        fingerprint,
        true,
        deadline,
    )
    .map_err(|error| match error {
        TransportError::Pin(error) => CameraError::Pin(error),
        TransportError::MissingCertificate => CameraError::MissingCertificate,
        TransportError::CertificateIdentity => CameraError::Identity,
        TransportError::Tls => CameraError::Tls,
        TransportError::Timeout => CameraError::Connect(io::Error::from(io::ErrorKind::TimedOut)),
        _ => CameraError::Connect(io::Error::from(io::ErrorKind::ConnectionRefused)),
    })?;
    // `timeout` here is the remaining connect budget, which on the RTSPS
    // fallback path can be a fraction of a second. A live stream needs the
    // profile's steady-state idle budget instead.
    transport::set_socket_idle_timeout(stream.get_ref(), profile.timeout())
        .map_err(|_| CameraError::Tls)?;
    Ok(stream)
```

- [ ] **Step 7: Run the tests and confirm all three call sites landed**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS, 166 tests in the `polimero-core` lib target.

Then: `rg -n 'set_socket_idle_timeout' crates/polimero-core/src/bambu/`

Expected: hits in `transport.rs` (definition plus its existing MQTT/FTPS uses), `rtsp.rs` (1), `tunnel.rs` (1), and `bambu.rs` (1).

- [ ] **Step 8: Commit**

```bash
git add crates/polimero-core/src/bambu.rs crates/polimero-core/src/bambu/transport.rs crates/polimero-core/src/bambu/rtsp.rs crates/polimero-core/src/bambu/tunnel.rs
git commit -m "fix(bambu): rearm stream sockets with a steady-state idle timeout"
```

---

### Task 5: Share the discovery announcement port with other slicers

**Problem:** `discovery.rs:47` binds UDP 2021 with no address reuse. Bambu Studio and OrcaSlicer bind the same port, so whenever one is running the bind fails and `discover()` silently degrades to SSDP-only — no error, just fewer printers. `socket2` 0.6.5 is already compiled in this workspace's lockfile via the tokio/reqwest tree, so this costs no new build time.

On Unix both `SO_REUSEADDR` and `SO_REUSEPORT` are required for two processes to share a UDP port; on Windows `SO_REUSEADDR` alone provides the sharing semantics.

**Files:**
- Modify: `Cargo.toml` (workspace dependency)
- Modify: `crates/polimero-core/Cargo.toml`
- Modify: `crates/polimero-core/src/bambu/discovery.rs:1-6, 44-60`
- Test: `crates/polimero-core/src/bambu/discovery.rs` `mod tests`

**Interfaces:**
- Consumes: nothing from Tasks 1-4.
- Produces: `fn open_announcements() -> io::Result<UdpSocket>` — private to `discovery.rs`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `crates/polimero-core/src/bambu/discovery.rs`:

```rust
    #[test]
    fn announcement_socket_shares_the_port_with_another_slicer() {
        // Bambu Studio and OrcaSlicer hold this port; a scan must not go
        // silently SSDP-only just because one of them is open.
        let first = open_announcements().expect("first bind");
        let second = open_announcements().expect("second bind while the first is held");
        assert_eq!(first.local_addr().unwrap().port(), BAMBU_BROADCAST_PORT);
        assert_eq!(second.local_addr().unwrap().port(), BAMBU_BROADCAST_PORT);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p polimero-core announcement_socket_shares_the_port`

Expected: FAIL to compile with `cannot find function 'open_announcements' in this scope`.

- [ ] **Step 3: Add the dependency**

In the workspace `Cargo.toml`, add to `[workspace.dependencies]` (after `zip`, line 30):

```toml
socket2 = "0.6"
```

In `crates/polimero-core/Cargo.toml`, add to `[dependencies]` (after `zip.workspace = true`, line 24):

```toml
socket2.workspace = true
```

- [ ] **Step 4: Add the reusable bind**

In `crates/polimero-core/src/bambu/discovery.rs`, change the `std::net` import on line 4 to include `SocketAddr`:

```rust
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
```

Then insert immediately after `fn open_ssdp` (line 100):

```rust
/// Binds the Bambu announcement port with address reuse, so a scan still works
/// while Bambu Studio or OrcaSlicer holds the same port. Unix needs
/// `SO_REUSEPORT` as well; Windows gets the sharing semantics from
/// `SO_REUSEADDR` alone.
fn open_announcements() -> io::Result<UdpSocket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_reuse_address(true)?;
    #[cfg(unix)]
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, BAMBU_BROADCAST_PORT)).into())?;
    Ok(socket.into())
}
```

- [ ] **Step 5: Use it in `discover`**

In `crates/polimero-core/src/bambu/discovery.rs:47`, replace:

```rust
    let announcements = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, BAMBU_BROADCAST_PORT));
```

with:

```rust
    let announcements = open_announcements();
```

- [ ] **Step 6: Run the discovery tests**

Run: `cargo test -p polimero-core bambu::discovery`

Expected: PASS, 3 tests in `bambu::discovery::tests`. If the new test fails to bind at all, something on the machine holds UDP 2021 without `SO_REUSEPORT`; confirm with `ss -lunp | grep 2021` before treating it as a code defect.

- [ ] **Step 7: Full suite and lint**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS, 167 tests in the `polimero-core` lib target.

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml Cargo.lock crates/polimero-core/Cargo.toml crates/polimero-core/src/bambu/discovery.rs
git commit -m "fix(bambu): share the discovery announcement port with other slicers"
```

---

### Task 6: Tear down RTSP streams without blocking the dropping thread

**Problem:** `Drop for H264Stream` (`rtsp.rs:218`) sends TEARDOWN through `authorized_request`, which calls `next_response()` and therefore drains every queued interleaved media frame before it sees the reply. Dropping a stream can block the calling thread for the socket's idle timeout — and after Task 4 that timeout is the full profile budget, making this worse, not better. Nothing is done with the response.

**Files:**
- Modify: `crates/polimero-core/src/bambu/rtsp.rs:417-441` (split `send_request`)
- Modify: `crates/polimero-core/src/bambu/rtsp.rs:218-229` (`Drop`)
- Test: `crates/polimero-core/src/bambu/rtsp.rs` (new `mod tests` — this file currently has none)

**Interfaces:**
- Consumes: Task 4 raised the socket idle timeout on this connection, which is what makes the blocking drop worth fixing now.
- Produces:
  - `fn request_bytes(method: rtsp_types::Method, uri: &rtsp_types::Url, cseq: u32, extra_headers: &[(rtsp_types::HeaderName, String)], authorization: Option<&str>) -> Result<Vec<u8>, CameraError>`
  - `fn write_request(connection: &mut FramedConnection, method: rtsp_types::Method, uri: &rtsp_types::Url, cseq: u32, extra_headers: &[(rtsp_types::HeaderName, String)], authorization: Option<&str>) -> Result<(), CameraError>`

- [ ] **Step 1: Write the failing test**

Append to `crates/polimero-core/src/bambu/rtsp.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_is_serialized_without_waiting_for_a_response() {
        let uri = rtsp_types::Url::parse("rtsps://printer.local:322/streaming/live/1").unwrap();
        let bytes = request_bytes(
            rtsp_types::Method::Teardown,
            &uri,
            9,
            &[(rtsp_types::headers::SESSION, "abc123".to_owned())],
            None,
        )
        .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("TEARDOWN rtsps://printer.local:322/streaming/live/1"));
        assert!(text.contains("CSeq: 9"));
        assert!(text.contains("Session: abc123"));
    }

    #[test]
    fn digest_response_matches_the_live555_scheme() {
        let header = digest_header("12345678", "DESCRIBE", "rtsps://printer.local:322/s", "n0nce");
        assert!(header.contains("username=\"bblp\""));
        assert!(header.contains("realm=\"LIVE555 Streaming Media\""));
        assert!(header.contains("nonce=\"n0nce\""));
        assert_eq!(extract_quoted(&header, "nonce").as_deref(), Some("n0nce"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p polimero-core bambu::rtsp`

Expected: FAIL to compile with `cannot find function 'request_bytes' in this scope`.

- [ ] **Step 3: Split serialization from the round trip**

In `crates/polimero-core/src/bambu/rtsp.rs`, replace `fn send_request` (lines 417-441) with these three functions:

```rust
/// Serializes one RTSP request. Kept separate from the socket so a teardown
/// can be written without also committing to read a reply.
fn request_bytes(
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: u32,
    extra_headers: &[(rtsp_types::HeaderName, String)],
    authorization: Option<&str>,
) -> Result<Vec<u8>, CameraError> {
    let mut builder = rtsp_types::Request::builder(method, rtsp_types::Version::V1_0)
        .request_uri(uri.clone())
        .header(rtsp_types::headers::CSEQ, format!("{cseq}"));
    for (name, value) in extra_headers {
        builder = builder.header(name.clone(), value.clone());
    }
    if let Some(authorization) = authorization {
        builder = builder.header(rtsp_types::headers::AUTHORIZATION, authorization.to_owned());
    }
    let mut bytes = Vec::new();
    builder
        .empty()
        .write(&mut bytes)
        .map_err(|_| stream_error("failed to serialize RTSP request"))?;
    Ok(bytes)
}

fn write_request(
    connection: &mut FramedConnection,
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: u32,
    extra_headers: &[(rtsp_types::HeaderName, String)],
    authorization: Option<&str>,
) -> Result<(), CameraError> {
    let bytes = request_bytes(method, uri, cseq, extra_headers, authorization)?;
    connection.write_all(&bytes)
}

fn send_request(
    connection: &mut FramedConnection,
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: u32,
    extra_headers: &[(rtsp_types::HeaderName, String)],
    authorization: Option<&str>,
) -> Result<rtsp_types::Response<Vec<u8>>, CameraError> {
    write_request(connection, method, uri, cseq, extra_headers, authorization)?;
    connection.next_response()
}
```

- [ ] **Step 4: Make `Drop` fire-and-forget**

In `crates/polimero-core/src/bambu/rtsp.rs`, replace `impl Drop for H264Stream` (lines 218-229) with:

```rust
impl Drop for H264Stream {
    fn drop(&mut self) {
        // Fire-and-forget. Waiting for the TEARDOWN reply means draining every
        // queued interleaved media frame first, blocking the dropping thread
        // for the socket's whole idle budget for a response nothing reads.
        let _ = write_request(
            &mut self.connection,
            rtsp_types::Method::Teardown,
            &self.uri,
            self.cseq,
            &[(rtsp_types::headers::SESSION, self.session.clone())],
            None,
        );
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS, 169 tests in the `polimero-core` lib target. `authorized_request` is unchanged and still uses `send_request`.

- [ ] **Step 6: Commit**

```bash
git add crates/polimero-core/src/bambu/rtsp.rs
git commit -m "fix(bambu): tear down RTSP streams without blocking the dropping thread"
```

---

### Task 7: Collapse two redundant guards

**Problem:** Two small pieces of dead logic. `next_sequence` (`transport.rs:2154`) repeats its wrap-around arithmetic in both the `fetch_update` closure and the `map` that follows, so a future edit has to change it in two places to stay correct. `tunnel::Connection::list` (`tunnel.rs:135`) guards with `path != "/" || !valid_wire_path(path)`, whose second half is unreachable — `valid_wire_path("/")` is always true, and the only path that reaches the second operand is `"/"`.

**Files:**
- Modify: `crates/polimero-core/src/bambu/transport.rs:2150-2172`
- Modify: `crates/polimero-core/src/bambu/tunnel.rs:135-137`
- Test: `crates/polimero-core/src/bambu/transport.rs` `mod tests`

**Interfaces:**
- Consumes: nothing from Tasks 1-6.
- Produces: `const MAX_SAFE_SEQUENCE: u64` and `fn wrap_sequence(previous: u64) -> u64` at module scope in `transport.rs`. The public behavior of `next_sequence()` is unchanged.

- [ ] **Step 1: Write the failing test**

`SEQUENCE` is a process-global atomic shared by every test in the binary, so it cannot be asserted on directly without flaking under the parallel test runner. Test the extracted pure function instead.

Add to `mod tests` in `crates/polimero-core/src/bambu/transport.rs`:

```rust
    #[test]
    fn sequence_ids_stay_in_the_firmware_safe_range_and_wrap_to_one() {
        assert_eq!(wrap_sequence(0), 1);
        assert_eq!(wrap_sequence(41), 42);
        // Bambu firmware treats the sequence id as a signed 32-bit value.
        assert_eq!(wrap_sequence(MAX_SAFE_SEQUENCE - 1), MAX_SAFE_SEQUENCE);
        assert_eq!(wrap_sequence(MAX_SAFE_SEQUENCE), 1);
        assert_eq!(wrap_sequence(u64::MAX), 1);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p polimero-core sequence_ids_stay_in_the_firmware_safe_range`

Expected: FAIL to compile with `cannot find function 'wrap_sequence'` and `cannot find value 'MAX_SAFE_SEQUENCE' in this scope`.

- [ ] **Step 3: Extract the wrap**

In `crates/polimero-core/src/bambu/transport.rs`, replace `fn next_sequence` (lines 2154-2172) with:

```rust
/// Bambu firmware treats the sequence id as a signed 32-bit value.
const MAX_SAFE_SEQUENCE: u64 = i32::MAX as u64;

fn wrap_sequence(previous: u64) -> u64 {
    if previous >= MAX_SAFE_SEQUENCE {
        1
    } else {
        previous + 1
    }
}

fn next_sequence() -> u64 {
    SEQUENCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |previous| {
            Some(wrap_sequence(previous))
        })
        .map_or(1, wrap_sequence)
}
```

- [ ] **Step 4: Drop the unreachable tunnel guard**

In `crates/polimero-core/src/bambu/tunnel.rs`, replace lines 135-137:

```rust
        if path != "/" || !valid_wire_path(path) {
            return Err(Error::Unsupported("nested :6000 file listing"));
        }
```

with:

```rust
        // Only the storage root is listable here; `normalize_device_path` has
        // already rejected traversal and control characters upstream.
        if path != "/" {
            return Err(Error::Unsupported("nested :6000 file listing"));
        }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS, 170 tests in the `polimero-core` lib target. Clippy must not report `valid_wire_path` as dead code — it is still used by `download`, `sub_file`, `upload`, and `delete`.

- [ ] **Step 6: Commit**

```bash
git add crates/polimero-core/src/bambu/transport.rs crates/polimero-core/src/bambu/tunnel.rs
git commit -m "refactor(bambu): collapse duplicated sequence wrap and dead list guard"
```

---

## Final verification

- [ ] **Full workspace build and test**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`

Expected: PASS. `polimero-core` lib target at 170 tests (160 baseline + 10 added), `tests/bambu_evidence.rs` still at 1.

- [ ] **UI contract unchanged**

Run: `cd ui && npm test`

Expected: PASS with no snapshot updates. The only new serialized field is `airductFans`, omitted when false.

- [ ] **Confirm every finding is addressed**

| Finding | Task |
| --- | --- |
| Fan acceptance predicate reads the wrong key space | 1 |
| `auxiliary` collides across both fan protocols | 2 |
| Signing-required gate never fires on the CLI path | 3 |
| Stream sockets inherit the connect deadline | 4 |
| Discovery bind has no address reuse | 5 |
| `H264Stream::drop` blocks draining media | 6 |
| `next_sequence` duplicates its wrap logic | 7 |
| `tunnel::list` has an unreachable guard | 7 |
