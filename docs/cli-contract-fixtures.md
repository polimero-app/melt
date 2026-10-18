# CLI contract fixtures

Last-updated: 2026-10-12

`fixtures/cli-contract/inventory.json` inventories Melt's public command paths,
arguments, core flags and implementation status. Schema version 2 stores paths
in two arrays, `commands` and `meltNativeCommands`; both participate in the
same fixture and help coverage. Preserve implemented rows, add new paths to
`meltNativeCommands`, and cover them with reproducible fixtures where feasible.

`fixtures/cli-contract/manifest.json` maps a reproducible invocation to its
exit code and JSON golden response. Goldens define the accepted output for the
current Melt contract. Review intended response changes before updating a
golden; history and previous expectations remain available in Git.

The inventory currently has 41 paths in `commands` and 17 in
`meltNativeCommands`, including command groups. All are marked implemented.
The inventory is not a complete command/option reference; see the
[CLI reference](cli.md) and command help for current syntax. The manifest has
19 hardware-independent JSON/error cases; it is representative fixture
coverage, not a successful network fixture for every public operation.
Unit/simulated network tests provide further behavior coverage.

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

Preserve existing command syntax and response semantics. Register a new
public path under `meltNativeCommands`, and document extra flags on an existing
path as extensions. Add a manifest case and golden for a deterministic success
or validation failure when feasible. Record why an existing response changes.
The harness currently asserts inventory/fixture counts; update those assertions
deliberately when adding rows/cases. Run `make contract` and the relevant CLI
unit tests. The [CLI reference](cli.md) describes current syntax and semantics.

## Additional commands and options

`firmware` and `firmware check` are listed in `meltNativeCommands`. Firmware
checks also accept `--include-history` and `--include-public-catalogue`; those
flags are implemented options beyond the inventory's recorded core flag list.
AMS drying paths are listed in `commands`. `printer capabilities` and
`jobs preflight` are implemented but currently absent from this inventory.

`camera add|edit|link|list|remove|select|unlink` are listed in
`meltNativeCommands`. `camera snapshot` and `camera stream` also accept the
optional `--camera` and `--quality` flags while preserving the positional
printer form. Like `printer add --model`, these options supplement the
inventory's recorded core flags. The [CLI reference](cli.md) and command help
describe the complete supported options.

`camera stream --format` accepts `mjpeg` and `h264`. The
`camera-stream-invalid-format-json` fixture uses `h265` as its invalid value,
and its golden message lists both accepted formats.
