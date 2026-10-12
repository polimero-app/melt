# CLI contract fixtures

Last-updated: 2026-10-12

`fixtures/cli-contract/inventory.json` is the migration inventory. It records
every public command in the Go contract, its arguments and flags, and whether
the Rust CLI implements it. Do not remove an inventory row when a command is
implemented; change its `implementation` value and add a fixture.

Schema version 2 keeps post-migration commands in `meltNativeCommands`. These
commands participate in the same fixture and help coverage without being
misrepresented as part of the frozen Go baseline.

`fixtures/cli-contract/manifest.json` maps a reproducible invocation to its
exit code and JSON golden response. Original migration responses were captured from the Go
working baseline at `6b67ff58b8b7a123f3e560f76c8c62a3a47e7a6d`. The accepted
contract names `16f2a9570affdc0ab20ee41b20d273bc4743a68d`, but that object is
not present in the clone used for the original captures. That provenance gap
is retained in the inventory; do not claim the captures came from the accepted
object or silently regenerate the frozen baseline. Melt-native additions and
accepted extensions (firmware, camera management, H.264) have Rust responses.

The inventory currently has 41 baseline paths and 9 Melt-native paths,
including command groups. All are marked implemented. The manifest has 13
hardware-independent JSON/error cases; it is representative fixture coverage,
not a successful network fixture for every public operation. Unit/simulated
network tests provide further behavior coverage.

## Running a fixture

Run the selected binary with the fixture `args`, `MELT_CONFIG_DIR` set to
`fixtures/cli-contract/config/empty`, and compare the process exit code and
JSON stdout. The harness must delete each `ignoreJsonPointers` location before
comparison; those values depend on the build host or timing. Stderr must remain
empty for every JSON fixture.

```sh
MELT_CONFIG_DIR="$PWD/fixtures/cli-contract/config/empty" \
  ./target/debug/melt version --output json
```

`make contract` runs `crates/melt-cli/tests/contract_fixtures.rs`. It invokes
`melt_cli::run` in process, compares returned exit code, stderr and JSON, and
removes only the documented host-dependent JSON pointers. It also checks
schema versions, inventory implementation flags, unique paths/fixture IDs and
valid envelope shapes. It does not spawn the packaged executable. That entry
point is separately exercised by the desktop package contract and
`make release-cli-check` after building it.

The manifest remains schema version 1 while the inventory is version 2.
Both golden and actual JSON are normalized only at `ignoreJsonPointers`;
unexpected differences or stderr are failures. Keep fixtures offline by using
invalid inputs or the checked-in empty configuration, never a developer's
printer/keychain.

## Adding or updating coverage

Preserve Go-baseline rows and their original flags. Register a new public path
under `meltNativeCommands`, and document extra flags on an existing path as
extensions. Add a manifest case and golden for a deterministic success or
validation failure when feasible. Record why any baseline response changes.
The harness currently asserts inventory/fixture counts; update those assertions
deliberately when adding rows/cases. Run `make contract` and the relevant CLI
unit tests. The [CLI reference](cli.md) describes current syntax and semantics.

## Melt-native extensions

`firmware` and `firmware check` are Melt-native paths. Firmware checks also
accept `--include-history` and `--include-public-catalogue`; those flags are
implemented extensions beyond the inventory's original native flag list.
`printer capabilities`, `jobs preflight`, and AMS drying paths are already
represented in the checked-in migration inventory.

`camera add|edit|link|list|remove|select|unlink` are listed in
`meltNativeCommands`. `camera snapshot` and `camera stream` also accept the
Melt-native `--camera` and `--quality` flags; like `printer add --model`, these
are not added to the frozen Go-baseline rows, whose positional printer
behavior is unchanged.

`camera stream --format` also accepts the Melt-native value `h264`. The
`camera-stream-invalid-format-json` fixture therefore uses `h265` as its
invalid value, and its golden message lists both accepted formats; the
Go baseline only accepted `mjpeg`.
