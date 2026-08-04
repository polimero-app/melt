# UI Review Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface the printer state the backend already reports but the UI discards, and fix the controls and feedback that currently mislead the operator.

**Architecture:** All work is in the Vue frontend (`ui/src/`). No Rust changes are required — every field this plan renders is already parsed and serialized by `polimero-core`. Logic that can be tested is extracted into pure functions in `ui/src/monitoring.ts` or a new `ui/src/formatting.ts` and covered by vitest; template wiring is verified by running the app.

**Tech Stack:** Vue 3 `<script setup>` + TypeScript, Tailwind v4, vitest, Tauri 2, Phosphor icons.

## Global Constraints

- **No new dependencies.** There is no component-testing library and this plan does not add one. Testable logic goes in pure modules; template changes are verified manually.
- **All user-facing strings go through i18n.** Add the key to the `MessageKey` union in `ui/src/i18n.ts` AND to both the `en` and `"pt-BR"` catalogs. `type Messages = Record<MessageKey, string>` (`i18n.ts:376`) makes a missing translation a compile error.
- **Type-check every task** with `bun run --cwd ui build` (runs `vue-tsc --noEmit` first). Records keyed by `PrinterBadge` will fail to compile until every variant is handled — that is intentional and is the safety net for Task 1.
- **Test command:** `bun test --cwd ui` (the `make ui-test` target).
- **Commit format:** conventional commits, ending with `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.
- **Do not touch** `--breakpoint-*` in `ui/src/style.css` or the window size in `tauri.conf.json`; those were settled on this branch.

## Findings Coverage

| # | Finding | Task |
|---|---|---|
| 1 | `error` state renders as "Offline" | 1 |
| 2 | `errors[]` / `warnings[]` never rendered | 2 |
| 3 | No "as of" timestamp | 3 |
| 4 | No time remaining | 5 |
| 5 | `preparation_percent` ignored | 6 |
| 6 | Job card has no thumbnail | 7 |
| 7 | File table hardcoded to 3 rows | 8 |
| 8 | Progress bar invisible to screen readers | 6 |
| 9 | Emergency stop buried in kebab menu | 9 |
| 10 | Errors render as successes | 4 |
| 11 | Fan slider doesn't track drag | 10 |
| 12 | Temperature control is ±5 only | 11 |
| 13 | Printers can't be edited | 12 |
| 14 | `downloadProgress` is dead state | 4 |
| 15 | Files view has no sort | 13 |
| 16 | Nav mixes printers with sections | 14 |
| 17 | `StatusBadge` duplicates English labels | 1 |

---

## Task 1: Distinguish printer faults from unreachable printers

Fixes findings 1 and 17. A printer reporting `state: 'error'` currently collapses into the `offline` badge, and the control panel unmounts in favour of "Printer unavailable — check your connection".

**Files:**
- Modify: `ui/src/monitoring.ts:1` (badge union), `:12-22` (`monitorBadge`)
- Modify: `ui/src/monitoring.test.ts:15-18`
- Modify: `ui/src/components/StatusBadge.vue:2,6-35`
- Modify: `ui/src/App.vue:307-315` (`statusDotClasses`), `:469` (`isReachable`), `:470` (`activeHasStatus`), `:476-484` (`badgeMessageKeys`)
- Modify: `ui/src/i18n.ts` (add `status.errorLabel`)

**Interfaces:**
- Produces: `PrinterBadge` gains the `'error'` variant. Every `Record<PrinterBadge, …>` in the codebase must handle it. Tasks 2 and 3 rely on `activeHasStatus` staying true during a fault.

- [ ] **Step 1: Update the failing test**

In `ui/src/monitoring.test.ts`, replace the assertion on line 17:

```ts
  it('reports offline only when no usable status remains', () => {
    expect(monitorBadge({ error: { code: 'printerTimeout' }, stale: true })).toBe('offline')
    expect(monitorBadge({ status: { state: 'unknown' } })).toBe('offline')
  })

  it('separates a reported fault from an unreachable printer', () => {
    expect(monitorBadge({ status: { state: 'error' } })).toBe('error')
    expect(monitorBadge({ status: { state: 'error' }, connectionState: 'live' })).toBe('error')
    // A fault we can no longer confirm is not a fault we should still assert.
    expect(monitorBadge({ status: { state: 'error' }, connectionState: 'offline' })).toBe('offline')
  })
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test --cwd ui`
Expected: FAIL — `expected 'offline' to be 'error'`

- [ ] **Step 3: Add the variant and the branch**

In `ui/src/monitoring.ts`, line 1:

```ts
export type PrinterBadge = 'idle' | 'busy' | 'error' | 'connecting' | 'synchronizing' | 'reconnecting' | 'offline' | 'unknown'
```

Then replace line 19 (`if (entry.status.state === 'error' || entry.status.state === 'unknown') return 'offline'`) with:

```ts
  if (entry.status.state === 'error') return 'error'
  if (entry.status.state === 'unknown') return 'offline'
```

The existing `connectionState === 'offline'` check on line 17 already runs first, so a fault we can no longer confirm still reports offline.

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test --cwd ui`
Expected: PASS (8 tests)

- [ ] **Step 5: Handle the new variant everywhere the compiler demands**

Run `bun run --cwd ui build` and fix each error it reports. The three sites are:

`ui/src/App.vue:307-315`:

```ts
const statusDotClasses: Record<PrinterBadge, string> = {
  idle: 'bg-green-500 ring-green-500/10',
  busy: 'bg-yellow-500 ring-yellow-500/10',
  error: 'bg-red-600 ring-red-600/20',
  reconnecting: 'bg-amber-500 ring-amber-500/10',
  connecting: 'bg-gray-400 ring-gray-400/10',
  synchronizing: 'bg-cyan-500 ring-cyan-500/10',
  offline: 'bg-red-500 ring-red-500/10',
  unknown: 'bg-gray-400 ring-gray-400/10',
}
```

`ui/src/App.vue:476-484`, add to `badgeMessageKeys`:

```ts
  error: 'status.errorLabel',
```

`ui/src/components/StatusBadge.vue` — add `'error'` to the `PrinterStatus` union on line 2, then to each map. Give it a solid fill so it is not mistaken for the tinted `offline` chip:

```ts
  error: 'bg-red-600 text-white dark:bg-red-500 dark:text-white',
```
```ts
  error: 'fill-white dark:fill-white',
```

- [ ] **Step 6: Delete the duplicated English labels (finding 17)**

`StatusBadge.vue` carries a `labels` record that duplicates the i18n catalog and is dead in practice — every call site passes `label`. Delete the `labels` const entirely and make the prop required:

```ts
const props = defineProps<{ status: PrinterStatus; label: string }>()
```

In the template, replace `{{ props.label ?? labels[props.status] }}` with:

```html
    {{ props.label }}
```

- [ ] **Step 7: Add the i18n key**

In `ui/src/i18n.ts`, add `| "status.errorLabel"` to the `MessageKey` union next to `"status.offlineLabel"` (line 121), then add to both catalogs:

```ts
    "status.errorLabel": "Error",
```
```ts
    "status.errorLabel": "Erro",
```

- [ ] **Step 8: Keep the control panel mounted during a fault**

`ui/src/App.vue:470`. A faulting printer is reachable enough to show its own panel, but must not be offered as a print target — so `isReachable` deliberately stays false for `error`:

```ts
const activeHasStatus = computed(() => isReachable(activeBadge.value) || activeBadge.value === 'reconnecting' || activeBadge.value === 'error')
```

- [ ] **Step 9: Verify**

Run: `bun test --cwd ui && bun run --cwd ui build`
Expected: 8 tests pass, build succeeds with no type errors.

- [ ] **Step 10: Commit**

```bash
git add ui/src/monitoring.ts ui/src/monitoring.test.ts ui/src/components/StatusBadge.vue ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
fix(ui): distinguish printer faults from unreachable printers

state:'error' collapsed into the offline badge, so a thermal fault or
filament runout looked identical to an unplugged machine — and because
offline is not reachable, the control panel unmounted in favour of
"check your connection" while the printer was reporting over a healthy
link. Add a distinct error badge and keep the panel mounted for it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Render printer errors and warnings

Fixes finding 2. `PrinterStatus.errors[]` and `.warnings[]` are declared, populated by the backend, and read by nothing.

**Files:**
- Modify: `ui/src/App.vue:167-168` (widen the types), `:1700` (insert panel above the camera row)
- Modify: `ui/src/i18n.ts`

**Interfaces:**
- Consumes: the `error` badge from Task 1.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Widen the error type to the fields the backend already sends**

`ui/src/App.vue:167-168`. `StatusError` in `moonraker.rs:1029` also carries `rawCode`, `imageId`, and `recoverable`:

```ts
  errors: { code: string; message: string; rawCode?: string; recoverable?: boolean }[]
  warnings: { code: string; message: string }[]
```

- [ ] **Step 2: Add a computed for the combined fault list**

In `ui/src/App.vue`, next to `progressPercent` (line 486):

```ts
const statusFaults = computed(() => {
  const status = selectedStatus.value
  if (!status) return []
  return [
    ...status.errors.map((entry) => ({ ...entry, severity: 'error' as const })),
    ...status.warnings.map((entry) => ({ ...entry, severity: 'warning' as const, recoverable: undefined })),
  ]
})
```

- [ ] **Step 3: Render the panel**

In `ui/src/App.vue`, insert immediately after the `reconnecting` banner block (which closes at line 1707, just before `<section class="grid gap-5 grid-cols-[3fr_1fr]">`):

```html
          <section v-if="statusFaults.length" class="mb-5 space-y-2" :aria-label="t('control.faults')">
            <div
              v-for="fault in statusFaults"
              :key="`${fault.severity}-${fault.code}`"
              class="flex items-start gap-3 rounded-lg border px-4 py-3 text-sm"
              :class="fault.severity === 'error'
                ? 'border-red-300 bg-red-50 text-red-900 dark:border-red-400/20 dark:bg-red-400/10 dark:text-red-200'
                : 'border-amber-300/70 bg-amber-50 text-amber-900 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-200'"
              :role="fault.severity === 'error' ? 'alert' : 'status'"
            >
              <PhWarning class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              <div class="min-w-0 flex-1">
                <p class="font-medium">{{ fault.message }}</p>
                <p class="mt-0.5 font-mono text-xs opacity-75">
                  {{ fault.rawCode ?? fault.code }}
                  <span v-if="fault.recoverable === false"> · {{ t('control.faultUnrecoverable') }}</span>
                </p>
              </div>
            </div>
          </section>
```

- [ ] **Step 4: Add the i18n keys**

Add `| "control.faults"` and `| "control.faultUnrecoverable"` to the `MessageKey` union, then to both catalogs:

```ts
    "control.faults": "Printer alerts",
    "control.faultUnrecoverable": "not recoverable",
```
```ts
    "control.faults": "Alertas da impressora",
    "control.faultUnrecoverable": "não recuperável",
```

- [ ] **Step 5: Verify**

Run: `bun run --cwd ui build`
Expected: build succeeds.

Then run `make run` and confirm: with a healthy printer no panel appears; the `progress_unavailable` / `temperature_data_unavailable` warnings that `transport.rs:2457,2478` emit render as amber rows when a printer reports no temperature or progress data.

- [ ] **Step 6: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): surface printer errors and warnings

The backend parses every fault code and warning the printer emits and
the UI dropped all of them. Render them above the camera row, errors as
alerts and warnings as status rows, including the raw firmware code and
the recoverable flag the backend already sends.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Show how fresh the reading is

Fixes finding 3. `MonitorEntry.observedAt` is declared and never displayed.

**Files:**
- Create: `ui/src/formatting.ts`
- Create: `ui/src/formatting.test.ts`
- Modify: `ui/src/App.vue` (import, computed, header markup at `:1677`)
- Modify: `ui/src/i18n.ts`

**Interfaces:**
- Produces: `formatting.ts` exporting `relativeAge(observedAt: string | undefined, nowMs: number): { seconds: number; bucket: 'live' | 'recent' | 'stale' } | undefined`. Task 5 adds `formatDuration` to the same file.

- [ ] **Step 1: Write the failing test**

Create `ui/src/formatting.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import { relativeAge } from './formatting'

describe('relative age', () => {
  const now = Date.parse('2026-08-04T12:00:30.000Z')

  it('buckets a fresh reading as live', () => {
    expect(relativeAge('2026-08-04T12:00:25.000Z', now)).toEqual({ seconds: 5, bucket: 'live' })
  })

  it('buckets a lagging reading as recent and an old one as stale', () => {
    expect(relativeAge('2026-08-04T11:59:50.000Z', now)?.bucket).toBe('recent')
    expect(relativeAge('2026-08-04T11:55:00.000Z', now)?.bucket).toBe('stale')
  })

  it('returns undefined for missing or unparseable timestamps', () => {
    expect(relativeAge(undefined, now)).toBeUndefined()
    expect(relativeAge('not a date', now)).toBeUndefined()
  })

  it('never reports a negative age when the clocks disagree', () => {
    expect(relativeAge('2026-08-04T12:00:45.000Z', now)?.seconds).toBe(0)
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test --cwd ui`
Expected: FAIL — cannot resolve `./formatting`

- [ ] **Step 3: Write the implementation**

Create `ui/src/formatting.ts`:

```ts
// Thresholds are generous: the monitor polls on an interval, so a reading a
// few seconds old is normal and should not look alarming.
const RECENT_AFTER_SECONDS = 15
const STALE_AFTER_SECONDS = 60

export type Freshness = { seconds: number; bucket: 'live' | 'recent' | 'stale' }

export function relativeAge(observedAt: string | undefined, nowMs: number): Freshness | undefined {
  if (!observedAt) return undefined
  const parsed = Date.parse(observedAt)
  if (Number.isNaN(parsed)) return undefined
  const seconds = Math.max(0, Math.round((nowMs - parsed) / 1000))
  if (seconds >= STALE_AFTER_SECONDS) return { seconds, bucket: 'stale' }
  if (seconds >= RECENT_AFTER_SECONDS) return { seconds, bucket: 'recent' }
  return { seconds, bucket: 'live' }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test --cwd ui`
Expected: PASS (12 tests)

- [ ] **Step 5: Wire it into the header**

In `ui/src/App.vue`, add to the imports:

```ts
import { relativeAge } from './formatting'
```

A ticking clock is needed or the label freezes between monitor pushes. Next to the other refs (near line 437):

```ts
const nowMs = ref(Date.now())
let clockTimer: number | undefined
```

In `onMounted` (line 1488), start it; the interval is coarse because the label only has second resolution:

```ts
  clockTimer = window.setInterval(() => { nowMs.value = Date.now() }, 1000)
```

In `onUnmounted` (line 1554):

```ts
  if (clockTimer !== undefined) window.clearInterval(clockTimer)
```

Then the computed, next to `activeBadge`:

```ts
const activeFreshness = computed(() => {
  const entry = monitoring.value.find((candidate) => candidate.name === activePrinter.value?.name)
  return relativeAge(entry?.observedAt, nowMs.value)
})
```

- [ ] **Step 6: Render it beside the status badge**

In `ui/src/App.vue`, immediately after the `<StatusBadge …/>` on line 1677:

```html
            <span
              v-if="activeFreshness"
              class="font-mono text-xs"
              :class="activeFreshness.bucket === 'stale'
                ? 'text-amber-600 dark:text-amber-400'
                : 'text-gray-500 dark:text-gray-400'"
              :title="t('control.observedAtTitle')"
            >{{ t('control.observedAt', { seconds: activeFreshness.seconds }) }}</span>
```

- [ ] **Step 7: Add the i18n keys**

```ts
    "control.observedAt": "{seconds}s ago",
    "control.observedAtTitle": "Age of the last reading received from the printer",
```
```ts
    "control.observedAt": "há {seconds}s",
    "control.observedAtTitle": "Idade da última leitura recebida da impressora",
```

- [ ] **Step 8: Verify**

Run: `bun test --cwd ui && bun run --cwd ui build`
Expected: 12 tests pass, build succeeds. Under `make run`, the counter ticks up and resets each time the monitor pushes.

- [ ] **Step 9: Commit**

```bash
git add ui/src/formatting.ts ui/src/formatting.test.ts ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): show the age of the last printer reading

observedAt was carried all the way to the UI and never displayed, so
there was no way to tell whether a temperature was two seconds or two
minutes old. Show a ticking age beside the status badge, amber once it
passes a minute.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Make failure feedback look like failure

Fixes findings 10 and 14. `showToast` is one channel with a hardcoded green check, auto-dismissing in 2200 ms — so a rejected job start looks like a success and vanishes before it can be read. `downloadProgress` is computed from transfer events and rendered nowhere.

**Files:**
- Modify: `ui/src/App.vue:392-397` (toast state), `:592-599` (`showToast`), all `showToast` call sites in catch blocks, `:1632-1656` (toast markup), `:1922-1937` (download button)
- Modify: `ui/src/i18n.ts`

**Interfaces:**
- Produces: `showToast(text: string, tone: 'success' | 'error' = 'success')`. Every existing call keeps working unchanged.

- [ ] **Step 1: Give the toast a tone**

In `ui/src/App.vue`, replace the `toast` ref (line ~392) with:

```ts
const toast = ref<{ text: string; tone: 'success' | 'error' }>()
```

Replace `showToast` (line 592):

```ts
// Errors persist until dismissed: 2.2s is not long enough to read a failure,
// and a silently vanishing error is indistinguishable from no error at all.
function showToast(text: string, tone: 'success' | 'error' = 'success') {
  toast.value = { text, tone }
  if (toastTimer !== undefined) window.clearTimeout(toastTimer)
  toastTimer = undefined
  if (tone === 'success') {
    toastTimer = window.setTimeout(() => {
      toast.value = undefined
      toastTimer = undefined
    }, 2200)
  }
}
```

- [ ] **Step 2: Mark the failure call sites**

Every `showToast(message(reason))` inside a `catch` is reporting a failure. Find them:

```bash
grep -n "showToast(message(reason))" ui/src/App.vue
```

Replace each with `showToast(message(reason), 'error')`. Also update these three, which report failures without going through `message()`:

- `ui/src/App.vue` — `showToast(t('errors.jobFileInvalid'))` → add `, 'error'`
- any `showToast(commandDetail(...) ?? …)` inside a catch → add `, 'error'`
- leave every success path (`showToast(t('control.emergencyStopSent'))` and friends) untouched.

- [ ] **Step 3: Update the toast markup**

In `ui/src/App.vue`, replace the toast block (lines 1632-1656) body. The `v-if` becomes `v-if="toast"`, the icon switches on tone, and the close button clears the object:

```html
            <div
              v-if="toast"
              class="pointer-events-auto w-full max-w-sm rounded-lg bg-white shadow-lg outline-1 outline-black/5 dark:bg-gray-800 dark:-outline-offset-1 dark:outline-white/10"
              :role="toast.tone === 'error' ? 'alert' : 'status'"
            >
              <div class="p-4">
                <div class="flex items-start">
                  <div class="shrink-0">
                    <PhWarningCircle v-if="toast.tone === 'error'" class="size-6 text-red-500 dark:text-red-400" aria-hidden="true" />
                    <PhCheckCircle v-else class="size-6 text-green-400" aria-hidden="true" />
                  </div>
                  <div class="ml-3 w-0 flex-1 pt-0.5">
                    <p class="text-sm font-medium text-gray-900 dark:text-white">{{ toast.text }}</p>
                  </div>
                  <div class="ml-4 flex shrink-0">
                    <button
                      type="button"
                      class="inline-flex rounded-md text-gray-400 hover:text-gray-500 dark:hover:text-white"
                      @click="toast = undefined"
                    >
                      <span class="sr-only">{{ t('common.close') }}</span>
                      <PhX class="size-5" aria-hidden="true" />
                    </button>
                  </div>
                </div>
              </div>
            </div>
```

Add `PhWarningCircle` to the `@phosphor-icons/vue` import block at the top of the file (the list ending at line 62).

- [ ] **Step 4: Render download progress**

`downloadProgress` already tracks bytes via the `transfer-progress` listener. In `ui/src/App.vue`, replace the download `IconButton` in the dashboard file table (lines 1931-1937) with a button that shows the percentage while it runs:

```html
                          <IconButton
                            v-if="capabilities?.fileDownload"
                            :title="downloadingPath === file.devicePath && downloadProgress !== undefined
                              ? t('filesView.downloadingPercent', { percent: Math.round(downloadProgress) })
                              : t('filesView.downloadFile')"
                            :aria-label="t('filesView.downloadNamed', { name: file.name })"
                            :disabled="downloadingPath === file.devicePath"
                            @click.stop="downloadFile(file)"
                          >
                            <span v-if="downloadingPath === file.devicePath && downloadProgress !== undefined" class="font-mono text-[10px]">{{ Math.round(downloadProgress) }}%</span>
                            <PhDownloadSimple v-else class="size-4" />
                          </IconButton>
```

- [ ] **Step 5: Add the i18n key**

```ts
    "filesView.downloadingPercent": "Downloading… {percent}%",
```
```ts
    "filesView.downloadingPercent": "Baixando… {percent}%",
```

- [ ] **Step 6: Verify**

Run: `bun test --cwd ui && bun run --cwd ui build`
Expected: passes.

Under `make run`: trigger a failure (pause a printer that is not printing) and confirm the toast is red, carries a warning icon, and stays until dismissed. Trigger a success and confirm it is still green and still auto-dismisses.

- [ ] **Step 7: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): distinguish error toasts from success toasts

Every failure rendered with a green checkmark and auto-dismissed after
2.2s, which is neither long enough to read nor visually distinct from
success. Errors are now red, carry an alert role, and persist until
dismissed. Also render the download progress the transfer-progress
listener was already computing and discarding.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: Show time remaining

Fixes finding 4. `Status.time_estimates` is populated from `mc_remaining_time` (`bambu/transport.rs:2624`) and serialized as `timeEstimates`. The UI type never declared it.

**Files:**
- Modify: `ui/src/formatting.ts`, `ui/src/formatting.test.ts`
- Modify: `ui/src/App.vue:162-173` (`PrinterStatus` type), job card at `:1758-1765`
- Modify: `ui/src/i18n.ts`

**Interfaces:**
- Consumes: `ui/src/formatting.ts` from Task 3.
- Produces: `formatDuration(seconds: number): string` — `"4h 12m"`, `"12m"`, `"< 1m"`.

- [ ] **Step 1: Write the failing test**

Append to `ui/src/formatting.test.ts`:

```ts
import { formatDuration, relativeAge } from './formatting'

describe('duration formatting', () => {
  it('renders hours and minutes above an hour', () => {
    expect(formatDuration(15120)).toBe('4h 12m')
  })

  it('renders minutes alone below an hour', () => {
    expect(formatDuration(720)).toBe('12m')
  })

  it('collapses sub-minute and negative values', () => {
    expect(formatDuration(45)).toBe('< 1m')
    expect(formatDuration(0)).toBe('< 1m')
    expect(formatDuration(-10)).toBe('< 1m')
  })

  it('drops a zero minute component', () => {
    expect(formatDuration(7200)).toBe('2h')
  })
})
```

Update the existing import line at the top of the file to the combined one above.

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test --cwd ui`
Expected: FAIL — `formatDuration is not a function`

- [ ] **Step 3: Write the implementation**

Append to `ui/src/formatting.ts`:

```ts
export function formatDuration(seconds: number): string {
  if (seconds < 60) return '< 1m'
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  if (!hours) return `${minutes}m`
  return minutes ? `${hours}h ${minutes}m` : `${hours}h`
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test --cwd ui`
Expected: PASS (16 tests)

- [ ] **Step 5: Declare the field**

In `ui/src/App.vue`, add to `PrinterStatus` (after line 166):

```ts
  timeEstimates?: { elapsedSeconds: number; remainingSeconds?: number; totalSeconds?: number }
```

Import the helper:

```ts
import { formatDuration, relativeAge } from './formatting'
```

- [ ] **Step 6: Render it in the job card**

In `ui/src/App.vue`, replace the layer row (lines 1758-1761) so remaining time sits opposite the layer counter:

```html
                <div class="mt-6">
                  <div class="mb-2 flex justify-between text-xs text-gray-500 dark:text-gray-400">
                    <span>{{ t('control.layer', { current: selectedStatus?.progress?.currentLayer ?? '—', total: selectedStatus?.progress?.totalLayers ?? '—' }) }}</span>
                    <span v-if="selectedStatus?.timeEstimates?.remainingSeconds !== undefined" class="font-mono">
                      {{ t('control.remaining', { duration: formatDuration(selectedStatus.timeEstimates.remainingSeconds) }) }}
                    </span>
                  </div>
```

- [ ] **Step 7: Add the i18n key**

```ts
    "control.remaining": "{duration} left",
```
```ts
    "control.remaining": "faltam {duration}",
```

- [ ] **Step 8: Verify**

Run: `bun test --cwd ui && bun run --cwd ui build`
Expected: passes. With a Bambu printer mid-print, the job card shows a countdown.

- [ ] **Step 9: Commit**

```bash
git add ui/src/formatting.ts ui/src/formatting.test.ts ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): show time remaining on the current job

The backend has parsed mc_remaining_time into Status.time_estimates all
along; the UI type simply never declared the field, so the most-wanted
number on a print monitor was the one it could not answer.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Preparation progress and an accessible progress bar

Fixes findings 5 and 8. `preparation_percent` is parsed (`transport.rs:2468`) and ignored, so heating and levelling look like a stalled 0%. The progress bar is a styled div with no ARIA role.

**Files:**
- Modify: `ui/src/App.vue:166` (`progress` type), `:486` (`progressPercent`), `:1762-1764` (bar markup)
- Modify: `ui/src/i18n.ts`

- [ ] **Step 1: Declare the field**

`ui/src/App.vue:166`:

```ts
  progress?: { percent: number; preparationPercent?: number; currentLayer?: number; totalLayers?: number }
```

- [ ] **Step 2: Add a preparing computed**

Next to `progressPercent` (line 486). Preparation only matters while the real progress is still zero — once layers start, percent is the truth:

```ts
const preparingPercent = computed(() => {
  const progress = selectedStatus.value?.progress
  if (!progress || progress.percent > 0) return undefined
  return progress.preparationPercent
})
```

- [ ] **Step 3: Render the preparation state and add ARIA**

Replace the bar (lines 1762-1764):

```html
                  <div
                    class="overflow-hidden rounded-full bg-gray-200 dark:bg-white/10"
                    role="progressbar"
                    :aria-valuenow="preparingPercent ?? progressPercent"
                    aria-valuemin="0"
                    aria-valuemax="100"
                    :aria-label="preparingPercent !== undefined ? t('control.preparing') : t('control.complete', { percent: progressPercent })"
                  >
                    <div
                      class="h-2 rounded-full transition-[width]"
                      :class="preparingPercent !== undefined ? 'bg-amber-500 dark:bg-amber-400' : 'bg-cyan-600 dark:bg-cyan-500'"
                      :style="{ width: `${preparingPercent ?? progressPercent}%` }"
                    ></div>
                  </div>
                  <p v-if="preparingPercent !== undefined" class="mt-2 text-xs text-amber-700 dark:text-amber-300">{{ t('control.preparing') }}</p>
```

- [ ] **Step 4: Add the i18n key**

```ts
    "control.preparing": "Preparing — heating and levelling",
```
```ts
    "control.preparing": "Preparando — aquecendo e nivelando",
```

- [ ] **Step 5: Verify**

Run: `bun run --cwd ui build`
Expected: passes. Start a print and confirm the bar runs amber with the preparing caption before switching to cyan once layers begin.

- [ ] **Step 6: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): show preparation progress and label the progress bar

gcode_file_prepare_percent was parsed and dropped, so heating and
levelling showed as a stalled 0%. The bar also had no progressbar role,
leaving it invisible to screen readers.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: Enrich the current job card

Partially fixes finding 6, and adds the `printMeta` fields the backend already sends.

**Scope note:** the original finding assumed `ModelThumbnail` was a drop-in. It is not — it renders **local** files through a Rust preview command, and a printing job's file lives on the printer. Rendering the real thumbnail needs a printer-side fetch, which is out of scope here. This task shows the thumbnail only when a file of the same name exists in the local library, and otherwise keeps the icon.

**Files:**
- Modify: `ui/src/App.vue` (`PrinterStatus` type, job card at `:1747-1757`)
- Modify: `ui/src/i18n.ts`

- [ ] **Step 1: Declare printMeta**

Add to `PrinterStatus` in `ui/src/App.vue`:

```ts
  printMeta?: { fileName: string; fileSize?: number; plateIndex?: number; plateCount?: number; bedType?: string }
```

- [ ] **Step 2: Resolve a local thumbnail when one exists**

Next to `preparingPercent`:

```ts
// The printing file lives on the printer, so a preview is only possible when
// the same name happens to sit in the local library. No match, no thumbnail.
const jobThumbnail = computed(() => {
  const name = selectedStatus.value?.job?.name
  if (!name) return undefined
  return libraryFiles.value.find((file) => file.type === 'file' && file.name === name)
})
```

- [ ] **Step 3: Render thumbnail and plate metadata**

Replace the job header block (lines 1748-1757):

```html
                <div class="flex items-start gap-3">
                  <ModelThumbnail
                    v-if="jobThumbnail"
                    :path="jobThumbnail.devicePath"
                    :size-bytes="jobThumbnail.sizeBytes"
                    :modified-at="jobThumbnail.modifiedAt"
                    :alt="jobThumbnail.name"
                    class="size-10 shrink-0 overflow-hidden rounded-lg"
                  />
                  <div v-else class="grid size-10 shrink-0 place-items-center rounded-lg bg-cyan-50 text-cyan-600 dark:bg-cyan-400/10 dark:text-cyan-400">
                    <PhHexagon v-if="!selectedStatus?.job" class="size-5" />
                    <PhCube v-else class="size-5" />
                  </div>
                  <div class="min-w-0">
                    <p class="truncate font-mono text-sm font-semibold text-gray-900 dark:text-white">{{ selectedStatus?.job?.name ?? t('dashboard.noJob') }}</p>
                    <p class="mt-1 font-mono text-xs text-gray-500 capitalize dark:text-gray-400">
                      {{ selectedStatus?.state ?? 'unknown' }}
                      <span v-if="selectedStatus?.printMeta?.plateIndex !== undefined">
                        · {{ t('control.plateOf', { index: selectedStatus.printMeta.plateIndex, total: selectedStatus.printMeta.plateCount ?? '—' }) }}
                      </span>
                    </p>
                  </div>
                </div>
```

- [ ] **Step 4: Add the i18n key**

```ts
    "control.plateOf": "plate {index} of {total}",
```
```ts
    "control.plateOf": "placa {index} de {total}",
```

- [ ] **Step 5: Verify**

Run: `bun run --cwd ui build`
Expected: passes. With a job whose name matches a library file, the thumbnail renders; otherwise the cube icon stays.

- [ ] **Step 6: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): enrich the current job card

Show the plate number from printMeta, and a real thumbnail when the
printing file also exists in the local library. Files that live only on
the printer keep the icon — previewing those needs a printer-side fetch
that does not exist yet.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Stop silently truncating the file table

Fixes finding 7. `ui/src/App.vue:1912` hardcodes `.slice(0, 3)`.

**Files:**
- Modify: `ui/src/App.vue:1899-1901` (card header), `:1912`
- Modify: `ui/src/i18n.ts`

- [ ] **Step 1: Name the limit and expose the overflow count**

Next to `printerFiles` (line ~1408):

```ts
const DASHBOARD_FILE_LIMIT = 3
const dashboardFiles = computed(() => printerFiles.value.slice(0, DASHBOARD_FILE_LIMIT))
const hiddenFileCount = computed(() => Math.max(0, printerFiles.value.length - DASHBOARD_FILE_LIMIT))
```

- [ ] **Step 2: Use it and disclose the remainder**

`ui/src/App.vue:1912`:

```html
                      v-for="file in dashboardFiles"
```

Then add a footer row inside `<tbody>`, immediately after the `v-if="!printerFiles.length"` row (line 1943):

```html
                    <tr v-else-if="hiddenFileCount">
                      <td colspan="5" class="px-4 py-3 text-sm text-gray-500 sm:px-6 dark:text-gray-400">
                        <button type="button" class="font-medium text-cyan-700 hover:underline dark:text-cyan-400" @click="goTo('files')">
                          {{ t('dashboard.moreFiles', { count: hiddenFileCount }) }}
                        </button>
                      </td>
                    </tr>
```

- [ ] **Step 3: Add the i18n key**

```ts
    "dashboard.moreFiles": "{count} more on the printer — open Files",
```
```ts
    "dashboard.moreFiles": "mais {count} na impressora — abrir Arquivos",
```

- [ ] **Step 4: Verify**

Run: `bun run --cwd ui build`
Expected: passes. A printer with more than three files shows the disclosure row.

- [ ] **Step 5: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
fix(ui): disclose truncated printer files

The dashboard table sliced to three rows with no indication more
existed. Name the limit and offer a way through to the files view.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Promote emergency stop out of the kebab menu

Fixes finding 9. It currently sits in `printerActionItems` (`ui/src/App.vue:1418`) directly above "Remove printer".

**Files:**
- Modify: `ui/src/App.vue:1411-1423` (`printerActionItems`), `:1676-1690` (header actions)

- [ ] **Step 1: Remove it from the menu**

In `ui/src/App.vue`, delete this block from `printerActionItems` (lines 1417-1419):

```ts
  if (capabilities.value?.emergencyStop) {
    items.push({ label: t('dashboard.emergency'), icon: PhStop, danger: true, onSelect: () => void emergencyStop() })
  }
```

- [ ] **Step 2: Add it as a standing control**

In `ui/src/App.vue`, insert immediately before the `<ActionMenu …/>` in the header (line 1685):

```html
            <Button
              v-if="capabilities?.emergencyStop"
              variant="danger"
              :disabled="!activeHasStatus"
              @click="emergencyStop"
            ><PhStop class="size-4" /> {{ t('dashboard.emergency') }}</Button>
```

The existing `askConfirmation` flow in `emergencyStop()` still guards it, so promoting the control does not remove the confirmation step.

- [ ] **Step 3: Verify**

Run: `bun run --cwd ui build`
Expected: passes. The button appears in the printer header, the kebab menu keeps only certificate refresh and remove, and clicking still raises the confirm panel.

- [ ] **Step 4: Commit**

```bash
git add ui/src/App.vue
git commit -m "$(cat <<'EOF'
fix(ui): promote emergency stop to a standing control

The one control with physical consequences was two clicks deep in a
kebab menu, directly above a config action. Confirmation is unchanged.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Make the fan slider track the drag

Fixes finding 11. `ui/src/App.vue:1833` binds `@change`, so the readout only moves after release and a round-trip.

**Files:**
- Modify: `ui/src/App.vue:1828-1834`

- [ ] **Step 1: Hold a local drag value**

Next to `fanRows` (line 494):

```ts
// While dragging, the readout follows the pointer; the committed value comes
// back from the printer on the next status push.
const fanDrafts = ref<Record<string, number>>({})

function previewFan(key: string, event: Event) {
  fanDrafts.value[key] = Number((event.target as HTMLInputElement).value)
}
```

In the existing `sendFan` function, clear the draft once the command is dispatched — find `sendFan` and add as its first statement inside the handler body:

```ts
  delete fanDrafts.value[key]
```

- [ ] **Step 2: Bind both events**

Replace lines 1829-1833:

```html
                  <span class="mb-2 flex justify-between">
                    <span class="text-sm/6 font-light text-gray-900 dark:text-white">{{ fanLabel(key) }}</span>
                    <span class="text-sm/6 text-gray-500 dark:text-gray-400">{{ fanDrafts[key] ?? value }}%</span>
                  </span>
                      <input :key="`${key}-${value}`" :value="value" :disabled="!capabilities?.fanControl" :name="`fan-${key}`" class="h-1 w-full cursor-pointer accent-cyan-600 disabled:cursor-not-allowed disabled:opacity-35 dark:accent-cyan-400" type="range" min="0" max="100" :aria-label="t('control.fanPower', { fan: fanLabel(key) })" @input="previewFan(key, $event)" @change="sendFan(key, $event)" />
```

- [ ] **Step 3: Verify**

Run: `bun run --cwd ui build`
Expected: passes. Dragging updates the percentage live; releasing sends the command and the value settles on whatever the printer reports.

- [ ] **Step 4: Commit**

```bash
git add ui/src/App.vue
git commit -m "$(cat <<'EOF'
fix(ui): track the fan slider while dragging

The readout was bound to @change, so it stayed frozen until release and
a status round-trip, making the control feel dead under the pointer.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 11: Direct temperature entry

Fixes finding 12. `ui/src/App.vue:1818-1819` offers only ±5 °C, so reaching 250 °C takes fifty clicks.

**Files:**
- Modify: `ui/src/App.vue` (temperature card at `:1812-1821`), plus a `setTemperature` function beside `adjustTemperature`
- Modify: `ui/src/i18n.ts`

**Interfaces:**
- Consumes: `adjustTemperature(kind, delta)`, which debounces through `pendingTemperatures` / `temperatureTimers` and invokes `printer_temperature_set` with `{ request: { name, [`${kind}Celsius`]: target } }`.
- Produces: `queueTemperature(kind, next)` — the debounced commit, extracted so both the steppers and the input share it.

- [ ] **Step 1: Extract the debounced commit**

`adjustTemperature` currently computes the next value *and* owns the debounce. Split it so an absolute setter can reuse the commit without recomputing a delta. Replace the whole function with:

```ts
function queueTemperature(kind: 'nozzle' | 'bed' | 'chamber', next: number) {
  const maximum = temperatureMaximums[kind]
  pendingTemperatures[kind] = Math.max(0, Math.min(maximum, Math.round(next / 5) * 5))
  if (temperatureTimers[kind] !== undefined) window.clearTimeout(temperatureTimers[kind])
  temperatureTimers[kind] = window.setTimeout(async () => {
    const printerName = activePrinter.value?.name
    const target = pendingTemperatures[kind]
    delete pendingTemperatures[kind]
    delete temperatureTimers[kind]
    if (!printerName || target === undefined) return
    try {
      await invoke('printer_temperature_set', {
        request: { name: printerName, [`${kind}Celsius`]: target },
      })
      void refreshMonitoring()
    } catch (reason) {
      showToast(message(reason), 'error')
    }
  }, 150)
}

function adjustTemperature(kind: 'nozzle' | 'bed' | 'chamber', delta: number) {
  if (!activePrinter.value || !selectedStatus.value) return
  const temperature = selectedStatus.value.temperatures?.[kind]
  if (!temperature) return
  const base = pendingTemperatures[kind] ?? temperature.targetCelsius ?? temperature.currentCelsius
  queueTemperature(kind, base + delta)
}

function setTemperature(kind: 'nozzle' | 'bed' | 'chamber', event: Event) {
  if (!activePrinter.value) return
  const target = Number((event.target as HTMLInputElement).value)
  if (!Number.isFinite(target)) return
  queueTemperature(kind, target)
}
```

Behaviour is unchanged for the steppers: same clamp, same rounding to 5, same 150 ms debounce, same command. The `'error'` tone on the catch comes from Task 4.

- [ ] **Step 3: Add the input to the card**

Replace the control cluster (lines 1817-1820):

```html
                  <div v-if="capabilities?.temperatureWrite" class="flex items-center gap-1">
                    <input
                      type="number"
                      min="0"
                      step="5"
                      :max="temperatureMaximums[row.key]"
                      :value="row.value.targetCelsius ?? 0"
                      :disabled="selectedStatus?.state !== 'idle'"
                      :aria-label="t('control.setTemp', { sensor: t(temperatureKeys[row.key]) })"
                      class="w-16 rounded-md bg-white px-2 py-1 text-right font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 disabled:opacity-50 dark:bg-white/5 dark:text-white dark:outline-white/10"
                      @change="setTemperature(row.key, $event)"
                    />
                    <IconButton variant="outline" :disabled="selectedStatus?.state !== 'idle'" :aria-label="t('control.decreaseTemp', { sensor: t(temperatureKeys[row.key]) })" @click="adjustTemperature(row.key, -5)"><PhMinus class="size-3.5" /></IconButton>
                    <IconButton variant="outline" :disabled="selectedStatus?.state !== 'idle'" :aria-label="t('control.increaseTemp', { sensor: t(temperatureKeys[row.key]) })" @click="adjustTemperature(row.key, 5)"><PhPlus class="size-3.5" /></IconButton>
                  </div>
```

- [ ] **Step 4: Add the i18n key**

```ts
    "control.setTemp": "Target temperature for {sensor}",
```
```ts
    "control.setTemp": "Temperatura alvo para {sensor}",
```

- [ ] **Step 5: Verify**

Run: `bun run --cwd ui build`
Expected: passes. On an idle printer, typing a target and blurring sends one command; the ± buttons still work; everything stays disabled while printing.

- [ ] **Step 6: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): allow direct temperature entry

Reaching a 250C target took fifty clicks of a +5 button. Reuse the same
command with an absolute target, keeping the steppers for nudges and the
idle-only guard intact.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 12: Edit a printer, and accept a discovered host

Fixes finding 13. Only add and remove exist, so a DHCP move means deleting the printer and re-entering the access code — even though the UI already detects the new address and shows it in amber (`ui/src/App.vue:2046`).

**Files:**
- Modify: `ui/src/App.vue` (addition slide-over at `:2314-2365`, printer card at `:2046-2049`)
- Modify: `ui/src/i18n.ts`

**Backend constraint (verified, do not design around it):** there is no update command. Only `create_configured_printer` (`{ request: draft }`, returns the stored `Printer`) and `remove_configured_printer` (`{ name }`) exist. `remove_configured_printer` calls `profiles::remove(dir, &SystemKeychain, &name)` (`main.rs:3034`), which **deletes the stored access code along with the profile**. So an edit implemented as remove-then-create cannot preserve the credential, and the access code must be re-entered. The field is therefore required in edit mode, not optional.

A proper in-place `update_configured_printer` command that keeps the keychain entry is the better long-term fix; it is deliberately out of scope here because this plan is frontend-only. Note it as a follow-up.

**Interfaces:**
- Consumes: the existing `draft` ref, `openAddition()`, `addPrinter()`, `printers` ref.
- Produces: `editingPrinter: Ref<string | undefined>` — when set, the slide-over is in edit mode.

- [ ] **Step 1: Add edit mode state**

Next to the `draft` ref:

```ts
const editingPrinter = ref<string>()

function openEdit(printer: Printer) {
  editingPrinter.value = printer.name
  draft.value = {
    name: printer.name,
    driver: printer.driver,
    host: printer.host,
    serial: printer.serial,
    timeout: printer.timeout,
    accessCode: '',
    insecure: printer.insecure,
  }
  additionOpen.value = true
}
```

Match the field names to the existing `draft` shape exactly — read its initialiser in `openAddition()` (line 830) and mirror it. In `closeAddition()`, add:

```ts
  editingPrinter.value = undefined
```

- [ ] **Step 2: Branch the submit**

In `addPrinter()`, replace the body of the `try` block. The old profile must be removed before creating the replacement, and the local `printers` array must drop the old entry or editing will duplicate the card:

```ts
  try {
    const replacing = editingPrinter.value
    if (replacing) await invoke('remove_configured_printer', { name: replacing })
    const profile = await invoke<Printer>('create_configured_printer', { request: draft.value })
    const printer = {
      name: profile.name,
      driver: profile.driver,
      host: profile.host,
      serial: profile.serial,
      timeout: profile.timeout,
      insecure: profile.insecure,
    }
    printers.value = [...printers.value.filter((entry) => entry.name !== replacing), printer]
      .sort((left, right) => left.name.localeCompare(right.name))
    additionOpen.value = false
    editingPrinter.value = undefined
    draft.value.accessCode = ''
    await selectPrinter(printer.name)
  } catch (reason) {
    additionError.value = message(reason)
  } finally {
    adding.value = false
  }
```

Note the failure mode this creates: if `create_configured_printer` rejects after the remove succeeded, the printer is gone. The error surfaces in `additionError` with the form still populated, so it can be resubmitted — but this is the strongest argument for the in-place update command noted above. Call it out in the commit message.

- [ ] **Step 3: Add the entry points**

On the printer card, make the suggested host actionable (replace lines 2046-2049):

```html
              <div v-if="printer.presence?.suggestedHost && printer.presence.suggestedHost !== printer.host" class="flex items-center justify-between gap-3 py-2">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('printersView.discoveredHost') }}</dt>
                <dd class="text-right">
                  <button
                    type="button"
                    class="font-mono text-amber-600 hover:underline dark:text-amber-400"
                    :title="t('printersView.useDiscoveredHost')"
                    @click="openEdit({ ...printer, host: printer.presence!.suggestedHost! })"
                  >{{ printer.presence.suggestedHost }}</button>
                </dd>
              </div>
```

And add an edit button beside "Open control" (line 2060):

```html
              <Button class="flex-1" @click="openEdit(printer)"><PhPencilSimple class="size-4" /> {{ t('printersView.editPrinter') }}</Button>
```

Add `PhPencilSimple` to the icon import block.

- [ ] **Step 4: Retitle the slide-over**

`ui/src/App.vue:2314`:

```html
    <SlideOver :open="additionOpen" :title="editingPrinter ? t('printersView.editPrinter') : t('addition.title')" :description="t('addition.description')" :close-label="t('common.closePanel')" @close="closeAddition">
```

The access code cannot be carried across a remove-and-recreate, so in edit mode it must be re-entered. Replace the label and the input on lines 2351-2352:

```html
            <label for="printer-access-code" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ editingPrinter ? t('addition.accessCodeReenter') : t('addition.accessCode') }}</label>
            <input id="printer-access-code" name="printer-access-code" v-model="draft.accessCode" type="password" :required="editingPrinter !== undefined" autocomplete="new-password" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
```

- [ ] **Step 5: Add the i18n keys**

```ts
    "printersView.editPrinter": "Edit printer",
    "printersView.useDiscoveredHost": "Use this address",
    "addition.accessCodeReenter": "Access code (re-enter to save changes)",
```
```ts
    "printersView.editPrinter": "Editar impressora",
    "printersView.useDiscoveredHost": "Usar este endereço",
    "addition.accessCodeReenter": "Código de acesso (informe novamente para salvar)",
```

- [ ] **Step 6: Verify**

Run: `bun run --cwd ui build`
Expected: passes.

Under `make run`: edit a printer's host, re-enter the access code, and confirm it reconnects and the card is not duplicated. Click a suggested host and confirm the form opens pre-filled with the new address. Then confirm the failure path: submit an edit with a deliberately invalid host and check that the error appears in the panel with the form still populated.

- [ ] **Step 7: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): edit printers and accept discovered hosts

Changing a host after a DHCP move meant deleting the printer and
re-entering its access code, even though presence detection already
showed the new address. Make that address a one-click action and add a
general edit path.

Edit is remove-then-create because no in-place update command exists,
so the access code still has to be re-entered and a failed create
leaves the profile removed. An update_configured_printer command that
preserves the keychain entry would fix both; noted as follow-up.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 13: Sort the files view

Fixes finding 15.

**Files:**
- Modify: `ui/src/App.vue` (toolbar at `:2229-2258`, `visibleFiles` at `:1406`)
- Modify: `ui/src/i18n.ts`

- [ ] **Step 1: Add sort state and apply it**

Next to `searchTerm`:

```ts
type SortKey = 'name' | 'size' | 'modified'
const sortKey = ref<SortKey>('name')
```

Replace `visibleFiles` (line 1406). Directories keep their own name ordering; only files sort:

```ts
const visibleFiles = computed(() => {
  const files = libraryFiles.value.filter((file) => file.type === 'file' && matchesSearch(file))
  return [...files].sort((left, right) => {
    if (sortKey.value === 'size') return (right.sizeBytes ?? 0) - (left.sizeBytes ?? 0)
    if (sortKey.value === 'modified') return Date.parse(right.modifiedAt ?? '') - Date.parse(left.modifiedAt ?? '')
    return left.name.localeCompare(right.name)
  })
})
```

`Date.parse` of a missing value yields `NaN`, which sorts those entries last rather than throwing.

- [ ] **Step 2: Add the control**

In `ui/src/App.vue`, insert before the search input wrapper (line 2247):

```html
          <div class="ml-auto grid grid-cols-1">
            <select
              v-model="sortKey"
              :aria-label="t('filesView.sortBy')"
              class="col-start-1 row-start-1 w-auto appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800"
            >
              <option value="name">{{ t('filesView.sortName') }}</option>
              <option value="size">{{ t('filesView.sortSize') }}</option>
              <option value="modified">{{ t('filesView.sortModified') }}</option>
            </select>
            <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-4 self-center justify-self-end text-gray-500 dark:text-gray-400" aria-hidden="true" />
          </div>
```

Then change the search wrapper on line 2247 from `class="ml-auto grid grid-cols-1"` to `class="grid grid-cols-1"` so the sort control owns the `ml-auto`.

- [ ] **Step 3: Add the i18n keys**

```ts
    "filesView.sortBy": "Sort by",
    "filesView.sortName": "Name",
    "filesView.sortSize": "Largest first",
    "filesView.sortModified": "Newest first",
```
```ts
    "filesView.sortBy": "Ordenar por",
    "filesView.sortName": "Nome",
    "filesView.sortSize": "Maiores primeiro",
    "filesView.sortModified": "Mais recentes primeiro",
```

- [ ] **Step 4: Verify**

Run: `bun run --cwd ui build`
Expected: passes. Each sort option reorders the grid; search still filters.

- [ ] **Step 5: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): sort the files view

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 14: Keep the nav usable past a handful of printers

Fixes finding 16. Printer tabs and section tabs share one strip (`ui/src/App.vue:1577-1615`) that scrolls horizontally as printers accumulate.

**Scope note:** this is the only finding that is a judgement call rather than a defect. Ship Tasks 1-13 first and confirm the change is still wanted — with three printers the current nav is fine.

**Files:**
- Modify: `ui/src/App.vue:1577-1592`
- Modify: `ui/src/i18n.ts`

- [ ] **Step 1: Add a threshold**

Next to `sectionTabs`:

```ts
// Below this the tab strip reads better than a dropdown; above it the strip
// starts scrolling and the active printer can end up off-screen.
const PRINTER_TAB_LIMIT = 5
const usePrinterMenu = computed(() => printers.value.length > PRINTER_TAB_LIMIT)
```

- [ ] **Step 2: Swap in a select past the threshold**

Wrap the existing printer-tab block (lines 1577-1591) in `v-if="printers.length && !usePrinterMenu"`, then add after it:

```html
          <div v-if="usePrinterMenu" class="flex items-center pr-8">
            <div class="grid grid-cols-1">
              <select
                :value="activePrinter?.name"
                :aria-label="t('nav.selectPrinter')"
                class="col-start-1 row-start-1 appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800"
                @change="selectPrinter(($event.target as HTMLSelectElement).value)"
              >
                <option v-for="printer in printers" :key="printer.name" :value="printer.name">{{ printer.name }} · {{ statusLabel(badgeFor(printer.name)) }}</option>
              </select>
              <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-4 self-center justify-self-end text-gray-500 dark:text-gray-400" aria-hidden="true" />
            </div>
          </div>
```

- [ ] **Step 3: Add the i18n key**

```ts
    "nav.selectPrinter": "Select printer",
```
```ts
    "nav.selectPrinter": "Selecionar impressora",
```

- [ ] **Step 4: Verify**

Run: `bun run --cwd ui build`
Expected: passes. With five or fewer printers the tabs are unchanged; with six the strip becomes a dropdown carrying the status label.

- [ ] **Step 5: Commit**

```bash
git add ui/src/App.vue ui/src/i18n.ts
git commit -m "$(cat <<'EOF'
feat(ui): collapse the printer strip into a picker past five printers

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Final verification

- [ ] **Run the full gate**

```bash
make ui-test
bun run --cwd ui build
make lint
```

Expected: all pass. `make lint` covers the Rust side, which this plan does not touch — it should be unaffected.

- [ ] **Manual pass under `make run`**

Walk each view once: control with a healthy printer, control with a faulting printer, printers, files, settings. Confirm no console errors and no layout regressions at the default window size.
