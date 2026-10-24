<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { save as saveFileDialog } from '@tauri-apps/plugin-dialog'
import type { Locale, MessageKey } from '../i18n'
import type { PrinterBadge } from '../monitoring'
import { clampTarget, formatAmsHumidity, formatDuration, remainingTimePresentation, formatSize, formatDate } from '../formatting'
import {
  dryingBounds,
  dryingDefaults,
  dryingFault,
  dryingRequestValid,
  dryingStoppable,
  dryingTargetShown,
  type DryingStatus,
} from '../drying'
import { printStageLabel } from '../stages'
import {
  awaitsFirstSample,
  filamentColor,
  filamentColorName,
  filamentFillPercent,
  isActiveJobState,
  lightModeMessageKey,
  materialSystemLabel,
  plateLabel,
  printerStateMessageKey,
  shouldRunCamera,
} from '../presentation'
import { linkedCameras, printerSource, type CameraSummary } from '../cameras'
import { plugsForPrinter, type PlugObservation, type PlugSummary } from '../plugs'
import { hmsGuideUrl, hmsSeverity, type HmsSeverity } from '../hms'
import { nextSort, sortedBy, type FileSort, type SortKey, fileTypeLabel } from '../library'
import { temperatureMaximums, type TemperaturePreset } from '../temperatures'
import StatusBadge from '../components/StatusBadge.vue'
import ActionMenu, { type ActionMenuItem } from '../components/ActionMenu.vue'
import SlideOver from '../components/SlideOver.vue'
import Button from '../components/Button.vue'
import IconButton from '../components/IconButton.vue'
import Switch from '../components/Switch.vue'
import Card from '../components/Card.vue'
import CardHeader from '../components/CardHeader.vue'
import ModelThumbnail from '../components/ModelThumbnail.vue'
import CameraViewer from '../components/CameraViewer.vue'
import PlugControl from '../components/PlugControl.vue'
import {
  PhArrowsClockwise,
  PhArrowsDownUp,
  PhArrowsOutCardinal,
  PhArrowSquareOut,
  PhBroadcast,
  PhCaretDown,
  PhCheckSquareOffset,
  PhCube,
  PhDownloadSimple,
  PhDrop,
  PhFan,
  PhFolder,
  PhHexagon,
  PhHouse,
  PhInfo,
  PhKey,
  PhMinus,
  PhNetwork,
  PhPause,
  PhPencilSimple,
  PhPlay,
  PhPlus,
  PhPrinter,
  PhStack,
  PhStop,
  PhSun,
  PhThermometerSimple,
  PhTrash,
  PhWarning,
  PhWarningCircle,
  PhWifiHigh,
  PhWifiLow,
  PhWifiMedium,
  PhWifiSlash,
} from '@phosphor-icons/vue'
import type {
  Printer,
  Capabilities,
  TemperatureControl,
  FanControl,
  LightControl,
  StatusFault,
  MonitorEntry,
  KeychainProbe,
  TransferProgress,
  FileEntry,
  FileList,
  MaterialSystemView,
  ViewActions,
} from '../app-types'

const {
  active,
  activePrinter,
  activeBadge,
  monitoring,
  capabilities,
  cameras,
  documentVisible,
  plugs,
  plugObservations,
  plugActivity,
  locale,
  libraryFiles,
  temperaturePresets,
  refreshing,
  refreshMonitoring,
  refreshPlugs,
  loadCameras,
  openEdit,
  statusLabel,
  isReachable,
  printPreparing,
  printBusy,
  requestPrint,
  t,
  message,
  showToast,
  askConfirmation,
} = defineProps<ViewActions & {
  active: boolean
  activePrinter: Printer | undefined
  activeBadge: PrinterBadge
  monitoring: MonitorEntry[]
  capabilities: Capabilities | undefined
  cameras: CameraSummary[]
  documentVisible: boolean
  plugs: PlugSummary[]
  plugObservations: Record<string, PlugObservation>
  plugActivity: Record<string, string>
  locale: Locale
  libraryFiles: FileEntry[]
  temperaturePresets: TemperaturePreset[]
  refreshing: boolean
  refreshMonitoring: () => Promise<void>
  refreshPlugs: () => Promise<PlugSummary[]>
  loadCameras: () => Promise<void>
  openEdit: (printer: Printer) => void
  statusLabel: (badge: PrinterBadge) => string
  isReachable: (badge: PrinterBadge) => boolean
  printPreparing: boolean
  printBusy: boolean
  requestPrint: (file: FileEntry, printer?: string) => void
}>()

const files = ref<FileEntry[]>([])
const keychainChecking = ref(false)
const filesLoading = ref(false)
const filesError = ref<string>()
const deletingPrinterPath = ref<string>()
let transferUnlisten: UnlistenFn | undefined
let fileRequest = 0
const stepSize = ref('1 mm')
const feedRate = ref(50)
const jogBusy = ref(false)
const speedProfile = ref<'silent' | 'standard' | 'sport' | 'ludicrous'>('standard')
const speedBusy = ref(false)
const downloadingPath = ref<string>()
const downloadProgress = ref<number>()
const selectedMonitor = computed(() => monitoring.find((entry) => entry.name === activePrinter?.name))
const selectedStatus = computed(() => selectedMonitor.value?.status)
const activeHasStatus = computed(() => isReachable(activeBadge) || activeBadge === 'reconnecting' || activeBadge === 'error')
const awaitingFirstSample = computed(() => awaitsFirstSample(activeBadge))
const hasActiveJob = computed(() => isActiveJobState(selectedStatus.value?.state))

const currentStageLabel = computed(() => {
  if (!hasActiveJob.value) return undefined
  return printStageLabel(locale, selectedStatus.value?.stage, selectedStatus.value?.stageCode)
})

const progressPercent = computed(() => selectedStatus.value?.progress?.percent ?? 0)

const preparingPercent = computed(() => {
  if (!hasActiveJob.value) return undefined
  const progress = selectedStatus.value?.progress
  if (!progress || progress.percent > 0) return undefined
  return progress.preparationPercent
})

// The printing file lives on the printer, so a preview is only possible when
// the same name happens to sit in the local library. No match, no thumbnail.
const jobThumbnail = computed(() => {
  const name = selectedStatus.value?.job?.name
  if (!name) return undefined
  return libraryFiles.find((file) => file.type === 'file' && file.name === name)
})

const remainingTimeLabel = computed(() => {
  const remaining = selectedStatus.value?.timeEstimates?.remainingSeconds
  if (!hasActiveJob.value || remaining === undefined) return undefined
  const presentation = remainingTimePresentation(remaining)
  return presentation.kind === 'almostComplete'
    ? t('control.almostComplete')
    : t('control.remaining', { duration: presentation.duration })
})

// aria-valuenow alone announces a bare "45". Spell out what the number means,
// and fold in the layer and remaining time a sighted user reads beside the bar.
const progressValueText = computed(() => {
  if (preparingPercent.value !== undefined) return t('control.preparing')
  const parts = [t('control.complete', { percent: progressPercent.value })]
  const progress = selectedStatus.value?.progress
  if (progress?.currentLayer !== undefined && progress.totalLayers !== undefined) {
    parts.push(t('control.layer', { current: progress.currentLayer, total: progress.totalLayers }))
  }
  if (remainingTimeLabel.value) parts.push(remainingTimeLabel.value)
  return parts.join(' · ')
})

const faultSeverityKeys: Record<HmsSeverity, MessageKey> = {
  error: 'control.faultSeverityError',
  warning: 'control.faultSeverityWarning',
  info: 'control.faultSeverityInfo',
}

const statusFaults = computed<StatusFault[]>(() => {
  const status = selectedStatus.value
  if (!status) return []
  return [
    ...status.errors.map((entry) => ({
      ...entry,
      severity: hmsSeverity(entry.rawCode),
      guideUrl: hmsGuideUrl(entry.rawCode),
    })),
    ...status.warnings.map((entry) => ({ ...entry, severity: 'warning' as const })),
  ]
})

// Annotated so both branches unify on TemperatureControl: the legacy fallback
// carries no advertised limits, and consumers must still see those fields as
// optional rather than absent.
const temperatureRows = computed<{ key: string; value: TemperatureControl }[]>(() => {
  const controls = Object.entries(selectedStatus.value?.controls?.temperatures ?? {})
  if (controls.length) return controls.map(([key, value]) => ({ key, value }))
  const temperatures = selectedStatus.value?.temperatures ?? {}
  return (['nozzle', 'bed', 'chamber'] as const).flatMap((key) => {
    const value = temperatures[key]
    return value ? [{ key, value: { kind: key, ...value, controllable: Boolean(capabilities?.temperatureWrite) } }] : []
  })
})

const fanRows = computed(() => {
  const controls = Object.entries(selectedStatus.value?.controls?.fans ?? {})
  if (controls.length) return controls
  return Object.entries(selectedStatus.value?.fans ?? {}).map(([key, speed]) => [key, {
    kind: 'unknown', mode: 'manual', speedPercent: speed, controllable: Boolean(capabilities?.fanControl),
  }] as [string, FanControl])
})

// While dragging, the readout follows the pointer; the committed value comes
// back from the printer on the next status push.
const fanDrafts = ref<Record<string, number>>({})

function previewFan(key: string, event: Event) {
  fanDrafts.value[key] = Number((event.target as HTMLInputElement).value)
}

const fanMessageKeys: Record<string, MessageKey> = {
  partCooling: 'control.fanPartCooling',
  parts: 'control.fanPartCooling',
  auxiliary: 'control.fanAuxiliary',
  heatbreak: 'control.fanHotend',
  hotend: 'control.fanHotend',
  exhaust: 'control.fanExhaust',
  chamber: 'control.fanChamber',
  mainboard: 'control.fanMainboard',
  heat: 'control.fanHeat',
  hotendSecondary: 'control.fanHotendSecondary',
  auxiliarySecondary: 'control.fanAuxiliarySecondary',
}

const fanLabel = (key: string) => {
  const messageKey = fanMessageKeys[key]
  return messageKey ? t(messageKey) : key
}

const temperatureLabel = (key: string, control: TemperatureControl) => t(
  control.kind === 'nozzle' && control.position === 'left'
    ? 'control.nozzleLeft'
    : control.kind === 'nozzle' && control.position === 'right'
      ? 'control.nozzleRight'
      : control.kind === 'nozzle'
        ? 'dashboard.nozzle'
        : control.kind === 'bed'
          ? 'dashboard.bed'
          : control.kind === 'chamber'
            ? 'control.chamber'
            : key as MessageKey,
)

const wifiDbm = computed(() => selectedStatus.value?.wifi?.signalDbm)

const lightRows = computed(() => {
  const controls = Object.entries(selectedStatus.value?.controls?.lights ?? {})
  if (controls.length) return controls
  return Object.entries(selectedStatus.value?.lights ?? {}).map(([key, mode]) => [key, {
    kind: key === 'chamber_light' ? 'lamp' : 'unknown', mode, controllable: Boolean(capabilities?.lightControl),
  }] as [string, LightControl])
})

const lightMessageKeys: Record<string, MessageKey> = {
  lamp: 'control.lamp',
  chamber_light: 'control.chamberLight',
  work_light: 'control.workLight',
  aux_light: 'control.auxLight',
}

const lightLabel = (key: string) => {
  const messageKey = lightMessageKeys[key]
  return messageKey ? t(messageKey) : key
}

const dryingStatusMessageKeys: Record<DryingStatus, MessageKey> = {
  off: 'drying.status.off',
  checking: 'drying.status.checking',
  drying: 'drying.status.drying',
  cooling: 'drying.status.cooling',
  stopping: 'drying.status.stopping',
  error: 'drying.status.error',
  heatOutOfControl: 'drying.status.heatOutOfControl',
  unknown: 'drying.status.unknown',
}

const dryingStatusKey = (status: DryingStatus) => dryingStatusMessageKeys[status] ?? 'drying.status.unknown'

const dryingBlockedMessageKeys: Record<string, MessageKey> = {
  printer_busy: 'drying.blocked.printer_busy',
  insufficient_power: 'drying.blocked.insufficient_power',
  ams_busy: 'drying.blocked.ams_busy',
  filament_at_outlet: 'drying.blocked.filament_at_outlet',
  already_starting: 'drying.blocked.already_starting',
  unsupported_in_2d_mode: 'drying.blocked.unsupported_in_2d_mode',
  already_drying: 'drying.blocked.already_drying',
  firmware_upgrading: 'drying.blocked.firmware_upgrading',
  external_power_required: 'drying.blocked.external_power_required',
  unknown: 'drying.blocked.unknown',
}

const dryingBlockedKey = (reason: string) => dryingBlockedMessageKeys[reason] ?? 'drying.blocked.unknown'

// Material inventory currently travels in the shared Bambu-shaped extension;
// its user-facing terminology still follows the selected printer's driver.
const materialSystems = computed<MaterialSystemView[]>(() => {
  const units = selectedStatus.value?.extensions?.['bambu-lan']?.ams?.units ?? []
  const driver = activePrinter?.driver ?? ''
  const materialUnitCount = units.filter((unit) => unit.id < 254).length
  return units.map((unit) => ({
    name: materialSystemLabel(driver, unit.id, t('materials.externalSpool'), materialUnitCount),
    unitId: unit.id,
    temperature: unit.temperature,
    humidity: formatAmsHumidity(unit.humidityPercent, unit.humidityIndex, locale),
    drying: unit.drying,
    firstFilament: unit.trays.find((tray) => tray.filamentType)?.filamentType,
    slots: unit.trays.map((tray) => {
      const color = filamentColor(tray.color)
      const colorName = filamentColorName(tray.color)
      return {
        slot: unit.id >= 254 ? t('materials.slotExternal') : String(tray.slot + 1),
        type: tray.filamentType ?? '—',
        color,
        colorLabel: colorName === 'materials.color.unknown'
          ? t(colorName)
          : t('materials.colorLabel', { name: t(colorName), hex: color }),
        remainingPercent: filamentFillPercent(tray.remainingPercent),
        remainingGrams: tray.remainingGrams,
      }
    }),
  }))
})

const controlCameras = computed(() => activePrinter ? linkedCameras(activePrinter.name, cameras) : [])
const controlCameraSource = computed(() => activePrinter ? printerSource(activePrinter.name, cameras) : undefined)

// '' is the printer's built-in feed. An external camera does not depend on
// the printer's capabilities or connection.
const controlCameraName = computed(() => controlCameraSource.value && 'camera' in controlCameraSource.value ? controlCameraSource.value.camera : '')
const cameraSupported = computed(() => Boolean(controlCameraName.value || capabilities?.cameraStream || capabilities?.cameraSnapshot))
const cameraMayRun = computed(() => shouldRunCamera(active ? 'control' : '', 'control', documentVisible, cameraSupported.value))
const controlCameraViewer = ref<InstanceType<typeof CameraViewer>>()

async function checkKeychain() {
  if (keychainChecking.value || !activePrinter) return
  keychainChecking.value = true
  try {
    const probe = await invoke<KeychainProbe>('probe_keychain', { name: activePrinter.name })
    if (probe.available && !probe.reason) {
      showToast(t('control.keychainAvailable'))
    } else {
      showToast(message({ code: probe.reason ?? 'keychainUnavailable' }), 'error')
    }
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    keychainChecking.value = false
  }
}

// The Control tab's file table always shows the printer's storage root; it
// has no breadcrumb navigation, unlike the local file library below.
async function loadFiles() {
  if (!activePrinter || !capabilities?.fileList) return
  const request = ++fileRequest
  const printerName = activePrinter.name
  filesLoading.value = true
  filesError.value = undefined
  try {
    const result = await invoke<FileList>('printer_files', {
      request: { name: printerName, path: '/', search: '' },
    })
    if (request === fileRequest && activePrinter?.name === printerName) files.value = result.entries
  } catch (reason) {
    filesError.value = message(reason)
  } finally {
    filesLoading.value = false
  }
}

async function selectControlFeed(camera: string) {
  const printer = activePrinter?.name
  if (!printer) return
  try {
    await invoke('camera_select', { printer, camera: camera || null })
  } catch (reason) {
    showToast(message(reason), 'error')
  }
  await loadCameras()
}

const jobToastKeys: Record<'pause' | 'resume' | 'cancel', MessageKey> = {
  pause: 'control.jobPaused',
  resume: 'control.jobResumed',
  cancel: 'control.cancelRequested',
}

function sendJobAction(action: 'pause' | 'resume' | 'cancel') {
  if (!activePrinter) return
  if (action !== 'cancel') return void runJobAction(action)
  askConfirmation(
    { title: 'confirm.cancelTitle', description: 'confirm.cancelDescription', confirm: 'confirm.cancelConfirm' },
    { name: activePrinter.name },
    () => runJobAction(action),
  )
}

async function runJobAction(action: 'pause' | 'resume' | 'cancel') {
  if (!activePrinter) return
  try {
    await invoke('printer_job_action', {
      request: { name: activePrinter.name, action, confirmed: true },
    })
    showToast(t(jobToastKeys[action]))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
  }
}

const temperatureTimers: Record<string, number | undefined> = {}
const pendingTemperatures: Record<string, number | undefined> = {}

function queueTemperature(key: string, control: TemperatureControl, next: number) {
  const maximum = control.maximumCelsius ?? temperatureMaximums[control.kind]
  const minimum = control.minimumCelsius ?? 0
  pendingTemperatures[key] = Math.max(minimum, clampTarget(maximum, next))
  if (temperatureTimers[key] !== undefined) window.clearTimeout(temperatureTimers[key])
  temperatureTimers[key] = window.setTimeout(async () => {
    const printerName = activePrinter?.name
    const target = pendingTemperatures[key]
    delete pendingTemperatures[key]
    delete temperatureTimers[key]
    if (!printerName || target === undefined) return
    try {
      await invoke('printer_temperature_set', {
        request: { name: printerName, temperature: key, targetCelsius: target },
      })
      void refreshMonitoring()
    } catch (reason) {
      showToast(message(reason), 'error')
    }
  }, 150)
}

function adjustTemperature(key: string, control: TemperatureControl, delta: number) {
  if (!activePrinter || !selectedStatus.value) return
  const base = pendingTemperatures[key] ?? control.targetCelsius ?? control.currentCelsius
  if (base === undefined) return
  queueTemperature(key, control, base + delta)
}

// Typed entry, so reaching 250 °C doesn't cost fifty clicks on the steppers.
// `queueTemperature` still clamps against the control's advertised limits, so
// an out-of-range keystroke can never reach the printer.
function setTemperature(key: string, control: TemperatureControl, event: Event) {
  if (!activePrinter || !selectedStatus.value) return
  const raw = (event.target as HTMLInputElement).value
  if (raw.trim() === '') return
  const next = Number(raw)
  if (!Number.isFinite(next)) return
  queueTemperature(key, control, Math.round(next))
}

// A preset only touches the kinds it knows about; chamber targets and any
// uncontrollable sensor are left exactly as they were.
const presetTargetRows = computed(() =>
  temperatureRows.value.filter((row) => row.value.controllable && (row.value.kind === 'nozzle' || row.value.kind === 'bed')),
)

const presetsAvailable = computed(() =>
  Boolean(capabilities?.temperatureWrite) && presetTargetRows.value.length > 0,
)

function applyPreset(preset?: TemperaturePreset) {
  if (!activePrinter || selectedStatus.value?.state !== 'idle') return
  for (const row of presetTargetRows.value) {
    // No preset means "cool down": every controllable heater goes to zero.
    const target = preset ? (row.value.kind === 'nozzle' ? preset.nozzleCelsius : preset.bedCelsius) : 0
    queueTemperature(row.key, row.value, target)
  }
  showToast(preset ? t('control.presetApplied', { name: preset.name }) : t('control.presetCooling'))
}

async function sendFan(fan: string, event: Event) {
  delete fanDrafts.value[fan]
  if (!activePrinter) return
  const speed = Number((event.target as HTMLInputElement).value)
  if (!Number.isInteger(speed)) return
  try {
    await invoke('printer_fan_set', {
      request: { name: activePrinter.name, fan, speedPercent: speed },
    })
    showToast(t('control.fanSet', { fan: fanLabel(fan), percent: speed }))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
    void refreshMonitoring()
  }
}

async function homeAxes() {
  if (!activePrinter) return
  try {
    await invoke('printer_motion_home', { name: activePrinter.name })
    showToast(t('control.homingStarted'))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
  }
}

const jogDisabled = computed(() => jogBusy.value || selectedStatus.value?.state !== 'idle')

function jogDistance() {
  const parsed = Number.parseFloat(stepSize.value)
  return Number.isFinite(parsed) ? parsed : 1
}

async function jog(axis: 'x' | 'y' | 'z', direction: 1 | -1) {
  if (!activePrinter || jogDisabled.value) return
  jogBusy.value = true
  const distance = jogDistance() * direction
  try {
    await invoke('printer_motion_jog', {
      request: {
        name: activePrinter.name,
        xMillimeters: axis === 'x' ? distance : undefined,
        yMillimeters: axis === 'y' ? distance : undefined,
        zMillimeters: axis === 'z' ? distance : undefined,
        feedrateMmPerMin: feedRate.value * 60,
      },
    })
    showToast(t('control.moved', { axis: axis.toUpperCase(), distance: `${distance} mm` }))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    jogBusy.value = false
  }
}

async function toggleLight(light: string, on: boolean) {
  if (!activePrinter) return
  try {
    await invoke('printer_light_set', {
      request: { name: activePrinter.name, light, on },
    })
    showToast(t('control.lightSet', { light: lightLabel(light), state: t(on ? 'common.enabled' : 'common.disabled') }))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
    void refreshMonitoring()
  }
}

const dryingForm = ref<{ unitId: number; temperatureC: number; hours: number; filament: string } | undefined>()
const dryingBusy = ref(false)
const dryingFormSystem = computed(() => materialSystems.value.find((system) => system.unitId === dryingForm.value?.unitId))

const dryingFormValid = computed(() =>
  Boolean(dryingForm.value) && dryingRequestValid(dryingForm.value!.unitId, dryingForm.value!.temperatureC, dryingForm.value!.hours),
)

// An open panel carries only a unit id, so a printer switch would aim Start at
// the newly selected printer's unit of the same id. Close it with the switch,
// and drop it when the status stops reporting that unit at all — judged only
// when other units are still reported, so a momentary gap keeps the panel.
watch(() => activePrinter?.name, () => {
  dryingForm.value = undefined
})

watch([dryingFormSystem, materialSystems], ([system, systems]) => {
  if (dryingForm.value && systems.length && !system) dryingForm.value = undefined
})

function openDryingForm(system: MaterialSystemView) {
  const defaults = dryingDefaults(system.unitId, system.firstFilament)
  if (defaults) dryingForm.value = { unitId: system.unitId, ...defaults }
}

function setDrying(system: MaterialSystemView, start: boolean) {
  if (!activePrinter) return
  const name = activePrinter.name
  const form = dryingForm.value
  const drying = start && form
    ? { action: 'start', temperatureC: form.temperatureC, hours: form.hours, filament: form.filament }
    : { action: 'stop' }
  askConfirmation(
    start
      ? { title: 'drying.startTitle', description: 'drying.startDescription', confirm: 'drying.startConfirm' }
      : { title: 'drying.stopTitle', description: 'drying.stopDescription', confirm: 'drying.stopConfirm' },
    { unit: system.name, temperature: String(form?.temperatureC ?? ''), hours: String(form?.hours ?? '') },
    async () => {
      dryingBusy.value = true
      try {
        await invoke('printer_ams_drying_set', { request: { name, amsId: system.unitId, drying } })
        dryingForm.value = undefined
        showToast(t(start ? 'drying.started' : 'drying.stopped', { unit: system.name }))
      } catch (reason) {
        showToast(message(reason), 'error')
      } finally {
        dryingBusy.value = false
        void refreshMonitoring()
      }
    },
    'primary',
  )
}

function dryingActionItems(system: MaterialSystemView): ActionMenuItem[] {
  if (!system.drying?.controllable) return []
  if (dryingStoppable(system.drying)) {
    return [{ label: `${system.name}: ${t('drying.stop')}`, icon: PhStop, disabled: dryingBusy.value, onSelect: () => setDrying(system, false) }]
  }
  // `controllable` is the printer-wide remote-dry bit. Starting also needs
  // presets and bounds for this unit, which only the known id ranges have.
  if (!dryingBounds(system.unitId)) return []
  return [
    {
      label: `${system.name}: ${t('drying.dry')}`,
      icon: PhSun,
      disabled: dryingBusy.value || Boolean(system.drying.blockedReasons?.length),
      onSelect: () => openDryingForm(system),
    },
  ]
}

const dryingMenuItems = computed<ActionMenuItem[]>(() =>
  capabilities?.amsDrying ? materialSystems.value.flatMap(dryingActionItems) : [],
)

function emergencyStop() {
  if (!activePrinter) return
  askConfirmation(
    { title: 'confirm.stopTitle', description: 'confirm.stopDescription', confirm: 'confirm.stopConfirm' },
    { name: activePrinter.name },
    () => runEmergencyStop(),
  )
}

async function runEmergencyStop() {
  if (!activePrinter) return
  try {
    await invoke('printer_emergency_stop', { name: activePrinter.name })
    showToast(t('control.emergencyStopSent'))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
  }
}

const speedProfileKeys: Record<typeof speedProfile.value, MessageKey> = {
  silent: 'control.speedSilent',
  standard: 'control.speedStandard',
  sport: 'control.speedSport',
  ludicrous: 'control.speedLudicrous',
}

async function setSpeedProfile(event: Event) {
  if (!activePrinter) return
  const value = (event.target as HTMLSelectElement).value as typeof speedProfile.value
  speedProfile.value = value
  if (speedBusy.value) return
  speedBusy.value = true
  try {
    await invoke('printer_speed_set', {
      request: { name: activePrinter.name, speedProfile: value },
    })
    showToast(t('control.speedSet', { profile: t(speedProfileKeys[value]) }))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    speedBusy.value = false
  }
}

async function downloadFile(file: FileEntry) {
  if (!activePrinter || downloadingPath.value) return
  const destination = await saveFileDialog({ defaultPath: file.name })
  if (!destination) return
  downloadingPath.value = file.devicePath
  const transferId = crypto.randomUUID()
  try {
    await invoke('printer_file_download', {
      request: { name: activePrinter.name, devicePath: file.devicePath, destination, transferId, totalBytes: file.sizeBytes },
    })
    showToast(t('filesView.downloaded', { name: file.name }))
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    downloadingPath.value = undefined
    downloadProgress.value = undefined
  }
}

function deletePrinterFile(file: FileEntry) {
  if (!activePrinter || !capabilities?.fileDelete || deletingPrinterPath.value) return
  askConfirmation(
    {
      title: 'filesView.printerDeleteTitle',
      description: 'filesView.printerDeleteDescription',
      confirm: 'filesView.printerDeleteConfirm',
    },
    { name: file.name },
    async () => {
      const printerName = activePrinter?.name
      if (!printerName) return
      deletingPrinterPath.value = file.devicePath
      try {
        await invoke('printer_file_delete', { request: { name: printerName, devicePath: file.devicePath } })
        showToast(t('filesView.printerDeleted', { name: file.name }))
        await loadFiles()
      } finally {
        deletingPrinterPath.value = undefined
      }
    },
  )
}

function formatTemperature(value: number) {
  return Math.round(value)
}

function textColorFor(color: string) {
  if (!color.startsWith('#')) return 'var(--color-white)'
  const r = parseInt(color.slice(1, 3), 16)
  const g = parseInt(color.slice(3, 5), 16)
  const b = parseInt(color.slice(5, 7), 16)
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255
  return luminance > 0.55 ? 'var(--color-gray-900)' : 'var(--color-white)'
}

function emptyFilamentOverlayFor(color: string) {
  return textColorFor(color) === 'var(--color-gray-900)'
    ? 'rgba(255, 255, 255, 0.72)'
    : 'rgba(0, 0, 0, 0.55)'
}

function wifiSignal(dbm: number | undefined) {
  if (dbm === undefined) return { icon: PhWifiSlash, label: t('control.signalNone') }
  if (dbm >= -55) return { icon: PhWifiHigh, label: t('control.signalExcellent') }
  if (dbm >= -70) return { icon: PhWifiMedium, label: t('control.signalGood') }
  return { icon: PhWifiLow, label: t('control.signalWeak') }
}

const printerSort = ref<FileSort>({ key: 'name', descending: false })
const printerFiles = computed(() => sortedBy(files.value.filter((file) => file.type === 'file'), printerSort.value))
const sortPrinterFiles = (key: SortKey) => { printerSort.value = nextSort(printerSort.value, key) }

const printerSortOrder = (key: SortKey) => printerSort.value.key !== key
  ? 'none'
  : printerSort.value.descending ? 'descending' : 'ascending'

watch([() => activePrinter?.name, () => capabilities], () => {
  fanDrafts.value = {}
  files.value = []
  filesError.value = undefined
  if (capabilities?.fileList) void loadFiles()
}, { immediate: true })

function plugChanged(plug: PlugSummary) {
  if (activePrinter && (plugsForPrinter(activePrinter.name, [plug]).length || plug.cameras.includes(controlCameraName.value))) {
    void controlCameraViewer.value?.refresh()
  }
}

onMounted(() => {
  void listen<TransferProgress>('transfer-progress', (event) => {
    if (event.payload.totalBytes) downloadProgress.value = Math.min(100, event.payload.bytesTransferred / event.payload.totalBytes * 100)
    if (event.payload.complete) downloadProgress.value = 100
  }).then((unlisten) => {
    transferUnlisten = unlisten
  })
})

onUnmounted(() => {
  transferUnlisten?.()
  Object.values(temperatureTimers).forEach((timer) => {
    if (timer !== undefined) window.clearTimeout(timer)
  })
})
</script>

<template>
  <template v-if="active && activePrinter">
    <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
      <div class="min-w-0 flex-1">
        <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ activePrinter.name }}</h2>
        <div class="mt-1 flex flex-col sm:mt-0 sm:flex-row sm:flex-wrap sm:space-x-6">
          <div class="mt-2 flex items-center font-mono text-sm text-gray-500 dark:text-gray-400">
            <PhPrinter class="mr-1.5 size-5 shrink-0 text-gray-500 dark:text-gray-400" aria-hidden="true" />
            {{ activePrinter.detectedModel || activePrinter.presence?.model || activePrinter.driver }}
          </div>
          <div class="mt-2 flex items-center font-mono text-sm text-gray-500 dark:text-gray-400">
            <PhNetwork class="mr-1.5 size-5 shrink-0 text-gray-500 dark:text-gray-400" aria-hidden="true" />
            {{ activePrinter.host }}
          </div>
        </div>
      </div>
      <div class="mt-5 flex lg:mt-0 lg:ml-4 items-center gap-2">
        <StatusBadge :status="activeBadge" :label="statusLabel(activeBadge)" />
        <span
          v-if="wifiDbm !== undefined"
          class="inline-flex items-center gap-x-1.5 rounded-md bg-gray-100 px-2 py-1 text-xs font-medium text-gray-600 dark:bg-gray-400/10 dark:text-gray-400"
          :title="wifiSignal(wifiDbm).label"
        >
          <component :is="wifiSignal(wifiDbm).icon" class="size-3.5" aria-hidden="true" />{{ wifiDbm }} dBm
        </span>
        <Button
          v-if="capabilities?.emergencyStop"
          variant="danger"
          :disabled="!activeHasStatus"
          @click="emergencyStop"
        ><PhStop class="size-4" aria-hidden="true" /> {{ t('dashboard.emergency') }}</Button>
      </div>
    </div>

    <!-- Power control stays reachable whatever the printer's connectivity. -->
    <div v-if="activePrinter && plugsForPrinter(activePrinter.name, plugs).length" class="mx-4 mb-5 space-y-3 rounded-lg border border-gray-200 px-4 py-3 sm:mx-0 dark:border-white/10">
      <PlugControl v-for="plug in plugsForPrinter(activePrinter.name, plugs)" :key="plug.id" :plug="plug" :observation="plugObservations[plug.id]" :activity="plugActivity" :refresh="refreshPlugs" :t="t" :locale="locale" @changed="plugChanged(plug)" />
    </div>

    <!-- Awaiting the first sample: neutral, no alarm language, no recovery
         CTA — nothing has failed yet. -->
    <div
      v-if="!activeHasStatus && awaitingFirstSample"
      class="mx-4 flex min-h-140 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15"
      role="status"
      aria-live="polite"
    >
      <PhBroadcast class="mx-auto size-12 animate-pulse text-gray-500 dark:text-gray-400" aria-hidden="true" />
      <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('control.awaitingTitle') }}</h3>
      <p class="mt-1 max-w-md text-sm text-gray-500 dark:text-gray-400">{{ t('control.awaitingDescription') }}</p>
    </div>

    <!-- True outage: Melt has no usable status and a polling error. -->
    <div
      v-else-if="!activeHasStatus"
      class="mx-4 flex min-h-140 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15"
    >
      <PhWarning class="mx-auto size-12 text-gray-500 dark:text-gray-400" aria-hidden="true" />
      <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('control.unavailableTitle') }}</h3>
      <p class="mt-1 max-w-md text-sm text-gray-500 dark:text-gray-400">{{ selectedMonitor?.error ? message(selectedMonitor.error) : t('control.unavailableDescription') }}</p>
      <div class="mt-6 flex flex-wrap justify-center gap-2">
        <Button v-if="selectedMonitor?.error?.code === 'keychainFailed'" variant="secondary" :disabled="keychainChecking" @click="checkKeychain"><PhKey class="size-4" :class="keychainChecking && 'animate-pulse'" aria-hidden="true" /> {{ t('control.checkKeychain') }}</Button>
        <Button v-if="selectedMonitor?.error?.code === 'keychainFailed' && activePrinter" variant="secondary" @click="openEdit(activePrinter)"><PhPencilSimple class="size-4" aria-hidden="true" /> {{ t('printersView.editPrinter') }}</Button>
        <Button variant="primary" :disabled="refreshing" @click="refreshMonitoring"><PhArrowsClockwise class="size-4" :class="refreshing && 'animate-spin'" aria-hidden="true" /> {{ t('control.refreshConnection') }}</Button>
      </div>
    </div>

    <div v-show="activeHasStatus">
      <div
        v-if="activeBadge === 'reconnecting'"
        class="mb-5 flex items-center justify-between gap-4 rounded-lg border border-amber-300/70 bg-amber-50 px-4 py-3 text-sm text-amber-900 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-200"
        role="status"
      >
        <span class="flex items-center gap-2"><PhArrowsClockwise class="size-4 shrink-0" aria-hidden="true" />{{ t('control.reconnecting') }}</span>
        <Button variant="secondary" :disabled="refreshing" @click="refreshMonitoring"><PhArrowsClockwise class="size-4" :class="refreshing && 'animate-spin'" aria-hidden="true" /> {{ t('control.refreshConnection') }}</Button>
      </div>
      <section v-if="statusFaults.length" class="mb-5 space-y-2" :aria-label="t('control.faults')">
        <div
          v-for="(fault, faultIndex) in statusFaults"
          :key="faultIndex"
          class="flex items-start gap-3 rounded-lg border px-4 py-3 text-sm"
          :class="{
            'border-red-300 bg-red-50 text-red-900 dark:border-red-400/20 dark:bg-red-400/10 dark:text-red-200': fault.severity === 'error',
            'border-amber-300/70 bg-amber-50 text-amber-900 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-200': fault.severity === 'warning',
            'border-sky-300/70 bg-sky-50 text-sky-900 dark:border-sky-400/20 dark:bg-sky-400/10 dark:text-sky-200': fault.severity === 'info',
          }"
          :role="fault.severity === 'error' ? 'alert' : 'status'"
        >
          <PhWarningCircle v-if="fault.severity === 'error'" class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <PhWarning v-else-if="fault.severity === 'warning'" class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <PhInfo v-else class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
          <div class="min-w-0 flex-1">
            <div class="flex flex-wrap items-start justify-between gap-x-3 gap-y-1">
              <p class="font-medium">{{ fault.message }}</p>
              <span class="rounded-full border border-current/20 px-2 py-0.5 text-xs font-semibold uppercase tracking-wide opacity-80">{{ t(faultSeverityKeys[fault.severity]) }}</span>
            </div>
            <div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
              <p class="font-mono opacity-75">
                {{ fault.rawCode ?? fault.code }}
                <span v-if="fault.recoverable === false"> · {{ t('control.faultUnrecoverable') }}</span>
              </p>
              <a
                v-if="fault.guideUrl"
                :href="fault.guideUrl"
                target="_blank"
                rel="noreferrer"
                class="inline-flex items-center gap-1 font-medium underline underline-offset-2 hover:no-underline focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-current"
              >
                {{ t('control.faultGuide') }}
                <PhArrowSquareOut class="size-3.5" aria-hidden="true" />
              </a>
            </div>
          </div>
        </div>
      </section>
      <section class="grid grid-cols-1 gap-5 xl:grid-cols-[minmax(0,2fr)_minmax(18rem,1fr)]">
        <CameraViewer
          ref="controlCameraViewer"
          :source="controlCameraSource"
          :active="cameraMayRun"
          :can-stream="Boolean(controlCameraName || capabilities?.cameraStream)"
          :can-snapshot="Boolean(controlCameraName || capabilities?.cameraSnapshot)"
          :t="t"
          @snapshot-saved="showToast(t('camera.snapshotSaved'))"
        >
          <template v-if="controlCameras.length" #controls>
            <select :value="controlCameraName" :aria-label="t('camerasView.feed')" class="rounded-md bg-white py-1.5 pl-2 text-xs font-medium text-gray-700 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-gray-200 dark:outline-white/10 dark:*:bg-gray-800" @change="selectControlFeed(($event.target as HTMLSelectElement).value)">
              <option value="">{{ t('camerasView.builtInFeed') }}</option>
              <option v-for="camera in controlCameras" :key="camera.id" :value="camera.name">{{ camera.name }}</option>
            </select>
          </template>
        </CameraViewer>

        <Card>
          <CardHeader :title="t('control.currentJob')" :icon="PhCheckSquareOffset">
            <span v-if="hasActiveJob" class="font-mono text-xs text-gray-500 dark:text-gray-400">{{ t('control.complete', { percent: progressPercent }) }}</span>
          </CardHeader>
          <div class="px-4 py-5 sm:p-6">
            <div class="flex items-start gap-3">
              <ModelThumbnail
                v-if="jobThumbnail"
                :path="jobThumbnail.devicePath"
                :size-bytes="jobThumbnail.sizeBytes"
                :modified-at="jobThumbnail.modifiedAt"
                :alt="jobThumbnail.name"
                :unavailable-label="t('filesView.previewUnavailable')"
                class="size-10 shrink-0 overflow-hidden rounded-lg"
              />
              <div v-else class="grid size-10 shrink-0 place-items-center rounded-lg bg-cyan-50 text-cyan-600 dark:bg-cyan-400/10 dark:text-cyan-400">
                <PhHexagon v-if="!selectedStatus?.job" class="size-5" aria-hidden="true" />
                <PhCube v-else class="size-5" aria-hidden="true" />
              </div>
              <div class="min-w-0">
                <p
                  class="line-clamp-2 wrap-anywhere font-mono text-sm font-semibold text-gray-900 dark:text-white"
                  :title="selectedStatus?.job?.name"
                  translate="no"
                >{{ selectedStatus?.job?.name ?? t('dashboard.noJob') }}</p>
                <p class="mt-1 font-mono text-xs text-gray-500 dark:text-gray-400">
                  {{ currentStageLabel ?? t(printerStateMessageKey(selectedStatus?.state)) }}
                  <span v-if="selectedStatus?.printMeta?.plateIndex !== undefined">
                    · {{ t(plateLabel(selectedStatus.printMeta.plateIndex, selectedStatus.printMeta.plateCount).key, plateLabel(selectedStatus.printMeta.plateIndex, selectedStatus.printMeta.plateCount).values) }}
                  </span>
                </p>
              </div>
            </div>
            <div v-if="hasActiveJob" class="mt-6">
              <div class="mb-2 flex justify-between text-xs text-gray-500 dark:text-gray-400">
                <span>{{ t('control.layer', { current: selectedStatus?.progress?.currentLayer ?? '—', total: selectedStatus?.progress?.totalLayers ?? '—' }) }}</span>
                <span v-if="remainingTimeLabel" class="font-mono text-sm font-semibold text-gray-900 dark:text-white">
                  {{ remainingTimeLabel }}
                </span>
              </div>
              <div
                class="overflow-hidden rounded-full bg-gray-200 dark:bg-white/10"
                role="progressbar"
                :aria-valuenow="preparingPercent ?? progressPercent"
                aria-valuemin="0"
                aria-valuemax="100"
                :aria-label="preparingPercent !== undefined ? t('control.preparing') : t('control.complete', { percent: progressPercent })"
                :aria-valuetext="progressValueText"
              >
                <div
                  class="h-2 rounded-full transition-[width]"
                  :class="preparingPercent !== undefined ? 'bg-amber-500 dark:bg-amber-400' : 'bg-cyan-600 dark:bg-cyan-500'"
                  :style="{ width: `${preparingPercent ?? progressPercent}%` }"
                ></div>
              </div>
              <p v-if="preparingPercent !== undefined" class="mt-2 text-xs text-amber-700 dark:text-amber-300">{{ t('control.preparing') }}</p>
            </div>
            <div v-if="hasActiveJob" class="mt-6 flex gap-2">
              <Button
                v-if="selectedStatus?.state === 'paused'"
                class="flex-1"
                :disabled="!capabilities?.jobResume"
                @click="sendJobAction('resume')"
              ><PhPlay class="size-4" aria-hidden="true" /> {{ t('dashboard.resume') }}</Button>
              <Button
                v-else
                class="flex-1"
                :disabled="!capabilities?.jobPause || selectedStatus?.state !== 'printing'"
                @click="sendJobAction('pause')"
              ><PhPause class="size-4" aria-hidden="true" /> {{ t('dashboard.pause') }}</Button>
              <Button
                variant="danger"
                :disabled="!capabilities?.jobCancel || !['printing', 'paused'].includes(selectedStatus?.state ?? '')"
                @click="sendJobAction('cancel')"
              ><PhStop class="size-4" aria-hidden="true" /> {{ t('common.cancel') }}</Button>
            </div>
            <div v-if="capabilities?.speedControl" class="mt-4">
              <label for="speed-profile" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('control.speedProfile') }}</label>
              <div class="mt-2 grid grid-cols-1">
                <select
                  id="speed-profile"
                  name="speed-profile"
                  :value="speedProfile"
                  :disabled="speedBusy || !['printing', 'paused'].includes(selectedStatus?.state ?? '')"
                  class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 disabled:opacity-50 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800"
                  @change="setSpeedProfile"
                >
                  <option value="silent">{{ t('control.speedSilent') }}</option>
                  <option value="standard">{{ t('control.speedStandard') }}</option>
                  <option value="sport">{{ t('control.speedSport') }}</option>
                  <option value="ludicrous">{{ t('control.speedLudicrous') }}</option>
                </select>
                <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
              </div>
            </div>
          </div>
        </Card>
      </section>

      <section class="mt-5 grid gap-5 lg:grid-cols-2 min-[1180px]:grid-cols-4">
        <Card v-if="temperatureRows.length">
          <CardHeader :title="t('dashboard.temperature')" :icon="PhThermometerSimple">
            <span v-if="capabilities?.temperatureWrite && selectedStatus?.state !== 'idle'" class="text-xs text-gray-500 dark:text-gray-400">{{ t('control.availableWhenIdle') }}</span>
          </CardHeader>
          <div class="space-y-4 px-4 py-5 sm:p-6">
            <div v-for="row in temperatureRows" :key="row.key" class="flex items-center justify-between gap-3">
              <div class="min-w-0">
                <p class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ temperatureLabel(row.key, row.value) }}</p>
                <p class="font-mono text-sm text-gray-500 dark:text-gray-400"><span class="font-bold">{{ row.value.currentCelsius === undefined ? '—' : formatTemperature(row.value.currentCelsius) }}</span> °C <span aria-hidden="true">/</span> <span class="font-bold">{{ row.value.targetCelsius === undefined ? '—' : formatTemperature(row.value.targetCelsius) }}</span> °C</p>
              </div>
              <div v-if="capabilities?.temperatureWrite && row.value.controllable" class="flex shrink-0 items-center gap-1">
                <IconButton variant="outline" class="!size-8 !min-h-0 !min-w-0" :disabled="selectedStatus?.state !== 'idle'" :aria-label="t('control.decreaseTemp', { sensor: temperatureLabel(row.key, row.value) })" @click="adjustTemperature(row.key, row.value, -5)"><PhMinus class="size-3.5" aria-hidden="true" /></IconButton>
                <input
                  type="number"
                  inputmode="numeric"
                  :name="`temperature-${row.key}`"
                  :value="row.value.targetCelsius === undefined ? '' : formatTemperature(row.value.targetCelsius)"
                  :min="row.value.minimumCelsius ?? 0"
                  :max="row.value.maximumCelsius ?? temperatureMaximums[row.value.kind]"
                  :disabled="selectedStatus?.state !== 'idle'"
                  :aria-label="t('control.setTemp', { sensor: temperatureLabel(row.key, row.value) })"
                  class="w-16 rounded-md bg-white py-1.5 text-center font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 disabled:opacity-50 dark:bg-white/5 dark:text-white dark:outline-white/10"
                  @change="setTemperature(row.key, row.value, $event)"
                />
                <IconButton variant="outline" class="!size-8 !min-h-0 !min-w-0" :disabled="selectedStatus?.state !== 'idle'" :aria-label="t('control.increaseTemp', { sensor: temperatureLabel(row.key, row.value) })" @click="adjustTemperature(row.key, row.value, 5)"><PhPlus class="size-3.5" aria-hidden="true" /></IconButton>
              </div>
              <p v-else-if="!row.value.controllable" class="shrink-0 text-right text-xs text-gray-500 dark:text-gray-400">{{ t('control.telemetryOnly') }}</p>
            </div>

            <!-- Material presets: set every controllable heater in one action. -->
            <div v-if="presetsAvailable" class="border-t border-gray-200 pt-4 dark:border-white/10">
              <p id="preset-group-label" class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('control.presets') }}</p>
              <div class="mt-2 flex flex-wrap gap-2" role="group" aria-labelledby="preset-group-label">
                <Button
                  v-for="preset in temperaturePresets"
                  :key="preset.id"
                  :disabled="selectedStatus?.state !== 'idle'"
                  :title="`${preset.nozzleCelsius} °C / ${preset.bedCelsius} °C`"
                  @click="applyPreset(preset)"
                >{{ preset.name }}</Button>
                <Button
                  :disabled="selectedStatus?.state !== 'idle'"
                  @click="applyPreset()"
                >{{ t('control.presetOff') }}</Button>
              </div>
            </div>
          </div>
        </Card>

        <Card v-if="fanRows.length">
          <CardHeader :title="t('control.fans')" :icon="PhFan" />
          <div class="space-y-4 px-4 py-5 sm:p-6">
            <div v-for="[key, value] in fanRows" :key="key" class="block">
              <span class="mb-2 flex justify-between">
                <span class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ fanLabel(key) }}</span>
                <span class="text-sm/6 text-gray-500 dark:text-gray-400">{{ fanDrafts[key] ?? value.speedPercent ?? '—' }}{{ (fanDrafts[key] ?? value.speedPercent) === undefined ? '' : '%' }}</span>
              </span>
              <input v-if="value.controllable && value.speedPercent !== undefined" :key="`${key}-${value.speedPercent}`" :value="value.speedPercent" :name="`fan-${key}`" class="h-1 w-full cursor-pointer accent-cyan-600 dark:accent-cyan-400" type="range" :min="value.minimumPercent ?? 0" :max="value.maximumPercent ?? 100" :aria-label="t('control.fanPower', { fan: fanLabel(key) })" @input="previewFan(key, $event)" @change="sendFan(key, $event)" />
              <p v-else class="text-xs text-gray-500 dark:text-gray-400">{{ t('control.telemetryOnly') }}</p>
            </div>
          </div>
        </Card>

        <Card v-if="lightRows.length">
          <CardHeader :title="t('control.lights')" :icon="PhSun" />
          <div class="space-y-4 px-4 py-5 sm:p-6">
            <div v-for="[key, value] in lightRows" :key="key" class="flex items-center justify-between gap-3">
              <label :for="`light-${key}`" class="grow cursor-pointer text-sm/6 font-medium text-gray-900 dark:text-white">{{ lightLabel(key) }}</label>
              <Switch
                v-if="capabilities?.lightControl && value.controllable"
                :id="`light-${key}`"
                :model-value="value.mode === 'on'"
                :label="lightLabel(key)"
                @update:model-value="toggleLight(key, $event)"
              />
              <span v-else class="text-xs text-gray-500 dark:text-gray-400">{{ t(lightModeMessageKey(value.mode)) }} · {{ t('control.telemetryOnly') }}</span>
            </div>
          </div>
        </Card>

        <Card v-if="capabilities?.motionControl">
          <CardHeader :title="t('dashboard.motion')" :icon="PhArrowsOutCardinal">
            <span v-if="selectedStatus?.state !== 'idle'" class="text-xs text-gray-500 dark:text-gray-400">{{ t('control.availableWhenIdle') }}</span>
          </CardHeader>
          <div class="px-4 py-5 sm:p-6">
            <div class="mb-4 flex min-h-27 items-center justify-center gap-8">
              <div class="grid grid-cols-3 grid-rows-3 gap-1">
                <IconButton variant="outline" class="col-start-2 row-start-1" :disabled="jogDisabled" :aria-label="t('control.jogAxis', { axis: 'Y+' })" @click="jog('y', 1)">Y+</IconButton>
                <IconButton variant="outline" class="col-start-1 row-start-2" :disabled="jogDisabled" :aria-label="t('control.jogAxis', { axis: 'X−' })" @click="jog('x', -1)">X−</IconButton>
                <IconButton variant="outline" class="col-start-2 row-start-2" :title="t('dashboard.home')" :aria-label="t('dashboard.home')" :disabled="selectedStatus?.state !== 'idle'" @click="homeAxes"><PhHouse class="size-4" aria-hidden="true" /></IconButton>
                <IconButton variant="outline" class="col-start-3 row-start-2" :disabled="jogDisabled" :aria-label="t('control.jogAxis', { axis: 'X+' })" @click="jog('x', 1)">X+</IconButton>
                <IconButton variant="outline" class="col-start-2 row-start-3" :disabled="jogDisabled" :aria-label="t('control.jogAxis', { axis: 'Y−' })" @click="jog('y', -1)">Y−</IconButton>
              </div>
              <div class="flex flex-col gap-1">
                <IconButton variant="outline" :disabled="jogDisabled" :aria-label="t('control.jogAxis', { axis: 'Z+' })" @click="jog('z', 1)">Z+</IconButton>
                <IconButton variant="outline" :disabled="jogDisabled" :aria-label="t('control.jogAxis', { axis: 'Z−' })" @click="jog('z', -1)">Z−</IconButton>
              </div>
            </div>
            <div class="grid grid-cols-2 gap-2">
              <div>
                <label for="step-size" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('control.stepSize') }}</label>
                <div class="mt-2 grid grid-cols-1">
                  <select id="step-size" name="step-size" v-model="stepSize" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 disabled:opacity-50 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
                    <option>0.1 mm</option>
                    <option>1 mm</option>
                    <option>10 mm</option>
                  </select>
                  <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
                </div>
              </div>
              <div>
                <label for="feed-rate" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('control.feedRate') }}</label>
                <div class="mt-2 grid grid-cols-1">
                  <select id="feed-rate" name="feed-rate" v-model="feedRate" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 disabled:opacity-50 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
                    <option :value="25">25 mm/s</option>
                    <option :value="50">50 mm/s</option>
                    <option :value="100">100 mm/s</option>
                  </select>
                  <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
                </div>
              </div>
            </div>
          </div>
        </Card>
      </section>

      <section class="mt-5 grid gap-5 xl:grid-cols-[1.35fr_1fr]">
        <!-- Table -->
        <Card v-if="capabilities?.fileList" class="overflow-hidden">
          <CardHeader :title="t('dashboard.files')" :icon="PhFolder">
            <IconButton :title="t('dashboard.loadFiles')" :aria-label="t('dashboard.loadFiles')" :disabled="filesLoading" @click="loadFiles"><PhArrowsClockwise class="size-4" :class="filesLoading && 'animate-spin'" aria-hidden="true" /></IconButton>
          </CardHeader>
          <div v-if="filesError && printerFiles.length" class="flex items-center justify-between gap-3 border-b border-amber-300/70 bg-amber-50 px-4 py-2 text-xs text-amber-900 sm:px-6 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-200" role="alert">
            <span class="min-w-0">{{ t('common.refreshFailed', { reason: filesError }) }}</span>
            <Button :disabled="filesLoading" @click="loadFiles">{{ t('common.retry') }}</Button>
          </div>
          <div
            class="max-h-80 overflow-auto focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:focus-visible:outline-cyan-400"
            role="region"
            :aria-label="t('dashboard.files')"
            tabindex="0"
          >
            <table class="relative w-full divide-y divide-gray-300 text-left dark:divide-white/15">
              <thead class="sticky top-0 z-10 bg-white dark:bg-gray-800">
                <tr>
                  <th scope="col" :aria-sort="printerSortOrder('name')" class="px-4 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase sm:px-6 dark:text-gray-400">
                    <button type="button" class="flex items-center gap-1 uppercase hover:text-gray-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:hover:text-gray-200 dark:focus-visible:outline-cyan-400" @click="sortPrinterFiles('name')">
                      {{ t('filesView.name') }}
                      <PhCaretDown v-if="printerSort.key === 'name'" class="size-3" :class="!printerSort.descending && 'rotate-180'" aria-hidden="true" />
                      <PhArrowsDownUp v-else class="size-3" aria-hidden="true" />
                    </button>
                  </th>
                  <th scope="col" class="w-20 px-2 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase dark:text-gray-400">{{ t('filesView.type') }}</th>
                  <th scope="col" :aria-sort="printerSortOrder('size')" class="hidden w-24 px-2 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase min-[1000px]:table-cell dark:text-gray-400">
                    <button type="button" class="flex items-center gap-1 uppercase hover:text-gray-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:hover:text-gray-200 dark:focus-visible:outline-cyan-400" @click="sortPrinterFiles('size')">
                      {{ t('filesView.size') }}
                      <PhCaretDown v-if="printerSort.key === 'size'" class="size-3" :class="!printerSort.descending && 'rotate-180'" aria-hidden="true" />
                      <PhArrowsDownUp v-else class="size-3" aria-hidden="true" />
                    </button>
                  </th>
                  <th scope="col" :aria-sort="printerSortOrder('modified')" class="hidden w-48 px-2 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase min-[1180px]:table-cell dark:text-gray-400">
                    <button type="button" class="flex items-center gap-1 uppercase hover:text-gray-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:hover:text-gray-200 dark:focus-visible:outline-cyan-400" @click="sortPrinterFiles('modified')">
                      {{ t('filesView.modified') }}
                      <PhCaretDown v-if="printerSort.key === 'modified'" class="size-3" :class="!printerSort.descending && 'rotate-180'" aria-hidden="true" />
                      <PhArrowsDownUp v-else class="size-3" aria-hidden="true" />
                    </button>
                  </th>
                  <th scope="col" class="w-24 px-4 py-3 text-right text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase sm:px-6 dark:text-gray-400">{{ t('filesView.actions') }}</th>
                </tr>
              </thead>
              <tbody class="divide-y divide-gray-200 dark:divide-white/10">
                <tr
                  v-for="file in printerFiles"
                  :key="file.devicePath"
                  class="hover:bg-gray-50 dark:hover:bg-white/5"
                >
                  <td class="max-w-0 px-4 py-4 font-mono text-sm font-medium text-gray-900 sm:px-6 dark:text-white" :title="file.name" translate="no"><span class="line-clamp-2 wrap-anywhere">{{ file.name }}</span></td>
                  <td class="px-2 py-4 text-sm whitespace-nowrap">
                    <span class="inline-flex items-center rounded-md bg-gray-100 px-2 py-1 text-xs font-medium text-gray-600 dark:bg-gray-400/10 dark:text-gray-400">{{ fileTypeLabel(file) }}</span>
                  </td>
                  <td class="hidden px-2 py-4 font-mono text-sm whitespace-nowrap text-gray-500 min-[1000px]:table-cell dark:text-gray-400">{{ formatSize(file.sizeBytes, locale) }}</td>
                  <td class="hidden px-2 py-4 font-mono text-sm whitespace-nowrap text-gray-500 min-[1180px]:table-cell dark:text-gray-400">{{ formatDate(file.modifiedAt, locale) }}</td>
                  <td class="px-4 py-4 text-sm whitespace-nowrap sm:px-6">
                    <div class="flex justify-end gap-1">
                      <IconButton
                        class="text-cyan-600 dark:text-cyan-400"
                        :title="t('filesView.printFile')"
                        :aria-label="t('filesView.printNamed', { name: file.name })"
                        :disabled="!capabilities?.jobStart || selectedStatus?.state !== 'idle' || printPreparing || printBusy"
                        @click.stop="requestPrint(file, activePrinter?.name)"
                      ><PhPlay class="size-4" aria-hidden="true" /></IconButton>
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
                        <PhDownloadSimple v-else class="size-4" aria-hidden="true" />
                      </IconButton>
                      <IconButton
                        v-if="capabilities?.fileDelete"
                        class="text-rose-600 dark:text-rose-400"
                        :title="t('filesView.printerDeleteNamed', { name: file.name })"
                        :aria-label="t('filesView.printerDeleteNamed', { name: file.name })"
                        :disabled="deletingPrinterPath === file.devicePath"
                        @click.stop="deletePrinterFile(file)"
                      >
                        <PhTrash class="size-4" aria-hidden="true" />
                      </IconButton>
                    </div>
                  </td>
                </tr>
                <tr v-if="!printerFiles.length">
                  <td colspan="5" class="px-4 py-4 text-sm text-gray-500 sm:px-6 dark:text-gray-400">{{ filesLoading ? t('common.loading') : filesError ?? t('dashboard.noFiles') }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </Card>

        <Card v-if="materialSystems.length">
          <CardHeader :title="t('materials.title')" :icon="PhStack">
            <ActionMenu v-if="dryingMenuItems.length" :label="t('drying.actions')" :items="dryingMenuItems" />
          </CardHeader>
          <div class="divide-y divide-gray-200 dark:divide-white/10">
            <div v-for="system in materialSystems" :key="system.name" class="px-4 py-5 sm:px-6">
              <div class="mb-3 flex flex-wrap items-center justify-between gap-2">
                <span class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ system.name }}</span>
                <div v-if="system.temperature !== undefined || system.humidity !== undefined" class="flex flex-wrap items-center justify-end gap-x-3 gap-y-1 text-xs text-gray-500 dark:text-gray-400">
                  <span v-if="system.humidity !== undefined" class="flex items-center gap-1"><PhDrop class="size-4" aria-hidden="true" /><span class="sr-only">{{ t('materials.humidity') }} </span><span class="font-mono">{{ system.humidity }}</span></span>
                  <span v-if="system.temperature !== undefined" class="flex items-center gap-1"><PhThermometerSimple class="size-4" aria-hidden="true" /><span class="sr-only">{{ t('materials.temperature') }} </span><span class="font-mono">{{ formatTemperature(system.temperature) }} °C</span></span>
                  <span v-if="system.drying?.active" class="flex items-center gap-1 text-orange-600 dark:text-orange-400">
                    <PhSun class="size-4" aria-hidden="true" />
                    <span>{{ system.drying.minutesRemaining ? t('drying.remaining', { time: formatDuration(system.drying.minutesRemaining * 60) }) : t(dryingStatusKey(system.drying.status)) }}</span>
                    <span v-if="system.drying.setting && dryingTargetShown(system.temperature, system.drying.setting.temperatureC)">· {{ t('drying.target', { temperature: system.drying.setting.temperatureC }) }}</span>
                  </span>
                  <span v-else-if="system.drying && dryingFault(system.drying)" class="flex items-center gap-1 text-red-600 dark:text-red-400">
                    <PhWarning class="size-4" aria-hidden="true" />
                    <span>{{ t(dryingStatusKey(system.drying.status)) }}</span>
                  </span>
                </div>
              </div>
              <div class="flex flex-wrap gap-3">
                <div
                  v-for="material in system.slots"
                  :key="`${system.name}-${material.slot}`"
                  class="relative size-18 shrink-0 overflow-hidden rounded-md border border-black/10 dark:border-white/15"
                  :style="{ backgroundColor: material.color, color: textColorFor(material.color) }"
                  :title="material.colorLabel"
                >
                  <span
                    class="absolute inset-x-0 top-0 transition-[height] duration-200"
                    :style="{ height: `${100 - material.remainingPercent}%`, backgroundColor: emptyFilamentOverlayFor(material.color) }"
                  ></span>
                  <span class="relative z-10 flex h-full flex-col items-center justify-center gap-1 p-1 text-center font-mono">
                    <span class="text-xs font-bold">{{ material.slot }}</span>
                    <span v-if="material.type !== '—'" class="text-[10px] font-semibold tracking-wide uppercase wrap-anywhere">{{ material.type }}</span>
                    <span class="sr-only">{{ material.colorLabel }}</span>
                    <span v-if="material.remainingGrams !== undefined" class="text-[10px] font-medium">{{ material.remainingGrams }} g</span>
                  </span>
                </div>
              </div>
              <div
                v-if="capabilities?.amsDrying && system.drying?.controllable && !system.drying.active && system.drying.blockedReasons?.length"
                class="mt-3 text-xs text-gray-500 dark:text-gray-400"
              >{{ t(dryingBlockedKey(system.drying.blockedReasons[0])) }}</div>
            </div>
          </div>
        </Card>
      </section>
    </div>
  </template>

  <SlideOver
    :open="dryingForm !== undefined"
    :title="t('drying.dry')"
    :description="dryingFormSystem?.name"
    :close-label="t('common.closePanel')"
    @close="!dryingBusy && (dryingForm = undefined)"
  >
    <div v-if="dryingForm" class="space-y-4">
      <label class="block text-sm font-medium text-gray-700 dark:text-gray-200">
        {{ t('drying.temperature') }}
        <div class="mt-1 flex items-center gap-2">
          <input
            v-model.number="dryingForm.temperatureC"
            type="number"
            inputmode="numeric"
            :min="dryingBounds(dryingForm.unitId)?.min"
            :max="dryingBounds(dryingForm.unitId)?.max"
            class="block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10"
          />
          <span class="shrink-0 text-gray-500 dark:text-gray-400">°C</span>
        </div>
      </label>
      <label class="block text-sm font-medium text-gray-700 dark:text-gray-200">
        {{ t('drying.hours') }}
        <div class="mt-1 flex items-center gap-2">
          <input
            v-model.number="dryingForm.hours"
            type="number"
            inputmode="numeric"
            min="1"
            max="24"
            class="block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10"
          />
          <span class="shrink-0 text-gray-500 dark:text-gray-400">h</span>
        </div>
      </label>
    </div>
    <template #footer>
      <Button :disabled="dryingBusy" @click="dryingForm = undefined">{{ t('common.cancel') }}</Button>
      <Button v-if="dryingFormSystem" variant="primary" :disabled="dryingBusy || !dryingFormValid" @click="setDrying(dryingFormSystem, true)">{{ t('drying.start') }}</Button>
    </template>
  </SlideOver>
</template>
