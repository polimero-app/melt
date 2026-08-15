# CLI contract fixtures

`fixtures/cli-contract/inventory.json` is the migration inventory. It records
every public command in the Go contract, its arguments and flags, and whether
the Rust CLI implements it. Do not remove an inventory row when a command is
implemented; change its `implementation` value and add a fixture.

`fixtures/cli-contract/manifest.json` maps a reproducible invocation to its
exit code and JSON golden response. Golden responses are captured from the Go
working baseline at `6b67ff58b8b7a123f3e560f76c8c62a3a47e7a6d`. The accepted
contract names `16f2a9570affdc0ab20ee41b20d273bc4743a68d`, but that object is
not present in the local Go clone; replace these captures after obtaining it.

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

The Rust contract test executes every fixture against `melt-cli`, compares
the exit code, stderr, and JSON envelope, and removes only the documented
host-dependent JSON pointers before comparison.
