# Release qualification

`make release-qualification` is the reproducible qualification gate. It
installs the locked UI dependencies, runs Rust formatting and Clippy, all Rust
and UI tests, builds the workspace and UI, checks every CLI fixture, and runs
the installed `polimero version --output json` entry point with an empty
configuration directory.

The tagged release workflow runs that gate on Linux, then builds the native
installer matrix:

| Platform | Artifacts |
| --- | --- |
| Linux amd64, arm64 | deb, rpm |
| macOS amd64, arm64 | dmg |
| Windows amd64 | msi |

The artifact builds and this gate are automatable. They do not establish that a
native installer launches on a real target OS, that a physical printer obeys a
control safely, or that a deployed updater serves a signed update. Those facts
need external/manual evidence because they depend on hardware, credentials,
network topology, signing infrastructure, or the released artifact.

## Frozen tag evidence

Before creating a `v*` tag, add these two completed files and commit them with
the release:

```text
docs/releases/<tag>/release-packaging-checklist.md
docs/releases/<tag>/updater-verification-report.md
```

Copy `docs/releases/TEMPLATE` to start the two files, then replace every
placeholder and complete each checkbox.

Each file must start with `Release tag: <tag>` and contain these headings:

```text
## Automated checks
## External/manual evidence
```

Use `- [x]` only for evidence actually completed; do not leave `- [ ]` items.
The tag workflow runs:

```sh
make release-evidence RELEASE_TAG=<tag>
```

This mechanically verifies the frozen files, tag binding, required
classification, and completion markers. It cannot verify the truth of
external evidence; the release approver attests to that evidence in the files.

## Evidence content

In both files, list `Commit: <SHA>`, `Workflow: <URL>`, and the relevant
artifact or environment. Under **Automated checks**, record the successful command or
workflow job. Under **External/manual evidence**, record the person, date,
target OS/device, exact action, and observed result.

The packaging checklist must cover installing, launching, and uninstalling
each produced installer on its target OS, including the packaged headless
`polimero version --output json` command. It must also cover one configured
Moonraker printer: status, a non-destructive control, emergency-stop handling,
and confirmation that unsupported capabilities still report capability errors.

There is currently no updater configuration in `tauri.conf.json`. The updater
report must explicitly say `Not applicable — no updater is configured` until
signed updater delivery is introduced. Once it is configured, replace that
statement with a real update-path result: prior version, signed update source,
target version, platform, and observed rollback or failure behavior.
