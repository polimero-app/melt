Release tag: <tag>
Commit: <commit SHA>
Workflow: <release workflow URL>

# Updater verification report

Last-updated: 2026-10-12

Template only. Melt currently has no application updater plugin/configuration,
signed updater manifest or in-app update channel. Installer/nightly delivery
and read-only printer firmware checks are separate. See
[release qualification](../../release-qualification.md).

## Automated checks

- [ ] The release workflow produced the intended installer artifacts.

## External/manual evidence

- [ ] Not applicable — no updater is configured. Record approver, date, and the `tauri.conf.json` revision inspected.

## Record

Inspected Melt commit and configuration revision:
Inspector/approver and UTC date:
Updater status and supporting evidence:

If an application updater is introduced, replace the not-applicable item with
real evidence: previous/target version, OS/architecture, signed manifest and
artifact source, key verification, observed success, and rollback/failure
behavior. Do not leave placeholders or claim an updater check from a printer
firmware report.
