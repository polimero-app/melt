# Design and implementation records

Last-updated: 2026-10-12

These dated records preserve the original rationale, task breakdowns, examples
and review findings. They are not a current backlog or command reference.
Unchecked boxes, line numbers, baseline test counts, dependency versions and
commit instructions in historical plans describe the original work and must
not be treated as outstanding requirements. Current implementation notes below
and the [documentation index](../README.md) point to maintained guides.

| Record | State in the checked-in project | Current reference |
| --- | --- | --- |
| [August UI remediation](plans/2026-08-04-ui-review-remediation.md) | Monitoring badges/alerts/freshness, job telemetry, file scrolling and prominent emergency control implemented; state handling has since expanded | [Desktop](../desktop.md), [interface system](../../.interface-design/system.md) |
| [Bambu LAN fixes](plans/2026-08-06-bambu-lan-driver-fixes.md) | Observed fan verification, explicit authorization precedence, transport idle handling and associated tests implemented | [Compatibility](../bambu-compatibility.md) |
| [File preview design](specs/2026-08-09-file-preview-dialog-design.md) | Dialog and large renders implemented; queue priority, bounded cache and loading feedback added later | [Desktop library](../desktop.md#file-library-and-slicers) |
| [AMS drying](plans/2026-09-19-ams-drying.md) | Telemetry and gated CLI/desktop start/stop implemented; physical evidence remains scoped separately | [Drying contract](../bambu-compatibility.md#ams-drying) |
| [Model detection](plans/2026-09-19-bambu-model-detection.md) | Firmware product/project, serial and profile trust order with conflict provenance implemented | [Model detection](../bambu-compatibility.md#model-detection) |
| [Firmware availability](plans/2026-09-19-firmware-update-availability.md) | Core, CLI, Moonraker/Bambu checks and separate desktop cache/UI implemented; extended by hybrid sources | [Firmware evidence](../firmware-updates.md) |
| [Hybrid firmware provenance](plans/2026-09-19-hybrid-firmware-provenance.md) | Optional device history/public sources, provenance rails, assessment and source preference implemented; A1 mini/H2C wiki fallbacks added | [Firmware evidence](../firmware-updates.md) |
| [September UI remediation](plans/2026-09-25-ui-review-remediation.md) | Reviewed contrast, plate labels, retained errors, preview loading, control explanations, hierarchy/copy and side panels implemented | [Desktop](../desktop.md), [interface system](../../.interface-design/system.md) |
| [IP camera design](specs/2026-10-08-ip-cameras-design.md) | Phases 1/2 implemented; ONVIF pan/tilt and optional discovery remain planned | [IP cameras](../ip-cameras.md) |
| [IP camera phase 1](plans/2026-10-08-ip-cameras.md) | Shared RTSP, schema v2, Tapo profiles/links, CLI snapshot/stream implemented; native H.264 output added | [CLI](../cli.md#cameras), [Tapo evidence](../../fixtures/tapo/README.md) |
| [IP camera phase 2](plans/2026-10-08-ip-cameras-desktop.md) | Shared sources, viewer IDs, Cameras view and feed selection implemented; Linux native H.264 and diagnostics added | [Desktop cameras](../desktop.md#cameras-and-feed-selection) |

The existing web-command proposal in the working tree is unimplemented and
has been preserved separately. No `melt web` command, HTTP application API or
web user store is registered in the source. Reading that proposal does not
make its planned dependencies or release checks current project behavior.

“Implemented” describes source behavior, not proof that every original manual
acceptance step passed. Use [physical qualification](../bambu-physical-qualification.md)
and [release qualification](../release-qualification.md) for actual evidence.
Current Rust MSRV is 1.95, UI tests run through `bun run --cwd ui test`
(Vitest), and the fleet navigation label is **Printers**.
