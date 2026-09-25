# UI Review Remediation (September) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the eleven findings from the 2026-09-25 UI review of `screenshots/`: one contrast failure, three misleading or missing states, and seven hierarchy or copy problems.

**Architecture:** Frontend only (`ui/src/`). No Rust changes. The plate label comes out as a pure function in `ui/src/presentation.ts` with a vitest case. Everything else is template or class changes, checked by running the app.

**Tech Stack:** Vue 3 `<script setup>` + TypeScript, Tailwind v4, Headless UI, vitest, Tauri 2, Phosphor icons.

## Global Constraints

- **No new dependencies.** No component-testing library. Only pure logic gets unit tests. Template changes are checked by hand against the Manual Checks list.
- **All user-facing strings go through i18n.** Add each key to the `MessageKey` union in `ui/src/i18n.ts` and to both the `en` and `"pt-BR"` catalogs. `type Messages = Record<MessageKey, string>` (`i18n.ts:551`) makes a missing translation a compile error.
- **Commands:** tests `bun run --cwd ui test`, type-check and build `bun run --cwd ui build`. Never bare `bun test`: it runs Bun's own test runner, not vitest.
- **Do not touch** `--breakpoint-*` in `ui/src/style.css` or the window size in `tauri.conf.json`. The minimum CSS viewport is about 853 px, so every layout change must still work there.
- **Keep safety gates unchanged.** Commands stay enabled or disabled under exactly the same conditions (`selectedStatus?.state !== 'idle'`, capability flags, `activeHasStatus`). This plan only changes how those conditions are explained.
- **Follow `.interface-design/system.md`.** Cyan is for actions and selection only. Hack (mono) is for identifiers and telemetry.
- **Commit format:** conventional commits, one per task, ending with `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>`.

## Findings Coverage

| # | Finding | Task |
|---|---|---|
| 1 | Primary button text fails contrast (2.36:1 dark, 3.6:1 light) | 1 |
| 2 | Job card shows impossible "plate 2 of 1" | 2 |
| 3 | File refresh errors hidden while stale entries remain | 3 |
| 4 | Preview dialog is blank while the large render loads | 4 |
| 5 | Camera crowds the current-job card | 5 |
| 6 | Disabled controls give no reason | 5, 6 |
| 7 | Printer card puts firmware above everyday control | 7 |
| 8 | Filenames truncated past recognition | 8 |
| 9 | "Upload files" actually adds one file to the local library | 9 |
| 10 | Firmware conclusion buried under evidence badges | 10 |
| 11 | Configuration cards stretched to equal height | 11 |

---

## Task 1: Primary button contrast

**Files:**
- Modify: `ui/src/components/Button.vue:9`

Measured with the Tailwind v4 oklch values: white on `cyan-600` is 3.6:1, white on `cyan-500` is 2.36:1. White on `cyan-700` is 5.28:1. `gray-950` on `cyan-500` is 8.51:1, and on `cyan-400` 11.12:1.

- [ ] **Step 1: Replace the primary variant**

```ts
  primary:
    'bg-cyan-700 text-white shadow-xs hover:bg-cyan-800 dark:bg-cyan-500 dark:text-gray-950 dark:shadow-none dark:hover:bg-cyan-400',
```

Light hover must get darker, not lighter: `cyan-600` would fall back to 3.6:1.

- [ ] **Step 2: Verify**

Run `bun run --cwd ui build`. Open Manage printers ("Discover printers") and the Add-printer slide-over (Save) in both themes. Check the icon, label, hover state and the global cyan focus ring are all readable. The other `variant="primary"` call sites (`App.vue:2650, 3252, 3260, 3945`) pick up the fix automatically.

- [ ] **Step 3: Commit** — `fix(ui): meet text contrast on primary buttons`

---

## Task 2: Never show an impossible plate total

**Files:**
- Modify: `ui/src/presentation.ts`, `ui/src/presentation.test.ts`
- Modify: `ui/src/App.vue:2763-2765`
- Modify: `ui/src/i18n.ts` (add `control.plate`)

`transport.rs:3729-3735` passes Bambu's `plate_idx` (or `plate_id`) and `plate_cnt` through unchanged. A plate taken from a multi-plate project and sent as a one-plate job can report index 2 with count 1. That data is correct, so no backend fix is needed. The UI just must not show it as a fraction.

- [ ] **Step 1: Write the failing test** (append to `presentation.test.ts`, import `plateLabel`)

```ts
describe('plateLabel', () => {
  it('shows a fraction only when the total can contain the index', () => {
    expect(plateLabel(2, 3)).toEqual({ key: 'control.plateOf', values: { index: 2, total: 3 } })
    expect(plateLabel(1, 1)).toEqual({ key: 'control.plateOf', values: { index: 1, total: 1 } })
  })

  it('drops a missing or contradictory total instead of inventing one', () => {
    expect(plateLabel(2, 1)).toEqual({ key: 'control.plate', values: { index: 2 } })
    expect(plateLabel(2, undefined)).toEqual({ key: 'control.plate', values: { index: 2 } })
  })
})
```

- [ ] **Step 2: Run** `bun run --cwd ui test` — expected FAIL (`plateLabel` is not exported).

- [ ] **Step 3: Implement** in `presentation.ts`

```ts
// Bambu reports the project plate number alongside the job's own plate
// count, so "plate 2 of 1" is real data — just not a meaningful fraction.
export function plateLabel(index: number, total: number | undefined): { key: MessageKey; values: Record<string, number> } {
  return total !== undefined && total >= index
    ? { key: 'control.plateOf', values: { index, total } }
    : { key: 'control.plate', values: { index } }
}
```

- [ ] **Step 4: Add the key** — `| "control.plate"` next to `control.plateOf`. en: `"plate {index}"`. pt-BR: `"placa {index}"`.

- [ ] **Step 5: Wire it** — replace the body of the `<span>` at `App.vue:2764`:

```html
· {{ t(plateLabel(selectedStatus.printMeta.plateIndex, selectedStatus.printMeta.plateCount).key, plateLabel(selectedStatus.printMeta.plateIndex, selectedStatus.printMeta.plateCount).values) }}
```

Add `plateLabel` to the `./presentation` import on `App.vue:12`.

- [ ] **Step 6: Verify** `bun run --cwd ui test && bun run --cwd ui build` both pass.

- [ ] **Step 7: Commit** — `fix(ui): drop contradictory plate totals from the job card`

---

## Task 3: Keep refresh failures visible when entries remain

**Files:**
- Modify: `ui/src/App.vue` (printer files card near `:2968`, library grid near `:3724`)
- Modify: `ui/src/i18n.ts` (add `common.refreshFailed`)

Right now `filesError` (`App.vue:3036`) and `libraryFilesError` (`:3774`) only render inside the empty-list placeholder. Neither loader clears its old entries on failure (`loadFiles` `:1310`, `loadLibraryFiles` `:1334`), so a failed refresh quietly leaves stale data looking current.

- [ ] **Step 1: Add the key** — en: `"Couldn't refresh: {reason}"`. pt-BR: `"Não foi possível atualizar: {reason}"`.

- [ ] **Step 2: Printer files** — directly under the card's `<CardHeader>`, before the scroll region:

```html
<div v-if="filesError && printerFiles.length" class="flex items-center justify-between gap-3 border-b border-amber-300/70 bg-amber-50 px-4 py-2 text-xs text-amber-900 sm:px-6 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-200" role="alert">
  <span class="min-w-0">{{ t('common.refreshFailed', { reason: filesError }) }}</span>
  <Button :disabled="filesLoading" @click="loadFiles">{{ t('common.retry') }}</Button>
</div>
```

Amber, because retained data during a failed refresh is the "reconnecting" meaning in `system.md`.

- [ ] **Step 3: Library** — the same block above the directory grid (`:3709`), with `px-4 sm:px-0 mb-5 rounded-md border` instead of the card-edge border. Condition: `libraryFilesError && libraryFiles.length`. Retry calls `loadLibraryFiles(libraryPath)`.

- [ ] **Step 4: Verify** — load both lists, stop the printer or rename the library folder, then refresh. The notice appears above the old entries. Retry after restoring clears it. The empty-list text stays as it is today.

- [ ] **Step 5: Commit** — `fix(ui): surface file refresh failures above retained entries`

---

## Task 4: Preview loading state

**Files:**
- Modify: `ui/src/components/FilePreviewDialog.vue`
- Modify: `ui/src/App.vue:3895-3902` (pass the label)
- Modify: `ui/src/i18n.ts` (add `filesView.previewLoading`)

- [ ] **Step 1: Track loading** — add `const loading = ref(false)`. In the watcher, set `loading.value = true` after the `if (!file) return` guard. Clear it in a `finally` only when `!controller.signal.aborted`, so a cancelled render can't clear the loading state of the next file.

- [ ] **Step 2: Render it** — add a `loadingLabel: string` prop. Replace the placeholder `<p>` at `:102`:

```html
<p v-else class="px-4 py-16 text-sm text-cyan-900/60 dark:text-cyan-100/60" role="status">{{ failed ? unavailableLabel : loading ? loadingLabel : '' }}</p>
```

A cached thumbnail sets `source` right away, so the loading text only appears when there's nothing to show yet.

- [ ] **Step 3: Add the key and pass it** — en: `"Rendering preview…"`. pt-BR: `"Gerando pré-visualização…"`. Pass `:loading-label="t('filesView.previewLoading')"`.

- [ ] **Step 4: Verify** — open a large uncached STL (loading text, then image), then open and close several quickly (no stale image or stuck loading text).

- [ ] **Step 5: Commit** — `feat(ui): show preview loading state`

---

## Task 5: Give the current job room and hide actions without a job

**Files:**
- Modify: `ui/src/App.vue:2702` (grid), `:2754-2760` (job name), `:2773-2779` (layer / remaining row), `:2795-2812` (actions)

- [ ] **Step 1: Split** — `xl:grid-cols-[minmax(0,3fr)_minmax(18rem,1fr)]` becomes `xl:grid-cols-[minmax(0,2fr)_minmax(18rem,1fr)]`. The camera keeps `object-contain`, so nothing is cropped.

- [ ] **Step 2: Two-line job name** — on the name `<p>`, replace `truncate` with `line-clamp-2 break-all`. Keep `:title` and `translate="no"`.

- [ ] **Step 3: Emphasize remaining time** — on the remaining-time `<span>`, change `font-mono` to `font-mono text-sm font-semibold text-gray-900 dark:text-white`. Layer count stays secondary.

- [ ] **Step 4: Actions only with a job** — wrap the Pause/Resume/Cancel `<div class="mt-6 flex gap-2">` and the progress block in `v-if="hasActiveJob"`. Keep the header's "% complete" only when `hasActiveJob`. The per-button `:disabled` conditions stay exactly as they are. The speed profile select already disables itself outside printing/paused, so leave it alone.

- [ ] **Step 5: Verify** at 853 px and at full width, for an idle printer (name + "idle" + speed only), printing, and paused. Check that nothing overflows and the camera isn't cropped.

- [ ] **Step 6: Commit** — `feat(ui): prioritize the current job over camera padding`

---

## Task 6: Explain controls locked while printing

**Files:**
- Modify: `ui/src/App.vue:2837` (temperature header), `:2917` (motion header)
- Modify: `ui/src/i18n.ts` (add `control.availableWhenIdle`)

- [ ] **Step 1: Add the key** — en: `"Available when idle"`. pt-BR: `"Disponível com a impressora ociosa"`.

- [ ] **Step 2: One line per card, not per control** — `CardHeader`'s default slot renders on the right. For the temperature card:

```html
<CardHeader :title="t('dashboard.temperature')" :icon="PhThermometerSimple">
  <span v-if="capabilities?.temperatureWrite && selectedStatus?.state !== 'idle'" class="text-xs text-gray-500 dark:text-gray-400">{{ t('control.availableWhenIdle') }}</span>
</CardHeader>
```

Do the same for motion, gated on `capabilities?.motionControl`. Unsupported controls keep their existing "Telemetry only" text, which is a different reason.

- [ ] **Step 3: Verify** — printing: both hints show and the controls stay disabled. Idle: no hints.

- [ ] **Step 4: Commit** — `feat(ui): explain temperature and motion locks while printing`

---

## Task 7: Printer card action hierarchy

**Files:**
- Modify: `ui/src/App.vue:1471-1481` (`printerActionItems`), `:3284` (trash button), `:3333-3340` (buttons)

- [ ] **Step 1: Removal into the menu** — append to `printerActionItems`:

```ts
{ label: t('printersView.removePrinter'), icon: PhTrash, danger: true, onSelect: () => removePrinter(printer.name) },
```

`ActionMenu` already puts danger items in their own section, and `removePrinter` already asks for confirmation. Delete the trash `IconButton` at `:3284`.

- [ ] **Step 2: Swap order** — the `Open control` + `ActionMenu` row comes first. `Firmware & software` goes below it with `mt-2`. Both stay `variant="secondary"`, so hierarchy comes from position and there isn't a second cyan fill on a card that already has cyan accents.

- [ ] **Step 3: Verify** — removal from the menu shows the confirm dialog, Escape cancels it, and focus goes back to the menu button.

- [ ] **Step 4: Commit** — `refactor(ui): lead printer cards with Open control`

---

## Task 8: Readable filenames

**Files:**
- Modify: `ui/src/App.vue:3761-3765` (library card body), `:3001` (printer file cell)

- [ ] **Step 1: Library cards** — remove the `<PhFile>` icon (the preview already shows the type). On the `<h3>`, replace `truncate` with `line-clamp-2 break-all`. Keep `:title`.

- [ ] **Step 2: Printer table** — the cell keeps `max-w-0` so the column still sizes. Move the text into an inner span:

```html
<td class="max-w-0 px-4 py-4 font-mono text-sm font-medium text-gray-900 sm:px-6 dark:text-white" :title="file.name" translate="no"><span class="line-clamp-2 break-all">{{ file.name }}</span></td>
```

Size and modified columns already hide below `1000px` and `1180px`. Leave them.

- [ ] **Step 3: Verify** — the Olaf folder (`Olaf_body01` and `Olaf_body02` are distinguishable), a 120-character name with no spaces, and a short name. Action buttons never move or overlap.

- [ ] **Step 4: Commit** — `fix(ui): wrap filenames to two lines instead of truncating`

---

## Task 9: Rename "Upload files" and delete dead upload copy

**Files:**
- Modify: `ui/src/i18n.ts`

`uploadFile()` (`App.vue:1923`) opens a single-select picker and copies into the local library. `filesView.uploadDescription`, `uploadFile`, `uploadHint`, `uploadingTo` and `uploadConfirm` are referenced nowhere outside `i18n.ts`.

- [ ] **Step 1: Retext** — `filesView.upload`: en `"Add file"`, pt-BR `"Adicionar arquivo"`. `filesView.uploadedTo`: en `"File added to {directory}"`, pt-BR `"Arquivo adicionado a {directory}"`. Keys stay the same, so `App.vue` doesn't change.

- [ ] **Step 2: Delete** the five unused keys from the union and both catalogs. Leave `filesView.uploadedToPrinter` alone: it's printer upload, not the library.

- [ ] **Step 3: Verify** `bun run --cwd ui build` passes (a remaining reference would fail type-checking). Add a file and check the toast.

- [ ] **Step 4: Commit** — `fix(ui): describe library import as adding a file`

---

## Task 10: Firmware conclusion first

**Files:**
- Modify: `ui/src/App.vue:3159-3172` (assessment and source rows), `:3213-3215` (issues list)
- Modify: `ui/src/i18n.ts` (add `firmware.evidence`)

The explanation users need ("The public catalogue reported a version older than the installed version.") already exists in `report.issues`. It just renders last.

- [ ] **Step 1: Lead with the assessment and its explanation** — keep the assessment pill. Move the `<ul v-if="selectedFirmwareUpdate.report.issues.length">` right below it and change its classes to `text-sm text-gray-700 dark:text-gray-300`.

- [ ] **Step 2: Put evidence behind a native disclosure** — wrap the evidence-role chips and the `selectedFirmwareUpdate.sources` row in:

```html
<details class="mb-4 text-xs text-gray-500 dark:text-gray-400">
  <summary class="cursor-pointer select-none">{{ t('firmware.evidence') }} · {{ selectedFirmwareUpdate.report.components.length }} {{ t('firmware.modules') }} · {{ firmwareComparableCount(selectedFirmwareUpdate) }} {{ t('firmware.comparable') }}</summary>
  <!-- existing chips + sources row -->
</details>
```

Keys: en `"Evidence"`, pt-BR `"Evidências"`. The per-module version list stays visible, because it's the primary data. The stale warning and `selectedFirmwareUpdate.error` stay visible too.

- [ ] **Step 3: Verify** — check the "Sources disagree" printer from the screenshot, a current printer, and a stale report. The conclusion reads first, nothing implies "up to date" when sources conflict, and the disclosure is keyboard-operable.

- [ ] **Step 4: Commit** — `feat(ui): lead firmware reports with the assessment`

---

## Task 11: Stop stretching configuration cards

**Files:**
- Modify: `ui/src/App.vue:3363`

- [ ] **Step 1:** `grid max-w-[1500px] gap-5 lg:grid-cols-2 xl:grid-cols-3` becomes `grid max-w-[1500px] items-start gap-5 lg:grid-cols-2 xl:grid-cols-3`. DOM and tab order stay the same.

- [ ] **Step 2: Verify** at 853 px (two columns) and full width (three columns). Short cards fit their content, and rows stay aligned at the top.

- [ ] **Step 3: Commit** — `fix(ui): let configuration cards size to content`

---

## Manual Checks (after all tasks)

- [ ] `bun run --cwd ui test && bun run --cwd ui build` pass.
- [ ] Light and dark themes, at an 853 px CSS width and at full width. Also check at 1.5× display scaling.
- [ ] Keyboard only: tab order in the nav, printer cards, file cards, firmware disclosure and dialogs. The focus ring stays visible and focus goes back to where it was when dialogs close.
- [ ] States: idle, printing, paused, reconnecting, offline. Empty folder, active search with no matches, refresh failure with entries.
- [ ] Portuguese locale: no overflow from the longer strings (`Disponível com a impressora ociosa`, `Adicionar arquivo`).
- [ ] Recapture the six images in `screenshots/` from the same printers and folders.
