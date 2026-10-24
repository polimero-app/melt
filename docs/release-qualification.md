# Release qualification

Last-updated: 2026-10-12

`make release-qualification` is the reproducible qualification gate. It
installs the locked UI dependencies, runs Rust formatting and Clippy, all Rust
and UI tests, builds the workspace and UI, checks every CLI fixture, and runs
the built `target/debug/melt version --output json` entry point with the
checked-in empty configuration directory. This is a build-tree executable
smoke test, not installation or launch verification of a native package.

## Tagged and manual package builds

[Release packaging](../.github/workflows/release.yml) runs on `v*` tag pushes
and manual dispatch. It runs the gate on both Linux runners and builds the native
installer matrix:

| Platform | Artifacts |
| --- | --- |
| Linux amd64, arm64 | deb, rpm |
| macOS amd64, arm64 | dmg |
| Windows amd64 | msi |

All jobs use the stable Rust toolchain and Bun 1.4.4. Linux installs GTK,
WebKitGTK, appindicator, SVG and RPM build dependencies; Windows sets the
runner's OpenSSL locations. Jobs install locked UI dependencies, build the UI,
install the Tauri CLI and run `cargo tauri build` with the platform bundle list.
The Tauri configuration points to `ui/dist` and includes the AGPL license and
application icons.

The workflow uploads `target/release/bundle/` as five workflow artifacts named
`<platform>-packages`. It does not create/publish a GitHub release, upload
updater metadata or configure application signing. Tagged builds run a
dependent frozen-evidence check after packaging; manual dispatch does not.
The checked-in version is `0.1.0` in Cargo, UI and Tauri metadata, and this
workflow does not derive it from the tag. Update version declarations before
tagging a differently versioned release; the desktop package test checks
Cargo/Tauri agreement.

The artifact builds and this gate are automatable. They do not establish that a
native installer launches on a real target OS, that a physical printer obeys a
control safely, or that a deployed updater serves a signed update. Those facts
need external/manual evidence because they depend on hardware, credentials,
network topology, signing infrastructure, or the released artifact. There are
no completed tag-specific evidence directories checked in at present.

## Nightly prereleases

[Nightly](../.github/workflows/nightly.yml) runs daily at 03:00 UTC and supports
manual dispatch. It compares the current commit with the latest dated tag and
skips packaging when they match. New runs create `YYYY.MM.DD` tags, using
`.2`, `.3`, etc. when the date is already tagged. A draft GitHub prerelease is
created before the same five-platform installer matrix runs.

The workflow overrides the Tauri bundle version with `YYYY.M.D` and the
Windows WiX/MSI version with `YY.M.D`; these are separate from the checked-in
Cargo/CLI version. It uploads installers to the prerelease and publishes only
after every package job succeeds. A failed package run removes its draft
release and tag. Cleanup retains the seven most recent dated prereleases and
deletes older dated releases and their tags.

Nightly packaging builds the UI and native artifacts but does **not** run
`make release-qualification` or the frozen-evidence gate. Ordinary CI provides
separate automated coverage. Nightly availability does not attest to manual
installer, hardware or updater qualification, and is not an in-app update
channel.

## Frozen tag evidence

Before creating a `v*` tag, add these two completed files and commit them with
the release:

```text
docs/releases/<tag>/release-packaging-checklist.md
docs/releases/<tag>/updater-verification-report.md
```

Copy `docs/releases/TEMPLATE` to start the two files, then replace every
placeholder and complete each checkbox.

Each file must contain an exact `Release tag: <tag>` line, non-empty `Commit:`
and `Workflow:` lines, and these headings. The templates put tag metadata first:

```text
## Automated checks
## External/manual evidence
```

Use `- [x]` only for evidence actually completed; do not leave `- [ ]` items.
The tag workflow runs:

```sh
make release-evidence RELEASE_TAG=<tag>
```

This mechanically checks file existence, the exact tag line, non-empty
commit/workflow lines, both headings, at least one checked item and no unchecked
items. It does not validate placeholder removal, SHA-to-tag agreement, workflow
success, artifact identity, signatures, or the truth of external evidence.
Reviewers must verify those facts and attest to the evidence in the files.

## Evidence content

In both files, list `Commit: <SHA>`, `Workflow: <URL>`, and the relevant
artifact or environment. Under **Automated checks**, record the successful command or
workflow job. Under **External/manual evidence**, record the person, date,
target OS/device, exact action, and observed result.

Record the exact candidate build SHA, package checksum and workflow job/run.
If committing the evidence changes the final tagged SHA, explain which source
revision produced the tested candidate and why the final change contains only
evidence. Do not describe a not-yet-run tag job as completed. Preview package
builds can use manual dispatch; their retained artifacts must be traceable to
the candidate that was tested.

The packaging checklist must cover installing, launching, and uninstalling
each produced installer on its target OS, including the packaged headless
`melt version --output json` command. It must also cover one configured
Moonraker printer: status, a non-destructive control, emergency-stop handling,
and confirmation that unsupported capabilities still report capability errors.
For releases that change Bambu behavior, attach the relevant completed rows
produced with
[`bambu-physical-qualification.md`](bambu-physical-qualification.md): printer and firmware identity, storage
transport, status stream, sliced-package preflight, AMS mapping, print-start
observation, and tested failure paths. Automated tests do not replace this
physical-printer evidence.

For camera changes, include scoped Tapo/desktop checks: add/edit/remove,
snapshot and both stream qualities, shared viewers and teardown, printer feed
selection while the printer is offline, native/fallback playback and redacted
diagnostics. State which OS/webview and camera hardware/firmware were exercised.
The checked-in SDP and simulated servers do not replace these live checks.

There is currently no updater configuration in `tauri.conf.json`. The updater
report must explicitly say `Not applicable — no updater is configured` until
signed updater delivery is introduced. Once it is configured, replace that
statement with a real update-path result: prior version, signed update source,
target version, platform, and observed rollback or failure behavior.

Read-only **Firmware & software** checks concern printer firmware/Klipper
software and are independent of application installer delivery. They neither
install printer updates nor prove an application updater is configured.
