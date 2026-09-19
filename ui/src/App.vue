<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch, watchEffect, type Component } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open as openFileDialog, save as saveFileDialog } from '@tauri-apps/plugin-dialog'
import { locales, preferredLocale, translate, type Locale, type MessageKey } from './i18n'
import { monitorBadge, type ConnectionState, type PrinterBadge } from './monitoring'
import { clampTarget, formatDuration, remainingTimePresentation } from './formatting'
import { dryingBounds, dryingDefaults, dryingFault, dryingStoppable, type AmsDrying, type DryingStatus } from './drying'
import { printTargetState } from './printing'
import { printStageLabel } from './stages'
import { awaitsFirstSample, badgeDotClasses, cameraViewState, filamentColor, filamentFillPercent, isActiveJobState, materialSystemLabel, printerStateMessageKey, serialNumberDisplay, shouldRunCamera } from './presentation'
import { commandDetail, commandMessage, type CommandError } from './errors'
import { hmsGuideUrl, hmsSeverity, type HmsSeverity } from './hms'
import { startH264Playback, supportsH264WebCodecs, type H264Playback } from './camera-stream'
import { printerDraftsMatch, validateSlicerDraft, type PrinterDraftFields } from './forms'
import {
  hasAvailableUpdate,
  hasRequiredUpdate,
  updateEntryFor,
  versionRail,
  type FirmwareUpdateComponentKind,
  type FirmwareUpdateEntry,
} from './firmware-updates'
import { naturallyDescending, nextSort, shouldDeferLibraryCards, sortedBy, type FileSort, type SortKey } from './library'
import {
  PRESET_NAME_MAX_LENGTH,
  defaultPresets,
  parseStoredPresets,
  presetValidationFailed,
  temperatureMaximums,
  validatePresetDraft,
  type TemperaturePreset,
} from './temperatures'
import { Listbox, ListboxButton, ListboxOption, ListboxOptions } from '@headlessui/vue'
import StatusBadge from './components/StatusBadge.vue'
import ActionMenu, { type ActionMenuItem } from './components/ActionMenu.vue'
import SlideOver from './components/SlideOver.vue'
import ConfirmDialog from './components/ConfirmDialog.vue'
import Button from './components/Button.vue'
import IconButton from './components/IconButton.vue'
import Switch from './components/Switch.vue'
import Card from './components/Card.vue'
import CardHeader from './components/CardHeader.vue'
import ModelThumbnail from './components/ModelThumbnail.vue'
import FilePreviewDialog from './components/FilePreviewDialog.vue'
import {
  PhArrowsClockwise,
  PhArrowsOutCardinal,
  PhArrowLeft,
  PhArrowSquareOut,
  PhArrowUp,
  PhBroadcast,
  PhCamera,
  PhCards,
  PhCaretDown,
  PhCaretRight,
  PhCheckCircle,
  PhCheckSquareOffset,
  PhCornersOut,
  PhCube,
  PhDesktop,
  PhDownloadSimple,
  PhDrop,
  PhEye,
  PhEyeSlash,
  PhFan,
  PhFile,
  PhFolder,
  PhFolderOpen,
  PhBellRinging,
  PhGearSix,
  PhHexagon,
  PhHouse,
  PhInfo,
  PhKey,
  PhMagnifyingGlass,
  PhMagnifyingGlassPlus,
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
  PhUploadSimple,
  PhVideoCamera,
  PhWarning,
  PhWarningCircle,
  PhWifiHigh,
  PhWifiLow,
  PhWifiMedium,
  PhWifiSlash,
  PhX,
} from '@phosphor-icons/vue'

type View = 'control' | 'printers' | 'settings' | 'files'
type ModelTone = 'cyan' | 'amber' | 'rose' | 'violet'

type AppInfo = {
  version: string
  modes: [string, string]
  license: string
  sourceUrl: string
}

type Printer = {
  name: string
  driver: string
  host: string
  serial: string
  timeout: string
  insecure: boolean
  presence?: {
    lastSeenUnixMs: number
    model: string
    firmware?: string
    suggestedHost?: string
  }
}

type Driver = {
  name: string
  description: string
}

type Capabilities = {
  status: boolean
  temperatureRead: boolean
  lightControl: boolean
  speedControl: boolean
  discovery: boolean
  cameraStream: boolean
  cameraSnapshot: boolean
  fileList: boolean
  fileDownload: boolean
  fileUpload: boolean
  fileDelete: boolean
  jobStart: boolean
  jobPause: boolean
  jobResume: boolean
  jobCancel: boolean
  emergencyStop: boolean
  temperatureWrite: boolean
  motionControl: boolean
  fanControl: boolean
  tlsRefresh: boolean
  firmwareUpdateCheck: boolean
  amsDrying?: boolean
}

type DiscoveredPrinter = {
  driver: string
  host: string
  serial: string
  model: string
  name: string
}

type Temperature = {
  currentCelsius: number
  targetCelsius?: number
}

type TemperatureControl = {
  kind: 'nozzle' | 'bed' | 'chamber'
  position?: 'left' | 'right'
  currentCelsius?: number
  targetCelsius?: number
  minimumCelsius?: number
  maximumCelsius?: number
  controllable: boolean
}

type FanControl = {
  kind: 'parts' | 'auxiliary' | 'hotend' | 'exhaust' | 'chamber' | 'mainboard' | 'heat' | 'unknown'
  speedPercent?: number
  mode: 'manual' | 'auto' | 'off'
  minimumPercent?: number
  maximumPercent?: number
  controllable: boolean
}

type LightControl = {
  kind: 'lamp' | 'work' | 'unknown'
  mode: 'on' | 'off' | 'flashing' | 'unknown'
  controllable: boolean
}

type AmsTrayStatus = {
  slot: number
  trayIndex?: number
  trayInfoId?: string
  settingId?: string
  tagUid?: string
  filamentType?: string
  color?: string
  remainingPercent?: number
  remainingGrams?: number
  nominalWeightGrams?: number
  diameterMm?: number
}

type AmsUnitStatus = {
  id: number
  kind?: string
  humidityRange?: string
  humidityLevel?: string
  temperature?: number
  trays: AmsTrayStatus[]
  drying?: AmsDrying
}

type PrinterStatus = {
  state: 'idle' | 'printing' | 'paused' | 'error' | 'unknown'
  temperatures?: { nozzle?: Temperature; bed?: Temperature; chamber?: Temperature }
  job?: { name: string }
  printMeta?: { fileName: string; fileSize?: number; plateIndex?: number; plateCount?: number; bedType?: string }
  progress?: { percent: number; preparationPercent?: number; currentLayer?: number; totalLayers?: number }
  timeEstimates?: { elapsedSeconds: number; remainingSeconds?: number; totalSeconds?: number }
  stage?: string
  stageCode?: number
  errors: { code: string; message: string; rawCode?: string; recoverable?: boolean }[]
  warnings: { code: string; message: string }[]
  fans?: Record<string, number>
  wifi?: { signalDbm: number }
  lights?: Record<string, string>
  controls?: {
    temperatures?: Record<string, TemperatureControl>
    fans?: Record<string, FanControl>
    lights?: Record<string, LightControl>
  }
  extensions?: { 'bambu-lan'?: { ams?: { units: AmsUnitStatus[] } } }
}

type StatusFault = {
  code: string
  message: string
  rawCode?: string
  recoverable?: boolean
  severity: HmsSeverity
  guideUrl?: string
}

type MonitorEntry = {
  name: string
  driver: string
  status?: PrinterStatus
  error?: CommandError
  stale: boolean
  connectionState: ConnectionState
  observedAt?: string
}

type KeychainProbe = {
  available: boolean
  reason?: 'accessCodeMissing' | 'keychainUnavailable'
}

type NotificationEvent = { kind: 'completion' | 'failure' | 'disconnection'; printer: string }
type TransferProgress = { transferId: string; bytesTransferred: number; totalBytes?: number; complete: boolean }
type PrintStageEvent = { stage: string; percent?: number; bytesTransferred?: number; detail?: string }

type FileEntry = {
  name: string
  root: string
  path: string
  devicePath: string
  type: 'file' | 'directory'
  mediaType: 'model' | 'timelapse' | 'video' | 'other'
  sizeBytes?: number
  modifiedAt?: string
}

type PrintPlate = {
  index: number
  name?: string
  gcodePath: string
  thumbnailPaths: string[]
  estimatedSeconds?: number
  estimatedGrams?: number
}

type PrintPackage = {
  sourceFileName: string
  sliced: boolean
  plates: PrintPlate[]
  issues: { severity: 'warning' | 'error'; code: string; message: string }[]
}

type FileList = {
  entries: FileEntry[]
}

type LibraryBreadcrumb = {
  name: string
  path: string
}

type LibraryListResponse = {
  path: string
  parent?: string
  breadcrumbs: LibraryBreadcrumb[]
  entries: FileEntry[]
}

type Diagnostics = {
  version: string
  platform: string
  configuredProfiles: number
  drivers: Record<string, number>
  identifiersRedacted: boolean
  monitorWorkers: number
  monitorIntervalSeconds: number
  protocolTracesIncluded: boolean
  bambu: BambuCompatibilityDiagnostic[]
}

type BambuCompatibilityDiagnostic = {
  printer: string
  modelRaw: string
  canonicalModel: string
  modelFamily: string
  firmwareModules: { name: string; software: string; hardware?: string }[]
  authorization: { effective: string; conflict: boolean }
  camera: { preferred: string; source: string; rejectedAdvertisement?: string; owner?: { transport: string; subscribers: number; reconnectAttempts: number } }
  storageTransport: string
  quirks: { id: string; qualification: string; active: boolean }[]
  tlsPinned: boolean
}

type CameraSnapshot = {
  dataUrl: string
}

type CameraStream = {
  url: string
}

type CameraH264Stream = CameraStream & {
  codec: string
}

type CameraWebRtcAnswer = {
  type: 'answer'
  sdp: string
}

type CameraTransport = 'webrtc' | 'webcodecs' | 'mjpeg'

interface MaterialSlot {
  slot: string
  type: string
  color: string
  remainingPercent: number
  remainingGrams?: number
}

interface MaterialSystemView {
  name: string
  unitId: number
  temperature?: number
  humidity?: string
  slots: MaterialSlot[]
  drying?: AmsDrying
  firstFilament?: string
}

type NotificationSetting = {
  id: 'completion' | 'failure' | 'disconnection'
  label: MessageKey
  description: MessageKey
  enabled: boolean
}

type SlicerSetting = {
  name: string
  path: string
  enabled: boolean
}

type BackendPreferences = {
  notifications: Record<NotificationSetting['id'], boolean>
  slicers: SlicerSetting[]
}

const modelTones: Record<ModelTone, { preview: string; shape: string }> = {
  cyan: {
    preview: 'bg-linear-135 from-cyan-100 to-slate-200 dark:from-cyan-950 dark:to-slate-900',
    shape: 'h-20 w-25 bg-cyan-400/75 shadow-[18px_18px_0_var(--color-cyan-800)]',
  },
  amber: {
    preview: 'bg-linear-135 from-amber-100 to-slate-200 dark:from-amber-950 dark:to-slate-900',
    shape: 'h-20 w-25 rounded-l-[40%] rounded-r-[12%] bg-amber-400/75 shadow-[18px_18px_0_var(--color-amber-800)]',
  },
  rose: {
    preview: 'bg-linear-135 from-rose-100 to-slate-200 dark:from-rose-950 dark:to-slate-900',
    shape: 'h-25 w-17.5 rounded-t-[50%] rounded-b-[12%] bg-rose-400/75 shadow-[15px_17px_0_var(--color-rose-800)]',
  },
  violet: {
    preview: 'bg-linear-135 from-violet-100 to-slate-200 dark:from-violet-950 dark:to-slate-900',
    shape: 'h-16.5 w-27.5 rounded-lg bg-violet-400/75 shadow-[17px_17px_0_var(--color-violet-800)]',
  },
}
// Tone follows the backend's media classification, so the color on a library
// card carries meaning. Hashing the filename produced a stable but arbitrary
// color that users could easily read as a category.
const mediaTones: Record<FileEntry['mediaType'], ModelTone> = {
  model: 'cyan',
  timelapse: 'violet',
  video: 'amber',
  other: 'rose',
}

const info = ref<AppInfo>()
const printers = ref<Printer[]>([])
const drivers = ref<Driver[]>([])
const monitoring = ref<MonitorEntry[]>([])
const firmwareUpdates = ref<FirmwareUpdateEntry[]>([])
const firmwareRefreshing = ref(false)
const firmwareError = ref<string>()
const capabilities = ref<Capabilities>()
const files = ref<FileEntry[]>([])
const loading = ref(true)
const refreshing = ref(false)
const keychainChecking = ref(false)
const filesLoading = ref(false)
const filesError = ref<string>()
const deletingPrinterPath = ref<string>()
const libraryFiles = ref<FileEntry[]>([])
const libraryFilesLoading = ref(false)
const libraryFilesError = ref<string>()
const libraryPath = ref<string>()
const libraryParent = ref<string>()
const libraryBreadcrumbs = ref<LibraryBreadcrumb[]>([])
const printTarget = ref<FileEntry>()
const printBusy = ref(false)
const currentPrintTargetState = computed(() => printTargetState(printers.value.length, printBusy.value))
const printStage = ref<PrintStageEvent>()
const printPackage = ref<PrintPackage>()
const printPlate = ref<number>()
const printDisplayName = ref('')
const loadError = ref<string>()
const profileError = ref<string>()
const adding = ref(false)
const additionOpen = ref(false)
const additionError = ref<string>()
const discovered = ref<DiscoveredPrinter[]>([])
const discovering = ref(false)
const discoveryError = ref<string>()
const tlsOpen = ref(false)
const tlsRefreshing = ref(false)
const tlsError = ref<string>()
const tlsFingerprint = ref<string>()
const tlsPrinter = ref<string>()
const diagnostics = ref<Diagnostics>()
const diagnosticsError = ref<string>()
const diagnosticsLoading = ref(false)
const pendingConfirm = ref<{ title: string; description: string; confirm: string; run: () => Promise<void> }>()
const confirming = ref(false)
const cameraUrl = ref<string>()
const cameraPeer = shallowRef<RTCPeerConnection>()
const cameraMediaStream = shallowRef<MediaStream>()
const cameraVideo = ref<HTMLVideoElement>()
const cameraCanvas = ref<HTMLCanvasElement>()
const cameraCanvasActive = ref(false)
const cameraCanvasLive = ref(false)
const cameraTransport = ref<CameraTransport>()
const cameraTransportDetail = ref<string>()
const cameraLoading = ref(false)
const cameraError = ref<string>()
const documentVisible = ref(document.visibilityState === 'visible')
let monitorUnlisten: UnlistenFn | undefined
let firmwareUpdatesUnlisten: UnlistenFn | undefined
let presenceUnlisten: UnlistenFn | undefined
let notificationUnlisten: UnlistenFn | undefined
let transferUnlisten: UnlistenFn | undefined
let printStageUnlisten: UnlistenFn | undefined
let libraryChangeTimer: number | undefined
let libraryChangeUnlisten: UnlistenFn | undefined
let selectionRequest = 0
let cameraRequest = 0
let cameraH264Playback: H264Playback | undefined
let fileRequest = 0
let libraryRequest = 0
let toastTimer: number | undefined

// Restored across launches like theme and locale: a desktop app that stays
// open for hours should reopen where it was left. A stored view that is no
// longer valid falls back to the control view.
const views: View[] = ['control', 'printers', 'settings', 'files']
const activeView = ref<View>(views.find((view) => view === localStorage.getItem('activeView')) ?? 'control')
const sectionTabs = computed<{ view: View; label: string; icon: Component }[]>(() => [
  { view: 'printers', label: t('nav.printers'), icon: PhPrinter },
  { view: 'settings', label: t('nav.settings'), icon: PhGearSix },
  { view: 'files', label: t('nav.files'), icon: PhFolder },
])
// Below this the tab strip reads better than a dropdown; above it the strip
// starts scrolling and the active printer can end up off-screen.
const PRINTER_TAB_LIMIT = 5
const usePrinterMenu = computed(() => printers.value.length > PRINTER_TAB_LIMIT)
const activePrinterId = ref(localStorage.getItem('activePrinterId') ?? '')
const firmwarePrinterId = ref<string>()
const revealedSerials = ref(new Set<string>())
const theme = ref<'light' | 'dark' | 'system'>(
  (['light', 'dark', 'system'] as const).find((value) => value === localStorage.getItem('theme')) ?? 'system',
)
const systemPrefersDark = ref(true)
let systemThemeQuery: MediaQueryList | undefined
function handleSystemThemeChange(event: MediaQueryListEvent) {
  systemPrefersDark.value = event.matches
}
function handleDocumentVisibility() {
  documentVisible.value = document.visibilityState === 'visible'
}
const isLightTheme = computed(() => theme.value === 'light' || (theme.value === 'system' && !systemPrefersDark.value))
watchEffect(() => {
  document.documentElement.classList.toggle('dark', !isLightTheme.value)
  document.querySelector('meta[name="theme-color"]')?.setAttribute('content', isLightTheme.value ? '#f9fafb' : '#111827')
})
watchEffect(() => {
  localStorage.setItem('theme', theme.value)
})
watchEffect(() => {
  localStorage.setItem('activeView', activeView.value)
})
watchEffect(() => {
  localStorage.setItem('activePrinterId', activePrinterId.value)
})
const locale = ref<Locale>(locales.find((value) => value === localStorage.getItem('locale')) ?? preferredLocale())
watchEffect(() => {
  localStorage.setItem('locale', locale.value)
})
function t(key: MessageKey, values?: Record<string, string | number>) {
  return translate(locale.value, key, values)
}
const cameraStage = ref<HTMLElement>()
const errorToast = ref<string>()
const successToast = ref<string>()
const toasts = computed(() => [
  ...(errorToast.value ? [{ key: 'error' as const, text: errorToast.value, tone: 'error' as const }] : []),
  ...(successToast.value ? [{ key: 'success' as const, text: successToast.value, tone: 'success' as const }] : []),
])
const searchTerm = ref('')
const sortKey = ref<SortKey>('name')
const stepSize = ref('1 mm')
const feedRate = ref(50)
const jogBusy = ref(false)
const speedProfile = ref<'silent' | 'standard' | 'sport' | 'ludicrous'>('standard')
const speedBusy = ref(false)
const uploadBusy = ref(false)
const downloadingPath = ref<string>()
const downloadProgress = ref<number>()
const defaultNotifications: NotificationSetting[] = [
  { id: 'completion', label: 'settingsView.notifyComplete', description: 'settingsView.notifyCompleteDescription', enabled: true },
  { id: 'failure', label: 'settingsView.notifyFailure', description: 'settingsView.notifyFailureDescription', enabled: true },
  { id: 'disconnection', label: 'settingsView.notifyDisconnected', description: 'settingsView.notifyDisconnectedDescription', enabled: true },
]
// Backend preferences are the source of truth; seeding platform-specific
// paths here only flashed wrong data on Windows and macOS before they load.
const defaultSlicers: SlicerSetting[] = []
const notifications = ref<NotificationSetting[]>(defaultNotifications)
const slicers = ref<SlicerSetting[]>(defaultSlicers)
const slicerDraft = ref({ name: '', path: '' })
const slicerBusy = ref(false)
const slicerError = ref<string>()
const slicerNameError = ref<string>()
const slicerPathError = ref<string>()
const slicerNameInput = ref<HTMLInputElement>()
const slicerPathInput = ref<HTMLInputElement>()
const draft = ref<PrinterDraftFields>({
  name: '',
  driver: '',
  host: '',
  serial: '',
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
}

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
  additionBaseline.value = { ...draft.value }
  resetAdditionPanelState()
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

const activePrinter = computed(() => printers.value.find((printer) => printer.name === activePrinterId.value) ?? printers.value[0])
const firmwarePrinter = computed(() => printers.value.find((printer) => printer.name === firmwarePrinterId.value))
const compactNavValue = computed({
  get: () => {
    if (activeView.value !== 'control') return `view:${activeView.value}`
    return activePrinter.value ? `printer:${activePrinter.value.name}` : 'view:printers'
  },
  set: navigateCompact,
})
const hasPrinters = computed(() => printers.value.length > 0)
const selectedMonitor = computed(() => monitoring.value.find((entry) => entry.name === activePrinter.value?.name))
const selectedStatus = computed(() => selectedMonitor.value?.status)
const selectedFirmwareUpdate = computed(() => updateEntryFor(firmwareUpdates.value, firmwarePrinter.value?.name))

const firmwareFor = (name: string) => updateEntryFor(firmwareUpdates.value, name)
const firmwareComponentLabel = (kind: FirmwareUpdateComponentKind) => t(
  kind === 'printerFirmware'
    ? 'firmware.printerFirmware'
    : kind === 'accessoryFirmware'
      ? 'firmware.accessoryFirmware'
      : 'firmware.printerSoftware',
)
const firmwareAvailabilityLabel = (entry: FirmwareUpdateEntry) => t(
  entry.report.availability === 'available'
    ? 'firmware.available'
    : entry.report.availability === 'current'
      ? 'firmware.current'
      : entry.report.availability === 'unsupported'
        ? 'firmware.unsupported'
        : 'firmware.unknown',
)

function badgeFor(name: string): PrinterBadge {
  const entry = monitoring.value.find((candidate) => candidate.name === name)
  return monitorBadge(entry)
}

const activeBadge = computed<PrinterBadge>(() => (activePrinter.value ? badgeFor(activePrinter.value.name) : 'offline'))
const isReachable = (badge: PrinterBadge) => badge === 'idle' || badge === 'busy'
const activeHasStatus = computed(() => isReachable(activeBadge.value) || activeBadge.value === 'reconnecting' || activeBadge.value === 'error')
const awaitingFirstSample = computed(() => awaitsFirstSample(activeBadge.value))
const badgeMessageKeys: Record<PrinterBadge, MessageKey> = {
  idle: 'status.onlineIdle',
  busy: 'status.onlineBusy',
  error: 'status.errorLabel',
  connecting: 'status.connecting',
  synchronizing: 'status.synchronizing',
  reconnecting: 'status.reconnecting',
  offline: 'status.offlineLabel',
  unknown: 'status.unknownLabel',
}
const statusLabel = (badge: PrinterBadge) => t(badgeMessageKeys[badge])
const hasActiveJob = computed(() => isActiveJobState(selectedStatus.value?.state))
const currentStageLabel = computed(() => {
  if (!hasActiveJob.value) return undefined
  return printStageLabel(locale.value, selectedStatus.value?.stage, selectedStatus.value?.stageCode)
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
  return libraryFiles.value.find((file) => file.type === 'file' && file.name === name)
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
    return value ? [{ key, value: { kind: key, ...value, controllable: Boolean(capabilities.value?.temperatureWrite) } }] : []
  })
})
const fanRows = computed(() => {
  const controls = Object.entries(selectedStatus.value?.controls?.fans ?? {})
  if (controls.length) return controls
  return Object.entries(selectedStatus.value?.fans ?? {}).map(([key, speed]) => [key, {
    kind: 'unknown', mode: 'manual', speedPercent: speed, controllable: Boolean(capabilities.value?.fanControl),
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
    kind: key === 'chamber_light' ? 'lamp' : 'unknown', mode, controllable: Boolean(capabilities.value?.lightControl),
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
  const driver = activePrinter.value?.driver ?? ''
  const materialUnitCount = units.filter((unit) => unit.id < 254).length
  return units.map((unit) => ({
    name: materialSystemLabel(driver, unit.id, t('materials.externalSpool'), materialUnitCount),
    unitId: unit.id,
    temperature: unit.temperature,
    humidity: unit.humidityLevel,
    drying: unit.drying,
    firstFilament: unit.trays.find((tray) => tray.filamentType)?.filamentType,
    slots: unit.trays.map((tray) => ({
      slot: unit.id >= 254 ? t('materials.slotExternal') : String(tray.slot + 1),
      type: tray.filamentType ?? '—',
      color: filamentColor(tray.color),
      remainingPercent: filamentFillPercent(tray.remainingPercent),
      remainingGrams: tray.remainingGrams,
    })),
  }))
})
const cameraHasMedia = computed(() => Boolean(cameraPeer.value || cameraCanvasLive.value || cameraUrl.value))
const cameraState = computed(() => cameraViewState({
  loading: cameraLoading.value,
  hasPeer: Boolean(cameraPeer.value),
  hasMedia: Boolean(cameraCanvasLive.value || cameraUrl.value),
  hasStreamTransport: cameraTransport.value !== undefined,
}))
const cameraStateLabel = computed(() => t({
  live: 'camera.live',
  snapshot: 'camera.snapshotLabel',
  loading: 'camera.loading',
  offline: 'status.offlineLabel',
}[cameraState.value] as MessageKey))
const cameraStateClasses = computed(() => ({
  live: 'bg-green-100 fill-green-500 text-green-700 dark:bg-green-400/10 dark:fill-green-400 dark:text-green-400',
  snapshot: 'bg-gray-100 fill-gray-400 text-gray-600 dark:bg-white/10 dark:fill-gray-500 dark:text-gray-400',
  loading: 'bg-yellow-100 fill-yellow-500 text-yellow-800 dark:bg-yellow-400/10 dark:fill-yellow-400 dark:text-yellow-500',
  offline: 'bg-gray-100 fill-gray-400 text-gray-600 dark:bg-white/10 dark:fill-gray-500 dark:text-gray-400',
}[cameraState.value]))
const cameraSupported = computed(() => Boolean(capabilities.value?.cameraStream || capabilities.value?.cameraSnapshot))
const cameraMayRun = computed(() => shouldRunCamera(activeView.value, documentVisible.value, cameraSupported.value))

watch(cameraMayRun, (shouldRun) => {
  if (shouldRun) {
    if (!cameraLoading.value && !cameraHasMedia.value) void refreshCamera()
  } else if (cameraLoading.value || cameraHasMedia.value || cameraTransport.value) {
    void stopCamera()
  }
})

watchEffect(() => {
  if (cameraVideo.value) cameraVideo.value.srcObject = cameraMediaStream.value ?? null
})

const fleetStats = computed(() => [
  { label: t('printersView.totalPrinters'), value: printers.value.length, tone: 'text-gray-900 dark:text-white' },
  { label: t('printersView.online'), value: printers.value.filter((printer) => isReachable(badgeFor(printer.name))).length, tone: 'text-green-600 dark:text-green-400' },
  { label: t('printersView.printingNow'), value: printers.value.filter((printer) => badgeFor(printer.name) === 'busy').length, tone: 'text-yellow-600 dark:text-yellow-400' },
])

// Commands reject with stable codes; unexpected runtime failures stay in logs
// instead of leaking implementation details into user-facing surfaces.
function message(reason: unknown) {
  return commandMessage(reason, t)
}

function cameraFailureDetail(reason: unknown) {
  return commandDetail(reason) ?? (reason instanceof Error ? reason.message : message(reason))
}

function askConfirmation(
  keys: { title: MessageKey; description: MessageKey; confirm: MessageKey },
  values: Record<string, string>,
  run: () => Promise<void>,
) {
  pendingConfirm.value = {
    title: t(keys.title, values),
    description: t(keys.description, values),
    confirm: t(keys.confirm),
    run,
  }
}

async function runConfirmation() {
  const pending = pendingConfirm.value
  if (!pending || confirming.value) return
  confirming.value = true
  try {
    await pending.run()
    pendingConfirm.value = undefined
  } catch (reason) {
    // Every confirmed action funnels through here, so one catch covers the
    // handlers that surface their own errors and the ones that don't. Without
    // it a rejecting `run` leaves the dialog open with no feedback at all.
    pendingConfirm.value = undefined
    showToast(message(reason), 'error')
  } finally {
    confirming.value = false
  }
}

// Errors persist until dismissed: 2.2s is not long enough to read a failure,
// and a silently vanishing error is indistinguishable from no error at all.
//
// Errors and successes hold separate slots and stack together, because
// `printer-notification` fires from the monitor loop at arbitrary times — a
// single slot let a background completion quietly delete an unread failure.
function showToast(text: string, tone: 'success' | 'error' = 'success') {
  if (tone === 'error') {
    errorToast.value = text
    return
  }
  successToast.value = text
  if (toastTimer !== undefined) window.clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => {
    successToast.value = undefined
    toastTimer = undefined
  }, 2200)
}

function dismissToast(key: 'error' | 'success') {
  if (key === 'error') errorToast.value = undefined
  else successToast.value = undefined
}

function applyPreferences(preferences: BackendPreferences) {
  notifications.value = defaultNotifications.map((notification) => ({
    ...notification,
    enabled: preferences.notifications[notification.id],
  }))
  slicers.value = preferences.slicers
}

async function loadPreferences() {
  try {
    applyPreferences(await invoke<BackendPreferences>('get_preferences'))
  } catch (reason) {
    showToast(message(reason), 'error')
  }
}

async function updateNotification(id: NotificationSetting['id'], enabled: boolean) {
  const previous = notifications.value.map((notification) => ({ ...notification }))
  const next = notifications.value.map((notification) => (notification.id === id ? { ...notification, enabled } : notification))
  notifications.value = next
  try {
    const saved = await invoke<BackendPreferences['notifications']>('update_notification_preferences', {
      request: Object.fromEntries(next.map((notification) => [notification.id, notification.enabled])),
    })
    notifications.value = notifications.value.map((notification) => ({ ...notification, enabled: saved[notification.id] }))
  } catch (reason) {
    notifications.value = previous
    showToast(message(reason), 'error')
  }
}

async function updateSlicerEnabled(name: string, enabled: boolean) {
  const previous = slicers.value
  slicers.value = slicers.value.map((slicer) => (slicer.name === name ? { ...slicer, enabled } : slicer))
  try {
    slicers.value = await invoke<SlicerSetting[]>('set_slicer_enabled', { request: { name, enabled } })
  } catch (reason) {
    slicers.value = previous
    showToast(message(reason), 'error')
  }
}

async function browseSlicerPath() {
  const selected = await openFileDialog({ multiple: false })
  if (!selected || Array.isArray(selected)) return
  slicerDraft.value.path = selected
  slicerPathError.value = undefined
}

async function addSlicer() {
  if (slicerBusy.value) return
  const validation = validateSlicerDraft(slicerDraft.value)
  slicerNameError.value = validation.nameRequired ? t('settingsView.slicerNameRequired') : undefined
  slicerPathError.value = validation.pathRequired ? t('settingsView.slicerPathRequired') : undefined
  if (validation.nameRequired || validation.pathRequired) {
    await nextTick()
    ;(validation.nameRequired ? slicerNameInput.value : slicerPathInput.value)?.focus()
    return
  }
  slicerBusy.value = true
  slicerError.value = undefined
  try {
    slicers.value = await invoke<SlicerSetting[]>('save_slicer', {
      request: { name: slicerDraft.value.name.trim(), path: slicerDraft.value.path.trim(), enabled: true },
    })
    slicerDraft.value = { name: '', path: '' }
    slicerNameError.value = undefined
    slicerPathError.value = undefined
  } catch (reason) {
    slicerError.value = message(reason)
  } finally {
    slicerBusy.value = false
  }
}

function removeSlicer(name: string) {
  askConfirmation(
    { title: 'settingsView.slicerRemoveTitle', description: 'settingsView.slicerRemoveDescription', confirm: 'settingsView.slicerRemoveConfirm' },
    { name },
    () => confirmRemoveSlicer(name),
  )
}

async function confirmRemoveSlicer(name: string) {
  try {
    slicers.value = await invoke<SlicerSetting[]>('remove_slicer', { name })
  } catch (reason) {
    showToast(message(reason), 'error')
  }
}

async function load() {
  loading.value = true
  loadError.value = undefined
  profileError.value = undefined
  void loadPreferences()

  const [appInfo, profiles, availableDrivers] = await Promise.allSettled([
    invoke<AppInfo>('app_info'),
    invoke<Printer[]>('configured_printers'),
    invoke<Driver[]>('registered_drivers'),
  ])

  if (appInfo.status === 'fulfilled') info.value = appInfo.value
  else loadError.value = message(appInfo.reason)

  if (profiles.status === 'fulfilled') {
    printers.value = profiles.value
    const retained = profiles.value.find((printer) => printer.name === activePrinterId.value)
    const printer = retained ?? profiles.value[0]
    // Skip the blocking status refresh here: the background monitor worker
    // (started with the app) polls all printers and emits `monitoring-updated`
    // on its own, so we don't need to await the same slow probe twice.
    if (printer) await selectPrinter(printer.name, false, false)
    else clearSelection()
  } else profileError.value = message(profiles.reason)

  if (availableDrivers.status === 'fulfilled') drivers.value = availableDrivers.value
  else loadError.value ??= message(availableDrivers.reason)

  loading.value = false
}

function clearSelection() {
  void stopCamera()
  activePrinterId.value = ''
  capabilities.value = undefined
  monitoring.value = []
  files.value = []
  cameraUrl.value = undefined
  cameraError.value = undefined
  firmwareError.value = undefined
}

async function loadFirmwareUpdates(name: string, refresh = false) {
  if (firmwareRefreshing.value && refresh) return
  if (refresh) firmwareRefreshing.value = true
  firmwareError.value = undefined
  try {
    const entry = await invoke<FirmwareUpdateEntry>('printer_firmware_updates', { name, refresh })
    const index = firmwareUpdates.value.findIndex((candidate) => candidate.profile === entry.profile)
    if (index === -1) firmwareUpdates.value.push(entry)
    else firmwareUpdates.value.splice(index, 1, entry)
  } catch (reason) {
    firmwareError.value = message(reason)
    if (refresh) showToast(firmwareError.value ?? t('firmware.checkFailed'), 'error')
  } finally {
    if (refresh) firmwareRefreshing.value = false
  }
}

// Refreshes only the active printer (every caller acts on it specifically),
// not a full re-probe of every configured printer. Other printers' badges
// stay fed by the periodic `monitoring-updated` broadcast.
async function refreshMonitoring() {
  if (refreshing.value || !activePrinterId.value) return
  refreshing.value = true
  try {
    const entry = await invoke<MonitorEntry>('printer_status', { name: activePrinterId.value })
    const index = monitoring.value.findIndex((candidate) => candidate.name === entry.name)
    if (index === -1) monitoring.value.push(entry)
    else monitoring.value.splice(index, 1, entry)
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    refreshing.value = false
  }
}

async function checkKeychain() {
  if (keychainChecking.value || !activePrinter.value) return
  keychainChecking.value = true
  try {
    const probe = await invoke<KeychainProbe>('probe_keychain', { name: activePrinter.value.name })
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

// focus defaults to refresh: user-initiated selections jump to the control
// view, background re-selections (load, removal fallback) keep the current view.
async function selectPrinter(name: string, refresh = true, focus = refresh) {
  const request = ++selectionRequest
  activePrinterId.value = name
  if (focus) {
    firmwarePrinterId.value = undefined
    activeView.value = 'control'
  }
  fanDrafts.value = {}
  capabilities.value = undefined
  files.value = []
  filesError.value = undefined
  cameraUrl.value = undefined
  cameraError.value = undefined
  await stopCamera()
  if (request !== selectionRequest || activePrinterId.value !== name) return
  try {
    const result = await invoke<{ capabilities: Capabilities }>('printer_capabilities', { name })
    if (request !== selectionRequest || activePrinterId.value !== name) return
    capabilities.value = result.capabilities
    if (result.capabilities.fileList) void loadFiles()
    if (result.capabilities.firmwareUpdateCheck) void loadFirmwareUpdates(name)
  } catch (reason) {
    showToast(message(reason), 'error')
  }
  if (refresh) await refreshMonitoring()
}

function goTo(view: View) {
  firmwarePrinterId.value = undefined
  activeView.value = view
  if (view === 'files') void loadLibraryFiles(libraryPath.value)
}

function openFirmwareManagement(printer: Printer) {
  firmwarePrinterId.value = printer.name
  activeView.value = 'printers'
  void loadFirmwareUpdates(printer.name)
}

function closeFirmwareManagement() {
  firmwarePrinterId.value = undefined
}

function navigateCompact(value: string) {
  const separator = value.indexOf(':')
  const kind = value.slice(0, separator)
  const destination = value.slice(separator + 1)
  if (kind === 'printer') void selectPrinter(destination)
  else if (kind === 'view') goTo(destination as View)
}

// The Control tab's file table always shows the printer's storage root; it
// has no breadcrumb navigation, unlike the local file library below.
async function loadFiles() {
  if (!activePrinter.value || !capabilities.value?.fileList) return
  const request = ++fileRequest
  const printerName = activePrinter.value.name
  filesLoading.value = true
  filesError.value = undefined
  try {
    const result = await invoke<FileList>('printer_files', {
      request: { name: printerName, path: '/', search: '' },
    })
    if (request === fileRequest && activePrinter.value?.name === printerName) files.value = result.entries
  } catch (reason) {
    filesError.value = message(reason)
  } finally {
    filesLoading.value = false
  }
}

// The file library lives on the local machine, independent of any printer,
// so this never touches printer capabilities or connectivity.
// `path` is an absolute filesystem path; omitted, the backend resolves the
// last-browsed folder (persisted in preferences) or a sensible default.
// This lists the whole folder once; search filters that snapshot locally,
// and the backend's folder watch triggers a re-list when files change.
async function loadLibraryFiles(path?: string) {
  const request = ++libraryRequest
  libraryFilesLoading.value = true
  libraryFilesError.value = undefined
  try {
    const result = await invoke<LibraryListResponse>('library_files', {
      request: { path: path ?? '' },
    })
    if (request !== libraryRequest) return
    libraryFiles.value = result.entries
    libraryParent.value = result.parent
    libraryBreadcrumbs.value = result.breadcrumbs
    if (result.path !== libraryPath.value) {
      libraryPath.value = result.path
      // Best-effort: losing the last-visited folder across restarts isn't
      // worth surfacing an error for.
      void invoke('set_library_path', { path: result.path }).catch(() => {})
    }
  } catch (reason) {
    libraryFilesError.value = message(reason)
  } finally {
    libraryFilesLoading.value = false
  }
}

function navigateToDirectory(path: string) {
  void loadLibraryFiles(path)
}

async function chooseLibraryFolder() {
  const selected = await openFileDialog({ directory: true })
  if (!selected || Array.isArray(selected)) return
  await loadLibraryFiles(selected)
}

// A file being copied in fires a burst of change events, so they settle
// into one re-list. Changes while another view is open are picked up by
// goTo('files') instead.
function handleLibraryChange(path: string) {
  if (libraryChangeTimer !== undefined) window.clearTimeout(libraryChangeTimer)
  libraryChangeTimer = window.setTimeout(() => {
    if (activeView.value === 'files' && path === libraryPath.value) void loadLibraryFiles(path)
  }, 300)
}

function openAddition() {
  draft.value = {
    name: '',
    driver: drivers.value.find((driver) => driver.name === 'moonraker')?.name ?? drivers.value[0]?.name ?? '',
    host: '',
    serial: '',
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
  if (!draft.value.name) draft.value.name = printer.name || printer.model
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
}

// Secondary printer actions live behind the overflow menu: three labelled
// buttons could not fit a 280px card, which is reachable whenever the window
// sits below the xl breakpoint.
function printerActionItems(printer: Printer): ActionMenuItem[] {
  return [
    { label: t('printersView.editPrinter'), icon: PhPencilSimple, onSelect: () => openEdit(printer) },
    {
      label: t('printersView.refreshCertificate'),
      icon: PhArrowsClockwise,
      disabled: tlsRefreshing.value,
      onSelect: () => void openTlsRefresh(printer.name),
    },
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
    printers.value = printers.value.filter((printer) => printer.name !== name)
    monitoring.value = monitoring.value.filter((printer) => printer.name !== name)
    showToast(t('printersView.removed'))
    const next = printers.value[0]
    if (next) await selectPrinter(next.name, false)
    else {
      clearSelection()
      activeView.value = 'printers'
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

const jobToastKeys: Record<'pause' | 'resume' | 'cancel', MessageKey> = {
  pause: 'control.jobPaused',
  resume: 'control.jobResumed',
  cancel: 'control.cancelRequested',
}

function sendJobAction(action: 'start' | 'pause' | 'resume' | 'cancel', devicePath?: string) {
  if (!activePrinter.value) return
  if (action !== 'cancel') return void runJobAction(action, devicePath)
  askConfirmation(
    { title: 'confirm.cancelTitle', description: 'confirm.cancelDescription', confirm: 'confirm.cancelConfirm' },
    { name: activePrinter.value.name },
    () => runJobAction(action, devicePath),
  )
}

async function runJobAction(action: 'start' | 'pause' | 'resume' | 'cancel', devicePath?: string) {
  if (!activePrinter.value) return
  try {
    await invoke('printer_job_action', {
      request: { name: activePrinter.value.name, action, devicePath, confirmed: true },
    })
    showToast(
      action === 'start'
        ? t('filesView.sentToPrinter', { name: devicePath?.split('/').pop() ?? '' })
        : t(jobToastKeys[action]),
    )
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
    const printerName = activePrinter.value?.name
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
  if (!activePrinter.value || !selectedStatus.value) return
  const base = pendingTemperatures[key] ?? control.targetCelsius ?? control.currentCelsius
  if (base === undefined) return
  queueTemperature(key, control, base + delta)
}

// Typed entry, so reaching 250 °C doesn't cost fifty clicks on the steppers.
// `queueTemperature` still clamps against the control's advertised limits, so
// an out-of-range keystroke can never reach the printer.
function setTemperature(key: string, control: TemperatureControl, event: Event) {
  if (!activePrinter.value || !selectedStatus.value) return
  const raw = (event.target as HTMLInputElement).value
  if (raw.trim() === '') return
  const next = Number(raw)
  if (!Number.isFinite(next)) return
  queueTemperature(key, control, Math.round(next))
}

const temperaturePresets = ref<TemperaturePreset[]>(parseStoredPresets(localStorage.getItem('temperaturePresets')))
watchEffect(() => {
  localStorage.setItem('temperaturePresets', JSON.stringify(temperaturePresets.value))
})

// A preset only touches the kinds it knows about; chamber targets and any
// uncontrollable sensor are left exactly as they were.
const presetTargetRows = computed(() =>
  temperatureRows.value.filter((row) => row.value.controllable && (row.value.kind === 'nozzle' || row.value.kind === 'bed')),
)
const presetsAvailable = computed(() =>
  Boolean(capabilities.value?.temperatureWrite) && presetTargetRows.value.length > 0,
)

function applyPreset(preset?: TemperaturePreset) {
  if (!activePrinter.value || selectedStatus.value?.state !== 'idle') return
  for (const row of presetTargetRows.value) {
    // No preset means "cool down": every controllable heater goes to zero.
    const target = preset ? (row.value.kind === 'nozzle' ? preset.nozzleCelsius : preset.bedCelsius) : 0
    queueTemperature(row.key, row.value, target)
  }
  showToast(preset ? t('control.presetApplied', { name: preset.name }) : t('control.presetCooling'))
}

const presetDraft = ref({ name: '', nozzleCelsius: '', bedCelsius: '' })
const presetErrors = ref<{ name?: string; nozzle?: string; bed?: string }>({})
const presetNameInput = ref<HTMLInputElement>()

function addPreset() {
  const validation = validatePresetDraft(presetDraft.value)
  presetErrors.value = {
    name: validation.nameRequired
      ? t('settingsView.presetNameRequired')
      : validation.nameTooLong
        ? t('settingsView.presetNameTooLong', { max: PRESET_NAME_MAX_LENGTH })
        : undefined,
    nozzle: validation.nozzleOutOfRange ? t('settingsView.presetNozzleInvalid', { max: temperatureMaximums.nozzle }) : undefined,
    bed: validation.bedOutOfRange ? t('settingsView.presetBedInvalid', { max: temperatureMaximums.bed }) : undefined,
  }
  if (presetValidationFailed(validation)) {
    void nextTick(() => presetNameInput.value?.focus())
    return
  }
  temperaturePresets.value = [
    ...temperaturePresets.value,
    {
      id: crypto.randomUUID(),
      name: presetDraft.value.name.trim(),
      nozzleCelsius: Number(presetDraft.value.nozzleCelsius),
      bedCelsius: Number(presetDraft.value.bedCelsius),
    },
  ]
  presetDraft.value = { name: '', nozzleCelsius: '', bedCelsius: '' }
  presetErrors.value = {}
}

function removePreset(preset: TemperaturePreset) {
  askConfirmation(
    { title: 'settingsView.presetRemoveTitle', description: 'settingsView.presetRemoveDescription', confirm: 'settingsView.presetRemoveConfirm' },
    { name: preset.name },
    async () => {
      temperaturePresets.value = temperaturePresets.value.filter((entry) => entry.id !== preset.id)
    },
  )
}

function restoreDefaultPresets() {
  temperaturePresets.value = [...defaultPresets]
}

async function sendFan(fan: string, event: Event) {
  delete fanDrafts.value[fan]
  if (!activePrinter.value) return
  const speed = Number((event.target as HTMLInputElement).value)
  if (!Number.isInteger(speed)) return
  try {
    await invoke('printer_fan_set', {
      request: { name: activePrinter.value.name, fan, speedPercent: speed },
    })
    showToast(t('control.fanSet', { fan: fanLabel(fan), percent: speed }))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
    void refreshMonitoring()
  }
}

async function homeAxes() {
  if (!activePrinter.value) return
  try {
    await invoke('printer_motion_home', { name: activePrinter.value.name })
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
  if (!activePrinter.value || jogDisabled.value) return
  jogBusy.value = true
  const distance = jogDistance() * direction
  try {
    await invoke('printer_motion_jog', {
      request: {
        name: activePrinter.value.name,
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
  if (!activePrinter.value) return
  try {
    await invoke('printer_light_set', {
      request: { name: activePrinter.value.name, light, on },
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

function openDryingForm(system: MaterialSystemView) {
  const defaults = dryingDefaults(system.unitId, system.firstFilament)
  if (defaults) dryingForm.value = { unitId: system.unitId, ...defaults }
}

function setDrying(system: MaterialSystemView, start: boolean) {
  if (!activePrinter.value) return
  const name = activePrinter.value.name
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
  )
}

function emergencyStop() {
  if (!activePrinter.value) return
  askConfirmation(
    { title: 'confirm.stopTitle', description: 'confirm.stopDescription', confirm: 'confirm.stopConfirm' },
    { name: activePrinter.value.name },
    () => runEmergencyStop(),
  )
}

async function runEmergencyStop() {
  if (!activePrinter.value) return
  try {
    await invoke('printer_emergency_stop', { name: activePrinter.value.name })
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
  if (!activePrinter.value) return
  const value = (event.target as HTMLSelectElement).value as typeof speedProfile.value
  speedProfile.value = value
  if (speedBusy.value) return
  speedBusy.value = true
  try {
    await invoke('printer_speed_set', {
      request: { name: activePrinter.value.name, speedProfile: value },
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
  if (!activePrinter.value || downloadingPath.value) return
  const destination = await saveFileDialog({ defaultPath: file.name })
  if (!destination) return
  downloadingPath.value = file.devicePath
  const transferId = crypto.randomUUID()
  try {
    await invoke('printer_file_download', {
      request: { name: activePrinter.value.name, devicePath: file.devicePath, destination, transferId, totalBytes: file.sizeBytes },
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
  if (!activePrinter.value || !capabilities.value?.fileDelete || deletingPrinterPath.value) return
  askConfirmation(
    {
      title: 'filesView.printerDeleteTitle',
      description: 'filesView.printerDeleteDescription',
      confirm: 'filesView.printerDeleteConfirm',
    },
    { name: file.name },
    async () => {
      const printerName = activePrinter.value?.name
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

async function uploadFile() {
  if (uploadBusy.value || !libraryPath.value) return
  const source = await openFileDialog({ multiple: false })
  if (!source || Array.isArray(source)) return
  uploadBusy.value = true
  try {
    const result = await invoke<LibraryListResponse>('library_add_file', {
      request: { source, directory: libraryPath.value },
    })
    libraryFiles.value = result.entries
    showToast(t('filesView.uploadedTo', { directory: result.path }))
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    uploadBusy.value = false
  }
}

async function refreshCamera() {
  if (!activePrinter.value || !cameraMayRun.value) return
  const printerName = activePrinter.value.name
  await stopCamera()
  if (!cameraMayRun.value || activePrinter.value?.name !== printerName) return
  const request = ++cameraRequest
  cameraLoading.value = true
  cameraError.value = undefined
  cameraTransportDetail.value = undefined
  try {
    if (capabilities.value?.cameraStream) {
      let peer: RTCPeerConnection | undefined
      try {
        const PeerConnection = window.RTCPeerConnection
        if (!PeerConnection) throw new Error('WebRTC is unavailable in the embedded webview')
        peer = new PeerConnection({ iceServers: [] })
        peer.addTransceiver('video', { direction: 'recvonly' })
        const track = waitForVideoTrack(peer)
        const offer = await peer.createOffer()
        await peer.setLocalDescription(offer)
        await waitForIceGathering(peer)
        const answer = await invoke<CameraWebRtcAnswer>('printer_camera_webrtc_offer', {
          name: printerName,
          offer: peer.localDescription?.sdp ?? offer.sdp,
        })
        await peer.setRemoteDescription(answer)
        const mediaStream = await waitForPeerReady(peer, track)
        if (request !== cameraRequest || activePrinter.value?.name !== printerName) {
          peer.close()
          return
        }
        peer.onconnectionstatechange = () => {
          if (peer?.connectionState === 'failed' && cameraPeer.value === peer) {
            void fallbackCamera(printerName, request, 'WebRTC connection failed')
          }
        }
        cameraMediaStream.value = mediaStream
        cameraPeer.value = peer
        cameraTransport.value = 'webrtc'
      } catch (reason) {
        peer?.close()
        if (request === cameraRequest) {
          await fallbackCamera(printerName, request, cameraFailureDetail(reason))
        }
      }
    } else if (capabilities.value?.cameraSnapshot) {
      const snapshot = await invoke<CameraSnapshot>('printer_camera_snapshot', { name: printerName })
      if (request === cameraRequest) cameraUrl.value = snapshot.dataUrl
    }
  } catch (reason) {
    if (request === cameraRequest) {
      cameraUrl.value = undefined
      cameraError.value = message(reason)
    }
  } finally {
    if (request === cameraRequest) cameraLoading.value = false
  }
}

function waitForVideoTrack(peer: RTCPeerConnection) {
  return new Promise<MediaStream>((resolve) => {
    peer.ontrack = (event) => resolve(event.streams[0] ?? new MediaStream([event.track]))
  })
}

async function waitForPeerReady(peer: RTCPeerConnection, track: Promise<MediaStream>) {
  const connection = new Promise<void>((resolve, reject) => {
    const finish = () => {
      peer.removeEventListener('connectionstatechange', onStateChange)
      if (peer.connectionState === 'connected') resolve()
      else reject(new Error(`WebRTC connection ${peer.connectionState}`))
    }
    const onStateChange = () => {
      if (peer.connectionState === 'connected' || peer.connectionState === 'failed' || peer.connectionState === 'closed') finish()
    }
    if (peer.connectionState === 'connected') resolve()
    else peer.addEventListener('connectionstatechange', onStateChange)
  })
  return await withTimeout(Promise.all([connection, track]).then(([, stream]) => stream), 10_000, 'WebRTC media timed out')
}

async function fallbackCamera(printerName: string, request: number, reason: string) {
  if (request !== cameraRequest || activePrinter.value?.name !== printerName) return
  cameraTransportDetail.value = reason
  cameraPeer.value?.close()
  cameraPeer.value = undefined
  cameraMediaStream.value?.getTracks().forEach((track) => track.stop())
  cameraMediaStream.value = undefined
  await stopCameraSession()
  try {
    const stream = await invoke<CameraH264Stream>('printer_camera_h264_stream', { name: printerName })
    if (request !== cameraRequest || activePrinter.value?.name !== printerName) {
      await stopCameraSession()
      return
    }
    if (!await supportsH264WebCodecs(stream.codec)) {
      throw new Error(`WebCodecs does not support ${stream.codec}`)
    }
    if (request !== cameraRequest || activePrinter.value?.name !== printerName) return
    cameraCanvasActive.value = true
    cameraCanvasLive.value = false
    cameraTransport.value = 'webcodecs'
    await nextTick()
    const canvas = cameraCanvas.value
    if (!canvas) throw new Error('Camera canvas is unavailable')
    const playback = startH264Playback(canvas, stream.url, stream.codec)
    cameraH264Playback = playback
    let ready = false
    void playback.done.catch((playbackError) => {
      if (!ready || cameraH264Playback !== playback || request !== cameraRequest) return
      cameraLoading.value = true
      void fallbackMjpegCamera(
        printerName,
        request,
        `${reason}; WebCodecs playback failed: ${cameraFailureDetail(playbackError)}`,
      ).catch((fallbackError) => {
        if (request === cameraRequest) cameraError.value = message(fallbackError)
      }).finally(() => {
        if (request === cameraRequest) cameraLoading.value = false
      })
    })
    await withTimeout(playback.firstFrame, 10_000, 'WebCodecs media timed out')
    if (request !== cameraRequest || activePrinter.value?.name !== printerName) {
      playback.stop()
      return
    }
    ready = true
    cameraCanvasLive.value = true
    return
  } catch (webCodecsError) {
    if (request !== cameraRequest || activePrinter.value?.name !== printerName) return
    await fallbackMjpegCamera(
      printerName,
      request,
      `${reason}; WebCodecs: ${cameraFailureDetail(webCodecsError)}`,
    )
  }
}

async function fallbackMjpegCamera(printerName: string, request: number, reason: string) {
  cameraTransportDetail.value = reason
  cameraH264Playback?.stop()
  cameraH264Playback = undefined
  cameraCanvasActive.value = false
  cameraCanvasLive.value = false
  const stream = await invoke<CameraStream>('printer_camera_stream', { name: printerName })
  if (request !== cameraRequest || activePrinter.value?.name !== printerName) {
    await stopCameraSession()
    return
  }
  cameraUrl.value = stream.url
  cameraTransport.value = 'mjpeg'
}

async function withTimeout<T>(promise: Promise<T>, timeout: number, reason: string) {
  let timer: number | undefined
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => {
        timer = window.setTimeout(() => reject(new Error(reason)), timeout)
      }),
    ])
  } finally {
    if (timer !== undefined) window.clearTimeout(timer)
  }
}

async function waitForIceGathering(peer: RTCPeerConnection) {
  if (peer.iceGatheringState === 'complete') return
  await new Promise<void>((resolve) => {
    const timer = window.setTimeout(resolve, 3000)
    const onStateChange = () => {
      if (peer.iceGatheringState !== 'complete') return
      window.clearTimeout(timer)
      peer.removeEventListener('icegatheringstatechange', onStateChange)
      resolve()
    }
    peer.addEventListener('icegatheringstatechange', onStateChange)
  })
}

async function stopCamera() {
  cameraRequest += 1
  cameraPeer.value?.close()
  cameraPeer.value = undefined
  cameraMediaStream.value?.getTracks().forEach((track) => track.stop())
  cameraMediaStream.value = undefined
  cameraH264Playback?.stop()
  cameraH264Playback = undefined
  cameraCanvasActive.value = false
  cameraCanvasLive.value = false
  cameraUrl.value = undefined
  cameraTransport.value = undefined
  cameraLoading.value = false
  await stopCameraSession()
}

async function stopCameraSession() {
  await Promise.allSettled([
    invoke('printer_camera_webrtc_stop'),
    invoke('printer_camera_stream_stop'),
  ])
}

async function saveSnapshot() {
  if (!activePrinter.value || cameraLoading.value || !capabilities.value?.cameraSnapshot) return
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')
  const destination = await saveFileDialog({
    defaultPath: `${activePrinter.value.name.replace(/[^\w.-]+/g, '-')}-${stamp}.jpg`,
    filters: [{ name: 'JPEG', extensions: ['jpg', 'jpeg'] }],
  })
  if (!destination) return
  cameraLoading.value = true
  cameraError.value = undefined
  try {
    cameraUrl.value = (await invoke<CameraSnapshot>('printer_camera_snapshot', { name: activePrinter.value.name, destination })).dataUrl
    showToast(t('camera.snapshotSaved'))
  } catch (reason) {
    cameraError.value = message(reason)
  } finally {
    cameraLoading.value = false
  }
}

function toggleCameraFullscreen() {
  if (document.fullscreenElement) document.exitFullscreen()
  else cameraStage.value?.requestFullscreen()
}

async function runDiagnostics() {
  diagnosticsLoading.value = true
  diagnostics.value = undefined
  diagnosticsError.value = undefined
  try {
    diagnostics.value = await invoke<Diagnostics>('diagnostics_report')
  } catch (reason) {
    diagnosticsError.value = message(reason)
  } finally {
    diagnosticsLoading.value = false
  }
}

function formatTemperature(value: number) {
  return Math.round(value)
}

function formatSize(sizeBytes?: number) {
  if (sizeBytes === undefined || sizeBytes < 0) return '—'
  const unit = sizeBytes < 1024 ? 'byte' : sizeBytes < 1024 * 1024 ? 'kilobyte' : 'megabyte'
  const divisor = unit === 'byte' ? 1 : unit === 'kilobyte' ? 1024 : 1024 * 1024
  return new Intl.NumberFormat(locale.value, {
    style: 'unit',
    unit,
    unitDisplay: 'short',
    maximumFractionDigits: unit === 'byte' ? 0 : 1,
  }).format(sizeBytes / divisor)
}

function formatDate(value?: string) {
  if (!value) return '—'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }).format(date)
}

function fileTypeLabel(file: FileEntry) {
  if (file.type === 'directory') return 'DIR'
  const dot = file.name.lastIndexOf('.')
  return dot > 0 ? file.name.slice(dot + 1).toUpperCase() : 'FILE'
}

function fileToneFor(file: FileEntry) {
  return modelTones[mediaTones[file.mediaType]]
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

const normalizedSearchTerm = computed(() => searchTerm.value.trim().toLocaleLowerCase(locale.value))
const matchesSearch = (file: FileEntry) => !normalizedSearchTerm.value || file.name.toLocaleLowerCase(locale.value).includes(normalizedSearchTerm.value)
// The backend lists the whole directory, so search is applied only here.
const visibleDirectories = computed(() => libraryFiles.value.filter((file) => file.type === 'directory' && matchesSearch(file)))
const visibleFiles = computed(() => sortedBy(
  libraryFiles.value.filter((file) => file.type === 'file' && matchesSearch(file)),
  { key: sortKey.value, descending: naturallyDescending[sortKey.value] },
))
const deferLibraryCards = computed(() => shouldDeferLibraryCards(visibleFiles.value.length))
const printerSort = ref<FileSort>({ key: 'name', descending: false })
const printerFiles = computed(() => sortedBy(files.value.filter((file) => file.type === 'file'), printerSort.value))
const sortPrinterFiles = (key: SortKey) => { printerSort.value = nextSort(printerSort.value, key) }
const printerSortOrder = (key: SortKey) => printerSort.value.key !== key
  ? 'none'
  : printerSort.value.descending ? 'descending' : 'ascending'
const enabledSlicers = computed(() => slicers.value.filter((slicer) => slicer.enabled))
const isModelFile = (file: FileEntry) => /\.(3mf|stl|obj|zip)$/i.test(file.name)
const previewFile = ref<FileEntry>()

function fileActionItems(file: FileEntry): ActionMenuItem[] {
  const items: ActionMenuItem[] = [
    {
      label: t('dashboard.print'),
      icon: PhPlay,
      disabled: !printers.value.length,
      onSelect: () => requestPrint(file),
    },
    ...enabledSlicers.value.map((slicer) => ({
      label: t('filesView.openWith', { name: slicer.name }),
      icon: PhDesktop,
      onSelect: () => void openWithSlicer(file, slicer.name),
    })),
    {
      label: t('common.delete'),
      icon: PhTrash,
      danger: true,
      onSelect: () => deleteFile(file),
    },
  ]
  return items
}

function deleteFile(file: FileEntry) {
  askConfirmation(
    { title: 'filesView.deleteTitle', description: 'filesView.deleteDescription', confirm: 'filesView.deleteConfirm' },
    { name: file.name },
    async () => {
      await invoke('library_delete_file', { request: { path: file.devicePath } })
      showToast(t('filesView.deleted', { name: file.name }))
      await loadLibraryFiles(libraryPath.value)
    },
  )
}

async function openWithSlicer(file: FileEntry, slicer: string) {
  try {
    await invoke('open_file_with_slicer', { request: { path: file.devicePath, slicer } })
    showToast(t('filesView.openingIn', { name: file.name, slicer }))
  } catch (reason) {
    showToast(message(reason), 'error')
  }
}

async function requestPrint(file: FileEntry) {
  printPackage.value = undefined
  printPlate.value = undefined
  printDisplayName.value = file.name.replace(/\.gcode\.3mf$|\.3mf$|\.gcode$/i, '')
  if (/\.3mf$/i.test(file.name)) {
    try {
      const inspected = await invoke<PrintPackage>('inspect_library_print', { request: { path: file.devicePath } })
      if (!inspected.sliced) throw new Error(t('errors.jobFileInvalid'))
      printPackage.value = inspected
      printPlate.value = inspected.plates.find((plate) => plate.gcodePath)?.index
    } catch (reason) {
      showToast(message(reason), 'error')
      return
    }
  }
  printTarget.value = file
}

async function printToPrinter(name: string) {
  const file = printTarget.value
  if (!file || printBusy.value) return
  printBusy.value = true
  printStage.value = { stage: 'inspect', percent: 0 }
  try {
    await invoke('print_library_file', {
      request: {
        printer: name,
        path: file.devicePath,
        plate: printPlate.value,
        displayName: printDisplayName.value || undefined,
      },
    })
    printTarget.value = undefined
    showToast(t('filesView.sentToPrinter', { name: file.name }))
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    printBusy.value = false
    printStage.value = undefined
  }
}

onMounted(() => {
  window.addEventListener('beforeunload', warnBeforeUnload)
  document.addEventListener('visibilitychange', handleDocumentVisibility)
  systemThemeQuery = window.matchMedia('(prefers-color-scheme: dark)')
  systemPrefersDark.value = systemThemeQuery.matches
  systemThemeQuery.addEventListener('change', handleSystemThemeChange)
  void load()
  // The library normally loads on the way in through goTo, which a restored
  // view skips — without this, reopening on Files shows an empty grid.
  if (activeView.value === 'files') void loadLibraryFiles(libraryPath.value)
  void listen<string>('library-changed', (event) => handleLibraryChange(event.payload)).then((unlisten) => {
    libraryChangeUnlisten = unlisten
  })
  void listen<MonitorEntry[]>('monitoring-updated', (event) => {
    monitoring.value = event.payload
  }).then((unlisten) => {
    monitorUnlisten = unlisten
    void invoke<MonitorEntry[] | null>('cached_monitoring').then((cached) => {
      if (cached && monitoring.value.length === 0) monitoring.value = cached
    })
  })
  void listen<FirmwareUpdateEntry>('firmware-updates-updated', (event) => {
    const index = firmwareUpdates.value.findIndex((candidate) => candidate.profile === event.payload.profile)
    if (index === -1) firmwareUpdates.value.push(event.payload)
    else firmwareUpdates.value.splice(index, 1, event.payload)
  }).then((unlisten) => {
    firmwareUpdatesUnlisten = unlisten
    void invoke<FirmwareUpdateEntry[]>('cached_firmware_updates').then((cached) => {
      if (firmwareUpdates.value.length === 0) firmwareUpdates.value = cached
    })
  })
  void listen('presence-updated', () => {
    void invoke<Printer[]>('configured_printers').then((profiles) => {
      printers.value = profiles
    })
  }).then((unlisten) => {
    presenceUnlisten = unlisten
  })
  void listen<NotificationEvent>('printer-notification', (event) => {
    const labels: Record<NotificationEvent['kind'], MessageKey> = {
      completion: 'settingsView.notifyComplete',
      failure: 'settingsView.notifyFailure',
      disconnection: 'settingsView.notifyDisconnected',
    }
    showToast(`${event.payload.printer}: ${t(labels[event.payload.kind])}`, event.payload.kind === 'completion' ? 'success' : 'error')
  }).then((unlisten) => {
    notificationUnlisten = unlisten
  })
  void listen<TransferProgress>('transfer-progress', (event) => {
    if (event.payload.totalBytes) downloadProgress.value = Math.min(100, event.payload.bytesTransferred / event.payload.totalBytes * 100)
    if (event.payload.complete) downloadProgress.value = 100
  }).then((unlisten) => {
    transferUnlisten = unlisten
  })
  void listen<PrintStageEvent>('print-stage', (event) => {
    printStage.value = event.payload
  }).then((unlisten) => {
    printStageUnlisten = unlisten
  })
})

onUnmounted(() => {
  window.removeEventListener('beforeunload', warnBeforeUnload)
  document.removeEventListener('visibilitychange', handleDocumentVisibility)
  void stopCamera()
  systemThemeQuery?.removeEventListener('change', handleSystemThemeChange)
  monitorUnlisten?.()
  firmwareUpdatesUnlisten?.()
  presenceUnlisten?.()
  notificationUnlisten?.()
  transferUnlisten?.()
  printStageUnlisten?.()
  libraryChangeUnlisten?.()
  if (libraryChangeTimer !== undefined) window.clearTimeout(libraryChangeTimer)
  if (toastTimer !== undefined) window.clearTimeout(toastTimer)
  Object.values(temperatureTimers).forEach((timer) => {
    if (timer !== undefined) window.clearTimeout(timer)
  })
})
</script>

<template>
  <div class="min-h-full min-w-0 overflow-x-clip scheme-light dark:scheme-dark">
    <a href="#main-content" class="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-white focus:px-3 focus:py-2 focus:text-sm focus:font-semibold focus:text-gray-900 dark:focus:bg-gray-800 dark:focus:text-white">Skip to content</a>
    <!-- Tabs with underline -->
    <header class="sticky top-0 z-20 bg-white/95 pt-[env(safe-area-inset-top)] backdrop-blur dark:bg-gray-900/95">
      <!-- Scrolling one axis clips the other, which would cut off the printer
           menu. Only the compact layout below `lg` needs to scroll, and it
           uses a native select that the clip cannot reach. -->
      <div class="mx-auto max-w-[1500px] overflow-x-auto px-4 sm:px-8 lg:overflow-x-visible lg:px-10">
        <nav class="flex min-w-0 items-stretch border-b border-gray-200 sm:min-w-max dark:border-white/10" :aria-label="t('nav.ariaLabel')">
          <div class="grid w-full grid-cols-1 py-2 lg:hidden">
            <select
              name="primary-navigation"
              v-model="compactNavValue"
              :aria-label="t('nav.ariaLabel')"
              class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-2 pr-9 pl-3 text-sm font-medium text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-gray-900 dark:text-white dark:outline-white/15 dark:*:bg-gray-800"
            >
              <optgroup v-if="printers.length" :label="t('nav.groupPrinters')">
                <option v-for="printer in printers" :key="`compact-${printer.name}`" :value="`printer:${printer.name}`">
                  {{ printer.name }} · {{ statusLabel(badgeFor(printer.name)) }}
                </option>
              </optgroup>
              <optgroup :label="t('nav.groupSections')">
                <option v-for="tab in sectionTabs" :key="`compact-${tab.view}`" :value="`view:${tab.view}`">{{ tab.label }}</option>
              </optgroup>
            </select>
            <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-3 size-4 self-center justify-self-end text-gray-500 dark:text-gray-400" aria-hidden="true" />
          </div>
          <div v-if="printers.length && !usePrinterMenu" class="hidden items-center space-x-8 pr-8 min-[1400px]:flex">
            <button
              v-for="printer in printers"
              :key="printer.name"
              type="button"
              class="group inline-flex items-center gap-x-2 border-b-2 px-1 py-4 text-sm font-medium whitespace-nowrap transition"
              :class="activeView === 'control' && activePrinter?.name === printer.name
                ? 'border-cyan-500 text-cyan-600 dark:border-cyan-400 dark:text-cyan-400'
                : 'border-transparent text-gray-500 hover:border-gray-300 hover:text-gray-800 dark:text-gray-400 dark:hover:border-white/20 dark:hover:text-gray-200'"
              :aria-current="activeView === 'control' && activePrinter?.name === printer.name ? 'page' : undefined"
              @click="selectPrinter(printer.name)"
            >
              <span class="size-1.5 shrink-0 rounded-full ring-3" :class="badgeDotClasses[badgeFor(printer.name)]"></span>{{ printer.name }}
            </button>
          </div>
          <div
            v-if="printers.length"
            class="hidden items-center pr-4 lg:flex min-[1400px]:pr-8"
            :class="!usePrinterMenu && 'min-[1400px]:hidden'"
          >
            <!-- A native option renders text only, so this mirrors the tab
                 strip's status dot through a listbox instead of naming the
                 state in the label. -->
            <Listbox
              as="div"
              class="relative"
              :model-value="activeView === 'control' ? activePrinter?.name ?? '' : ''"
              @update:model-value="selectPrinter"
            >
              <ListboxButton
                :aria-label="t('nav.selectPrinter')"
                class="flex w-44 items-center gap-x-2 rounded-md bg-white py-1.5 pr-8 pl-3 text-left text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 min-[900px]:w-52 dark:bg-white/5 dark:text-white dark:outline-white/10"
              >
                <span
                  v-if="activeView === 'control' && activePrinter"
                  class="size-1.5 shrink-0 rounded-full ring-3"
                  :class="badgeDotClasses[activeBadge]"
                ></span>
                <span class="truncate">{{ activeView === 'control' && activePrinter ? activePrinter.name : t('nav.selectPrinter') }}</span>
                <span v-if="activeView === 'control' && activePrinter" class="sr-only">{{ statusLabel(activeBadge) }}</span>
                <PhCaretDown class="pointer-events-none absolute inset-y-0 right-0 mr-2 size-4 self-center text-gray-500 dark:text-gray-400" aria-hidden="true" />
              </ListboxButton>
              <ListboxOptions
                class="absolute left-0 z-30 mt-1 max-h-80 w-44 overflow-auto rounded-md bg-white py-1 shadow-lg outline-1 outline-black/5 min-[900px]:w-52 dark:bg-gray-800 dark:shadow-none dark:-outline-offset-1 dark:outline-white/10"
              >
                <ListboxOption
                  v-for="printer in printers"
                  :key="printer.name"
                  v-slot="{ active, selected }"
                  :value="printer.name"
                  as="template"
                >
                  <li
                    class="flex cursor-pointer items-center gap-x-2 px-3 py-2 text-sm"
                    :class="[
                      active ? 'bg-gray-100 outline-hidden dark:bg-white/5' : '',
                      selected ? 'font-medium text-cyan-600 dark:text-cyan-400' : 'text-gray-700 dark:text-gray-300',
                    ]"
                  >
                    <span class="size-1.5 shrink-0 rounded-full ring-3" :class="badgeDotClasses[badgeFor(printer.name)]"></span>
                    <span class="truncate">{{ printer.name }}</span>
                    <!-- The dot carries the state visually; keep it readable. -->
                    <span class="sr-only">{{ statusLabel(badgeFor(printer.name)) }}</span>
                  </li>
                </ListboxOption>
              </ListboxOptions>
            </Listbox>
          </div>
          <span v-if="printers.length" class="my-3 hidden w-px shrink-0 bg-gray-200 lg:block dark:bg-white/15" aria-hidden="true"></span>
          <div class="hidden items-center space-x-4 pl-4 lg:flex min-[1400px]:space-x-8 min-[1400px]:pl-8">
            <button
              v-for="tab in sectionTabs"
              :key="tab.view"
              type="button"
              class="group inline-flex items-center border-b-2 px-1 py-4 text-sm font-medium whitespace-nowrap transition"
              :class="activeView === tab.view
                ? 'border-cyan-500 text-cyan-600 dark:border-cyan-400 dark:text-cyan-400'
                : 'border-transparent text-gray-500 hover:border-gray-300 hover:text-gray-800 dark:text-gray-400 dark:hover:border-white/20 dark:hover:text-gray-200'"
              :aria-current="activeView === tab.view ? 'page' : undefined"
              @click="goTo(tab.view)"
            >
              <component
                :is="tab.icon"
                class="mr-2 -ml-0.5 size-5"
                :class="activeView === tab.view
                  ? 'text-cyan-500 dark:text-cyan-400'
                  : 'text-gray-400 group-hover:text-gray-500 dark:text-gray-500 dark:group-hover:text-gray-400'"
                aria-hidden="true"
              />
              <span>{{ tab.label }}</span>
            </button>
          </div>
        </nav>
      </div>
    </header>

    <main id="main-content" tabindex="-1" class="mx-auto max-w-[1500px] py-7 outline-none sm:px-8 lg:px-10">
      <h1 class="sr-only">{{ t('app.title') }}</h1>
      <!-- Notification -->
      <div aria-live="polite" class="pointer-events-none fixed inset-0 z-50 flex items-end px-4 py-6 sm:p-6">
        <transition-group
          tag="div"
          class="flex w-full flex-col items-center space-y-4 sm:items-end"
          enter-active-class="transform ease-out duration-300 transition"
          enter-from-class="translate-y-2 opacity-0 sm:translate-y-0 sm:translate-x-2"
          enter-to-class="translate-y-0 sm:translate-x-0"
          leave-active-class="transition ease-in duration-100"
          leave-to-class="opacity-0"
        >
          <div
            v-for="item in toasts"
            :key="item.key"
            class="pointer-events-auto w-full max-w-sm rounded-lg bg-white shadow-lg outline-1 outline-black/5 dark:bg-gray-800 dark:-outline-offset-1 dark:outline-white/10"
            :role="item.tone === 'error' ? 'alert' : 'status'"
          >
            <div class="p-4">
              <div class="flex items-start">
                <div class="shrink-0">
                  <PhWarningCircle v-if="item.tone === 'error'" class="size-6 text-red-500 dark:text-red-400" aria-hidden="true" />
                  <PhCheckCircle v-else class="size-6 text-green-400" aria-hidden="true" />
                </div>
                <div class="ml-3 w-0 flex-1 pt-0.5">
                  <p class="text-sm font-medium text-gray-900 dark:text-white">{{ item.text }}</p>
                </div>
                <div class="ml-4 flex shrink-0">
                  <button
                    type="button"
                    class="inline-flex rounded-md text-gray-400 hover:text-gray-500 dark:hover:text-white"
                    @click="dismissToast(item.key)"
                  >
                    <span class="sr-only">{{ t('common.close') }}</span>
                    <PhX class="size-5" aria-hidden="true" />
                  </button>
                </div>
              </div>
            </div>
          </div>
        </transition-group>
      </div>

      <template v-if="activeView === 'control' && activePrinter">
        <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
          <div class="min-w-0 flex-1">
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ activePrinter.name }}</h2>
            <div class="mt-1 flex flex-col sm:mt-0 sm:flex-row sm:flex-wrap sm:space-x-6">
              <div class="mt-2 flex items-center font-mono text-sm text-gray-500 dark:text-gray-400">
                <PhPrinter class="mr-1.5 size-5 shrink-0 text-gray-400 dark:text-gray-500" aria-hidden="true" />
                {{ activePrinter.driver }}
              </div>
              <div class="mt-2 flex items-center font-mono text-sm text-gray-500 dark:text-gray-400">
                <PhNetwork class="mr-1.5 size-5 shrink-0 text-gray-400 dark:text-gray-500" aria-hidden="true" />
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

        <!-- Awaiting the first sample: neutral, no alarm language, no recovery
             CTA — nothing has failed yet. -->
        <div
          v-if="!activeHasStatus && awaitingFirstSample"
          class="mx-4 flex min-h-140 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15"
          role="status"
          aria-live="polite"
        >
          <PhBroadcast class="mx-auto size-12 animate-pulse text-gray-400 dark:text-gray-500" aria-hidden="true" />
          <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('control.awaitingTitle') }}</h3>
          <p class="mt-1 max-w-md text-sm text-gray-500 dark:text-gray-400">{{ t('control.awaitingDescription') }}</p>
        </div>

        <!-- True outage: Melt has no usable status and a polling error. -->
        <div
          v-else-if="!activeHasStatus"
          class="mx-4 flex min-h-140 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15"
        >
          <PhWarning class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
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
                  <span class="rounded-full border border-current/20 px-2 py-0.5 text-[0.65rem] font-semibold uppercase tracking-wide opacity-80">{{ t(faultSeverityKeys[fault.severity]) }}</span>
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
          <section class="grid grid-cols-1 gap-5 xl:grid-cols-[minmax(0,3fr)_minmax(18rem,1fr)]">
            <Card class="overflow-hidden">
              <CardHeader :title="t('camera.title')" :icon="PhVideoCamera">
                <template #suffix>
                  <span
                    class="inline-flex items-center gap-x-1.5 rounded-md px-2 py-1 text-xs font-medium"
                    :title="cameraTransportDetail"
                    :class="cameraStateClasses"
                  >
                    <svg class="size-1.5" viewBox="0 0 6 6" aria-hidden="true"><circle cx="3" cy="3" r="3" /></svg>{{ cameraStateLabel }}
                  </span>
                </template>
                <IconButton :title="t('camera.refresh')" :aria-label="t('camera.refresh')" :disabled="cameraLoading || !cameraSupported" @click="refreshCamera"><PhArrowsClockwise class="size-4" :class="cameraLoading && 'animate-spin'" aria-hidden="true" /></IconButton>
                <IconButton :title="t('camera.snapshot')" :aria-label="t('camera.snapshot')" :disabled="cameraLoading || !capabilities?.cameraSnapshot" @click="saveSnapshot"><PhCamera class="size-4" aria-hidden="true" /></IconButton>
                <IconButton :title="t('camera.maximize')" :aria-label="t('camera.maximize')" :disabled="!cameraHasMedia" @click="toggleCameraFullscreen"><PhCornersOut class="size-4" aria-hidden="true" /></IconButton>
              </CardHeader>
              <div
                ref="cameraStage"
                class="relative min-h-60 overflow-hidden sm:min-h-77.5"
                :class="cameraHasMedia ? 'bg-black' : 'grid place-items-center bg-gray-100 dark:bg-gray-800'"
              >
                <video v-if="cameraPeer" ref="cameraVideo" :aria-label="t('camera.title')" class="absolute inset-0 size-full object-contain" autoplay muted playsinline />
                <canvas v-else-if="cameraCanvasActive" ref="cameraCanvas" :aria-label="t('camera.title')" class="absolute inset-0 size-full object-contain" />
                <img v-else-if="cameraUrl" :src="cameraUrl" :alt="t('camera.title')" width="1280" height="720" class="absolute inset-0 size-full object-contain" @error="cameraUrl = undefined" />
                <div v-else class="relative z-10 text-center">
                  <PhWarning class="mx-auto size-8 text-yellow-600 dark:text-yellow-400" aria-hidden="true" />
                  <p class="mt-3 font-medium text-gray-900 dark:text-white">{{ t('camera.unavailable') }}</p>
                  <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ cameraError ?? t('camera.checkConnection') }}</p>
                  <Button v-if="cameraSupported" class="mt-4" :disabled="cameraLoading" @click="refreshCamera"><PhArrowsClockwise class="size-4" :class="cameraLoading && 'animate-spin'" aria-hidden="true" /> {{ t('camera.refreshFeed') }}</Button>
                </div>
              </div>
            </Card>

            <Card>
              <CardHeader :title="t('control.currentJob')" :icon="PhCheckSquareOffset">
                <span class="font-mono text-xs text-gray-500 dark:text-gray-400">{{ t('control.complete', { percent: progressPercent }) }}</span>
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
                      class="truncate font-mono text-sm font-semibold text-gray-900 dark:text-white"
                      :title="selectedStatus?.job?.name"
                      translate="no"
                    >{{ selectedStatus?.job?.name ?? t('dashboard.noJob') }}</p>
                    <p class="mt-1 font-mono text-xs text-gray-500 dark:text-gray-400">
                      {{ t(printerStateMessageKey(selectedStatus?.state)) }}
                      <span v-if="currentStageLabel"> · {{ currentStageLabel }}</span>
                      <span v-if="selectedStatus?.printMeta?.plateIndex !== undefined">
                        · {{ t('control.plateOf', { index: selectedStatus.printMeta.plateIndex, total: selectedStatus.printMeta.plateCount ?? '—' }) }}
                      </span>
                    </p>
                  </div>
                </div>
                <div class="mt-6">
                  <div class="mb-2 flex justify-between text-xs text-gray-500 dark:text-gray-400">
                    <span>{{ t('control.layer', { current: selectedStatus?.progress?.currentLayer ?? '—', total: selectedStatus?.progress?.totalLayers ?? '—' }) }}</span>
                    <span v-if="remainingTimeLabel" class="font-mono">
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
                <div class="mt-6 flex gap-2">
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
                  <label for="speed-profile" class="block text-sm/6 font-light text-gray-900 dark:text-white">{{ t('control.speedProfile') }}</label>
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
              <CardHeader :title="t('dashboard.temperature')" :icon="PhThermometerSimple" />
              <div class="font-light space-y-4 px-4 py-5 sm:p-6">
                <div v-for="row in temperatureRows" :key="row.key" class="flex items-center justify-between gap-3">
                  <div class="min-w-0">
                    <p class="text-sm/6 font-light text-gray-900 dark:text-white">{{ temperatureLabel(row.key, row.value) }}</p>
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
                  <p v-else-if="!row.value.controllable" class="shrink-0 text-right text-xs text-gray-400 dark:text-gray-500">{{ t('control.telemetryOnly') }}</p>
                </div>

                <!-- Material presets: set every controllable heater in one action. -->
                <div v-if="presetsAvailable" class="border-t border-gray-200 pt-4 dark:border-white/10">
                  <p id="preset-group-label" class="text-sm/6 font-light text-gray-900 dark:text-white">{{ t('control.presets') }}</p>
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
              <div class="font-light space-y-4 px-4 py-5 sm:p-6">
                <div v-for="[key, value] in fanRows" :key="key" class="block">
                  <span class="mb-2 flex justify-between">
                    <span class="text-sm/6 font-light text-gray-900 dark:text-white">{{ fanLabel(key) }}</span>
                    <span class="text-sm/6 text-gray-500 dark:text-gray-400">{{ fanDrafts[key] ?? value.speedPercent ?? '—' }}{{ (fanDrafts[key] ?? value.speedPercent) === undefined ? '' : '%' }}</span>
                  </span>
                  <input v-if="value.controllable && value.speedPercent !== undefined" :key="`${key}-${value.speedPercent}`" :value="value.speedPercent" :name="`fan-${key}`" class="h-1 w-full cursor-pointer accent-cyan-600 dark:accent-cyan-400" type="range" :min="value.minimumPercent ?? 0" :max="value.maximumPercent ?? 100" :aria-label="t('control.fanPower', { fan: fanLabel(key) })" @input="previewFan(key, $event)" @change="sendFan(key, $event)" />
                  <p v-else class="text-xs text-gray-400 dark:text-gray-500">{{ t('control.telemetryOnly') }}</p>
                </div>
              </div>
            </Card>

            <Card v-if="lightRows.length">
              <CardHeader :title="t('control.lights')" :icon="PhSun" />
              <div class="font-light space-y-4 px-4 py-5 sm:p-6">
                <div v-for="[key, value] in lightRows" :key="key" class="flex items-center justify-between gap-3">
                  <label :for="`light-${key}`" class="grow cursor-pointer text-sm/6 font-light text-gray-900 dark:text-white">{{ lightLabel(key) }}</label>
                  <Switch
                    v-if="capabilities?.lightControl && value.controllable"
                    :id="`light-${key}`"
                    :model-value="value.mode === 'on'"
                    :label="lightLabel(key)"
                    @update:model-value="toggleLight(key, $event)"
                  />
                  <span v-else class="text-xs text-gray-400 dark:text-gray-500">{{ value.mode }} · {{ t('control.telemetryOnly') }}</span>
                </div>
              </div>
            </Card>

            <Card v-if="capabilities?.motionControl">
              <CardHeader :title="t('dashboard.motion')" :icon="PhArrowsOutCardinal" />
              <div class="font-light px-4 py-5 sm:p-6">
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
                    <label for="step-size" class="block text-sm/6 font-light text-gray-900 dark:text-white">{{ t('control.stepSize') }}</label>
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
                    <label for="feed-rate" class="block text-sm/6 font-light text-gray-900 dark:text-white">{{ t('control.feedRate') }}</label>
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
                        </button>
                      </th>
                      <th scope="col" class="w-20 px-2 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase dark:text-gray-400">{{ t('filesView.type') }}</th>
                      <th scope="col" :aria-sort="printerSortOrder('size')" class="hidden w-24 px-2 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase min-[1000px]:table-cell dark:text-gray-400">
                        <button type="button" class="flex items-center gap-1 uppercase hover:text-gray-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:hover:text-gray-200 dark:focus-visible:outline-cyan-400" @click="sortPrinterFiles('size')">
                          {{ t('filesView.size') }}
                          <PhCaretDown v-if="printerSort.key === 'size'" class="size-3" :class="!printerSort.descending && 'rotate-180'" aria-hidden="true" />
                        </button>
                      </th>
                      <th scope="col" :aria-sort="printerSortOrder('modified')" class="hidden w-48 px-2 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase min-[1180px]:table-cell dark:text-gray-400">
                        <button type="button" class="flex items-center gap-1 uppercase hover:text-gray-700 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:hover:text-gray-200 dark:focus-visible:outline-cyan-400" @click="sortPrinterFiles('modified')">
                          {{ t('filesView.modified') }}
                          <PhCaretDown v-if="printerSort.key === 'modified'" class="size-3" :class="!printerSort.descending && 'rotate-180'" aria-hidden="true" />
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
                      <td class="max-w-0 truncate px-4 py-4 font-mono text-sm font-medium text-gray-900 sm:px-6 dark:text-white" :title="file.name" translate="no">{{ file.name }}</td>
                      <td class="px-2 py-4 text-sm whitespace-nowrap">
                        <span class="inline-flex items-center rounded-md bg-gray-100 px-2 py-1 text-xs font-medium text-gray-600 dark:bg-gray-400/10 dark:text-gray-400">{{ fileTypeLabel(file) }}</span>
                      </td>
                      <td class="hidden px-2 py-4 font-mono text-sm whitespace-nowrap text-gray-500 min-[1000px]:table-cell dark:text-gray-400">{{ formatSize(file.sizeBytes) }}</td>
                      <td class="hidden px-2 py-4 font-mono text-sm whitespace-nowrap text-gray-500 min-[1180px]:table-cell dark:text-gray-400">{{ formatDate(file.modifiedAt) }}</td>
                      <td class="px-4 py-4 text-sm whitespace-nowrap sm:px-6">
                        <div class="flex justify-end gap-1">
                          <IconButton
                            class="text-cyan-600 dark:text-cyan-400"
                            :title="t('filesView.printFile')"
                            :aria-label="t('filesView.printNamed', { name: file.name })"
                            :disabled="!capabilities?.jobStart || selectedStatus?.state !== 'idle'"
                            @click.stop="sendJobAction('start', file.devicePath)"
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
              <CardHeader :title="t('materials.title')" :icon="PhStack" />
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
                        <span v-if="system.drying.setting" class="font-mono">{{ system.drying.setting.temperatureC }} °C</span>
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
                    >
                      <span
                        class="absolute inset-x-0 top-0 transition-[height] duration-200"
                        :style="{ height: `${100 - material.remainingPercent}%`, backgroundColor: emptyFilamentOverlayFor(material.color) }"
                      ></span>
                      <span class="relative z-10 flex h-full flex-col items-center justify-center gap-1 p-1 text-center font-mono">
                        <span class="text-xs font-bold">{{ material.slot }}</span>
                        <span v-if="material.type !== '—'" class="text-[10px] font-semibold tracking-wide uppercase wrap-anywhere">{{ material.type }}</span>
                        <span v-if="material.remainingGrams !== undefined" class="text-[10px] font-medium">{{ material.remainingGrams }} g</span>
                      </span>
                    </div>
                  </div>
                  <div v-if="capabilities?.amsDrying && system.drying" class="mt-3 flex flex-wrap items-center gap-2 text-xs">
                    <template v-if="system.drying.controllable">
                      <Button v-if="dryingStoppable(system.drying)" :disabled="dryingBusy" @click="setDrying(system, false)">{{ t('drying.stop') }}</Button>
                      <template v-else-if="dryingForm?.unitId === system.unitId">
                        <label class="flex items-center gap-1">{{ t('drying.temperature') }}
                          <input
                            v-model.number="dryingForm.temperatureC"
                            type="number"
                            inputmode="numeric"
                            :min="dryingBounds(system.unitId)?.min"
                            :max="dryingBounds(system.unitId)?.max"
                            class="w-16 rounded-md bg-white py-1.5 text-center font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
                          /> °C</label>
                        <label class="flex items-center gap-1">{{ t('drying.hours') }}
                          <input
                            v-model.number="dryingForm.hours"
                            type="number"
                            inputmode="numeric"
                            min="1"
                            max="24"
                            class="w-14 rounded-md bg-white py-1.5 text-center font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
                          /> h</label>
                        <Button variant="primary" :disabled="dryingBusy" @click="setDrying(system, true)">{{ t('drying.start') }}</Button>
                        <Button @click="dryingForm = undefined">{{ t('common.cancel') }}</Button>
                      </template>
                      <Button v-else :disabled="dryingBusy || Boolean(system.drying.blockedReasons?.length)" @click="openDryingForm(system)">{{ t('drying.dry') }}</Button>
                      <span v-if="!system.drying.active && system.drying.blockedReasons?.length" class="text-gray-500 dark:text-gray-400">{{ t(dryingBlockedKey(system.drying.blockedReasons[0])) }}</span>
                    </template>
                    <span v-else class="text-gray-500 dark:text-gray-400">{{ t('drying.screenOnly') }}</span>
                  </div>
                </div>
              </div>
            </Card>
          </section>
        </div>
      </template>

      <template v-else-if="activeView === 'printers' || (!hasPrinters && activeView === 'control')">
        <template v-if="firmwarePrinter">
          <div class="mb-7 px-4 sm:px-0">
            <Button variant="secondary" @click="closeFirmwareManagement">
              <PhArrowLeft class="size-4" aria-hidden="true" /> {{ t('firmware.backToPrinters') }}
            </Button>
            <div class="mt-5 lg:flex lg:items-end lg:justify-between lg:gap-6">
              <div class="min-w-0 flex-1">
                <h2 class="text-2xl/7 font-bold text-gray-900 sm:text-3xl sm:tracking-tight dark:text-white">{{ t('firmware.title') }}</h2>
                <p class="mt-2 max-w-3xl text-sm text-gray-500 dark:text-gray-400">{{ t('firmware.description', { name: firmwarePrinter.name }) }}</p>
                <div class="mt-3 flex flex-wrap gap-x-6 gap-y-2 font-mono text-xs text-gray-500 dark:text-gray-400">
                  <span class="inline-flex items-center gap-1.5"><PhPrinter class="size-4 text-gray-400 dark:text-gray-500" aria-hidden="true" />{{ firmwarePrinter.driver }}</span>
                  <span class="inline-flex items-center gap-1.5"><PhNetwork class="size-4 text-gray-400 dark:text-gray-500" aria-hidden="true" />{{ firmwarePrinter.host }}</span>
                </div>
              </div>
              <div class="mt-5 flex items-center gap-2 lg:mt-0">
                <StatusBadge :status="badgeFor(firmwarePrinter.name)" :label="statusLabel(badgeFor(firmwarePrinter.name))" />
                <Button @click="selectPrinter(firmwarePrinter.name)"><PhCards class="size-4" aria-hidden="true" /> {{ t('printersView.openControl') }}</Button>
              </div>
            </div>
          </div>

          <Card class="overflow-hidden">
            <CardHeader :title="firmwarePrinter.name" :icon="PhPrinter">
              <template #suffix>
                <span
                  v-if="selectedFirmwareUpdate"
                  class="inline-flex items-center rounded-md px-2 py-1 text-xs font-medium"
                  :class="selectedFirmwareUpdate.report.availability === 'available'
                    ? 'bg-amber-100 text-amber-800 dark:bg-amber-400/10 dark:text-amber-300'
                    : selectedFirmwareUpdate.report.availability === 'current'
                      ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-400/10 dark:text-emerald-300'
                      : 'bg-gray-100 text-gray-600 dark:bg-white/10 dark:text-gray-300'"
                >{{ firmwareAvailabilityLabel(selectedFirmwareUpdate) }}</span>
              </template>
              <Button
                variant="secondary"
                :disabled="firmwareRefreshing"
                @click="loadFirmwareUpdates(firmwarePrinter.name, true)"
              ><PhArrowsClockwise class="size-4" :class="firmwareRefreshing && 'animate-spin'" aria-hidden="true" /> {{ t('firmware.checkAgain') }}</Button>
            </CardHeader>
            <div class="px-4 py-5 sm:p-6" :aria-busy="firmwareRefreshing">
              <div v-if="!selectedFirmwareUpdate" class="flex items-start gap-3 text-sm text-gray-500 dark:text-gray-400" role="status">
                <PhArrowsClockwise v-if="!firmwareError" class="mt-0.5 size-4 shrink-0 animate-spin" aria-hidden="true" />
                <PhWarning v-else class="mt-0.5 size-4 shrink-0 text-amber-500" aria-hidden="true" />
                <p>{{ firmwareError ?? t('firmware.checking') }}</p>
              </div>
              <template v-else>
                <p v-if="selectedFirmwareUpdate.report.source === 'moonrakerUpdateManager'" class="mb-4 text-sm text-gray-500 dark:text-gray-400">
                  {{ t('firmware.klipperScope') }}
                </p>
                <div class="divide-y divide-gray-200 dark:divide-white/10">
                  <div
                    v-for="component in selectedFirmwareUpdate.report.components"
                    :key="component.id"
                    class="grid gap-3 py-4 first:pt-0 last:pb-0 sm:grid-cols-[minmax(10rem,1fr)_minmax(0,2fr)] sm:items-center"
                  >
                    <div class="min-w-0">
                      <div class="flex flex-wrap items-center gap-2">
                        <p class="truncate text-sm font-semibold text-gray-900 dark:text-white">{{ component.label }}</p>
                        <span v-if="component.required" class="rounded-full bg-amber-100 px-2 py-0.5 text-[0.65rem] font-semibold uppercase tracking-wide text-amber-800 dark:bg-amber-400/10 dark:text-amber-300">{{ t('firmware.required') }}</span>
                      </div>
                      <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ firmwareComponentLabel(component.kind) }}</p>
                    </div>
                    <div class="grid grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-3 rounded-md bg-gray-50 px-3 py-2 dark:bg-white/5">
                      <div class="min-w-0">
                        <p class="text-[0.65rem] font-medium uppercase tracking-wide text-gray-400 dark:text-gray-500">{{ t('firmware.installed') }}</p>
                        <p class="truncate font-mono text-sm text-gray-800 dark:text-gray-200" :title="component.currentVersion">{{ versionRail(component).installed }}</p>
                      </div>
                      <PhCaretRight class="size-4 text-gray-400" aria-hidden="true" />
                      <div class="min-w-0 text-right">
                        <p class="text-[0.65rem] font-medium uppercase tracking-wide text-gray-400 dark:text-gray-500">{{ t('firmware.advertised') }}</p>
                        <p class="truncate font-mono text-sm" :class="component.availability === 'available' ? 'font-semibold text-amber-700 dark:text-amber-300' : 'text-gray-800 dark:text-gray-200'" :title="component.availableVersion">{{ versionRail(component).advertised }}</p>
                      </div>
                    </div>
                  </div>
                </div>
                <div class="mt-4 flex flex-wrap items-center justify-between gap-2 border-t border-gray-200 pt-4 text-xs text-gray-500 dark:border-white/10 dark:text-gray-400">
                  <span>{{ t('firmware.lastChecked', { date: formatDate(selectedFirmwareUpdate.checkedAt) }) }}</span>
                  <span v-if="selectedFirmwareUpdate.stale" class="font-medium text-amber-700 dark:text-amber-300">{{ t('firmware.stale') }}</span>
                </div>
                <p v-if="selectedFirmwareUpdate.error" class="mt-3 text-xs text-amber-700 dark:text-amber-300">{{ message(selectedFirmwareUpdate.error) }}</p>
                <ul v-if="selectedFirmwareUpdate.report.issues.length" class="mt-3 space-y-1 text-xs text-gray-500 dark:text-gray-400">
                  <li v-for="issue in selectedFirmwareUpdate.report.issues" :key="issue.code">{{ issue.message }}</li>
                </ul>
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

        <!-- Stats -->
        <dl v-if="hasPrinters" class="mb-5 grid grid-cols-3 gap-2 sm:gap-5">
          <Card v-for="stat in fleetStats" :key="stat.label" as="div" class="px-3 py-4 sm:p-6">
            <dt class="truncate text-sm font-medium text-gray-500 dark:text-gray-400">{{ stat.label }}</dt>
            <dd class="mt-1 text-3xl font-semibold tracking-tight" :class="stat.tone">{{ stat.value }}</dd>
          </Card>
        </dl>

        <!-- Loading / error states -->
        <div v-if="loading" class="mx-4 flex min-h-97.5 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15" role="status" aria-live="polite">
          <PhPrinter class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
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
          <PhPrinter class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
          <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('printersView.emptyTitle') }}</h3>
          <p class="mt-1 max-w-sm text-sm text-gray-500 dark:text-gray-400">{{ t('printersView.emptyDescription') }}</p>
          <Button class="mt-6" variant="primary" :disabled="!drivers.length" @click="openAddition"><PhBroadcast class="size-4" aria-hidden="true" /> {{ t('printersView.scan') }}</Button>
        </div>

        <div v-else class="grid gap-5 grid-cols-[repeat(auto-fit,minmax(280px,1fr))]">
          <Card v-for="printer in printers" :key="printer.name" as="article" class="px-4 py-5 sm:p-6">
            <div class="flex items-start justify-between">
              <div class="flex items-center gap-3">
                <span class="grid size-10 place-items-center rounded-lg bg-gray-100 text-cyan-600 dark:bg-white/10 dark:text-cyan-400"><PhPrinter class="size-5" aria-hidden="true" /></span>
                <div>
                  <h3 class="font-semibold text-gray-900 dark:text-white">{{ printer.name }}</h3>
                  <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ printer.driver }}</p>
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
                <IconButton class="hover:text-red-600 dark:hover:text-red-400" :title="t('printersView.removePrinter')" :aria-label="t('printersView.removeNamed', { name: printer.name })" @click="removePrinter(printer.name)"><PhTrash class="size-4" aria-hidden="true" /></IconButton>
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
            <Button variant="secondary" class="mt-5 w-full" @click="openFirmwareManagement(printer)">
              <PhInfo class="size-4" aria-hidden="true" /> {{ t('firmware.title') }}
            </Button>
            <div class="mt-2 flex items-center gap-2">
              <Button class="min-w-0 flex-1" @click="selectPrinter(printer.name)"><PhCards class="size-4" aria-hidden="true" /> {{ t('printersView.openControl') }}</Button>
              <ActionMenu :label="t('control.printerActions')" :items="printerActionItems(printer)" />
            </div>
          </Card>
          <button
            type="button"
            class="relative flex min-h-55 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 p-12 text-center hover:border-gray-400 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:border-white/15 dark:hover:border-white/25 dark:focus-visible:outline-cyan-500"
            @click="openAddition"
          >
            <PhPlus class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
            <span class="mt-2 block text-sm font-semibold text-gray-900 dark:text-white">{{ t('printersView.addAnother') }}</span>
          </button>
        </div>
        </template>
      </template>

      <template v-else-if="activeView === 'settings'">
        <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
          <div class="min-w-0 flex-1">
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ t('nav.settings') }}</h2>
            <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">{{ t('settingsView.description') }}</p>
          </div>
        </div>
        <div class="grid max-w-[1500px] gap-5 lg:grid-cols-2 xl:grid-cols-3">
          <Card as="section" class="overflow-hidden">
            <CardHeader :title="t('settingsView.notifications')" :icon="PhBellRinging" />
            <div class="divide-y divide-gray-200 dark:divide-white/10">
              <label v-for="notification in notifications" :key="notification.id" :for="`notification-${notification.id}`" class="flex cursor-pointer items-center justify-between gap-3 px-4 py-5 hover:bg-gray-50 sm:px-6 dark:hover:bg-white/5">
                <span class="flex grow flex-col">
                  <span class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ t(notification.label) }}</span>
                  <span class="text-sm text-gray-500 dark:text-gray-400">{{ t(notification.description) }}</span>
                </span>
                <Switch :id="`notification-${notification.id}`" v-model="notification.enabled" :label="t(notification.label)" @update:model-value="updateNotification(notification.id, $event)" />
              </label>
            </div>
          </Card>
          <Card as="section" class="overflow-hidden">
            <CardHeader :title="t('settingsView.slicers')" :icon="PhDesktop" />
            <div class="divide-y divide-gray-200 dark:divide-white/10">
              <div v-for="(slicer, slicerIndex) in slicers" :key="slicer.name" class="flex items-center justify-between gap-3 px-4 py-5 hover:bg-gray-50 sm:px-6 dark:hover:bg-white/5">
                <label :for="`slicer-${slicerIndex}`" class="flex min-w-0 grow cursor-pointer flex-col">
                  <span class="truncate text-sm/6 font-medium text-gray-900 dark:text-white" translate="no">{{ slicer.name }}</span>
                  <span class="truncate font-mono text-sm text-gray-500 dark:text-gray-400" translate="no">{{ slicer.path }}</span>
                </label>
                <div class="flex items-center gap-2">
                  <Switch :id="`slicer-${slicerIndex}`" v-model="slicer.enabled" :label="slicer.name" @update:model-value="updateSlicerEnabled(slicer.name, $event)" />
                  <IconButton
                    class="hover:text-red-600 dark:hover:text-red-400"
                    :title="t('settingsView.removeSlicer')"
                    :aria-label="t('settingsView.removeNamedSlicer', { name: slicer.name })"
                    @click="removeSlicer(slicer.name)"
                  ><PhTrash class="size-4" aria-hidden="true" /></IconButton>
                </div>
              </div>
              <p v-if="!slicers.length" class="px-4 py-5 text-sm text-gray-500 sm:px-6 dark:text-gray-400">{{ t('settingsView.noSlicers') }}</p>
            </div>
            <form autocomplete="off" class="grid gap-3 border-t border-gray-200 px-4 py-4 sm:px-6 dark:border-white/10" @submit.prevent="addSlicer">
              <div class="min-w-0">
                <label for="slicer-name" class="block text-xs font-medium text-gray-500 dark:text-gray-400">{{ t('settingsView.slicerName') }}</label>
                <input
                  ref="slicerNameInput"
                  id="slicer-name"
                  name="slicer-name"
                  v-model="slicerDraft.name"
                  type="text"
                  autocomplete="off"
                  :aria-invalid="Boolean(slicerNameError)"
                  :aria-describedby="[slicerNameError && 'slicer-name-error', slicerError && 'slicer-error'].filter(Boolean).join(' ') || undefined"
                  class="mt-1 block w-full rounded-md bg-white px-3 py-1.5 text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
                  @input="slicerNameError = undefined"
                />
                <p v-if="slicerNameError" id="slicer-name-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ slicerNameError }}</p>
              </div>
              <div class="min-w-0">
                <label for="slicer-path" class="block text-xs font-medium text-gray-500 dark:text-gray-400">{{ t('settingsView.slicerPath') }}</label>
                <div class="mt-1 flex gap-2">
                  <input
                    ref="slicerPathInput"
                    id="slicer-path"
                    name="slicer-path"
                    :value="slicerDraft.path"
                    type="text"
                    readonly
                    autocomplete="off"
                    :placeholder="t('settingsView.slicerPathPlaceholder')"
                    :aria-invalid="Boolean(slicerPathError)"
                    :aria-describedby="[slicerPathError && 'slicer-path-error', slicerError && 'slicer-error'].filter(Boolean).join(' ') || undefined"
                    class="block w-full min-w-0 rounded-md bg-white px-3 py-1.5 font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 placeholder:font-sans placeholder:text-gray-400 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:placeholder:text-gray-500"
                  />
                  <Button class="shrink-0" :disabled="slicerBusy" @click="browseSlicerPath">{{ t('settingsView.browse') }}</Button>
                </div>
                <p v-if="slicerPathError" id="slicer-path-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ slicerPathError }}</p>
              </div>
              <Button type="submit" class="w-full" :disabled="slicerBusy">{{ t('settingsView.addSlicer') }}</Button>
            </form>
            <p v-if="slicerError" id="slicer-error" class="px-4 pb-4 text-xs text-red-600 sm:px-6 dark:text-red-400" role="alert">{{ slicerError }}</p>
          </Card>
          <Card as="section" class="overflow-hidden">
            <CardHeader :title="t('settingsView.presets')" :icon="PhThermometerSimple">
              <IconButton :title="t('settingsView.restorePresets')" :aria-label="t('settingsView.restorePresets')" @click="restoreDefaultPresets"><PhArrowsClockwise class="size-4" aria-hidden="true" /></IconButton>
            </CardHeader>
            <p class="px-4 pt-4 text-xs text-gray-500 sm:px-6 dark:text-gray-400">
              {{ t('settingsView.presetsDescription', { nozzle: temperatureMaximums.nozzle, bed: temperatureMaximums.bed }) }}
            </p>
            <div class="mt-4 divide-y divide-gray-200 dark:divide-white/10">
              <div v-for="preset in temperaturePresets" :key="preset.id" class="flex items-center justify-between gap-3 px-4 py-4 hover:bg-gray-50 sm:px-6 dark:hover:bg-white/5">
                <div class="flex min-w-0 grow flex-col">
                  <span class="truncate text-sm/6 font-medium text-gray-900 dark:text-white">{{ preset.name }}</span>
                  <span class="font-mono text-xs text-gray-500 dark:text-gray-400">
                    {{ t('settingsView.presetNozzle') }} {{ preset.nozzleCelsius }} <span aria-hidden="true">·</span> {{ t('settingsView.presetBed') }} {{ preset.bedCelsius }}
                  </span>
                </div>
                <IconButton
                  class="hover:text-red-600 dark:hover:text-red-400"
                  :title="t('settingsView.removePreset')"
                  :aria-label="t('settingsView.removeNamedPreset', { name: preset.name })"
                  @click="removePreset(preset)"
                ><PhTrash class="size-4" aria-hidden="true" /></IconButton>
              </div>
              <p v-if="!temperaturePresets.length" class="px-4 py-5 text-sm text-gray-500 sm:px-6 dark:text-gray-400">{{ t('settingsView.noPresets') }}</p>
            </div>
            <form autocomplete="off" class="grid gap-3 border-t border-gray-200 px-4 py-4 sm:px-6 dark:border-white/10" @submit.prevent="addPreset">
              <div class="min-w-0">
                <label for="preset-name" class="block text-xs font-medium text-gray-500 dark:text-gray-400">{{ t('settingsView.presetName') }}</label>
                <input
                  ref="presetNameInput"
                  id="preset-name"
                  name="preset-name"
                  v-model="presetDraft.name"
                  type="text"
                  autocomplete="off"
                  :maxlength="PRESET_NAME_MAX_LENGTH"
                  :aria-invalid="Boolean(presetErrors.name)"
                  :aria-describedby="presetErrors.name ? 'preset-name-error' : undefined"
                  class="mt-1 block w-full rounded-md bg-white px-3 py-1.5 text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
                />
                <p v-if="presetErrors.name" id="preset-name-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ presetErrors.name }}</p>
              </div>
              <div class="grid grid-cols-2 gap-3">
                <div class="min-w-0">
                  <label for="preset-nozzle" class="block text-xs font-medium text-gray-500 dark:text-gray-400">{{ t('settingsView.presetNozzle') }}</label>
                  <input
                    id="preset-nozzle"
                    name="preset-nozzle"
                    v-model="presetDraft.nozzleCelsius"
                    type="number"
                    inputmode="numeric"
                    min="0"
                    :max="temperatureMaximums.nozzle"
                    autocomplete="off"
                    :aria-invalid="Boolean(presetErrors.nozzle)"
                    :aria-describedby="presetErrors.nozzle ? 'preset-nozzle-error' : undefined"
                    class="mt-1 block w-full rounded-md bg-white px-3 py-1.5 font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
                  />
                  <p v-if="presetErrors.nozzle" id="preset-nozzle-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ presetErrors.nozzle }}</p>
                </div>
                <div class="min-w-0">
                  <label for="preset-bed" class="block text-xs font-medium text-gray-500 dark:text-gray-400">{{ t('settingsView.presetBed') }}</label>
                  <input
                    id="preset-bed"
                    name="preset-bed"
                    v-model="presetDraft.bedCelsius"
                    type="number"
                    inputmode="numeric"
                    min="0"
                    :max="temperatureMaximums.bed"
                    autocomplete="off"
                    :aria-invalid="Boolean(presetErrors.bed)"
                    :aria-describedby="presetErrors.bed ? 'preset-bed-error' : undefined"
                    class="mt-1 block w-full rounded-md bg-white px-3 py-1.5 font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
                  />
                  <p v-if="presetErrors.bed" id="preset-bed-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ presetErrors.bed }}</p>
                </div>
              </div>
              <Button type="submit" class="w-full">{{ t('settingsView.addPreset') }}</Button>
            </form>
          </Card>
          <Card as="section">
            <CardHeader :title="t('settingsView.appearance')" :icon="PhSun" />
            <div class="px-4 py-5 sm:p-6">
              <div>
                <label for="theme" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('settingsView.theme') }}</label>
                <div class="mt-2 grid grid-cols-1">
                  <select id="theme" name="theme" v-model="theme" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
                    <option value="system">{{ t('settingsView.themeSystem') }}</option>
                    <option value="light">{{ t('settingsView.themeLight') }}</option>
                    <option value="dark">{{ t('settingsView.themeDark') }}</option>
                  </select>
                  <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
                </div>
              </div>
              <div class="mt-6">
                <label for="language" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('app.locale') }}</label>
                <div class="mt-2 grid grid-cols-1">
                  <select id="language" name="language" v-model="locale" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
                    <option value="en">{{ t('app.english') }}</option>
                    <option value="pt-BR">{{ t('app.portuguese') }}</option>
                  </select>
                  <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
                </div>
              </div>
            </div>
          </Card>
          <Card as="section" class="overflow-hidden">
            <CardHeader :title="t('settingsView.about')" :icon="PhInfo" />
            <div class="px-4 py-5 sm:p-6">
              <dl class="divide-y divide-gray-200 text-sm dark:divide-white/10">
                <div class="flex items-center justify-between py-2">
                  <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.version') }}</dt>
                  <dd class="font-mono text-gray-900 dark:text-gray-300">{{ info ? `v${info.version}` : '—' }}</dd>
                </div>
                <div class="flex items-center justify-between gap-4 py-2">
                  <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.license') }}</dt>
                  <dd class="text-right text-gray-900 dark:text-gray-300">
                    <a href="https://www.gnu.org/licenses/agpl-3.0.html" target="_blank" rel="noreferrer" class="text-cyan-700 underline hover:text-cyan-600 dark:text-cyan-400">{{ info?.license ?? 'AGPL-3.0-only' }}</a>
                  </dd>
                </div>
                <div class="flex items-center justify-between gap-4 py-2">
                  <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.source') }}</dt>
                  <dd class="text-right text-gray-900 dark:text-gray-300">
                    <a :href="info?.sourceUrl ?? 'https://github.com/polimero-app/melt'" target="_blank" rel="noreferrer" class="text-cyan-700 underline hover:text-cyan-600 dark:text-cyan-400">{{ t('settingsView.sourceLink') }}</a>
                  </dd>
                </div>
                <div class="flex items-center justify-between gap-4 py-2">
                  <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.icon') }}</dt>
                  <dd class="text-right text-gray-900 dark:text-gray-300">
                    <a href="https://thenounproject.com/icon/melt-8239158/" target="_blank" rel="noreferrer" class="text-cyan-700 underline hover:text-cyan-600 dark:text-cyan-400">{{ t('settingsView.iconCredit') }}</a>
                  </dd>
                </div>
                <div v-if="diagnostics" class="flex items-center justify-between py-2">
                  <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.platform') }}</dt>
                  <dd class="font-mono text-gray-900 dark:text-gray-300">{{ diagnostics.platform }}</dd>
                </div>
                <div v-if="diagnostics" class="flex items-center justify-between py-2">
                  <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.profiles') }}</dt>
                  <dd class="font-mono text-gray-900 dark:text-gray-300">{{ diagnostics.configuredProfiles }}</dd>
                </div>
                <div v-if="diagnostics" class="flex items-center justify-between py-2">
                  <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.monitor') }}</dt>
                  <dd class="font-mono text-gray-900 dark:text-gray-300">{{ t('settingsView.monitorValue', { workers: diagnostics.monitorWorkers, seconds: diagnostics.monitorIntervalSeconds }) }}</dd>
                </div>
              </dl>

              <!-- Per-printer compatibility is a set of nested reports, not a
                   term/definition pair of the summary list above. -->
              <section
                v-for="printer in diagnostics?.bambu ?? []"
                :key="printer.printer"
                class="mt-4 border-t border-gray-200 pt-3 dark:border-white/10"
              >
                <h4 class="text-sm font-medium text-gray-900 dark:text-white">{{ t('settingsView.bambuCompatibility') }} · {{ printer.printer }}</h4>
                <dl class="mt-2 space-y-1 text-xs text-gray-600 dark:text-gray-300">
                  <div class="flex flex-wrap gap-x-1">
                    <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.model') }}:</dt>
                    <dd class="font-mono">{{ printer.modelRaw || printer.canonicalModel }}</dd>
                  </div>
                  <div class="flex flex-wrap gap-x-1">
                    <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.authorization') }}:</dt>
                    <dd class="font-mono">{{ printer.authorization.effective }}</dd>
                  </div>
                  <div class="flex flex-wrap gap-x-1">
                    <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.cameraTransport') }}:</dt>
                    <dd class="font-mono">{{ printer.camera.preferred }} · {{ printer.camera.source }}</dd>
                  </div>
                  <div class="flex flex-wrap gap-x-1">
                    <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.storageTransport') }}:</dt>
                    <dd class="font-mono">{{ printer.storageTransport }}</dd>
                  </div>
                  <div v-if="printer.firmwareModules.length" class="flex flex-wrap gap-x-1">
                    <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.firmwareModules') }}:</dt>
                    <dd class="font-mono">{{ printer.firmwareModules.map((module) => `${module.name} ${module.software}`).join(', ') }}</dd>
                  </div>
                  <div class="flex flex-wrap gap-x-1">
                    <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.quirks') }}:</dt>
                    <dd>{{ printer.quirks.length ? printer.quirks.map((quirk) => `${quirk.id} (${quirk.active ? 'active' : quirk.qualification})`).join(', ') : t('settingsView.none') }}</dd>
                  </div>
                </dl>
                <p v-if="printer.camera.rejectedAdvertisement" class="mt-1 text-xs text-amber-700 dark:text-amber-300">{{ printer.camera.rejectedAdvertisement }}</p>
              </section>
              <p v-if="diagnosticsError" class="mt-3 text-xs text-red-600 dark:text-red-400" role="alert">{{ diagnosticsError }}</p>
              <Button class="mt-4 w-full" :disabled="diagnosticsLoading" @click="runDiagnostics"><PhDesktop class="size-4" aria-hidden="true" /> {{ diagnosticsLoading ? t('common.loading') : t('diagnostics.open') }}</Button>
            </div>
          </Card>
        </div>
      </template>

      <template v-else-if="activeView === 'files'">
        <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
          <div class="min-w-0 flex-1">
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ t('filesView.title') }}</h2>
            <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">{{ t('filesView.description') }}</p>
          </div>
          <div class="mt-5 flex gap-2 lg:mt-0 lg:ml-4 max-lg:w-full">
            <Button class="max-lg:flex-1" :disabled="libraryFilesLoading" @click="chooseLibraryFolder"><PhFolderOpen class="size-4" aria-hidden="true" /> {{ t('filesView.changeFolder') }}</Button>
            <Button class="max-lg:flex-1" :disabled="uploadBusy || !libraryPath" @click="uploadFile"><PhUploadSimple class="size-4" :class="uploadBusy && 'animate-pulse'" aria-hidden="true" /> {{ t('filesView.upload') }}</Button>
            <Button class="max-lg:flex-1" :disabled="libraryFilesLoading" @click="loadLibraryFiles(libraryPath)"><PhArrowsClockwise class="size-4" :class="libraryFilesLoading && 'animate-spin'" aria-hidden="true" /> {{ t('dashboard.refresh') }}</Button>
          </div>
        </div>

        <div class="mb-5 flex flex-wrap items-center gap-2 px-4 sm:px-0">
          <!-- Breadcrumbs -->
          <IconButton :title="t('filesView.up')" :aria-label="t('filesView.up')" :disabled="!libraryParent || libraryFilesLoading" @click="navigateToDirectory(libraryParent!)"><PhArrowUp class="size-4" aria-hidden="true" /></IconButton>
          <nav class="flex overflow-x-auto" :aria-label="t('filesView.breadcrumb')">
            <ol role="list" class="flex items-center space-x-4">
              <li v-for="(breadcrumb, index) in libraryBreadcrumbs" :key="breadcrumb.path" class="flex items-center">
                <PhCaretRight v-if="index > 0" class="mr-4 size-5 shrink-0 text-gray-400 dark:text-gray-500" aria-hidden="true" />
                <button
                  type="button"
                  class="text-sm font-medium whitespace-nowrap text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
                  :class="breadcrumb.path === libraryPath ? 'text-gray-900 dark:text-white' : ''"
                  :aria-current="breadcrumb.path === libraryPath ? 'page' : undefined"
                  @click="navigateToDirectory(breadcrumb.path)"
                >{{ breadcrumb.name }}</button>
              </li>
            </ol>
          </nav>
          <div class="ml-auto grid grid-cols-1">
            <select
              name="file-sort"
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
          <!-- Input with leading icon -->
          <div class="grid grid-cols-1">
            <input
              name="file-search"
              v-model="searchTerm"
              autocomplete="off"
              class="col-start-1 row-start-1 block w-40 rounded-md bg-white py-1.5 pr-3 pl-10 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 placeholder:text-gray-400 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:w-64 sm:pl-9 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:placeholder:text-gray-500"
              :aria-label="t('filesView.search')"
              :placeholder="t('filesView.search') + '…'"
              type="search"
            />
            <PhMagnifyingGlass class="pointer-events-none col-start-1 row-start-1 ml-3 size-5 self-center text-gray-400 sm:size-4 dark:text-gray-500" aria-hidden="true" />
          </div>
        </div>

        <!-- The search term survives directory navigation, so an empty folder
             could just be an active filter. Say so, and offer a way out. -->
        <div v-if="normalizedSearchTerm" class="mb-5 flex flex-wrap items-center gap-2 px-4 sm:px-0" role="status">
          <span class="inline-flex items-center gap-x-2 rounded-md bg-cyan-50 px-2 py-1 text-xs font-medium text-cyan-700 dark:bg-cyan-400/10 dark:text-cyan-400">
            {{ t('filesView.filteredBy', { term: searchTerm.trim(), count: visibleDirectories.length + visibleFiles.length }) }}
            <button
              type="button"
              class="rounded-sm hover:text-cyan-900 dark:hover:text-cyan-200"
              :aria-label="t('filesView.clearSearch')"
              :title="t('filesView.clearSearch')"
              @click="searchTerm = ''"
            ><PhX class="size-3.5" aria-hidden="true" /></button>
          </span>
        </div>

        <div v-if="visibleDirectories.length" class="mb-5 grid gap-4 grid-cols-[repeat(auto-fit,minmax(220px,1fr))]">
          <button
            v-for="directory in visibleDirectories"
            :key="directory.devicePath"
            type="button"
            class="min-h-28 bg-white px-4 py-5 text-left shadow-sm transition hover:bg-cyan-50 hover:shadow-md sm:rounded-lg sm:p-6 dark:bg-gray-800/50 dark:shadow-none dark:outline dark:-outline-offset-1 dark:outline-white/10 dark:hover:bg-cyan-400/5 dark:hover:outline-cyan-400/30"
            @click="navigateToDirectory(directory.devicePath)"
          >
            <PhFolder class="size-7 text-cyan-600 dark:text-cyan-400" aria-hidden="true" />
            <span class="mt-3 block truncate text-sm font-medium text-gray-900 dark:text-white" :title="directory.name" translate="no">{{ directory.name }}</span>
                    <span class="mt-1 block text-xs text-gray-500 dark:text-gray-400">{{ t('filesView.folderModified', { date: formatDate(directory.modifiedAt) }) }}</span>
          </button>
        </div>

        <!-- Grid list -->
        <div class="grid gap-4 grid-cols-[repeat(auto-fit,minmax(220px,1fr))]">
          <Card
            v-for="file in visibleFiles"
            :key="file.devicePath"
            as="article"
            class="transition hover:shadow-md dark:hover:outline-cyan-400/30"
            :class="deferLibraryCards && 'library-card-deferred'"
          >
            <!-- Only model files have a render worth enlarging, and a hover
                 overlay alone reaches neither keyboard nor touch, so the ones
                 that do become a real button. -->
            <component
              :is="isModelFile(file) ? 'button' : 'div'"
              :type="isModelFile(file) ? 'button' : undefined"
              :aria-label="isModelFile(file) ? t('filesView.openPreview', { name: file.name }) : undefined"
              class="group relative block w-full min-h-45 overflow-hidden sm:rounded-t-lg"
              :class="fileToneFor(file).preview"
              @click="isModelFile(file) && (previewFile = file)"
            >
              <ModelThumbnail
                v-if="isModelFile(file)"
                :path="file.devicePath"
                :size-bytes="file.sizeBytes"
                :modified-at="file.modifiedAt"
                :alt="file.name"
                :unavailable-label="t('filesView.previewUnavailable')"
                class="size-full min-h-45"
              />
              <div v-else class="grid min-h-45 place-items-center">
                <span class="block transform-[perspective(200px)_rotateX(10deg)_rotateZ(-8deg)]" :class="fileToneFor(file).shape"></span>
              </div>
              <span
                v-if="isModelFile(file)"
                class="pointer-events-none absolute inset-0 grid place-items-center bg-gray-950/45 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
              ><PhMagnifyingGlassPlus class="size-8 text-white" aria-hidden="true" /></span>
              <span class="absolute right-3 bottom-2.5 text-[10px] font-bold text-gray-500 uppercase dark:text-slate-300/45">.{{ fileTypeLabel(file).toLowerCase() }}</span>
            </component>
            <div class="flex items-start gap-3 px-4 py-5 sm:p-6">
              <PhFile class="mt-0.5 size-4 shrink-0 text-gray-400 dark:text-gray-500" aria-hidden="true" />
              <div class="min-w-0 flex-1">
                <h3 class="truncate text-sm font-medium text-gray-900 dark:text-white" :title="file.name" translate="no">{{ file.name }}</h3>
                <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ formatSize(file.sizeBytes) }} · {{ formatDate(file.modifiedAt) }}</p>
              </div>
              <ActionMenu v-if="fileActionItems(file).length" class="-mt-2 -mr-2" :label="t('filesView.moreActions')" :items="fileActionItems(file)" />
            </div>
          </Card>
          <div v-if="!visibleDirectories.length && !visibleFiles.length" class="col-span-full mx-4 flex min-h-65 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15">
            <PhFolder class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
            <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('filesView.emptyTitle') }}</h3>
            <p class="mt-1 text-sm text-gray-500 dark:text-gray-400">{{ libraryFilesLoading ? t('common.loading') : libraryFilesError ?? t('filesView.emptyDescription') }}</p>
          </div>
        </div>
      </template>
    </main>

    <SlideOver :open="additionOpen" :title="editingPrinter ? t('printersView.editPrinter') : t('addition.title')" :description="t('addition.description')" :close-label="t('common.closePanel')" @close="closeAddition">
      <form id="add-printer-form" @submit.prevent="addPrinter">
        <Button class="w-full" :disabled="discovering" @click="discoverPrinters"><PhBroadcast class="size-4" :class="discovering && 'animate-pulse'" aria-hidden="true" /> {{ discovering ? t('printersView.scanning') : t('printersView.discover') }}</Button>
        <p v-if="discoveryError" class="mt-2 text-xs text-red-600 dark:text-red-400" role="alert">{{ discoveryError }}</p>
        <ul v-if="discovered.length" class="mt-3 divide-y divide-gray-200 rounded-md border border-gray-200 dark:divide-white/10 dark:border-white/10">
          <li v-for="printer in discovered" :key="`${printer.serial}:${printer.host}`" class="flex items-center justify-between gap-2 px-3 py-2">
            <span class="min-w-0 truncate text-sm text-gray-900 dark:text-white">{{ printer.name || printer.model || printer.host }} · <span class="font-mono text-xs text-gray-500 dark:text-gray-400">{{ printer.host }}</span></span>
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
            <label for="printer-timeout" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.timeout') }}</label>
            <input id="printer-timeout" name="printer-timeout" v-model="draft.timeout" required class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
          </div>
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

    <SlideOver :open="tlsOpen" :title="t('printersView.refreshCertificate')" :description="t('tls.confirmDescription')" :close-label="t('common.closePanel')" @close="!tlsRefreshing && (tlsOpen = false)">
      <code v-if="tlsFingerprint" class="block rounded-md bg-gray-100 p-3 font-mono text-xs break-all text-gray-900 dark:bg-white/10 dark:text-white">{{ tlsFingerprint }}</code>
      <p v-if="tlsError" class="mt-4 text-sm text-red-600 dark:text-red-400" role="alert">{{ tlsError }}</p>
      <template #footer>
        <Button :disabled="tlsRefreshing" @click="tlsOpen = false">{{ t('common.cancel') }}</Button>
        <Button variant="danger" :disabled="tlsRefreshing" @click="refreshTls">{{ tlsRefreshing ? t('common.sending') : t('tls.confirm') }}</Button>
      </template>
    </SlideOver>

    <SlideOver
      :open="printTarget !== undefined"
      :title="t('filesView.printer')"
      :description="printTarget?.name"
      :close-label="t('common.closePanel')"
      @close="!printBusy && (printTarget = undefined)"
    >
      <ul v-if="printers.length" class="divide-y divide-gray-200 dark:divide-white/10">
        <li v-for="printer in printers" :key="printer.name" class="flex items-center justify-between gap-3 py-3">
          <span class="flex min-w-0 flex-col items-start gap-1">
            <span class="flex min-w-0 items-center gap-2">
              <span class="size-1.5 shrink-0 rounded-full ring-3" :class="badgeDotClasses[badgeFor(printer.name)]"></span>
              <span class="truncate text-sm text-gray-900 dark:text-white">{{ printer.name }}</span>
            </span>
            <span v-if="!isReachable(badgeFor(printer.name))" class="ml-3.5 text-xs font-medium text-red-600 dark:text-red-400">{{ t('status.offlineLabel') }}</span>
          </span>
          <Button
            :disabled="printBusy || !isReachable(badgeFor(printer.name))"
            @click="printToPrinter(printer.name)"
          >{{ t('dashboard.print') }}</Button>
        </li>
      </ul>
      <div v-if="printPackage" class="mt-5 space-y-4 border-t border-gray-200 pt-4 dark:border-white/10">
        <label class="block text-sm font-medium text-gray-700 dark:text-gray-200">
          {{ t('filesView.displayName') }}
          <input v-model="printDisplayName" name="print-display-name" maxlength="99" autocomplete="off" class="mt-1 block w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 dark:border-white/15 dark:bg-white/5 dark:text-white" />
        </label>
        <label class="block text-sm font-medium text-gray-700 dark:text-gray-200">
          {{ t('filesView.plate') }}
          <select v-model="printPlate" name="print-plate" class="mt-1 block w-full rounded-md border border-gray-300 bg-white px-3 py-2 text-sm text-gray-900 dark:border-white/15 dark:bg-gray-900 dark:text-white">
            <option v-for="plate in printPackage.plates.filter((candidate) => candidate.gcodePath)" :key="plate.index" :value="plate.index">
              {{ plate.name || `${t('filesView.plate')} ${plate.index}` }}
            </option>
          </select>
        </label>
      </div>
      <div v-if="currentPrintTargetState === 'busy'" class="mt-4 space-y-2">
        <div class="flex justify-between text-xs text-gray-500 dark:text-gray-400">
          <span>{{ t('common.sending') }}</span>
          <span>{{ printStage?.percent ?? 0 }}%</span>
        </div>
        <div class="h-2 overflow-hidden rounded-full bg-gray-200 dark:bg-white/10">
          <div class="h-full rounded-full bg-cyan-600 transition-[width]" :style="{ width: `${printStage?.percent ?? 0}%` }"></div>
        </div>
      </div>
      <p v-else-if="currentPrintTargetState === 'empty'" class="text-sm text-gray-500 dark:text-gray-400">{{ t('filesView.noPrinters') }}</p>
      <template #footer>
        <Button :disabled="printBusy" @click="printTarget = undefined">{{ t('common.cancel') }}</Button>
      </template>
    </SlideOver>

    <FilePreviewDialog
      :file="previewFile"
      :meta="previewFile ? `${formatSize(previewFile.sizeBytes)} · ${formatDate(previewFile.modifiedAt)}` : ''"
      :items="previewFile ? fileActionItems(previewFile) : []"
      :close-label="t('common.close')"
      :unavailable-label="t('filesView.previewUnavailable')"
      @close="previewFile = undefined"
    />

    <ConfirmDialog
      :open="pendingConfirm !== undefined"
      :title="pendingConfirm?.title ?? ''"
      :description="pendingConfirm?.description"
      :confirm-label="confirming ? t('common.sending') : pendingConfirm?.confirm ?? ''"
      :cancel-label="t('common.cancel')"
      :busy="confirming"
      @close="!confirming && (pendingConfirm = undefined)"
      @confirm="runConfirmation"
    />
  </div>
</template>
