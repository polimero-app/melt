<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watchEffect, type Component } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { locales, preferredLocale, translate, type Locale, type MessageKey } from './i18n'
import StatusBadge from './components/StatusBadge.vue'
import ActionMenu, { type ActionMenuItem } from './components/ActionMenu.vue'
import SlideOver from './components/SlideOver.vue'
import Button from './components/Button.vue'
import IconButton from './components/IconButton.vue'
import Switch from './components/Switch.vue'
import Card from './components/Card.vue'
import CardHeader from './components/CardHeader.vue'
import {
  PhArrowsClockwise,
  PhArrowsOutCardinal,
  PhBellRinging,
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
  PhDrop,
  PhFan,
  PhFile,
  PhFolder,
  PhGearSix,
  PhHexagon,
  PhHouse,
  PhInfo,
  PhMagnifyingGlass,
  PhMinus,
  PhNetwork,
  PhPause,
  PhPlay,
  PhPlus,
  PhPrinter,
  PhStack,
  PhStop,
  PhSun,
  PhThermometerSimple,
  PhTrash,
  PhVideoCamera,
  PhWarning,
  PhWifiHigh,
  PhWifiLow,
  PhWifiMedium,
  PhWifiSlash,
  PhX,
} from '@phosphor-icons/vue'

type View = 'control' | 'printers' | 'settings' | 'files'
type PrinterBadge = 'idle' | 'busy' | 'offline'
type ModelTone = 'cyan' | 'amber' | 'rose' | 'violet'

type AppInfo = {
  version: string
  modes: [string, string]
}

type Printer = {
  name: string
  driver: string
  host: string
}

type Driver = {
  name: string
  description: string
}

type NewPrinter = Printer & {
  serial: string
  timeout: string
  insecure: boolean
}

type Capabilities = {
  status: boolean
  discovery: boolean
  cameraStream: boolean
  cameraSnapshot: boolean
  fileList: boolean
  fileDownload: boolean
  fileUpload: boolean
  jobStart: boolean
  jobPause: boolean
  jobResume: boolean
  jobCancel: boolean
  emergencyStop: boolean
  temperatureWrite: boolean
  motionControl: boolean
  fanControl: boolean
  tlsRefresh: boolean
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

type PrinterStatus = {
  state: 'idle' | 'printing' | 'paused' | 'error' | 'unknown'
  temperatures?: { nozzle?: Temperature; bed?: Temperature; chamber?: Temperature }
  job?: { name: string }
  progress?: { percent: number; currentLayer?: number; totalLayers?: number }
  errors: { code: string; message: string }[]
  warnings: { code: string; message: string }[]
  fans?: Record<string, number>
}

type MonitorEntry = {
  name: string
  driver: string
  status?: PrinterStatus
  error?: string
}

type FileEntry = {
  name: string
  root: string
  path: string
  devicePath: string
  type: 'file' | 'directory'
  sizeBytes?: number
  modifiedAt?: string
}

type FileList = {
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
}

type CameraSnapshot = {
  dataUrl: string
}

type CameraStream = {
  url: string
}

interface MaterialSlot {
  slot: string
  status: string
  type: string
  name: string
  color: string
  remainingPercent?: number
}

interface MaterialSystemView {
  name: string
  temperature?: number
  humidity?: string
  slots: MaterialSlot[]
}

const statusDotClasses: Record<PrinterBadge, string> = {
  idle: 'bg-green-500 ring-green-500/10',
  busy: 'bg-yellow-500 ring-yellow-500/10',
  offline: 'bg-red-500 ring-red-500/10',
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
const fileTones: ModelTone[] = ['cyan', 'amber', 'rose', 'violet']

const info = ref<AppInfo>()
const printers = ref<Printer[]>([])
const drivers = ref<Driver[]>([])
const monitoring = ref<MonitorEntry[]>([])
const capabilities = ref<Capabilities>()
const files = ref<FileEntry[]>([])
const loading = ref(true)
const refreshing = ref(false)
const filesLoading = ref(false)
const filesError = ref<string>()
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
const cameraUrl = ref<string>()
const cameraLoading = ref(false)
const cameraError = ref<string>()
let monitorTimer: number | undefined

const activeView = ref<View>('control')
const sectionTabs = computed<{ view: View; label: string; icon: Component }[]>(() => [
  { view: 'printers', label: t('nav.printers'), icon: PhPrinter },
  { view: 'settings', label: t('nav.settings'), icon: PhGearSix },
  { view: 'files', label: t('nav.files'), icon: PhFolder },
])
const activePrinterId = ref('')
const theme = ref<'light' | 'dark' | 'system'>(
  (['light', 'dark', 'system'] as const).find((value) => value === localStorage.getItem('theme')) ?? 'system',
)
const systemPrefersDark = ref(true)
let systemThemeQuery: MediaQueryList | undefined
function handleSystemThemeChange(event: MediaQueryListEvent) {
  systemPrefersDark.value = event.matches
}
const isLightTheme = computed(() => theme.value === 'light' || (theme.value === 'system' && !systemPrefersDark.value))
watchEffect(() => {
  document.documentElement.classList.toggle('dark', !isLightTheme.value)
})
watchEffect(() => {
  localStorage.setItem('theme', theme.value)
})
const locale = ref<Locale>(locales.find((value) => value === localStorage.getItem('locale')) ?? preferredLocale())
watchEffect(() => {
  localStorage.setItem('locale', locale.value)
})
function t(key: MessageKey, values?: Record<string, string | number>) {
  return translate(locale.value, key, values)
}
const cameraStage = ref<HTMLElement>()
const toast = ref('')
const selectedFile = ref('')
const currentDirectory = ref('/')
const stepSize = ref('1 mm')
const feedRate = ref(50)
const notifications = ref<{ label: MessageKey; description: MessageKey; enabled: boolean }[]>([
  { label: 'settingsView.notifyComplete', description: 'settingsView.notifyCompleteDescription', enabled: true },
  { label: 'settingsView.notifyFailure', description: 'settingsView.notifyFailureDescription', enabled: true },
  { label: 'settingsView.notifyDisconnected', description: 'settingsView.notifyDisconnectedDescription', enabled: true },
])
const draft = ref({
  name: '',
  driver: '',
  host: '',
  serial: '',
  timeout: '10s',
  insecure: false,
  accessCode: '',
})

const activePrinter = computed(() => printers.value.find((printer) => printer.name === activePrinterId.value) ?? printers.value[0])
const hasPrinters = computed(() => printers.value.length > 0)
const selectedMonitor = computed(() => monitoring.value.find((entry) => entry.name === activePrinter.value?.name))
const selectedStatus = computed(() => selectedMonitor.value?.status)

function badgeFor(name: string): PrinterBadge {
  const entry = monitoring.value.find((candidate) => candidate.name === name)
  if (!entry) return 'idle'
  if (entry.error || entry.status === undefined || ['error', 'unknown'].includes(entry.status.state)) return 'offline'
  return ['printing', 'paused'].includes(entry.status.state) ? 'busy' : 'idle'
}

const activeBadge = computed(() => (activePrinter.value ? badgeFor(activePrinter.value.name) : 'offline'))
const badgeMessageKeys: Record<PrinterBadge, MessageKey> = {
  idle: 'status.onlineIdle',
  busy: 'status.onlineBusy',
  offline: 'status.offlineLabel',
}
const statusLabel = (badge: PrinterBadge) => t(badgeMessageKeys[badge])
const progressPercent = computed(() => selectedStatus.value?.progress?.percent ?? 0)
const temperatureRows = computed(() => {
  const temperatures = selectedStatus.value?.temperatures ?? {}
  return (['nozzle', 'bed', 'chamber'] as const).flatMap((key) => {
    const value = temperatures[key]
    return value ? [{ key, value }] : []
  })
})
const fanRows = computed(() => Object.entries(selectedStatus.value?.fans ?? {}))
const fanMessageKeys: Record<string, MessageKey> = {
  partCooling: 'control.fanPartCooling',
  auxiliary: 'control.fanAuxiliary',
  heatbreak: 'control.fanHeatbreak',
}
const fanLabel = (key: string) => {
  const messageKey = fanMessageKeys[key]
  return messageKey ? t(messageKey) : key
}
const temperatureKeys: Record<'nozzle' | 'bed' | 'chamber', MessageKey> = {
  nozzle: 'dashboard.nozzle',
  bed: 'dashboard.bed',
  chamber: 'control.chamber',
}
// The desktop backend does not report wifi signal or material trays yet; the
// markup stays behind these guards for when a driver provides them.
const wifiDbm = computed<number | undefined>(() => undefined)
const materialSystems = computed<MaterialSystemView[]>(() => [])
const cameraOnline = computed(() => Boolean(cameraUrl.value))
const cameraSupported = computed(() => Boolean(capabilities.value?.cameraStream || capabilities.value?.cameraSnapshot))

const fleetStats = computed(() => [
  { label: t('printersView.totalPrinters'), value: printers.value.length, tone: 'text-gray-900 dark:text-white' },
  { label: t('printersView.online'), value: printers.value.filter((printer) => badgeFor(printer.name) !== 'offline').length, tone: 'text-green-600 dark:text-green-400' },
  { label: t('printersView.printingNow'), value: printers.value.filter((printer) => badgeFor(printer.name) === 'busy').length, tone: 'text-yellow-600 dark:text-yellow-400' },
])

function message(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason)
}

function showToast(text: string) {
  toast.value = text
  window.setTimeout(() => {
    toast.value = ''
  }, 2200)
}

async function load() {
  loading.value = true
  loadError.value = undefined
  profileError.value = undefined

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
    if (printer) await selectPrinter(printer.name, false)
    else clearSelection()
  } else profileError.value = message(profiles.reason)

  if (availableDrivers.status === 'fulfilled') drivers.value = availableDrivers.value
  else loadError.value ??= message(availableDrivers.reason)

  loading.value = false
}

function clearSelection() {
  activePrinterId.value = ''
  capabilities.value = undefined
  monitoring.value = []
  files.value = []
  cameraUrl.value = undefined
  cameraError.value = undefined
}

async function refreshMonitoring() {
  if (refreshing.value || !printers.value.length) return
  refreshing.value = true
  try {
    monitoring.value = await invoke<MonitorEntry[]>('monitored_printers')
  } catch (reason) {
    showToast(message(reason))
  } finally {
    refreshing.value = false
  }
}

// focus defaults to refresh: user-initiated selections jump to the control
// view, background re-selections (load, removal fallback) keep the current view.
async function selectPrinter(name: string, refresh = true, focus = refresh) {
  activePrinterId.value = name
  if (focus) activeView.value = 'control'
  capabilities.value = undefined
  files.value = []
  filesError.value = undefined
  currentDirectory.value = '/'
  cameraUrl.value = undefined
  cameraError.value = undefined
  try {
    const result = await invoke<{ capabilities: Capabilities }>('printer_capabilities', { name })
    capabilities.value = result.capabilities
    if (result.capabilities.fileList) void loadFiles()
  } catch (reason) {
    showToast(message(reason))
  }
  if (refresh) await refreshMonitoring()
}

function goTo(view: View) {
  activeView.value = view
  if (view === 'files') void loadFiles()
}

async function loadFiles() {
  if (!activePrinter.value || filesLoading.value || !capabilities.value?.fileList) return
  filesLoading.value = true
  filesError.value = undefined
  try {
    files.value = (await invoke<FileList>('printer_files', { name: activePrinter.value.name })).entries
  } catch (reason) {
    filesError.value = message(reason)
  } finally {
    filesLoading.value = false
  }
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
  additionError.value = undefined
  discoveryError.value = undefined
  discovered.value = []
  additionOpen.value = true
  void discoverPrinters()
}

function closeAddition() {
  if (!adding.value) additionOpen.value = false
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
    const profile = await invoke<NewPrinter>('create_configured_printer', { request: draft.value })
    const printer = { name: profile.name, driver: profile.driver, host: profile.host }
    printers.value.push(printer)
    additionOpen.value = false
    draft.value.accessCode = ''
    await selectPrinter(printer.name)
  } catch (reason) {
    additionError.value = message(reason)
  } finally {
    adding.value = false
  }
}

async function removePrinter(name: string) {
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
    showToast(message(reason))
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
    showToast(message(reason))
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
    await refreshMonitoring()
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

async function sendJobAction(action: 'start' | 'pause' | 'resume' | 'cancel', devicePath?: string) {
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
    await refreshMonitoring()
  } catch (reason) {
    showToast(message(reason))
  }
}

async function adjustTemperature(kind: 'nozzle' | 'bed' | 'chamber', delta: number) {
  if (!activePrinter.value || !selectedStatus.value || kind === 'chamber') return
  const temperature = selectedStatus.value.temperatures?.[kind]
  if (!temperature) return
  const maximum = kind === 'nozzle' ? 300 : 120
  const base = temperature.targetCelsius ?? temperature.currentCelsius
  const next = Math.max(0, Math.min(maximum, Math.round((base + delta) / 5) * 5))
  try {
    await invoke('printer_temperature_set', {
      request: { name: activePrinter.value.name, [`${kind}Celsius`]: next },
    })
    await refreshMonitoring()
  } catch (reason) {
    showToast(message(reason))
  }
}

async function sendFan(fan: string, event: Event) {
  if (!activePrinter.value) return
  const speed = Number((event.target as HTMLInputElement).value)
  if (!Number.isInteger(speed)) return
  try {
    await invoke('printer_fan_set', {
      request: { name: activePrinter.value.name, fan, speedPercent: speed, confirmed: true },
    })
    showToast(t('control.fanSet', { fan: fanLabel(fan), percent: speed }))
    await refreshMonitoring()
  } catch (reason) {
    showToast(message(reason))
    await refreshMonitoring()
  }
}

async function homeAxes() {
  if (!activePrinter.value) return
  try {
    await invoke('printer_motion_home', {
      request: { name: activePrinter.value.name, confirmed: true },
    })
    showToast(t('control.homingStarted'))
    await refreshMonitoring()
  } catch (reason) {
    showToast(message(reason))
  }
}

async function emergencyStop() {
  if (!activePrinter.value) return
  try {
    await invoke('printer_emergency_stop', { name: activePrinter.value.name })
    showToast(t('control.emergencyStopSent'))
    await refreshMonitoring()
  } catch (reason) {
    showToast(message(reason))
  }
}

async function refreshCamera() {
  if (!activePrinter.value || cameraLoading.value || !cameraSupported.value) return
  cameraLoading.value = true
  cameraError.value = undefined
  try {
    if (capabilities.value?.cameraStream) {
      cameraUrl.value = (await invoke<CameraStream>('printer_camera_stream', { name: activePrinter.value.name })).url
    } else if (capabilities.value?.cameraSnapshot) {
      cameraUrl.value = (await invoke<CameraSnapshot>('printer_camera_snapshot', { name: activePrinter.value.name })).dataUrl
    }
  } catch (reason) {
    cameraUrl.value = undefined
    cameraError.value = message(reason)
  } finally {
    cameraLoading.value = false
  }
}

async function saveSnapshot() {
  if (!activePrinter.value || cameraLoading.value || !capabilities.value?.cameraSnapshot) return
  cameraLoading.value = true
  cameraError.value = undefined
  try {
    cameraUrl.value = (await invoke<CameraSnapshot>('printer_camera_snapshot', { name: activePrinter.value.name })).dataUrl
    showToast(t('camera.snapshotCaptured'))
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
  if (sizeBytes < 1024) return `${sizeBytes} B`
  if (sizeBytes < 1024 * 1024) return `${(sizeBytes / 1024).toFixed(1)} KB`
  return `${(sizeBytes / (1024 * 1024)).toFixed(1)} MB`
}

function fileTypeLabel(file: FileEntry) {
  if (file.type === 'directory') return 'DIR'
  const dot = file.name.lastIndexOf('.')
  return dot > 0 ? file.name.slice(dot + 1).toUpperCase() : 'FILE'
}

function fileToneFor(name: string) {
  return modelTones[fileTones[Math.abs([...name].reduce((hash, char) => hash * 31 + char.charCodeAt(0), 0)) % fileTones.length]!]!
}

function parentDirectory(path: string) {
  const normalized = path.startsWith('/') ? path.slice(1) : path
  const index = normalized.lastIndexOf('/')
  return index < 0 ? '/' : `/${normalized.slice(0, index)}`
}

function textColorFor(color: string) {
  if (!color.startsWith('#')) return 'var(--color-white)'
  const r = parseInt(color.slice(1, 3), 16)
  const g = parseInt(color.slice(3, 5), 16)
  const b = parseInt(color.slice(5, 7), 16)
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255
  return luminance > 0.55 ? 'var(--color-gray-900)' : 'var(--color-white)'
}

function wifiSignal(dbm: number | undefined) {
  if (dbm === undefined) return { icon: PhWifiSlash, label: t('control.signalNone') }
  if (dbm >= -55) return { icon: PhWifiHigh, label: t('control.signalExcellent') }
  if (dbm >= -70) return { icon: PhWifiMedium, label: t('control.signalGood') }
  return { icon: PhWifiLow, label: t('control.signalWeak') }
}

const visibleDirectories = computed(() =>
  files.value.filter((file) => file.type === 'directory' && parentDirectory(file.path) === currentDirectory.value),
)
const visibleFiles = computed(() =>
  files.value.filter((file) => file.type === 'file' && parentDirectory(file.path) === currentDirectory.value),
)
const printerFiles = computed(() => files.value.filter((file) => file.type === 'file'))
const breadcrumbs = computed(() => {
  const parts = currentDirectory.value.split('/').filter(Boolean)
  return [{ name: t('filesView.title'), path: '/' }, ...parts.map((part, index) => ({ name: part, path: `/${parts.slice(0, index + 1).join('/')}` }))]
})

function navigateToDirectory(path: string) {
  currentDirectory.value = path
}

const printerActionItems = computed<ActionMenuItem[]>(() => {
  if (!activePrinter.value) return []
  const items: ActionMenuItem[] = []
  if (capabilities.value?.tlsRefresh) {
    items.push({ label: t('printersView.refreshCertificate'), icon: PhArrowsClockwise, onSelect: () => void openTlsRefresh(activePrinter.value!.name) })
  }
  if (capabilities.value?.emergencyStop) {
    items.push({ label: t('dashboard.emergency'), icon: PhStop, danger: true, onSelect: () => void emergencyStop() })
  }
  items.push({ label: t('printersView.removePrinter'), icon: PhTrash, danger: true, onSelect: () => void removePrinter(activePrinter.value!.name) })
  return items
})

function fileActionItems(file: FileEntry): ActionMenuItem[] {
  const items: ActionMenuItem[] = []
  if (capabilities.value?.jobStart && selectedStatus.value?.state === 'idle') {
    items.push({ label: t('dashboard.print'), icon: PhPlay, onSelect: () => void sendJobAction('start', file.devicePath) })
  }
  return items
}

onMounted(() => {
  systemThemeQuery = window.matchMedia('(prefers-color-scheme: dark)')
  systemPrefersDark.value = systemThemeQuery.matches
  systemThemeQuery.addEventListener('change', handleSystemThemeChange)
  void load()
  monitorTimer = window.setInterval(() => void refreshMonitoring(), 5000)
})

onUnmounted(() => {
  systemThemeQuery?.removeEventListener('change', handleSystemThemeChange)
  if (monitorTimer !== undefined) window.clearInterval(monitorTimer)
})
</script>

<template>
  <div class="min-w-80 scheme-light dark:scheme-dark">
    <!-- Tabs with underline -->
    <header class="sticky top-0 z-20 bg-white/95 backdrop-blur dark:bg-gray-900/95">
      <div class="mx-auto max-w-[1500px] overflow-x-auto px-4 sm:px-8 lg:px-10">
        <nav class="-mb-px flex min-w-max items-stretch border-b border-gray-200 dark:border-white/10" :aria-label="t('nav.ariaLabel')">
          <div v-if="printers.length" class="flex items-center space-x-8 pr-8">
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
              <span class="size-1.5 shrink-0 rounded-full ring-3" :class="statusDotClasses[badgeFor(printer.name)]"></span>{{ printer.name }}
            </button>
          </div>
          <span v-if="printers.length" class="my-3 w-px shrink-0 bg-gray-200 dark:bg-white/15" aria-hidden="true"></span>
          <div class="flex items-center space-x-8 pl-8">
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

    <main class="mx-auto max-w-[1500px] py-7 sm:px-8 lg:px-10">
      <!-- Notification -->
      <div aria-live="assertive" class="pointer-events-none fixed inset-0 z-50 flex items-end px-4 py-6 sm:items-start sm:p-6">
        <div class="flex w-full flex-col items-center space-y-4 sm:items-end">
          <transition
            enter-active-class="transform ease-out duration-300 transition"
            enter-from-class="translate-y-2 opacity-0 sm:translate-y-0 sm:translate-x-2"
            enter-to-class="translate-y-0 sm:translate-x-0"
            leave-active-class="transition ease-in duration-100"
            leave-to-class="opacity-0"
          >
            <div
              v-if="toast"
              class="pointer-events-auto w-full max-w-sm rounded-lg bg-white shadow-lg outline-1 outline-black/5 dark:bg-gray-800 dark:-outline-offset-1 dark:outline-white/10"
            >
              <div class="p-4">
                <div class="flex items-start">
                  <div class="shrink-0">
                    <PhCheckCircle class="size-6 text-green-400" aria-hidden="true" />
                  </div>
                  <div class="ml-3 w-0 flex-1 pt-0.5">
                    <p class="text-sm font-medium text-gray-900 dark:text-white">{{ toast }}</p>
                  </div>
                  <div class="ml-4 flex shrink-0">
                    <button
                      type="button"
                      class="inline-flex rounded-md text-gray-400 hover:text-gray-500 dark:hover:text-white"
                      @click="toast = ''"
                    >
                      <span class="sr-only">{{ t('common.close') }}</span>
                      <PhX class="size-5" aria-hidden="true" />
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </transition>
        </div>
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
              class="inline-flex items-center gap-x-1.5 rounded-md bg-cyan-100 px-2 py-1 text-xs font-medium text-cyan-700 dark:bg-cyan-400/10 dark:text-cyan-400"
              :title="wifiSignal(wifiDbm).label"
            >
              <component :is="wifiSignal(wifiDbm).icon" class="size-3.5" />{{ wifiDbm }} dBm
            </span>
            <ActionMenu :label="t('control.printerActions')" :items="printerActionItems" />
          </div>
        </div>

        <!-- Empty state -->
        <div
          v-if="activeBadge === 'offline'"
          class="mx-4 flex min-h-140 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15"
        >
          <PhWarning class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
          <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('control.unavailableTitle') }}</h3>
          <p class="mt-1 max-w-md text-sm text-gray-500 dark:text-gray-400">{{ selectedMonitor?.error ?? t('control.unavailableDescription') }}</p>
          <Button class="mt-6" variant="primary" :disabled="refreshing" @click="refreshMonitoring"><PhArrowsClockwise class="size-4" /> {{ t('control.refreshConnection') }}</Button>
        </div>

        <div v-show="activeBadge !== 'offline'">
          <section class="grid gap-5 xl:grid-cols-4">
            <Card class="overflow-hidden xl:col-span-3">
              <CardHeader :title="t('camera.title')" :icon="PhVideoCamera">
                <template #suffix>
                  <span
                    class="inline-flex items-center gap-x-1.5 rounded-md px-2 py-1 text-xs font-medium"
                    :class="cameraOnline
                      ? 'bg-green-100 fill-green-500 text-green-700 dark:bg-green-400/10 dark:fill-green-400 dark:text-green-400'
                      : 'bg-yellow-100 fill-yellow-500 text-yellow-800 dark:bg-yellow-400/10 dark:fill-yellow-400 dark:text-yellow-500'"
                  >
                    <svg class="size-1.5" viewBox="0 0 6 6" aria-hidden="true"><circle cx="3" cy="3" r="3" /></svg>{{ cameraOnline ? t('camera.live') : t('status.offlineLabel') }}
                  </span>
                </template>
                <IconButton :title="t('camera.refresh')" :aria-label="t('camera.refresh')" :disabled="cameraLoading || !cameraSupported" @click="refreshCamera"><PhArrowsClockwise class="size-4" /></IconButton>
                <IconButton :title="t('camera.snapshot')" :aria-label="t('camera.snapshot')" :disabled="cameraLoading || !capabilities?.cameraSnapshot" @click="saveSnapshot"><PhCamera class="size-4" /></IconButton>
                <IconButton :title="t('camera.maximize')" :aria-label="t('camera.maximize')" :disabled="!cameraOnline" @click="toggleCameraFullscreen"><PhCornersOut class="size-4" /></IconButton>
              </CardHeader>
              <div
                ref="cameraStage"
                class="relative min-h-60 overflow-hidden sm:min-h-77.5"
                :class="cameraOnline ? 'bg-black' : 'grid place-items-center bg-gray-100 dark:bg-gray-800'"
              >
                <img v-if="cameraOnline" :src="cameraUrl" :alt="t('camera.title')" class="absolute inset-0 size-full object-contain" />
                <div v-else class="relative z-10 text-center">
                  <PhWarning class="mx-auto size-8 text-yellow-600 dark:text-yellow-400" />
                  <p class="mt-3 font-medium text-gray-900 dark:text-white">{{ t('camera.unavailable') }}</p>
                  <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ cameraError ?? t('camera.checkConnection') }}</p>
                  <Button v-if="cameraSupported" class="mt-4" :disabled="cameraLoading" @click="refreshCamera"><PhArrowsClockwise class="size-4" /> {{ t('camera.refreshFeed') }}</Button>
                </div>
              </div>
            </Card>

            <Card>
              <CardHeader :title="t('control.currentJob')" :icon="PhCheckSquareOffset">
                <span class="text-xs text-gray-500 dark:text-gray-400">{{ t('control.complete', { percent: progressPercent }) }}</span>
              </CardHeader>
              <div class="px-4 py-5 sm:p-6">
                <div class="flex items-start gap-3">
                  <div class="grid size-10 shrink-0 place-items-center rounded-lg bg-cyan-50 text-cyan-600 dark:bg-cyan-400/10 dark:text-cyan-400">
                    <PhHexagon v-if="!selectedStatus?.job" class="size-5" />
                    <PhCube v-else class="size-5" />
                  </div>
                  <div class="min-w-0">
                    <p class="truncate font-mono text-sm font-semibold text-gray-900 dark:text-white">{{ selectedStatus?.job?.name ?? t('dashboard.noJob') }}</p>
                    <p class="mt-1 font-mono text-xs text-gray-500 capitalize dark:text-gray-400">{{ selectedStatus?.state ?? 'unknown' }}</p>
                  </div>
                </div>
                <div class="mt-6">
                  <div class="mb-2 flex justify-between text-xs text-gray-500 dark:text-gray-400">
                    <span>{{ t('control.layer', { current: selectedStatus?.progress?.currentLayer ?? '—', total: selectedStatus?.progress?.totalLayers ?? '—' }) }}</span>
                  </div>
                  <div class="overflow-hidden rounded-full bg-gray-200 dark:bg-white/10">
                    <div class="h-2 rounded-full bg-cyan-600 dark:bg-cyan-500" :style="{ width: `${progressPercent}%` }"></div>
                  </div>
                </div>
                <div class="mt-6 flex gap-2">
                  <Button
                    v-if="selectedStatus?.state === 'paused'"
                    class="flex-1"
                    :disabled="!capabilities?.jobResume"
                    @click="sendJobAction('resume')"
                  ><PhPlay class="size-4" /> {{ t('dashboard.resume') }}</Button>
                  <Button
                    v-else
                    class="flex-1"
                    :disabled="!capabilities?.jobPause || selectedStatus?.state !== 'printing'"
                    @click="sendJobAction('pause')"
                  ><PhPause class="size-4" /> {{ t('dashboard.pause') }}</Button>
                  <Button
                    variant="danger"
                    :disabled="!capabilities?.jobCancel || !['printing', 'paused'].includes(selectedStatus?.state ?? '')"
                    @click="sendJobAction('cancel')"
                  ><PhStop class="size-4" /> {{ t('common.cancel') }}</Button>
                </div>
              </div>
            </Card>
          </section>

          <section class="mt-5 grid gap-5 lg:grid-cols-2 xl:grid-cols-4">
            <Card v-if="temperatureRows.length">
              <CardHeader :title="t('dashboard.temperature')" :icon="PhThermometerSimple" />
              <div class="font-light space-y-4 px-4 py-5 sm:p-6">
                <div v-for="row in temperatureRows" :key="row.key" class="flex items-center justify-between gap-3">
                  <div>
                    <p class="text-sm/6 font-light text-gray-900 dark:text-white">{{ t(temperatureKeys[row.key]) }}</p>
                    <p class="font-mono text-sm text-gray-500 dark:text-gray-400"><span class="font-bold">{{ formatTemperature(row.value.currentCelsius) }}</span> °C <span aria-hidden="true">/</span> <span class="font-bold">{{ row.value.targetCelsius === undefined ? '—' : formatTemperature(row.value.targetCelsius) }}</span> °C</p>
                  </div>
                  <div v-if="capabilities?.temperatureWrite && row.key !== 'chamber'" class="flex gap-1">
                    <IconButton variant="outline" :disabled="selectedStatus?.state !== 'idle'" :aria-label="t('control.decreaseTemp', { sensor: t(temperatureKeys[row.key]) })" @click="adjustTemperature(row.key, -5)"><PhMinus class="size-3.5" /></IconButton>
                    <IconButton variant="outline" :disabled="selectedStatus?.state !== 'idle'" :aria-label="t('control.increaseTemp', { sensor: t(temperatureKeys[row.key]) })" @click="adjustTemperature(row.key, 5)"><PhPlus class="size-3.5" /></IconButton>
                  </div>
                </div>
              </div>
            </Card>

            <Card v-if="fanRows.length">
              <CardHeader :title="t('control.fans')" :icon="PhFan" />
              <div class="font-light space-y-4 px-4 py-5 sm:p-6">
                <label v-for="[key, value] in fanRows" :key="key" class="block">
                  <span class="mb-2 flex justify-between">
                    <span class="text-sm/6 font-light text-gray-900 dark:text-white">{{ fanLabel(key) }}</span>
                    <span class="text-sm/6 text-gray-500 dark:text-gray-400">{{ value }}%</span>
                  </span>
                  <input :value="value" :disabled="!capabilities?.fanControl" class="h-1 w-full cursor-pointer accent-cyan-600 disabled:cursor-not-allowed disabled:opacity-35 dark:accent-cyan-400" type="range" min="0" max="100" :aria-label="t('control.fanPower', { fan: fanLabel(key) })" @change="sendFan(key, $event)" />
                </label>
              </div>
            </Card>

            <Card v-if="capabilities?.motionControl">
              <CardHeader :title="t('dashboard.motion')" :icon="PhArrowsOutCardinal" />
              <div class="font-light px-4 py-5 sm:p-6">
                <div class="mb-4 flex min-h-27 items-center justify-center gap-8">
                  <div class="grid grid-cols-3 grid-rows-3 gap-1">
                    <IconButton variant="outline" class="col-start-2 row-start-1" disabled :title="t('control.jogUnsupported')">Y+</IconButton>
                    <IconButton variant="outline" class="col-start-1 row-start-2" disabled :title="t('control.jogUnsupported')">X−</IconButton>
                    <IconButton variant="outline" class="col-start-2 row-start-2" :title="t('dashboard.home')" :aria-label="t('dashboard.home')" :disabled="selectedStatus?.state !== 'idle'" @click="homeAxes"><PhHouse class="size-4" /></IconButton>
                    <IconButton variant="outline" class="col-start-3 row-start-2" disabled :title="t('control.jogUnsupported')">X+</IconButton>
                    <IconButton variant="outline" class="col-start-2 row-start-3" disabled :title="t('control.jogUnsupported')">Y−</IconButton>
                  </div>
                  <div class="flex flex-col gap-1">
                    <IconButton variant="outline" disabled :title="t('control.jogUnsupported')">Z+</IconButton>
                    <IconButton variant="outline" disabled :title="t('control.jogUnsupported')">Z−</IconButton>
                  </div>
                </div>
                <div class="grid grid-cols-2 gap-2">
                  <div>
                    <label for="step-size" class="block text-sm/6 font-light text-gray-900 dark:text-white">{{ t('control.stepSize') }}</label>
                    <div class="mt-2 grid grid-cols-1">
                      <select id="step-size" v-model="stepSize" disabled class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 disabled:opacity-50 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
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
                      <select id="feed-rate" v-model="feedRate" disabled class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 disabled:opacity-50 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
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
                <IconButton :title="t('dashboard.loadFiles')" :aria-label="t('dashboard.loadFiles')" :disabled="filesLoading" @click="loadFiles"><PhArrowsClockwise class="size-4" /></IconButton>
              </CardHeader>
              <div class="overflow-x-auto">
                <table class="relative min-w-full divide-y divide-gray-300 text-left dark:divide-white/15">
                  <thead>
                    <tr>
                      <th v-for="column in [t('filesView.type'), t('filesView.size'), t('filesView.modified'), t('filesView.name')]" :key="column" scope="col" class="px-4 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase sm:px-6 dark:text-gray-400">{{ column }}</th>
                      <th scope="col" class="px-4 py-3 text-right text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase sm:px-6 dark:text-gray-400">{{ t('filesView.actions') }}</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-gray-200 dark:divide-white/10">
                    <tr
                      v-for="file in printerFiles.slice(0, 3)"
                      :key="file.devicePath"
                      class="cursor-pointer"
                      :class="selectedFile === file.devicePath ? 'bg-cyan-50 dark:bg-cyan-400/5' : 'hover:bg-gray-50 dark:hover:bg-white/5'"
                      @click="selectedFile = file.devicePath"
                    >
                      <td class="px-4 py-4 text-sm whitespace-nowrap sm:px-6">
                        <span class="inline-flex items-center rounded-md bg-gray-100 px-2 py-1 text-xs font-medium text-gray-600 dark:bg-gray-400/10 dark:text-gray-400">{{ fileTypeLabel(file) }}</span>
                      </td>
                      <td class="px-4 py-4 font-mono text-sm whitespace-nowrap text-gray-500 sm:px-6 dark:text-gray-400">{{ formatSize(file.sizeBytes) }}</td>
                      <td class="px-4 py-4 font-mono text-sm whitespace-nowrap text-gray-500 sm:px-6 dark:text-gray-400">{{ file.modifiedAt ?? '—' }}</td>
                      <td class="px-4 py-4 font-mono text-sm font-medium whitespace-nowrap text-gray-900 sm:px-6 dark:text-white">{{ file.name }}</td>
                      <td class="px-4 py-4 text-sm whitespace-nowrap sm:px-6">
                        <div class="flex justify-end gap-1">
                          <IconButton
                            class="text-cyan-600 dark:text-cyan-400"
                            :title="t('filesView.printFile')"
                            :aria-label="t('filesView.printNamed', { name: file.name })"
                            :disabled="!capabilities?.jobStart || selectedStatus?.state !== 'idle'"
                            @click.stop="sendJobAction('start', file.devicePath)"
                          ><PhPlay class="size-4" /></IconButton>
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
                    <div v-if="system.temperature !== undefined || system.humidity" class="flex items-center gap-3 font-mono text-xs text-gray-500 dark:text-gray-400">
                      <span v-if="system.humidity" class="flex items-center gap-1"><PhDrop class="size-4" /> {{ system.humidity }}</span>
                      <span v-if="system.temperature !== undefined" class="flex items-center gap-1"><PhThermometerSimple class="size-4" /> {{ formatTemperature(system.temperature) }} °C</span>
                    </div>
                  </div>
                  <div class="flex flex-wrap gap-3">
                    <div
                      v-for="material in system.slots"
                      :key="`${system.name}-${material.slot}`"
                      class="relative size-18 shrink-0 overflow-hidden rounded-md border border-gray-300 bg-gray-100 dark:border-white/15 dark:bg-white/10"
                    >
                      <span class="absolute inset-x-0 bottom-0 transition-[height] duration-200" :style="{ height: `${material.remainingPercent ?? 100}%`, backgroundColor: material.color }"></span>
                      <span class="relative z-10 flex h-full flex-col items-center justify-center gap-1 p-1 text-center font-mono" :style="{ color: textColorFor(material.color) }">
                        <span class="text-xs font-bold">{{ material.slot }}</span>
                        <span v-if="material.type !== '—'" class="text-[9px] font-semibold tracking-wide uppercase wrap-anywhere">{{ material.type }}</span>
                      </span>
                    </div>
                  </div>
                </div>
              </div>
            </Card>
          </section>
        </div>
      </template>

      <template v-else-if="activeView === 'printers' || !hasPrinters">
        <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
          <div class="min-w-0 flex-1">
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ t('printersView.title') }}</h2>
            <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">{{ t('printersView.description') }}</p>
          </div>
          <div class="mt-5 flex lg:mt-0 lg:ml-4 max-lg:w-full">
            <Button variant="primary" class="max-lg:w-full" :disabled="!drivers.length" @click="openAddition"><PhBroadcast class="size-4" /> {{ t('printersView.discover') }}</Button>
          </div>
        </div>

        <!-- Stats -->
        <dl v-if="hasPrinters" class="mb-5 grid grid-cols-1 gap-5 sm:grid-cols-3">
          <Card v-for="stat in fleetStats" :key="stat.label" as="div" class="px-4 py-5 sm:p-6">
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
          <Button class="mt-6" variant="primary" @click="load"><PhArrowsClockwise class="size-4" /> {{ t('common.retry') }}</Button>
        </div>

        <!-- Empty state -->
        <div v-else-if="!hasPrinters" class="mx-4 flex min-h-97.5 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15">
          <PhPrinter class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
          <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('printersView.emptyTitle') }}</h3>
          <p class="mt-1 max-w-sm text-sm text-gray-500 dark:text-gray-400">{{ t('printersView.emptyDescription') }}</p>
          <Button class="mt-6" variant="primary" :disabled="!drivers.length" @click="openAddition"><PhBroadcast class="size-4" /> {{ t('printersView.scan') }}</Button>
        </div>

        <div v-else class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
          <Card v-for="printer in printers" :key="printer.name" as="article" class="px-4 py-5 sm:p-6">
            <div class="flex items-start justify-between">
              <div class="flex items-center gap-3">
                <span class="grid size-10 place-items-center rounded-lg bg-gray-100 text-cyan-600 dark:bg-white/10 dark:text-cyan-400"><PhPrinter class="size-5" /></span>
                <div>
                  <h3 class="font-semibold text-gray-900 dark:text-white">{{ printer.name }}</h3>
                  <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ printer.driver }}</p>
                </div>
              </div>
              <IconButton class="hover:text-red-600 dark:hover:text-red-400" :title="t('printersView.removePrinter')" :aria-label="t('printersView.removeNamed', { name: printer.name })" @click="removePrinter(printer.name)"><PhTrash class="size-4" /></IconButton>
            </div>
            <div class="mt-6 flex items-center justify-between border-y border-gray-200 py-3 dark:border-white/10">
              <span class="text-xs text-gray-500 dark:text-gray-400">{{ t('printersView.status') }}</span>
              <StatusBadge :status="badgeFor(printer.name)" :label="statusLabel(badgeFor(printer.name))" />
            </div>
            <!-- Description list -->
            <dl class="mt-2 divide-y divide-gray-200 text-xs dark:divide-white/10">
              <div class="flex items-center justify-between py-2">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('addition.serial') }}</dt>
                <dd class="text-gray-900 dark:text-gray-300">—</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('printersView.host') }}</dt>
                <dd class="text-gray-900 dark:text-gray-300">{{ printer.host }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('addition.timeout') }}</dt>
                <dd class="text-gray-900 dark:text-gray-300">—</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('printersView.tlsVerification') }}</dt>
                <dd class="text-gray-900 dark:text-gray-300">—</dd>
              </div>
            </dl>
            <div class="mt-5 flex gap-2">
              <Button class="flex-1" @click="selectPrinter(printer.name)"><PhCards class="size-4" /> {{ t('printersView.openControl') }}</Button>
              <Button class="flex-1" :disabled="tlsRefreshing" :aria-label="t('printersView.refreshCertificateNamed', { name: printer.name })" @click="openTlsRefresh(printer.name)"><PhArrowsClockwise class="size-4" /> {{ t('printersView.refreshCertificate') }}</Button>
            </div>
          </Card>
          <button
            type="button"
            class="relative mx-4 flex min-h-55 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 p-12 text-center hover:border-gray-400 focus:outline-2 focus:outline-offset-2 focus:outline-cyan-600 sm:mx-0 dark:border-white/15 dark:hover:border-white/25 dark:focus:outline-cyan-500"
            @click="openAddition"
          >
            <PhPlus class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
            <span class="mt-2 block text-sm font-semibold text-gray-900 dark:text-white">{{ t('printersView.addAnother') }}</span>
          </button>
        </div>
      </template>

      <template v-else-if="activeView === 'settings'">
        <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
          <div class="min-w-0 flex-1">
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ t('nav.settings') }}</h2>
            <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">{{ t('settingsView.description') }}</p>
          </div>
        </div>
        <div class="grid max-w-[1500px] gap-5 lg:grid-cols-2 xl:grid-cols-3">
          <!-- Toggle lists -->
          <Card as="section" class="overflow-hidden">
            <CardHeader :title="t('settingsView.notifications')" :icon="PhBellRinging" />
            <div class="divide-y divide-gray-200 dark:divide-white/10">
              <div v-for="notification in notifications" :key="notification.label" class="flex items-center justify-between gap-3 px-4 py-5 hover:bg-gray-50 sm:px-6 dark:hover:bg-white/5">
                <span class="flex grow flex-col">
                  <span class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ t(notification.label) }}</span>
                  <span class="text-sm text-gray-500 dark:text-gray-400">{{ t(notification.description) }}</span>
                </span>
                <Switch v-model="notification.enabled" :label="t(notification.label)" />
              </div>
            </div>
          </Card>
          <Card as="section">
            <CardHeader :title="t('settingsView.appearance')" :icon="PhSun" />
            <div class="px-4 py-5 sm:p-6">
              <div>
                <label for="theme" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('settingsView.theme') }}</label>
                <div class="mt-2 grid grid-cols-1">
                  <select id="theme" v-model="theme" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
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
                  <select id="language" v-model="locale" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
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
              <p v-if="diagnosticsError" class="mt-3 text-xs text-red-600 dark:text-red-400" role="alert">{{ diagnosticsError }}</p>
              <Button class="mt-4 w-full" :disabled="diagnosticsLoading" @click="runDiagnostics"><PhDesktop class="size-4" /> {{ diagnosticsLoading ? t('common.loading') : t('diagnostics.open') }}</Button>
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
          <div class="mt-5 flex lg:mt-0 lg:ml-4 max-lg:w-full">
            <Button class="max-lg:w-full" :disabled="filesLoading || !capabilities?.fileList" @click="loadFiles"><PhArrowsClockwise class="size-4" /> {{ t('dashboard.refresh') }}</Button>
          </div>
        </div>

        <div class="mb-5 flex flex-wrap items-center gap-2 px-4 sm:px-0">
          <!-- Breadcrumbs -->
          <nav class="flex" :aria-label="t('filesView.breadcrumb')">
            <ol role="list" class="flex items-center space-x-4">
              <li v-for="(breadcrumb, index) in breadcrumbs" :key="breadcrumb.path" class="flex items-center">
                <PhCaretRight v-if="index > 0" class="mr-4 size-5 shrink-0 text-gray-400 dark:text-gray-500" />
                <button
                  type="button"
                  class="text-sm font-medium text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
                  :class="breadcrumb.path === currentDirectory ? 'text-gray-900 dark:text-white' : ''"
                  :aria-current="breadcrumb.path === currentDirectory ? 'page' : undefined"
                  @click="navigateToDirectory(breadcrumb.path)"
                >{{ breadcrumb.name }}</button>
              </li>
            </ol>
          </nav>
          <!-- Input with leading icon -->
          <div class="ml-auto grid grid-cols-1">
            <input
              class="col-start-1 row-start-1 block w-40 rounded-md bg-white py-1.5 pr-3 pl-10 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 placeholder:text-gray-400 focus:outline-2 focus:-outline-offset-2 focus:outline-cyan-600 sm:w-64 sm:pl-9 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:placeholder:text-gray-500"
              :aria-label="t('filesView.search')"
              :placeholder="t('filesView.search')"
              type="search"
            />
            <PhMagnifyingGlass class="pointer-events-none col-start-1 row-start-1 ml-3 size-5 self-center text-gray-400 sm:size-4 dark:text-gray-500" aria-hidden="true" />
          </div>
        </div>

        <div v-if="visibleDirectories.length" class="mb-5 grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <button
            v-for="directory in visibleDirectories"
            :key="directory.devicePath"
            type="button"
            class="min-h-28 bg-white px-4 py-5 text-left shadow-sm transition hover:bg-cyan-50 hover:shadow-md sm:rounded-lg sm:p-6 dark:bg-gray-800/50 dark:shadow-none dark:outline dark:-outline-offset-1 dark:outline-white/10 dark:hover:bg-cyan-400/5 dark:hover:outline-cyan-400/30"
            @click="navigateToDirectory(`/${directory.path.replace(/^\//, '')}`)"
          >
            <PhFolder class="size-7 text-cyan-600 dark:text-cyan-400" />
            <span class="mt-3 block truncate text-sm font-medium text-gray-900 dark:text-white">{{ directory.name }}</span>
            <span class="mt-1 block text-xs text-gray-500 dark:text-gray-400">{{ t('filesView.folderModified', { date: directory.modifiedAt ?? '—' }) }}</span>
          </button>
        </div>

        <!-- Grid list -->
        <div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <Card
            v-for="file in visibleFiles"
            :key="file.devicePath"
            as="article"
            class="transition hover:shadow-md dark:hover:outline-cyan-400/30"
          >
            <div class="relative grid min-h-45 place-items-center overflow-hidden sm:rounded-t-lg" :class="fileToneFor(file.name).preview">
              <span class="block transform-[perspective(200px)_rotateX(10deg)_rotateZ(-8deg)]" :class="fileToneFor(file.name).shape"></span>
              <span class="absolute right-3 bottom-2.5 text-[10px] font-bold text-gray-500 uppercase dark:text-slate-300/45">.{{ fileTypeLabel(file).toLowerCase() }}</span>
            </div>
            <div class="flex items-start gap-3 px-4 py-5 sm:p-6">
              <PhFile class="mt-0.5 size-4 shrink-0 text-gray-400 dark:text-gray-500" />
              <div class="min-w-0 flex-1">
                <h3 class="truncate text-sm font-medium text-gray-900 dark:text-white">{{ file.name }}</h3>
                <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ formatSize(file.sizeBytes) }} · {{ file.modifiedAt ?? '—' }}</p>
              </div>
              <ActionMenu v-if="fileActionItems(file).length" class="-mt-2 -mr-2" :label="t('filesView.moreActions')" :items="fileActionItems(file)" />
            </div>
          </Card>
          <div v-if="!visibleDirectories.length && !visibleFiles.length" class="col-span-full mx-4 flex min-h-65 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15">
            <PhFolder class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
            <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('filesView.emptyTitle') }}</h3>
            <p class="mt-1 text-sm text-gray-500 dark:text-gray-400">{{ filesLoading ? t('common.loading') : filesError ?? (capabilities?.fileList ? t('filesView.emptyDescription') : t('filesView.notSupported')) }}</p>
          </div>
        </div>
      </template>
    </main>

    <SlideOver :open="additionOpen" :title="t('addition.title')" :description="t('addition.description')" :close-label="t('common.closePanel')" @close="closeAddition">
      <form id="add-printer-form" @submit.prevent="addPrinter">
        <Button class="w-full" :disabled="discovering" @click="discoverPrinters"><PhBroadcast class="size-4" /> {{ discovering ? t('printersView.scanning') : t('printersView.discover') }}</Button>
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
            <input id="printer-name" v-model="draft.name" required maxlength="64" autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
          </div>
          <div>
            <label for="printer-driver" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.driver') }}</label>
            <div class="mt-2 grid grid-cols-1">
              <select id="printer-driver" v-model="draft.driver" required class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
                <option v-for="driver in drivers" :key="driver.name" :value="driver.name">{{ driver.name }}</option>
              </select>
              <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
            </div>
          </div>
          <div>
            <label for="printer-host" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.host') }}</label>
            <input id="printer-host" v-model="draft.host" required autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
          </div>
          <div>
            <label for="printer-serial" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.serial') }}</label>
            <input id="printer-serial" v-model="draft.serial" autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
          </div>
          <div>
            <label for="printer-timeout" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.timeout') }}</label>
            <input id="printer-timeout" v-model="draft.timeout" required class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
          </div>
          <div>
            <label for="printer-access-code" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.accessCode') }}</label>
            <input id="printer-access-code" v-model="draft.accessCode" type="password" autocomplete="new-password" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
          </div>
          <div class="flex items-center justify-between">
            <span class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.insecure') }}</span>
            <Switch v-model="draft.insecure" :label="t('addition.insecure')" />
          </div>
        </div>
        <p v-if="additionError" class="mt-4 text-sm text-red-600 dark:text-red-400" role="alert">{{ additionError }}</p>
      </form>
      <template #footer>
        <Button :disabled="adding" @click="closeAddition">{{ t('common.cancel') }}</Button>
        <Button variant="primary" type="submit" form="add-printer-form" :disabled="adding">{{ adding ? t('common.adding') : t('profiles.add') }}</Button>
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
  </div>
</template>
