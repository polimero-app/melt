Release tag: <tag>
Commit: <commit SHA>
Workflow: <release workflow URL>

# Release packaging checklist

Last-updated: 2026-10-12

Template only; no qualification is claimed. Copy to `docs/releases/<tag>/`,
replace placeholders, and record candidate SHA, package/checksum, operator,
UTC date, target OS/device, action and observed result for each check.
See [release qualification](../../release-qualification.md) for the gate and
candidate/final commit relationship. Resolve not-applicable items explicitly
with a reason and review; do not mark an unperformed test as passed.

## Automated checks

- [ ] `make release-qualification` passed for this commit.
- [ ] The release workflow built and retained every required package artifact.
- [ ] Cargo/Tauri/UI version declarations match the intended release, and the tested candidate is traceable to the final tag.
- [ ] `make release-evidence RELEASE_TAG=<tag>` passed after these records were completed.

## External/manual evidence

- [ ] Each target installer was installed, launched, and uninstalled on its target OS; record platform, artifact, person, date, and result.
- [ ] The packaged `melt version --output json` command succeeded on each target OS; record platform, person, date, and result.
- [ ] A configured Moonraker printer reported status and accepted one non-destructive control; record device, person, date, and result.
- [ ] Emergency stop behavior and unsupported-capability errors were observed on that printer; record device, person, date, and result.
- [ ] Bambu changes have completed [physical qualification rows](../../templates/bambu-qualification-row.md) for the changed behaviors, or are explicitly not applicable with a scoped reason.
- [ ] Camera changes have scoped Tapo and desktop playback/selection/teardown/diagnostics evidence, or are explicitly not applicable with a scoped reason.

## Artifact and device records

| Platform/device | Candidate SHA and artifact/checksum | Operator/date | Actions and observed result | Evidence location |
| --- | --- | --- | --- | --- |
| <platform or scoped model/firmware> | <record> | <record> | <record> | <record> |

## Exceptions and approval

Not-applicable items and reasons:
Remaining limitations:
Release approver and UTC date:
