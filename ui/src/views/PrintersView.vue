<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Locale, MessageKey } from '../i18n'
import type { PrinterBadge } from '../monitoring'
import { formatDate } from '../formatting'
import { serialNumberDisplay } from '../presentation'
import { printerDraftsMatch, type PrinterDraftFields } from '../forms'
import {
  firmwareAssessmentClasses,
  hasAvailableUpdate,
  hasRequiredUpdate,
  updateEntryFor,
  type FirmwareEvidenceRole,
  type FirmwareSourceCheck,
  type FirmwareUpdateAssessment,
  type FirmwareUpdateComponentKind,
  type FirmwareUpdateEntry,
} from '../firmware-updates'
import StatusBadge from '../components/StatusBadge.vue'
import ActionMenu, { type ActionMenuItem } from '../components/ActionMenu.vue'
import SlideOver from '../components/SlideOver.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import TimeoutField from '../components/TimeoutField.vue'
import Button from '../components/Button.vue'
import IconButton from '../components/IconButton.vue'
import Switch from '../components/Switch.vue'
import Card from '../components/Card.vue'
import CardHeader from '../components/CardHeader.vue'
import {
  PhArrowsClockwise,
  PhArrowLeft,
  PhBroadcast,
  PhCards,
  PhCaretDown,
  PhCpu,
  PhEye,
  PhEyeSlash,
  PhGlobe,
  PhInfo,
  PhNetwork,
  PhPencilSimple,
  PhPlus,
  PhPrinter,
  PhTrash,
  PhWarning,
} from '@phosphor-icons/vue'
import type { View, Printer, Driver, DiscoveredPrinter, MonitorEntry, ViewActions } from '../app-types'

const {
  active,
  drivers,
  loading,
  loadError,
  profileError,
  firmwareUpdates,
  firmwareRefreshing,
  firmwareRefreshSource,
  firmwareError,
  locale,
  hasPrinters,
  badgeFor,
  isReachable,
  statusLabel,
  selectPrinter,
  clearSelection,
  goTo,
  load,
  loadCameras,
  loadPlugs,
  refreshMonitoring,
  loadFirmwareUpdates,
  loadPublicFirmwareCatalogue,
  t,
  message,
  showToast,
  askConfirmation,
} = defineProps<ViewActions & {
  active: boolean
  drivers: Driver[]
  loading: boolean
  loadError: string | undefined
  profileError: string | undefined
  firmwareUpdates: FirmwareUpdateEntry[]
  firmwareRefreshing: boolean
  firmwareRefreshSource: 'printer' | 'publicCatalogue' | undefined
  firmwareError: string | undefined
  locale: Locale
  hasPrinters: boolean
  badgeFor: (name: string) => PrinterBadge
  isReachable: (badge: PrinterBadge) => boolean
  statusLabel: (badge: PrinterBadge) => string
  selectPrinter: (name: string, refresh?: boolean, focus?: boolean) => Promise<void>
  clearSelection: () => void
  goTo: (view: View) => void
  load: () => Promise<void>
  loadCameras: () => Promise<void>
  loadPlugs: () => Promise<void>
  refreshMonitoring: () => Promise<void>
  loadFirmwareUpdates: (name: string, refresh?: boolean) => Promise<void>
  loadPublicFirmwareCatalogue: (name: string) => Promise<void>
}>()

const printers = defineModel<Printer[]>('printers', { required: true })
const monitoring = defineModel<MonitorEntry[]>('monitoring', { required: true })
const firmwarePrinterId = defineModel<string | undefined>('firmwarePrinterId', { required: true })
const adding = ref(false)
const additionOpen = ref(false)
const additionError = ref<string>()
const discovered = ref<DiscoveredPrinter[]>([])

// Display only. The model is resolved from the printer, so the panel shows
// it rather than accepting it; `draft.model` still carries the stored value
// so saving never discards the hint a hand-added profile was created with.
const detectedModelName = ref('')
const discovering = ref(false)
const discoveryError = ref<string>()
const tlsOpen = ref(false)
const tlsRefreshing = ref(false)
const tlsError = ref<string>()
const tlsFingerprint = ref<string>()
const tlsPrinter = ref<string>()
const revealedSerials = ref(new Set<string>())

const draft = ref<PrinterDraftFields>({
  name: '',
  driver: '',
  host: '',
  serial: '',
  model: '',
  timeout: '10s',
  insecure: false,
  accessCode: '',
})

const additionBaseline = ref<PrinterDraftFields>({ ...draft.value })
const editingPrinter = ref<string>()
const additionDirty = computed(() => additionOpen.value && !printerDraftsMatch(draft.value, additionBaseline.value))

function resetAdditionPanelState() {
  additionError.value = undefined
  discoveryError.value = undefined
  discovered.value = []
  detectedModelName.value = ''
}

function openEdit(printer: Printer) {
  editingPrinter.value = printer.name
  draft.value = {
    name: printer.name,
    driver: printer.driver,
    host: printer.host,
    serial: printer.serial,
    model: printer.model || printer.presence?.model || '',
    timeout: printer.timeout,
    accessCode: '',
    insecure: printer.insecure,
  }
  additionBaseline.value = { ...draft.value }
  resetAdditionPanelState()
  detectedModelName.value = printer.detectedModel || printer.presence?.model || ''
  additionOpen.value = true
}

function serialIsRevealed(name: string) {
  return revealedSerials.value.has(name)
}

function toggleSerial(name: string) {
  const next = new Set(revealedSerials.value)
  if (next.has(name)) next.delete(name)
  else next.add(name)
  revealedSerials.value = next
}

const firmwarePrinter = computed(() => printers.value.find((printer) => printer.name === firmwarePrinterId.value))

// Bambu's public pages are per model, so without one there is nothing to look
// up. Detection resolves it from the serial or the printer's own report; the
// typed string and the discovery broadcast are the remaining fallbacks.
const firmwarePrinterModel = computed(() => firmwarePrinter.value?.detectedModel || firmwarePrinter.value?.model || firmwarePrinter.value?.presence?.model || '')
const selectedFirmwareUpdate = computed(() => updateEntryFor(firmwareUpdates, firmwarePrinter.value?.name))

// Only explain the disabled catalogue button when the report has not already
// reported the same unknown model, otherwise the card says it twice.
const firmwareNeedsModel = computed(() => {
  const report = selectedFirmwareUpdate.value?.report
  if (!report || report.source !== 'bambuMqtt' || firmwarePrinterModel.value) return false
  return !report.issues.some((issue) => issue.code === 'publicCatalogueUnsupportedModel')
})

const firmwareFor = (name: string) => updateEntryFor(firmwareUpdates, name)

const firmwareComponentLabel = (kind: FirmwareUpdateComponentKind) => t(
  kind === 'printerFirmware'
    ? 'firmware.printerFirmware'
    : kind === 'accessoryFirmware'
      ? 'firmware.accessoryFirmware'
      : 'firmware.printerSoftware',
)

const firmwareAssessmentLabel = (assessment: FirmwareUpdateAssessment) => {
  const key: MessageKey = assessment === 'required'
    ? 'firmware.assessmentRequired'
    : assessment === 'deviceCatalogueNewer'
      ? 'firmware.assessmentDeviceCatalogue'
      : assessment === 'publicReleaseNewer'
        ? 'firmware.assessmentPublicRelease'
        : assessment === 'confirmedAvailable'
          ? 'firmware.assessmentConfirmed'
          : assessment === 'current'
            ? 'firmware.assessmentCurrent'
            : assessment === 'conflict'
              ? 'firmware.assessmentConflict'
              : assessment === 'unsupported'
                ? 'firmware.unsupported'
                : 'firmware.assessmentUnknown'
  return t(key)
}

const firmwareEvidenceLabel = (role: FirmwareEvidenceRole) => {
  const key: MessageKey = role === 'printerAdvertised'
    ? 'firmware.sourcePrinter'
    : role === 'deviceCatalogue'
      ? 'firmware.sourceDeviceCatalogue'
      : role === 'publicStable'
        ? 'firmware.sourcePublicStable'
        : 'firmware.installed'
  return t(key)
}

const firmwareEvidenceFor = (component: FirmwareUpdateEntry['report']['components'][number]) =>
  (component.evidence ?? []).filter((evidence) => evidence.role !== 'installed' && Boolean(evidence.version))

const firmwareComparableCount = (entry: FirmwareUpdateEntry) => entry.report.components.filter((component) => firmwareEvidenceFor(component).length > 0).length

const firmwareSourceName = (source: FirmwareSourceCheck['source']) => {
  if (source === 'bambuLanInventory') return t('firmware.installed')
  if (source === 'bambuLanAdvertisement') return t('firmware.sourcePrinter')
  if (source === 'bambuLanHistory') return t('firmware.sourceDeviceCatalogue')
  if (source === 'bambuPublicCatalogue') return t('firmware.sourcePublicStable')
  return t('firmware.printerSoftware')
}

const firmwareSourceOutcomeLabel = (outcome: FirmwareSourceCheck['outcome']): string => {
  const key: MessageKey = outcome === 'success'
    ? 'firmware.sourceSuccess'
    : outcome === 'empty'
      ? 'firmware.sourceEmpty'
      : outcome === 'failed'
        ? 'firmware.sourceFailed'
        : outcome === 'disabled'
          ? 'firmware.sourceDisabled'
          : 'firmware.sourceUnsupported'
  return t(key)
}

const busyPrinters = computed(() => printers.value.filter((printer) => badgeFor(printer.name) === 'busy').length)
const fleetStats = computed(() => [
  { label: t('printersView.totalPrinters'), value: printers.value.length, tone: 'text-gray-900 dark:text-white' },
  { label: t('printersView.online'), value: printers.value.filter((printer) => isReachable(badgeFor(printer.name))).length, tone: 'text-green-600 dark:text-green-400' },
  { label: t('printersView.printingNow'), value: busyPrinters.value, tone: busyPrinters.value ? 'text-yellow-600 dark:text-yellow-400' : 'text-gray-900 dark:text-white' },
])

function openFirmwareManagement(printer: Printer) {
  firmwarePrinterId.value = printer.name
  void loadFirmwareUpdates(printer.name)
}

function closeFirmwareManagement() {
  firmwarePrinterId.value = undefined
}

function openAddition() {
  draft.value = {
    name: '',
    driver: drivers.find((driver) => driver.name === 'moonraker')?.name ?? drivers[0]?.name ?? '',
    host: '',
    serial: '',
    model: '',
    timeout: '10s',
    insecure: false,
    accessCode: '',
  }
  additionBaseline.value = { ...draft.value }
  resetAdditionPanelState()
  additionOpen.value = true
  void discoverPrinters()
}

function closeAdditionImmediately() {
  additionOpen.value = false
  editingPrinter.value = undefined
}

function closeAddition() {
  if (adding.value) return
  if (!additionDirty.value) {
    closeAdditionImmediately()
    return
  }
  askConfirmation(
    { title: 'addition.discardTitle', description: 'addition.discardDescription', confirm: 'addition.discardConfirm' },
    {},
    async () => closeAdditionImmediately(),
  )
}

function warnBeforeUnload(event: BeforeUnloadEvent) {
  if (!additionDirty.value) return
  event.preventDefault()
  event.returnValue = ''
}

async function discoverPrinters() {
  if (discovering.value) return
  discovering.value = true
  discoveryError.value = undefined
  try {
    discovered.value = await invoke<DiscoveredPrinter[]>('discover_printers')
  } catch (reason) {
    discoveryError.value = message(reason)
  } finally {
    discovering.value = false
  }
}

function useDiscoveredPrinter(printer: DiscoveredPrinter) {
  draft.value.driver = printer.driver
  draft.value.host = printer.host
  draft.value.serial = printer.serial
  draft.value.model = printer.model
  detectedModelName.value = printer.displayModel
  if (!draft.value.name) draft.value.name = printer.name || printer.displayModel
  discovered.value = []
}

async function addPrinter() {
  if (adding.value) return
  adding.value = true
  additionError.value = undefined

  try {
    const replacing = editingPrinter.value
    const profile = replacing
      ? await invoke<Printer>('update_configured_printer', { name: replacing, request: draft.value })
      : await invoke<Printer>('create_configured_printer', { request: draft.value })
    // Re-read the list rather than rebuilding a row by hand: the backend
    // derives fields the create/update result does not carry, such as the
    // detected model name, and it already returns them sorted.
    printers.value = await invoke<Printer[]>('configured_printers')
    void loadCameras()
    void loadPlugs()
    additionOpen.value = false
    editingPrinter.value = undefined
    draft.value.accessCode = ''
    await selectPrinter(profile.name)
  } catch (reason) {
    additionError.value = message(reason)
  } finally {
    adding.value = false
  }
}

// Secondary printer actions live behind the overflow menu: three labelled
// buttons could not fit a 280px card, which is reachable whenever the window
// sits below the xl breakpoint.
function printerActionItems(printer: Printer): ActionMenuItem[] {
  return [
    { label: t('printersView.editPrinter'), icon: PhPencilSimple, onSelect: () => openEdit(printer) },
    { label: t('printersView.firmware'), icon: PhCpu, onSelect: () => openFirmwareManagement(printer) },
    {
      label: t('printersView.refreshCertificate'),
      icon: PhArrowsClockwise,
      disabled: tlsRefreshing.value,
      onSelect: () => void openTlsRefresh(printer.name),
    },
    { label: t('printersView.removePrinter'), icon: PhTrash, danger: true, onSelect: () => removePrinter(printer.name) },
  ]
}

function removePrinter(name: string) {
  askConfirmation(
    { title: 'removal.title', description: 'removal.description', confirm: 'removal.confirm' },
    { name },
    () => confirmRemovePrinter(name),
  )
}

async function confirmRemovePrinter(name: string) {
  try {
    await invoke('remove_configured_printer', { name })
    const remaining = printers.value.filter((printer) => printer.name !== name)
    printers.value = remaining
    monitoring.value = monitoring.value.filter((printer) => printer.name !== name)
    showToast(t('printersView.removed'))
    void loadCameras()
    void loadPlugs()
    const next = remaining[0]
    if (next) await selectPrinter(next.name, false)
    else {
      clearSelection()
      goTo('printers')
    }
  } catch (reason) {
    showToast(message(reason), 'error')
  }
}

async function openTlsRefresh(name: string) {
  if (tlsRefreshing.value) return
  tlsRefreshing.value = true
  tlsError.value = undefined
  tlsFingerprint.value = undefined
  tlsPrinter.value = name
  try {
    tlsFingerprint.value = await invoke<string>('preview_printer_tls', { name })
    tlsOpen.value = true
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    tlsRefreshing.value = false
  }
}

async function refreshTls() {
  if (!tlsPrinter.value || !tlsFingerprint.value || tlsRefreshing.value) return
  tlsRefreshing.value = true
  tlsError.value = undefined
  try {
    await invoke('refresh_printer_tls', {
      request: { name: tlsPrinter.value, fingerprint: tlsFingerprint.value, confirmed: true },
    })
    tlsOpen.value = false
    showToast(t('printersView.tlsRefreshRequested'))
    void refreshMonitoring()
  } catch (reason) {
    tlsError.value = message(reason)
  } finally {
    tlsRefreshing.value = false
  }
}

onMounted(() => window.addEventListener('beforeunload', warnBeforeUnload))
onUnmounted(() => window.removeEventListener('beforeunload', warnBeforeUnload))

defineExpose({ openEdit })
</script>

<template>
  <template v-if="active">
    <template v-if="firmwarePrinter">
      <div class="mb-7 px-4 sm:px-0">
        <Button variant="secondary" @click="closeFirmwareManagement">
          <PhArrowLeft class="size-4" aria-hidden="true" /> {{ t('firmware.backToPrinters') }}
        </Button>
        <div class="mt-5 lg:flex lg:items-end lg:justify-between lg:gap-6">
          <div class="min-w-0 flex-1">
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ firmwarePrinter.name }}</h2>
            <p class="mt-2 max-w-3xl text-sm text-gray-500 dark:text-gray-400">{{ t('firmware.description') }}</p>
            <div class="mt-3 flex flex-wrap gap-x-6 gap-y-2 font-mono text-xs text-gray-500 dark:text-gray-400">
              <span class="inline-flex items-center gap-1.5"><PhPrinter class="size-4 text-gray-500 dark:text-gray-400" aria-hidden="true" />{{ firmwarePrinterModel || firmwarePrinter.driver }}</span>
              <span class="inline-flex items-center gap-1.5"><PhNetwork class="size-4 text-gray-500 dark:text-gray-400" aria-hidden="true" />{{ firmwarePrinter.host }}</span>
            </div>
          </div>
          <div class="mt-5 flex items-center gap-2 lg:mt-0">
            <StatusBadge :status="badgeFor(firmwarePrinter.name)" :label="statusLabel(badgeFor(firmwarePrinter.name))" />
            <Button @click="selectPrinter(firmwarePrinter.name)"><PhCards class="size-4" aria-hidden="true" /> {{ t('printersView.openControl') }}</Button>
          </div>
        </div>
      </div>

      <Card class="overflow-hidden">
        <CardHeader :title="t('firmware.title')" :icon="PhCpu">
          <Button
            variant="secondary"
            :disabled="firmwareRefreshing"
            @click="loadFirmwareUpdates(firmwarePrinter.name, true)"
          ><PhArrowsClockwise class="size-4" :class="firmwareRefreshSource === 'printer' && 'animate-spin'" aria-hidden="true" /> {{ t('firmware.checkAgain') }}</Button>
          <Button
            variant="secondary"
            :disabled="firmwareRefreshing || !firmwarePrinterModel"
            @click="loadPublicFirmwareCatalogue(firmwarePrinter.name)"
          ><PhGlobe class="size-4" :class="firmwareRefreshSource === 'publicCatalogue' && 'animate-spin'" aria-hidden="true" /> {{ t('firmware.checkPublicCatalogue') }}</Button>
        </CardHeader>
        <div class="px-4 py-5 sm:p-6" :aria-busy="firmwareRefreshing">
          <div v-if="!selectedFirmwareUpdate" class="flex items-start gap-3 text-sm text-gray-500 dark:text-gray-400" role="status">
            <PhArrowsClockwise v-if="!firmwareError" class="mt-0.5 size-4 shrink-0 animate-spin" aria-hidden="true" />
            <PhWarning v-else class="mt-0.5 size-4 shrink-0 text-amber-500" aria-hidden="true" />
            <p>{{ firmwareError ?? t('firmware.checking') }}</p>
          </div>
          <template v-else>
            <p v-if="firmwareNeedsModel" class="mb-4 flex items-start gap-2 text-sm text-gray-500 dark:text-gray-400">
              <PhInfo class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
              {{ t('firmware.modelRequired') }}
            </p>
            <p v-if="selectedFirmwareUpdate.report.source === 'moonrakerUpdateManager'" class="mb-4 text-sm text-gray-500 dark:text-gray-400">
              {{ t('firmware.klipperScope') }}
            </p>
            <div class="mb-4 flex flex-wrap items-center gap-2 rounded-md border border-gray-200 bg-gray-50 px-3 py-2 dark:border-white/10 dark:bg-white/5">
              <span class="text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">{{ t('firmware.assessment') }}</span>
              <span class="rounded-full px-2 py-0.5 text-xs font-semibold" :class="firmwareAssessmentClasses(selectedFirmwareUpdate.report.assessment)">{{ firmwareAssessmentLabel(selectedFirmwareUpdate.report.assessment) }}</span>
            </div>
            <ul v-if="selectedFirmwareUpdate.report.issues.length" class="mb-4 space-y-1 text-sm text-gray-700 dark:text-gray-300">
              <li v-for="issue in selectedFirmwareUpdate.report.issues" :key="issue.code">{{ issue.message }}</li>
            </ul>
            <details class="mb-4 text-xs text-gray-500 dark:text-gray-400">
              <summary class="cursor-pointer select-none">{{ t('firmware.evidence') }} · {{ selectedFirmwareUpdate.report.components.length }} {{ t('firmware.modules') }} · {{ firmwareComparableCount(selectedFirmwareUpdate) }} {{ t('firmware.comparable') }}</summary>
              <div v-if="selectedFirmwareUpdate.sources?.length" class="mt-2 flex flex-wrap gap-2">
                <span v-for="source in selectedFirmwareUpdate.sources" :key="source.source" class="inline-flex items-center gap-1 rounded border border-gray-200 px-2 py-1 dark:border-white/10">
                  <span class="font-medium text-gray-700 dark:text-gray-300">{{ firmwareSourceName(source.source) }}</span>
                  <span>· {{ firmwareSourceOutcomeLabel(source.outcome) }}</span>
                </span>
              </div>
            </details>
            <div class="divide-y divide-gray-200 dark:divide-white/10">
              <div
                v-for="component in selectedFirmwareUpdate.report.components"
                :key="component.id"
                class="grid gap-3 py-4 first:pt-0 last:pb-0 sm:grid-cols-[minmax(10rem,1fr)_minmax(0,2fr)] sm:items-center"
              >
                <div class="min-w-0">
                  <div class="flex flex-wrap items-center gap-2">
                    <p class="truncate text-sm font-semibold text-gray-900 dark:text-white">{{ component.label }}</p>
                    <span v-if="component.required" class="rounded-full bg-amber-100 px-2 py-0.5 text-xs font-semibold uppercase tracking-wide text-amber-800 dark:bg-amber-400/10 dark:text-amber-300">{{ t('firmware.required') }}</span>
                  </div>
                  <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ component.label === firmwareComponentLabel(component.kind) ? component.id : firmwareComponentLabel(component.kind) }}</p>
                </div>
                <div class="min-w-0 rounded-md bg-gray-50 px-3 py-2 dark:bg-white/5">
                  <div class="flex items-baseline justify-between gap-3">
                    <span class="text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">{{ t('firmware.installed') }}</span>
                    <span class="truncate font-mono text-sm text-gray-800 dark:text-gray-200" :title="component.currentVersion">{{ component.currentVersion || '—' }}</span>
                  </div>
                  <div v-if="firmwareEvidenceFor(component).length" class="mt-2 flex flex-wrap gap-1.5">
                    <span
                      v-for="evidence in firmwareEvidenceFor(component)"
                      :key="`${evidence.role}-${evidence.version}`"
                      class="inline-flex items-center gap-1 rounded border px-2 py-1 text-xs"
                      :class="evidence.role === 'printerAdvertised'
                        ? 'border-amber-300/60 bg-amber-100/60 text-amber-800 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-300'
                        : evidence.role === 'deviceCatalogue'
                          ? 'border-cyan-300/60 bg-cyan-100/60 text-cyan-800 dark:border-cyan-400/20 dark:bg-cyan-400/10 dark:text-cyan-300'
                          : 'border-gray-300 bg-white text-gray-700 dark:border-white/15 dark:bg-white/5 dark:text-gray-300'"
                    >
                      <span>{{ firmwareEvidenceLabel(evidence.role) }}</span>
                      <span class="font-mono font-semibold">{{ evidence.version }}</span>
                    </span>
                  </div>
                  <span v-else class="mt-2 block text-xs text-gray-500 dark:text-gray-400">{{ t('firmware.noComparable') }}</span>
                </div>
              </div>
            </div>
            <div class="mt-4 flex flex-wrap items-center justify-between gap-2 border-t border-gray-200 pt-4 text-xs text-gray-500 dark:border-white/10 dark:text-gray-400">
              <span>{{ t('firmware.lastChecked', { date: formatDate(selectedFirmwareUpdate.checkedAt, locale) }) }}</span>
              <span v-if="selectedFirmwareUpdate.stale" class="font-medium text-amber-700 dark:text-amber-300">{{ t('firmware.stale') }}</span>
            </div>
            <p v-if="selectedFirmwareUpdate.error" class="mt-3 text-xs text-amber-700 dark:text-amber-300">{{ message(selectedFirmwareUpdate.error) }}</p>
          </template>
        </div>
      </Card>
    </template>

    <template v-else>
    <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
      <div class="min-w-0 flex-1">
        <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ t('printersView.title') }}</h2>
        <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">{{ t('printersView.description') }}</p>
      </div>
      <div class="mt-5 flex lg:mt-0 lg:ml-4 max-lg:w-full">
        <Button variant="primary" class="max-lg:w-full" :disabled="!drivers.length" @click="openAddition"><PhBroadcast class="size-4" aria-hidden="true" /> {{ t('printersView.discover') }}</Button>
      </div>
    </div>

    <dl v-if="hasPrinters" class="mb-5 flex flex-wrap gap-x-6 gap-y-1 px-4 text-sm sm:px-0">
      <div v-for="stat in fleetStats" :key="stat.label" class="flex items-baseline gap-1.5">
        <dt class="text-gray-500 dark:text-gray-400">{{ stat.label }}</dt>
        <dd class="font-semibold tabular-nums" :class="stat.tone">{{ stat.value }}</dd>
      </div>
    </dl>

    <!-- Loading / error states -->
    <div v-if="loading" class="mx-4 flex min-h-97.5 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15" role="status" aria-live="polite">
      <PhPrinter class="mx-auto size-12 text-gray-500 dark:text-gray-400" aria-hidden="true" />
      <p class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('profiles.loading') }}</p>
    </div>
    <div v-else-if="profileError || loadError" class="mx-4 flex min-h-97.5 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15" role="alert">
      <PhWarning class="mx-auto size-12 text-red-400 dark:text-red-500" aria-hidden="true" />
      <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('profiles.error') }}</h3>
      <p class="mt-1 max-w-sm text-sm text-gray-500 dark:text-gray-400">{{ profileError ?? loadError }}</p>
      <Button class="mt-6" variant="primary" @click="load"><PhArrowsClockwise class="size-4" aria-hidden="true" /> {{ t('common.retry') }}</Button>
    </div>

    <!-- Empty state -->
    <div v-else-if="!hasPrinters" class="mx-4 flex min-h-97.5 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15">
      <PhPrinter class="mx-auto size-12 text-gray-500 dark:text-gray-400" aria-hidden="true" />
      <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('printersView.emptyTitle') }}</h3>
      <p class="mt-1 max-w-sm text-sm text-gray-500 dark:text-gray-400">{{ t('printersView.emptyDescription') }}</p>
      <Button class="mt-6" variant="primary" :disabled="!drivers.length" @click="openAddition"><PhBroadcast class="size-4" aria-hidden="true" /> {{ t('printersView.scan') }}</Button>
    </div>

    <div v-else class="grid gap-5 grid-cols-[repeat(auto-fill,minmax(280px,1fr))]">
      <Card v-for="printer in printers" :key="printer.name" as="article" class="px-4 py-5 sm:p-6">
        <div class="flex items-start justify-between">
          <div class="flex items-center gap-3">
            <span class="grid size-10 place-items-center rounded-lg bg-gray-100 text-cyan-600 dark:bg-white/10 dark:text-cyan-400"><PhPrinter class="size-5" aria-hidden="true" /></span>
            <div>
              <h3 class="font-semibold text-gray-900 dark:text-white">{{ printer.name }}</h3>
              <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ printer.detectedModel || printer.presence?.model || '—' }}</p>
            </div>
          </div>
          <div class="flex shrink-0 items-center gap-1">
            <IconButton
              v-if="printer.serial"
              :title="t(serialIsRevealed(printer.name) ? 'printersView.hideSerialNamed' : 'printersView.showSerialNamed', { name: printer.name })"
              :aria-label="t(serialIsRevealed(printer.name) ? 'printersView.hideSerialNamed' : 'printersView.showSerialNamed', { name: printer.name })"
              :aria-pressed="serialIsRevealed(printer.name)"
              @click="toggleSerial(printer.name)"
            >
              <PhEyeSlash v-if="serialIsRevealed(printer.name)" class="size-4" aria-hidden="true" />
              <PhEye v-else class="size-4" aria-hidden="true" />
            </IconButton>
          </div>
        </div>
        <div class="mt-6 flex items-center justify-between border-y border-gray-200 py-3 dark:border-white/10">
          <span class="text-xs text-gray-500 dark:text-gray-400">{{ t('printersView.status') }}</span>
          <div class="flex flex-wrap items-center justify-end gap-2">
            <span
              v-if="hasAvailableUpdate(firmwareFor(printer.name))"
              class="inline-flex items-center rounded-md bg-amber-100 px-2 py-1 text-xs font-medium text-amber-800 dark:bg-amber-400/10 dark:text-amber-300"
            >{{ hasRequiredUpdate(firmwareFor(printer.name)) ? t('firmware.requiredUpdate') : t('firmware.updateAvailable') }}</span>
            <StatusBadge :status="badgeFor(printer.name)" :label="statusLabel(badgeFor(printer.name))" />
          </div>
        </div>
        <!-- Description list -->
        <dl class="mt-2 divide-y divide-gray-200 text-xs dark:divide-white/10">
          <div class="flex items-center justify-between py-2">
            <dt class="text-gray-500 dark:text-gray-400">{{ t('addition.driver') }}</dt>
            <dd class="text-gray-900 dark:text-gray-300">{{ printer.driver }}</dd>
          </div>
          <div class="flex items-center justify-between py-2">
            <dt class="text-gray-500 dark:text-gray-400">{{ t('addition.serial') }}</dt>
            <dd class="font-mono text-gray-900 dark:text-gray-300">
              <span aria-live="polite">
                <span class="sr-only">{{ printer.serial ? (serialIsRevealed(printer.name) ? printer.serial : t('printersView.serialHidden')) : '—' }}</span>
                <span aria-hidden="true">{{ serialNumberDisplay(printer.serial, serialIsRevealed(printer.name)) }}</span>
              </span>
            </dd>
          </div>
          <div class="flex items-center justify-between py-2">
            <dt class="text-gray-500 dark:text-gray-400">{{ t('printersView.host') }}</dt>
            <dd class="text-gray-900 dark:text-gray-300">{{ printer.host }}</dd>
          </div>
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
          <div class="flex items-center justify-between py-2">
            <dt class="text-gray-500 dark:text-gray-400">{{ t('addition.timeout') }}</dt>
            <dd class="font-mono text-gray-900 dark:text-gray-300">{{ printer.timeout }}</dd>
          </div>
          <div class="flex items-center justify-between py-2">
            <dt class="text-gray-500 dark:text-gray-400">{{ t('printersView.tlsVerification') }}</dt>
            <dd :class="printer.insecure ? 'text-red-600 dark:text-red-400' : 'text-gray-900 dark:text-gray-300'">{{ printer.insecure ? t('printersView.tlsDisabled') : t('printersView.tlsVerified') }}</dd>
          </div>
        </dl>
        <div class="mt-5 flex items-center gap-2">
          <Button class="min-w-0 flex-1" @click="selectPrinter(printer.name)"><PhCards class="size-4" aria-hidden="true" /> {{ t('printersView.openControl') }}</Button>
          <ActionMenu :label="t('control.printerActions')" :items="printerActionItems(printer)" />
        </div>
      </Card>
      <button
        type="button"
        class="relative flex min-h-55 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 p-12 text-center hover:border-gray-400 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:border-white/15 dark:hover:border-white/25 dark:focus-visible:outline-cyan-500"
        @click="openAddition"
      >
        <PhPlus class="mx-auto size-12 text-gray-500 dark:text-gray-400" aria-hidden="true" />
        <span class="mt-2 block text-sm font-semibold text-gray-900 dark:text-white">{{ t('printersView.addAnother') }}</span>
      </button>
    </div>
    </template>
  </template>

  <SlideOver :open="additionOpen" :title="editingPrinter ? t('printersView.editPrinter') : t('addition.title')" :description="t('addition.description')" :close-label="t('common.closePanel')" @close="closeAddition">
    <form id="add-printer-form" @submit.prevent="addPrinter">
      <Button class="w-full" :disabled="discovering" @click="discoverPrinters"><PhBroadcast class="size-4" :class="discovering && 'animate-pulse'" aria-hidden="true" /> {{ discovering ? t('printersView.scanning') : t('printersView.discover') }}</Button>
      <p v-if="discoveryError" class="mt-2 text-xs text-red-600 dark:text-red-400" role="alert">{{ discoveryError }}</p>
      <ul v-if="discovered.length" class="mt-3 divide-y divide-gray-200 rounded-md border border-gray-200 dark:divide-white/10 dark:border-white/10">
        <li v-for="printer in discovered" :key="`${printer.serial}:${printer.host}`" class="flex items-center justify-between gap-2 px-3 py-2">
          <span class="min-w-0 truncate text-sm text-gray-900 dark:text-white">{{ printer.name || printer.displayModel || printer.host }} · <span class="text-xs text-gray-500 dark:text-gray-400">{{ printer.displayModel || t('printersView.unknownModel') }}</span> · <span class="font-mono text-xs text-gray-500 dark:text-gray-400">{{ printer.host }}</span></span>
          <Button @click="useDiscoveredPrinter(printer)">{{ t('addition.useDiscovery') }}</Button>
        </li>
      </ul>
      <div class="mt-6 space-y-4">
        <div>
          <label for="printer-name" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.name') }}</label>
          <input id="printer-name" name="printer-name" v-model="draft.name" required maxlength="64" autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
        </div>
        <div>
          <label for="printer-driver" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.driver') }}</label>
          <div class="mt-2 grid grid-cols-1">
            <select id="printer-driver" name="printer-driver" v-model="draft.driver" required class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
              <option v-for="driver in drivers" :key="driver.name" :value="driver.name">{{ driver.name }}</option>
            </select>
            <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
          </div>
        </div>
        <div>
          <label for="printer-host" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.host') }}</label>
          <input id="printer-host" name="printer-host" v-model="draft.host" required autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
        </div>
        <div>
          <label for="printer-serial" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.serial') }}</label>
          <input id="printer-serial" name="printer-serial" v-model="draft.serial" autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
        </div>
        <div>
          <label for="printer-model" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.model') }}</label>
          <input id="printer-model" name="printer-model" :value="detectedModelName" readonly aria-readonly="true" :placeholder="t('addition.modelPending')" class="mt-2 block w-full cursor-default rounded-md bg-gray-50 px-3 py-1.5 text-base text-gray-500 outline-1 -outline-offset-1 outline-gray-300 sm:text-sm/6 dark:bg-white/5 dark:text-gray-400 dark:outline-white/10" />
          <p class="mt-2 text-xs text-gray-500 dark:text-gray-400">{{ t('addition.modelHint') }}</p>
        </div>
        <TimeoutField id="printer-timeout" v-model="draft.timeout" :t="t" />
        <div>
          <label for="printer-access-code" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ editingPrinter ? t('addition.accessCodeKeep') : t('addition.accessCode') }}</label>
          <input id="printer-access-code" name="printer-access-code" v-model="draft.accessCode" type="password" autocomplete="new-password" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
        </div>
        <div class="flex items-center justify-between">
          <label for="printer-insecure" class="grow cursor-pointer text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.insecure') }}</label>
          <Switch id="printer-insecure" v-model="draft.insecure" :label="t('addition.insecure')" />
        </div>
      </div>
      <p v-if="additionError" class="mt-4 text-sm text-red-600 dark:text-red-400" role="alert">{{ additionError }}</p>
    </form>
    <template #footer>
      <Button :disabled="adding" @click="closeAddition">{{ t('common.cancel') }}</Button>
      <Button variant="primary" type="submit" form="add-printer-form" :disabled="adding">{{ editingPrinter ? (adding ? t('common.saving') : t('profiles.save')) : (adding ? t('common.adding') : t('profiles.add')) }}</Button>
    </template>
  </SlideOver>

  <ConfirmDialog
    :open="tlsOpen"
    :title="t('tls.confirmTitle', { name: tlsPrinter ?? '' })"
    :description="t('tls.confirmDescription')"
    :confirm-label="tlsRefreshing ? t('common.sending') : t('tls.confirm')"
    :cancel-label="t('common.cancel')"
    :busy="tlsRefreshing"
    @close="!tlsRefreshing && (tlsOpen = false)"
    @confirm="refreshTls"
  >
    <code v-if="tlsFingerprint" class="block rounded-md bg-gray-100 p-3 font-mono text-xs break-all text-gray-900 dark:bg-white/10 dark:text-white">{{ tlsFingerprint }}</code>
    <p v-if="tlsError" class="mt-4 text-sm text-red-600 dark:text-red-400" role="alert">{{ tlsError }}</p>
  </ConfirmDialog>
</template>
