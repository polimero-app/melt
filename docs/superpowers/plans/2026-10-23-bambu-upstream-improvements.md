# Bambu upstream correctness and monitoring improvements

Date: 2026-10-23

## Scope

Implement the correctness fixes and monitoring enrichments selected during the
upstream review. Reuse the shared Rust status and preflight paths, the existing
CLI and desktop presentation, and the existing evidence/test tooling. Do not
add a second protocol client or new dependencies.

## Atomic commit sequence

1. Document this plan and add/enrich the upstream references in the root README.
2. Correct AMS humidity: retain the reported relative-humidity percentage
   separately from the reversed ordinal index. Validate finite readings,
   preserve zero and missing values distinctly, and update CLI/desktop output.
3. Respect physical spool presence: decode `tray_exist_bits`, retain unknown
   presence when it is absent, and exclude explicitly absent trays from print
   preflight even when their cached material metadata still matches.
4. Add bounded exact-model capability defaults from pinned manufacturer
   profiles. Keep firmware scope and provenance visible; explicit live values,
   including false, outrank defaults. Do not infer capabilities for unknown
   models or alter authorization, TLS, transport negotiation, or delta merging.
5. Add offline HMS and print-error descriptions for English and Brazilian
   Portuguese. Preserve model-specific variants, raw identifiers, existing
   severity/alert policy, useful wiki links, upstream notices, and a reproducible
   source-data update path. Unknown codes retain an explicit fallback.
6. Estimate remaining filament only from valid internal-spool data when
   reported grams are unavailable. Reported zero must not trigger an estimate.
   Mark estimates as approximate and never infer an external spool's quantity.
7. Report the active filament using the active extruder's `snow` field on modern
   firmware and `tray_now` on legacy reports. Respect extruder IDs, AMS HT
   numbering, external/no-filament sentinels, and unknown values in CLI/desktop
   presentation.

Each behavior commit includes focused regression checks, directly dependent
call-site/type changes, and maintained compatibility documentation. Shared
evidence is introduced with the first behavior that exercises it.

## Reviewed sources

| Source | Pinned revision | Use |
| --- | --- | --- |
| [ha-bambulab](https://github.com/greghesp/ha-bambulab) | `0e027ff135a6d9265cb756d3e246747954c76722` | P2S report shape, humidity direction, model-aware HMS/print-error text and wiki catalogues; MIT |
| [Bambuddy](https://github.com/maziggy/bambuddy) | `98ecaf5020e3a20a5de49859b8ad5a754d5ad85c` | Presence-bit precedence and humidity regression cases; AGPLv3 |
| [Bambu Studio](https://github.com/bambulab/BambuStudio) | `79df7ae309be398ed30c9f397c13c9911c2edc57` | Manufacturer profiles, physical presence, remaining-weight precedence, active extruder/tray parsing, and settings format; AGPLv3 |
| [Bambu Bar](https://github.com/rolottr/bambu-bar) | `474224a4c69d3897b9e22f042b361aff5f76fa3e` | Secondary source index and monitoring presentation; MIT |
| [bambu-cli](https://github.com/tobiasbischoff/bambu-cli) | `bba9349dc76165a79d8846a448da50a8acda20be` | Protocol reference only; no declared license found, no implementation copied |

The P2S sample in Bambu Bar is byte-identical to ha-bambulab's
`custom_components/bambu_lab/pybambu/mock_data/MOCK-P2S.json`, which identifies
OTA firmware `01.01.00.00`. It is not a second capture or evidence for Bambu
Bar's separately stated `01.03.00.00` development setup.

## Evidence and acceptance

- Add minimal, sanitized, manifest-governed evidence tied to the source
  revisions. Preserve source/executable-test provenance and provisional
  hardware confidence; do not claim new physical-printer qualification.
- Exercise raw-versus-index humidity, absent-versus-unknown occupancy, stale
  metadata in preflight, profile/live conflicts, unknown models, model-specific
  and missing error translations, reported-versus-estimated grams, and
  modern/legacy/external/HT active-slot selection.
- Run focused Rust and Vitest cases while implementing, then the necessary
  workspace tests, CLI contract fixtures, Bambu evidence validation, rustfmt,
  Clippy, and UI type-check/production build. Use the repository's Bun scripts.
- Review the final diff for unrelated changes, credentials, copied raw device
  identifiers, license notices, and independently reviewable commits before
  publishing the pull request.

## Deferred

Explicit Bambu Studio/OrcaSlicer settings import, filament vendor/colour-name
enrichment, a cancellation/outcome model redesign, new history/notification
features, native widgets, and printer artwork are outside this PR. Record them
as future options rather than scaffolding them here. Maintain the existing
array-replacement delta semantics confirmed by Bambu Studio.

Current behavior and qualification limits belong in
[Bambu compatibility](../../bambu-compatibility.md) and the
[protocol evidence policy](../../bambu-protocol-evidence.md).
