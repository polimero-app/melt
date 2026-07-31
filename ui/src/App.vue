<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watchEffect, type Component } from 'vue'
import StatusBadge from './components/StatusBadge.vue'
import ActionMenu, { type ActionMenuItem } from './components/ActionMenu.vue'
import SlideOver from './components/SlideOver.vue'
import Button from './components/Button.vue'
import IconButton from './components/IconButton.vue'
import Switch from './components/Switch.vue'
import Card from './components/Card.vue'
import CardHeader from './components/CardHeader.vue'
import alaska from './fixtures/alaska.json'
import dakota from './fixtures/dakota.json'
import georgia from './fixtures/georgia.json'
import {
  PhArrowSquareOut,
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
  PhDownloadSimple,
  PhDrop,
  PhFan,
  PhFile,
  PhFolder,
  PhGearSix,
  PhHexagon,
  PhHouse,
  PhLightbulb,
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
  PhUploadSimple,
  PhVideoCamera,
  PhWarning,
  PhWifiHigh,
  PhWifiLow,
  PhWifiMedium,
  PhWifiSlash,
  PhX,
} from '@phosphor-icons/vue'

type View = 'control' | 'printers' | 'settings' | 'files'
type PrinterStatus = 'idle' | 'busy' | 'offline'
type TemperatureKey = 'nozzle' | 'bed' | 'chamber'
type MaterialSystem = 'external' | 'AMS' | 'CFS' | 'BMCU'
type ModelTone = 'cyan' | 'amber' | 'rose' | 'violet'

interface MaterialSetup {
  system: MaterialSystem
  units: number
  slots: 1 | 4
}

interface RawTray {
  slot: number
  filamentType?: string
  color?: string
  remainingPercent?: number
}

interface RawMaterialUnit {
  temperature?: number
  humidityLevel?: string
  trays?: RawTray[]
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

type PrinterData = Record<string, any>
const printerSources: PrinterData[] = [alaska.data, dakota.data, georgia.data]

interface Printer {
  id: string
  name: string
  model: string
  status: PrinterStatus
  ip: string
  host: string
  serial: string
  timeout: string
  insecure: boolean
  progress: number
  materialSetup: MaterialSetup
  data: PrinterData
}

const emptySlotColor = 'var(--color-gray-500)'

const statusDotClasses: Record<PrinterStatus, string> = {
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

function materialSetupFor(data: PrinterData): MaterialSetup {
  const units = data.extensions?.['bambu-lan']?.ams?.units ?? []
  const firstUnitSlots = units[0]?.trays?.length === 1 ? 1 : 4
  return { system: units.length ? 'AMS' : 'external', units: units.length || 1, slots: firstUnitSlots }
}

const printers = ref<Printer[]>(printerSources.map((data) => ({
  id: data.profile,
  name: data.profile[0].toUpperCase() + data.profile.slice(1),
  model: data.profile === 'alaska' || data.profile === 'georgia' ? 'Bambu Lab A1 Mini' : data.profile === 'dakota' ? 'Bambu Lab H2C' : data.driver,
  status: data.profile === 'georgia' ? 'offline' : data.state === 'printing' ? 'busy' : 'idle',
  ip: data.profile === 'alaska' ? '10.20.20.5' : data.profile === 'dakota' ? '10.20.20.10' : '10.20.20.6',
  host: data.profile === 'alaska' ? '10.20.20.5' : data.profile === 'dakota' ? '10.20.20.10' : '10.20.20.6',
  serial: data.profile === 'alaska' ? '0300HA622100768' : data.profile === 'dakota' ? '31B8AP5B1301209' : '0300HA5C1600431',
  timeout: '10s',
  insecure: false,
  progress: data.progress.percent,
  materialSetup: materialSetupFor(data),
  data,
})))
const activeView = ref<View>('control')
const sectionTabs: { view: View; label: string; icon: Component }[] = [
  { view: 'printers', label: 'Manage printers', icon: PhPrinter },
  { view: 'settings', label: 'Configuration', icon: PhGearSix },
  { view: 'files', label: 'Files', icon: PhFolder },
]
const activePrinterId = ref(printers.value[0]?.id ?? '')
const theme = ref<'light' | 'dark' | 'system'>('system')
const systemPrefersDark = ref(true)
let systemThemeQuery: MediaQueryList | undefined
function handleSystemThemeChange(event: MediaQueryListEvent) {
  systemPrefersDark.value = event.matches
}
onMounted(() => {
  systemThemeQuery = window.matchMedia('(prefers-color-scheme: dark)')
  systemPrefersDark.value = systemThemeQuery.matches
  systemThemeQuery.addEventListener('change', handleSystemThemeChange)
})
onUnmounted(() => {
  systemThemeQuery?.removeEventListener('change', handleSystemThemeChange)
})
const isLightTheme = computed(() => theme.value === 'light' || (theme.value === 'system' && !systemPrefersDark.value))
watchEffect(() => {
  document.documentElement.classList.toggle('dark', !isLightTheme.value)
})
const language = ref('English')
const cameraOnline = ref(true)
const cameraStage = ref<HTMLElement>()
const jobPaused = ref(false)
const jobCancelled = ref(false)
const toast = ref('')
const selectedFile = ref('bracket-v4.3mf')
const currentDirectory = ref('/')
const uploadOpen = ref(false)
const uploadPrinterId = ref('')

function openUpload() {
  uploadPrinterId.value = activePrinter.value?.id ?? ''
  uploadOpen.value = true
}

function confirmUpload() {
  uploadOpen.value = false
  const target = printers.value.find((printer) => printer.id === uploadPrinterId.value)
  showToast(target ? `Files uploaded to ${target.name} · ${currentDirectory.value}` : `Files uploaded to ${currentDirectory.value}`)
}
const stepSize = ref('1 mm')
const feedRate = ref(50)

function temperaturesFor(data: PrinterData) {
  return {
    nozzle: { current: data.temperatures.nozzle.currentCelsius, target: data.temperatures.nozzle.targetCelsius },
    bed: { current: data.temperatures.bed.currentCelsius, target: data.temperatures.bed.targetCelsius },
    chamber: { current: data.temperatures.chamber.currentCelsius, target: data.temperatures.chamber.targetCelsius ?? 0 },
  }
}

const temperature = ref(temperaturesFor(printerSources[0]))
const fanPower = ref({ ...printerSources[0].fans })
const fanLabel = (key: string) => (key === 'partCooling' ? 'Part cooling' : key)
const lights = ref({ chamber: printerSources[0].lights.chamber_light === 'on', aux: false })
const notifications = ref([
  { label: 'Print complete', description: 'Show a notification when a print finishes.', enabled: true },
  { label: 'Print failure', description: 'Show a notification when a print fails or is aborted.', enabled: true },
  { label: 'Printer disconnected', description: 'Show a notification when the connection is lost.', enabled: true },
])
const slicers = ref([
  { name: 'Bambu Studio', path: '/usr/bin/bambustudio', enabled: true },
  { name: 'Orca Slicer', path: '/usr/bin/orcaslicer', enabled: true },
  { name: 'PrusaSlicer', path: '/usr/bin/prusa-slicer', enabled: false },
])

const activePrinter = computed(() => printers.value.find((printer) => printer.id === activePrinterId.value) ?? printers.value[0])
const hasPrinters = computed(() => printers.value.length > 0)
const fleetStats = computed(() => [
  { label: 'Total printers', value: printers.value.length, tone: 'text-gray-900 dark:text-white' },
  { label: 'Online', value: printers.value.filter((printer) => printer.status !== 'offline').length, tone: 'text-green-600 dark:text-green-400' },
  { label: 'Printing now', value: printers.value.filter((printer) => printer.status === 'busy').length, tone: 'text-yellow-600 dark:text-yellow-400' },
])
function showToast(message: string) {
  toast.value = message
  window.setTimeout(() => {
    toast.value = ''
  }, 2200)
}

function goTo(view: View) {
  activeView.value = view
}

function selectPrinter(id: string) {
  activePrinterId.value = id
  const printer = printers.value.find((item) => item.id === id)
  if (printer) {
    temperature.value = temperaturesFor(printer.data)
    if (printer.status === 'offline') {
      temperature.value = { nozzle: { current: 0, target: 0 }, bed: { current: 0, target: 0 }, chamber: { current: 0, target: 0 } }
      fanPower.value = { auxiliary: 0, chamber: 0, heatbreak: 0, partCooling: 0 }
      lights.value = { chamber: false, aux: false }
    } else {
      fanPower.value = { ...printer.data.fans }
      lights.value = { chamber: printer.data.lights.chamber_light === 'on', aux: printer.data.lights.work_light === 'on' }
    }
    cameraOnline.value = printer.status !== 'offline'
  }
  activeView.value = 'control'
}

function discoverPrinter() {
  if (printers.value.some((printer) => printer.id === 'p3')) {
    showToast('Discovery scan complete')
    return
  }
  printers.value.push({ id: 'p3', name: 'Workshop MMU', model: 'Open-source BMCU', status: 'offline', ip: '10.20.20.44', host: '10.20.20.44', serial: 'Not reported', timeout: '10s', insecure: false, progress: 0, materialSetup: { system: 'BMCU', units: 1, slots: 4 }, data: alaska.data })
  showToast('1 printer discovered')
}

function removePrinter(id: string) {
  printers.value = printers.value.filter((printer) => printer.id !== id)
  if (activePrinterId.value === id) {
    activePrinterId.value = printers.value[0]?.id ?? ''
    if (!printers.value.length) activeView.value = 'printers'
  }
  showToast('Printer removed')
}

const printerActionItems = computed<ActionMenuItem[]>(() => activePrinter.value ? [
  { label: 'Refresh certificate', icon: PhArrowsClockwise, onSelect: () => showToast('TLS certificate refresh requested') },
  { label: 'Remove printer', icon: PhTrash, danger: true, onSelect: () => removePrinter(activePrinter.value!.id) },
] : [])

function fileActionItems(file: { name: string }): ActionMenuItem[] {
  return [
    { label: 'Print', icon: PhPlay, onSelect: () => showToast(`${file.name} sent to printer`) },
    { label: 'Open with Bambu Studio', icon: PhArrowSquareOut, onSelect: () => showToast(`Opening ${file.name} in Bambu Studio`) },
    { label: 'Delete', icon: PhTrash, danger: true, onSelect: () => showToast(`${file.name} deleted`) },
  ]
}

function refreshCamera() {
  cameraOnline.value = true
  showToast('Camera connection restored')
}

function saveSnapshot() {
  showToast('Snapshot saved to file library')
}

function toggleCameraFullscreen() {
  if (document.fullscreenElement) document.exitFullscreen()
  else cameraStage.value?.requestFullscreen()
}

function move(axis: string, direction: string) {
  showToast(`${axis}${direction} moved ${stepSize.value}`)
}

function updateTemperature(key: TemperatureKey, amount: number) {
  temperature.value[key].target = Math.max(0, temperature.value[key].target + amount)
}

function formatTemperature(value: number) {
  return Math.round(value)
}

function wifiSignal(dbm: number | undefined) {
  if (dbm === undefined) return { icon: PhWifiSlash, label: 'No signal' }
  if (dbm >= -55) return { icon: PhWifiHigh, label: 'Excellent signal' }
  if (dbm >= -70) return { icon: PhWifiMedium, label: 'Good signal' }
  return { icon: PhWifiLow, label: 'Weak signal' }
}

function toggleJob() {
  jobPaused.value = !jobPaused.value
  showToast(jobPaused.value ? 'Job paused' : 'Job resumed')
}

const files: { name: string; type: string; size: string; modified: string; tone: ModelTone; directory: string }[] = [
  { name: 'bracket-v4.3mf', type: '3MF', size: '18.4 MB', modified: 'Today, 09:42', tone: 'cyan', directory: '/' },
  { name: 'gear-housing.stl', type: 'STL', size: '4.8 MB', modified: 'Yesterday', tone: 'amber', directory: '/' },
  { name: 'mounting-arm.obj', type: 'OBJ', size: '2.1 MB', modified: 'Jun 18, 2026', tone: 'rose', directory: '/' },
  { name: 'enclosure-panel.stl', type: 'STL', size: '12.6 MB', modified: 'Jun 15, 2026', tone: 'violet', directory: '/' },
  { name: 'hinge-v2.3mf', type: '3MF', size: '8.7 MB', modified: 'Jun 12, 2026', tone: 'cyan', directory: '/Projects' },
  { name: 'mounting-arm-final.obj', type: 'OBJ', size: '3.3 MB', modified: 'Jun 08, 2026', tone: 'rose', directory: '/Projects' },
  { name: 'bed-level-test.stl', type: 'STL', size: '840 KB', modified: 'May 27, 2026', tone: 'amber', directory: '/Calibration' },
]

const directories = [
  { name: 'Projects', path: '/Projects', parent: '/', modified: 'Today' },
  { name: 'Calibration', path: '/Calibration', parent: '/', modified: 'Jun 20, 2026' },
  { name: 'Archive', path: '/Archive', parent: '/', modified: 'May 04, 2026' },
]

const visibleDirectories = computed(() => directories.filter((directory) => directory.parent === currentDirectory.value))
const visibleFiles = computed(() => files.filter((file) => file.directory === currentDirectory.value))
const breadcrumbs = computed(() => {
  const parts = currentDirectory.value.split('/').filter(Boolean)
  return [{ name: 'File library', path: '/' }, ...parts.map((part, index) => ({ name: part, path: `/${parts.slice(0, index + 1).join('/')}` }))]
})

function navigateToDirectory(path: string) {
  currentDirectory.value = path
}

const materials = computed<MaterialSlot[]>(() => {
  if (activePrinter.value?.status === 'offline') return [{ slot: 'EXT', status: 'Empty', type: '—', name: '', color: emptySlotColor }]
  const units = (activePrinter.value?.data.extensions?.['bambu-lan']?.ams?.units ?? []) as RawMaterialUnit[]
  const trays = units.flatMap((unit) => unit.trays ?? [])
  if (!trays.length) return [{ slot: 'EXT', status: 'In use', type: 'PLA', name: 'PLA', color: '#58c49b' }]
  return trays.map(materialFromTray)
})

function textColorFor(color: string) {
  if (!color.startsWith('#')) return 'var(--color-white)'
  const r = parseInt(color.slice(1, 3), 16)
  const g = parseInt(color.slice(3, 5), 16)
  const b = parseInt(color.slice(5, 7), 16)
  const luminance = (0.299 * r + 0.587 * g + 0.114 * b) / 255
  return luminance > 0.55 ? 'var(--color-gray-900)' : 'var(--color-white)'
}

function materialFromTray(tray: RawTray): MaterialSlot {
  const knownPercent = tray.remainingPercent !== undefined && tray.remainingPercent !== -1
  return {
    slot: String(tray.slot + 1).padStart(2, '0'),
    status: knownPercent ? `${tray.remainingPercent}%` : tray.filamentType ? (activePrinter.value?.status === 'busy' ? 'Active' : 'Ready') : 'Empty',
    type: tray.filamentType ?? '—',
    name: tray.filamentType ?? '',
    color: tray.color ? `#${tray.color.slice(0, 6)}` : emptySlotColor,
    remainingPercent: knownPercent ? tray.remainingPercent : undefined,
  }
}

const materialSystems = computed<MaterialSystemView[]>(() => {
  if (!activePrinter.value || activePrinter.value.status === 'offline' || activePrinter.value.materialSetup.system === 'external') {
    return [{ name: 'External spool', slots: materials.value }]
  }
  const setup = activePrinter.value.materialSetup
  const units = (activePrinter.value.data.extensions?.['bambu-lan']?.ams?.units ?? []) as RawMaterialUnit[]
  if (activePrinter.value.data.profile === 'alaska' || activePrinter.value.data.profile === 'georgia') {
    return [
      { name: 'AMS 1', temperature: units[0]?.temperature, humidity: units[0]?.humidityLevel, slots: (units[0]?.trays ?? []).map(materialFromTray) },
      { name: 'External spool', slots: [{ slot: 'EXT', status: 'Empty', type: '—', name: '', color: emptySlotColor }] },
    ]
  }
  return Array.from({ length: setup.units }, (_, index) => ({
    name: setup.units > 1 ? `${setup.system} ${index + 1}` : setup.system,
    temperature: units[index]?.temperature,
    humidity: units[index]?.humidityLevel,
    slots: (units[index]?.trays ?? []).map(materialFromTray),
  }))
})
const printerFiles = computed(() => activePrinter.value?.status === 'offline' ? [] : files)
</script>

<template>
  <div class="min-w-80 scheme-light dark:scheme-dark">
    <!-- Tabs with underline -->
    <header class="sticky top-0 z-20 bg-white/95 backdrop-blur dark:bg-gray-900/95">
      <div class="mx-auto max-w-[1500px] overflow-x-auto px-4 sm:px-8 lg:px-10">
        <nav class="-mb-px flex min-w-max items-stretch border-b border-gray-200 dark:border-white/10" aria-label="Workspace navigation">
          <div v-if="printers.length" class="flex items-center space-x-8 pr-8">
            <button
              v-for="printer in printers"
              :key="printer.id"
              type="button"
              class="group inline-flex items-center gap-x-2 border-b-2 px-1 py-4 text-sm font-medium whitespace-nowrap transition"
              :class="activeView === 'control' && activePrinterId === printer.id
                ? 'border-cyan-500 text-cyan-600 dark:border-cyan-400 dark:text-cyan-400'
                : 'border-transparent text-gray-500 hover:border-gray-300 hover:text-gray-800 dark:text-gray-400 dark:hover:border-white/20 dark:hover:text-gray-200'"
              :aria-current="activeView === 'control' && activePrinterId === printer.id ? 'page' : undefined"
              @click="selectPrinter(printer.id)"
            >
              <span class="size-1.5 shrink-0 rounded-full ring-3" :class="statusDotClasses[printer.status]"></span>{{ printer.name }}
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
                      <span class="sr-only">Close</span>
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
                {{ activePrinter.model }}
              </div>
              <div class="mt-2 flex items-center font-mono text-sm text-gray-500 dark:text-gray-400">
                <PhNetwork class="mr-1.5 size-5 shrink-0 text-gray-400 dark:text-gray-500" aria-hidden="true" />
                {{ activePrinter.ip }}
              </div>
            </div>
          </div>
          <div class="mt-5 flex lg:mt-0 lg:ml-4 items-center gap-2">
            <StatusBadge :status="activePrinter.status" />
            <span
              v-if="activePrinter.status !== 'offline'"
              class="inline-flex items-center gap-x-1.5 rounded-md bg-cyan-100 px-2 py-1 text-xs font-medium text-cyan-700 dark:bg-cyan-400/10 dark:text-cyan-400"
              :title="wifiSignal(activePrinter.data.wifi?.signalDbm).label"
            >
              <component :is="wifiSignal(activePrinter.data.wifi?.signalDbm).icon" class="size-3.5" />{{ activePrinter.data.wifi?.signalDbm }} dBm
            </span>
            <ActionMenu label="Printer actions" :items="printerActionItems" />
          </div>
        </div>

        <!-- Empty state -->
        <div
          v-if="activePrinter.status === 'offline'"
          class="mx-4 flex min-h-140 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15"
        >
          <PhWarning class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
          <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">Printer unavailable</h3>
          <p class="mt-1 max-w-md text-sm text-gray-500 dark:text-gray-400">Polimero cannot reach this printer right now. Check the printer connection and try again.</p>
          <Button class="mt-6" variant="primary" @click="showToast('Trying to reconnect to printer')"><PhArrowsClockwise class="size-4" /> Refresh connection</Button>
        </div>

        <div v-show="activePrinter.status !== 'offline'">
          <section class="grid gap-5 xl:grid-cols-4">
            <Card class="overflow-hidden xl:col-span-3">
              <CardHeader title="Camera" :icon="PhVideoCamera">
                <template #suffix>
                  <span
                    class="inline-flex items-center gap-x-1.5 rounded-md px-2 py-1 text-xs font-medium"
                    :class="cameraOnline
                      ? 'bg-green-100 fill-green-500 text-green-700 dark:bg-green-400/10 dark:fill-green-400 dark:text-green-400'
                      : 'bg-yellow-100 fill-yellow-500 text-yellow-800 dark:bg-yellow-400/10 dark:fill-yellow-400 dark:text-yellow-500'"
                  >
                    <svg class="size-1.5" viewBox="0 0 6 6" aria-hidden="true"><circle cx="3" cy="3" r="3" /></svg>{{ cameraOnline ? 'LIVE' : 'Offline' }}
                  </span>
                </template>
                <IconButton title="Refresh camera" aria-label="Refresh camera" @click="refreshCamera"><PhArrowsClockwise class="size-4" /></IconButton>
                <IconButton title="Save snapshot" aria-label="Save snapshot" :disabled="!cameraOnline" @click="saveSnapshot"><PhCamera class="size-4" /></IconButton>
                <IconButton title="Maximize camera" aria-label="Maximize camera" :disabled="!cameraOnline" @click="toggleCameraFullscreen"><PhCornersOut class="size-4" /></IconButton>
              </CardHeader>
              <div
                ref="cameraStage"
                class="relative min-h-60 overflow-hidden sm:min-h-77.5"
                :class="cameraOnline
                  ? 'bg-linear-145 from-slate-300 via-slate-400 to-slate-300 dark:from-slate-700 dark:via-slate-900 dark:to-slate-800'
                  : 'grid place-items-center bg-gray-100 dark:bg-gray-800'"
              >
                <template v-if="cameraOnline">
                  <div class="pointer-events-none absolute inset-x-[12%] inset-y-[15%] border border-slate-400/25 transform-[perspective(450px)_rotateX(52deg)]"></div>
                  <div class="absolute inset-0 bg-[linear-gradient(rgb(148_163_184/10%)_1px,transparent_1px),linear-gradient(90deg,rgb(148_163_184/10%)_1px,transparent_1px)] bg-[size:48px_48px] opacity-35"></div>
                  <div class="absolute bottom-[16%] left-[22%] h-[28%] w-[56%] bg-linear-135 from-slate-500/65 to-slate-800/85 shadow-[0_0_70px] shadow-cyan-400/10 transform-[perspective(450px)_rotateX(52deg)]">
                    <div class="absolute bottom-[19%] left-[36%] flex h-[55%] w-[28%] items-end justify-center gap-1 -rotate-x-52">
                      <span class="block h-[66%] w-[28%] bg-emerald-400 shadow-[0_0_18px] shadow-emerald-400/40"></span>
                      <span class="block h-full w-[28%] bg-emerald-400 shadow-[0_0_18px] shadow-emerald-400/40"></span>
                      <span class="block h-[78%] w-[28%] bg-emerald-400 shadow-[0_0_18px] shadow-emerald-400/40"></span>
                    </div>
                    <div class="absolute top-[22%] left-[46%] size-7 rounded-full border-3 border-amber-300 shadow-[0_0_16px] shadow-amber-400/35"></div>
                  </div>
                  <div class="absolute inset-x-3.5 bottom-3 flex justify-end">
                    <span class="inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-xs font-medium text-gray-600 inset-ring inset-ring-gray-500/10 dark:bg-gray-400/10 dark:text-gray-400 dark:inset-ring-gray-400/20">11:26:04</span>
                  </div>
                </template>
                <div v-else class="relative z-10 text-center">
                  <PhWarning class="mx-auto size-8 text-yellow-600 dark:text-yellow-400" />
                  <p class="mt-3 font-medium text-gray-900 dark:text-white">Camera unavailable</p>
                  <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">Check the printer connection and try again.</p>
                  <Button class="mt-4" @click="refreshCamera"><PhArrowsClockwise class="size-4" /> Refresh feed</Button>
                </div>
              </div>
            </Card>

            <Card>
              <CardHeader title="Current job" :icon="PhCheckSquareOffset">
                <span class="text-xs text-gray-500 dark:text-gray-400">{{ activePrinter.progress }}% complete</span>
              </CardHeader>
              <div class="px-4 py-5 sm:p-6">
                <div class="flex items-start gap-3">
                  <div class="grid size-10 shrink-0 place-items-center rounded-lg bg-cyan-50 text-cyan-600 dark:bg-cyan-400/10 dark:text-cyan-400">
                    <PhHexagon v-if="!activePrinter.data.job" class="size-5" />
                    <PhCube v-else class="size-5" />
                  </div>
                  <div class="min-w-0">
                    <p class="truncate font-mono text-sm font-semibold text-gray-900 dark:text-white">{{ activePrinter.data.job?.name ?? 'No active print' }}</p>
                    <p v-if="activePrinter.status !== 'offline'" class="mt-1 font-mono text-xs text-gray-500 dark:text-gray-400">{{ activePrinter.status === 'busy' ? (activePrinter.data.timeEstimates.remainingSeconds ? `${Math.ceil(activePrinter.data.timeEstimates.remainingSeconds / 60)}m remaining` : 'Time estimate unavailable') : 'Idle' }}</p>
                  </div>
                </div>
                <div class="mt-6">
                  <div class="mb-2 flex justify-between text-xs text-gray-500 dark:text-gray-400">
                    <span>Layer {{ activePrinter.status === 'offline' ? 0 : activePrinter.data.progress.currentLayer }} / {{ activePrinter.status === 'offline' ? 0 : activePrinter.data.progress.totalLayers }}</span>
                  </div>
                  <div class="overflow-hidden rounded-full bg-gray-200 dark:bg-white/10">
                    <div class="h-2 rounded-full bg-cyan-600 dark:bg-cyan-500" :style="{ width: `${activePrinter.status === 'offline' ? 0 : activePrinter.progress}%` }"></div>
                  </div>
                </div>
                <div class="mt-6 flex gap-2">
                  <Button class="flex-1" :disabled="activePrinter.status !== 'busy'" @click="toggleJob">
                    <PhPause v-if="!jobPaused" class="size-4" /><PhPlay v-else class="size-4" />{{ jobPaused ? 'Resume' : 'Pause' }}
                  </Button>
                  <Button variant="danger" :disabled="activePrinter.status !== 'busy'" @click="jobCancelled = true"><PhStop class="size-4" /> Cancel</Button>
                </div>
                <p v-if="jobCancelled && activePrinter.status === 'busy'" class="mt-3 text-xs text-red-600 dark:text-red-400">Cancel requested. Printer is finishing the current move.</p>
              </div>
            </Card>
          </section>

          <section class="mt-5 grid gap-5 lg:grid-cols-2 xl:grid-cols-4">
            <Card>
              <CardHeader title="Temperature" :icon="PhThermometerSimple" />
              <div class="font-light space-y-4 px-4 py-5 sm:p-6">
                <div v-for="(value, key) in temperature" :key="key" class="flex items-center justify-between gap-3">
                  <div>
                    <p class="text-sm/6 font-light text-gray-900 capitalize dark:text-white">{{ key }}</p>
                    <p class="font-mono text-sm text-gray-500 dark:text-gray-400"><span class="font-bold">{{ formatTemperature(value.current) }}</span> °C <span aria-hidden="true">/</span> <span class="font-bold">{{ formatTemperature(value.target) }}</span> °C</p>
                  </div>
                  <div class="flex gap-1">
                    <IconButton variant="outline" :aria-label="`Decrease ${key} target temperature`" @click="updateTemperature(key, -5)"><PhMinus class="size-3.5" /></IconButton>
                    <IconButton variant="outline" :aria-label="`Increase ${key} target temperature`" @click="updateTemperature(key, 5)"><PhPlus class="size-3.5" /></IconButton>
                  </div>
                </div>
              </div>
            </Card>

            <Card>
              <CardHeader title="Fans" :icon="PhFan" />
              <div class="font-light space-y-4 px-4 py-5 sm:p-6">
                <label v-for="(value, key) in fanPower" :key="key" class="block">
                  <span class="mb-2 flex justify-between">
                    <span class="text-sm/6 font-light text-gray-900 capitalize dark:text-white">{{ fanLabel(String(key)) }}</span>
                    <span class="text-sm/6 text-gray-500 dark:text-gray-400">{{ value }}%</span>
                  </span>
                  <input v-model="fanPower[key]" class="h-1 w-full cursor-pointer accent-cyan-600 dark:accent-cyan-400" type="range" min="0" max="100" :aria-label="`${fanLabel(String(key))} fan power`" />
                </label>
              </div>
            </Card>

            <Card>
              <CardHeader title="Lights" :icon="PhLightbulb" />
              <div class="font-light space-y-3 px-4 py-5 sm:p-6">
                <div v-for="(_, key) in lights" :key="key" class="flex items-center justify-between">
                  <span class="text-sm/6 font-light text-gray-900 capitalize dark:text-white">{{ key }} light</span>
                  <Switch v-model="lights[key]" :label="`${key} light`" />
                </div>
              </div>
            </Card>

            <Card>
              <CardHeader title="Motion" :icon="PhArrowsOutCardinal" />
              <div class="font-light px-4 py-5 sm:p-6">
                <div class="mb-4 flex min-h-27 items-center justify-center gap-8">
                  <div class="grid grid-cols-3 grid-rows-3 gap-1">
                    <IconButton variant="outline" class="col-start-2 row-start-1" @click="move('Y', '+')">Y+</IconButton>
                    <IconButton variant="outline" class="col-start-1 row-start-2" @click="move('X', '-')">X−</IconButton>
                    <IconButton variant="outline" class="col-start-2 row-start-2" title="Home all axes" aria-label="Home all axes" @click="showToast('Axes homing started')"><PhHouse class="size-4" /></IconButton>
                    <IconButton variant="outline" class="col-start-3 row-start-2" @click="move('X', '+')">X+</IconButton>
                    <IconButton variant="outline" class="col-start-2 row-start-3" @click="move('Y', '-')">Y−</IconButton>
                  </div>
                  <div class="flex flex-col gap-1">
                    <IconButton variant="outline" @click="move('Z', '+')">Z+</IconButton>
                    <IconButton variant="outline" @click="move('Z', '-')">Z−</IconButton>
                  </div>
                </div>
                <div class="grid grid-cols-2 gap-2">
                  <div>
                    <label for="step-size" class="block text-sm/6 font-light text-gray-900 dark:text-white">Step size</label>
                    <div class="mt-2 grid grid-cols-1">
                      <select id="step-size" v-model="stepSize" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
                        <option>0.1 mm</option>
                        <option>1 mm</option>
                        <option>10 mm</option>
                      </select>
                      <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
                    </div>
                  </div>
                  <div>
                    <label for="feed-rate" class="block text-sm/6 font-light text-gray-900 dark:text-white">Feed rate</label>
                    <div class="mt-2 grid grid-cols-1">
                      <select id="feed-rate" v-model="feedRate" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
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
            <Card class="overflow-hidden">
              <CardHeader title="Printer files" :icon="PhFolder" />
              <div class="overflow-x-auto">
                <table class="relative min-w-full divide-y divide-gray-300 text-left dark:divide-white/15">
                  <thead>
                    <tr>
                      <th v-for="column in ['Type', 'Size', 'Modified', 'Name']" :key="column" scope="col" class="px-4 py-3 text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase sm:px-6 dark:text-gray-400">{{ column }}</th>
                      <th scope="col" class="px-4 py-3 text-right text-xs font-medium tracking-wide whitespace-nowrap text-gray-500 uppercase sm:px-6 dark:text-gray-400">Actions</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-gray-200 dark:divide-white/10">
                    <tr
                      v-for="file in printerFiles.slice(0, 3)"
                      :key="file.name"
                      class="cursor-pointer"
                      :class="selectedFile === file.name ? 'bg-cyan-50 dark:bg-cyan-400/5' : 'hover:bg-gray-50 dark:hover:bg-white/5'"
                      @click="selectedFile = file.name"
                    >
                      <td class="px-4 py-4 text-sm whitespace-nowrap sm:px-6">
                        <span class="inline-flex items-center rounded-md bg-gray-100 px-2 py-1 text-xs font-medium text-gray-600 dark:bg-gray-400/10 dark:text-gray-400">{{ file.type }}</span>
                      </td>
                      <td class="px-4 py-4 font-mono text-sm whitespace-nowrap text-gray-500 sm:px-6 dark:text-gray-400">{{ file.size }}</td>
                      <td class="px-4 py-4 font-mono text-sm whitespace-nowrap text-gray-500 sm:px-6 dark:text-gray-400">{{ file.modified }}</td>
                      <td class="px-4 py-4 font-mono text-sm font-medium whitespace-nowrap text-gray-900 sm:px-6 dark:text-white">{{ file.name }}</td>
                      <td class="px-4 py-4 text-sm whitespace-nowrap sm:px-6">
                        <div class="flex justify-end gap-1">
                          <IconButton title="Download file" :aria-label="`Download ${file.name}`" @click.stop="showToast(`${file.name} download started`)"><PhDownloadSimple class="size-4" /></IconButton>
                          <IconButton class="text-cyan-600 dark:text-cyan-400" title="Print file" :aria-label="`Print ${file.name}`" @click.stop="showToast(`${file.name} sent to printer`)"><PhPlay class="size-4" /></IconButton>
                        </div>
                      </td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </Card>

            <Card>
              <CardHeader title="Filament Spools &amp; Material Systems" :icon="PhStack" />
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
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">Printer management</h2>
            <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">Discover and maintain the printers available on your local network.</p>
          </div>
          <div class="mt-5 flex lg:mt-0 lg:ml-4 max-lg:w-full">
            <Button variant="primary" class="max-lg:w-full" @click="discoverPrinter"><PhBroadcast class="size-4" /> Discover printers</Button>
          </div>
        </div>

        <!-- Stats -->
        <dl v-if="hasPrinters" class="mb-5 grid grid-cols-1 gap-5 sm:grid-cols-3">
          <Card v-for="stat in fleetStats" :key="stat.label" as="div" class="px-4 py-5 sm:p-6">
            <dt class="truncate text-sm font-medium text-gray-500 dark:text-gray-400">{{ stat.label }}</dt>
            <dd class="mt-1 text-3xl font-semibold tracking-tight" :class="stat.tone">{{ stat.value }}</dd>
          </Card>
        </dl>

        <!-- Empty state -->
        <div v-if="!hasPrinters" class="mx-4 flex min-h-97.5 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15">
          <PhPrinter class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
          <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">No printers connected</h3>
          <p class="mt-1 max-w-sm text-sm text-gray-500 dark:text-gray-400">Discover a printer on your local network to start monitoring jobs and materials.</p>
          <Button class="mt-6" variant="primary" @click="discoverPrinter"><PhBroadcast class="size-4" /> Scan local network</Button>
        </div>

        <div v-else class="grid gap-5 md:grid-cols-2 xl:grid-cols-3">
          <Card v-for="printer in printers" :key="printer.id" as="article" class="px-4 py-5 sm:p-6">
            <div class="flex items-start justify-between">
              <div class="flex items-center gap-3">
                <span class="grid size-10 place-items-center rounded-lg bg-gray-100 text-cyan-600 dark:bg-white/10 dark:text-cyan-400"><PhPrinter class="size-5" /></span>
                <div>
                  <h3 class="font-semibold text-gray-900 dark:text-white">{{ printer.name }}</h3>
                  <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ printer.model }}</p>
                </div>
              </div>
              <IconButton class="hover:text-red-600 dark:hover:text-red-400" title="Remove printer" :aria-label="`Remove ${printer.name}`" @click="removePrinter(printer.id)"><PhTrash class="size-4" /></IconButton>
            </div>
            <div class="mt-6 flex items-center justify-between border-y border-gray-200 py-3 dark:border-white/10">
              <span class="text-xs text-gray-500 dark:text-gray-400">Status</span>
              <StatusBadge :status="printer.status" />
            </div>
            <!-- Description list -->
            <dl class="mt-2 divide-y divide-gray-200 text-xs dark:divide-white/10">
              <div class="flex items-center justify-between py-2">
                <dt class="text-gray-500 dark:text-gray-400">Serial number</dt>
                <dd class="text-gray-900 dark:text-gray-300">{{ printer.serial }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-gray-500 dark:text-gray-400">Host</dt>
                <dd class="text-gray-900 dark:text-gray-300">{{ printer.host }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-gray-500 dark:text-gray-400">Connection timeout</dt>
                <dd class="text-gray-900 dark:text-gray-300">{{ printer.timeout }}</dd>
              </div>
              <div class="flex items-center justify-between py-2">
                <dt class="text-gray-500 dark:text-gray-400">TLS verification</dt>
                <dd :class="printer.insecure ? 'text-yellow-600 dark:text-yellow-400' : 'text-green-600 dark:text-green-400'">{{ printer.insecure ? 'Disabled' : 'Enabled' }}</dd>
              </div>
            </dl>
            <div class="mt-5 flex gap-2">
              <Button class="flex-1" @click="selectPrinter(printer.id)"><PhCards class="size-4" /> Open control</Button>
              <Button class="flex-1" :aria-label="`Refresh certificate for ${printer.name}`" @click="showToast('TLS certificate refresh requested')"><PhArrowsClockwise class="size-4" /> Refresh certificate</Button>
            </div>
          </Card>
          <button
            type="button"
            class="relative mx-4 flex min-h-55 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 p-12 text-center hover:border-gray-400 focus:outline-2 focus:outline-offset-2 focus:outline-cyan-600 sm:mx-0 dark:border-white/15 dark:hover:border-white/25 dark:focus:outline-cyan-500"
            @click="discoverPrinter"
          >
            <PhPlus class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
            <span class="mt-2 block text-sm font-semibold text-gray-900 dark:text-white">Add another printer</span>
          </button>
        </div>
      </template>

      <template v-else-if="activeView === 'settings'">
        <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
          <div class="min-w-0 flex-1">
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">Configuration</h2>
            <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">Choose how Polimero keeps you informed and which tools are available to your team.</p>
          </div>
        </div>
        <div class="grid max-w-[1500px] gap-5 lg:grid-cols-2 xl:grid-cols-3">
          <!-- Toggle lists -->
          <Card as="section" class="overflow-hidden">
            <CardHeader title="Notifications" :icon="PhBellRinging" />
            <div class="divide-y divide-gray-200 dark:divide-white/10">
              <div v-for="notification in notifications" :key="notification.label" class="flex items-center justify-between gap-3 px-4 py-5 hover:bg-gray-50 sm:px-6 dark:hover:bg-white/5">
                <span class="flex grow flex-col">
                  <span class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ notification.label }}</span>
                  <span class="text-sm text-gray-500 dark:text-gray-400">{{ notification.description }}</span>
                </span>
                <Switch v-model="notification.enabled" :label="notification.label" />
              </div>
            </div>
          </Card>
          <Card as="section" class="overflow-hidden">
            <CardHeader title="Slicer applications" :icon="PhDesktop" />
            <div class="divide-y divide-gray-200 dark:divide-white/10">
              <div v-for="slicer in slicers" :key="slicer.name" class="flex items-center justify-between gap-3 px-4 py-5 hover:bg-gray-50 sm:px-6 dark:hover:bg-white/5">
                <span class="flex grow flex-col">
                  <span class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ slicer.name }}</span>
                  <span class="font-mono text-sm text-gray-500 dark:text-gray-400">{{ slicer.path }}</span>
                </span>
                <Switch v-model="slicer.enabled" :label="slicer.name" />
              </div>
            </div>
          </Card>
          <Card as="section">
            <CardHeader title="Appearance" :icon="PhSun" />
            <div class="px-4 py-5 sm:p-6">
              <div>
                <label for="theme" class="block text-sm/6 font-medium text-gray-900 dark:text-white">Theme</label>
                <div class="mt-2 grid grid-cols-1">
                  <select id="theme" v-model="theme" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
                    <option value="system">System</option>
                    <option value="light">Light</option>
                    <option value="dark">Dark</option>
                  </select>
                  <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
                </div>
              </div>
              <div class="mt-6">
                <label for="language" class="block text-sm/6 font-medium text-gray-900 dark:text-white">Language</label>
                <div class="mt-2 grid grid-cols-1">
                  <select id="language" v-model="language" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
                    <option>English</option>
                    <option>Português</option>
                    <option>Español</option>
                  </select>
                  <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
                </div>
              </div>
            </div>
          </Card>
        </div>
      </template>

      <template v-else-if="activeView === 'files'">
        <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
          <div class="min-w-0 flex-1">
            <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">File library</h2>
            <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">Browse models and directories ready to organize, slice, or print.</p>
          </div>
          <div class="mt-5 flex lg:mt-0 lg:ml-4 max-lg:w-full">
            <Button variant="primary" class="max-lg:w-full" @click="openUpload"><PhUploadSimple class="size-4" /> Upload files</Button>
          </div>
        </div>

        <div class="mb-5 flex flex-wrap items-center gap-2 px-4 sm:px-0">
          <!-- Breadcrumbs -->
          <nav class="flex" aria-label="Breadcrumb">
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
              aria-label="Search files"
              placeholder="Search files"
              type="search"
            />
            <PhMagnifyingGlass class="pointer-events-none col-start-1 row-start-1 ml-3 size-5 self-center text-gray-400 sm:size-4 dark:text-gray-500" aria-hidden="true" />
          </div>
        </div>

        <div v-if="visibleDirectories.length" class="mb-5 grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <button
            v-for="directory in visibleDirectories"
            :key="directory.path"
            type="button"
            class="min-h-28 bg-white px-4 py-5 text-left shadow-sm transition hover:bg-cyan-50 hover:shadow-md sm:rounded-lg sm:p-6 dark:bg-gray-800/50 dark:shadow-none dark:outline dark:-outline-offset-1 dark:outline-white/10 dark:hover:bg-cyan-400/5 dark:hover:outline-cyan-400/30"
            @click="navigateToDirectory(directory.path)"
          >
            <PhFolder class="size-7 text-cyan-600 dark:text-cyan-400" />
            <span class="mt-3 block truncate text-sm font-medium text-gray-900 dark:text-white">{{ directory.name }}</span>
            <span class="mt-1 block text-xs text-gray-500 dark:text-gray-400">Folder · {{ directory.modified }}</span>
          </button>
        </div>

        <!-- Grid list -->
        <div class="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <Card
            v-for="file in visibleFiles"
            :key="file.name"
            as="article"
            class="transition hover:shadow-md dark:hover:outline-cyan-400/30"
          >
            <div class="relative grid min-h-45 place-items-center overflow-hidden sm:rounded-t-lg" :class="modelTones[file.tone].preview">
              <span class="block transform-[perspective(200px)_rotateX(10deg)_rotateZ(-8deg)]" :class="modelTones[file.tone].shape"></span>
              <span class="absolute right-3 bottom-2.5 text-[10px] font-bold text-gray-500 uppercase dark:text-slate-300/45">.{{ file.type.toLowerCase() }}</span>
            </div>
            <div class="flex items-start gap-3 px-4 py-5 sm:p-6">
              <PhFile class="mt-0.5 size-4 shrink-0 text-gray-400 dark:text-gray-500" />
              <div class="min-w-0 flex-1">
                <h3 class="truncate text-sm font-medium text-gray-900 dark:text-white">{{ file.name }}</h3>
                <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ file.size }} · {{ file.modified }}</p>
              </div>
              <ActionMenu class="-mt-2 -mr-2" label="More file actions" :items="fileActionItems(file)" />
            </div>
          </Card>
          <div v-if="!visibleDirectories.length && !visibleFiles.length" class="col-span-full mx-4 flex min-h-65 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15">
            <PhFolder class="mx-auto size-12 text-gray-400 dark:text-gray-500" aria-hidden="true" />
            <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">Nothing here</h3>
            <p class="mt-1 text-sm text-gray-500 dark:text-gray-400">This directory is empty.</p>
          </div>
        </div>
      </template>
    </main>

    <SlideOver :open="uploadOpen" title="Upload files" description="Add new models to your local file library." @close="uploadOpen = false">
      <!-- File upload -->
      <div class="flex justify-center rounded-lg border border-dashed border-gray-900/25 px-6 py-10 dark:border-white/25">
        <div class="text-center">
          <PhUploadSimple class="mx-auto size-12 text-gray-300 dark:text-gray-600" aria-hidden="true" />
          <div class="mt-4 flex text-sm/6 text-gray-600 dark:text-gray-400">
            <label for="file-upload" class="relative cursor-pointer rounded-md bg-transparent font-semibold text-cyan-600 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-cyan-600 hover:text-cyan-500 dark:text-cyan-400 dark:focus-within:outline-cyan-500 dark:hover:text-cyan-300">
              <span>Upload a file</span>
              <input id="file-upload" name="file-upload" type="file" multiple accept=".3mf,.stl,.obj" class="sr-only" />
            </label>
            <p class="pl-1">or drag and drop</p>
          </div>
          <p class="text-xs/5 text-gray-600 dark:text-gray-400">3MF, STL, OBJ up to 200 MB</p>
        </div>
      </div>
      <div class="mt-6">
        <label for="upload-printer" class="block text-sm/6 font-medium text-gray-900 dark:text-white">Printer</label>
        <div class="mt-2 grid grid-cols-1">
          <select id="upload-printer" v-model="uploadPrinterId" :disabled="!hasPrinters" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 disabled:opacity-50 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
            <option v-if="!hasPrinters" value="">No printers available</option>
            <option v-for="printer in printers" :key="printer.id" :value="printer.id" :disabled="printer.status === 'offline'">{{ printer.name }}{{ printer.status === 'offline' ? ' — offline' : '' }}</option>
          </select>
          <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
        </div>
      </div>
      <p class="mt-4 text-sm/6 text-gray-600 dark:text-gray-400">Uploading to <span class="font-mono font-medium text-gray-900 dark:text-white">{{ currentDirectory }}</span></p>
      <template #footer>
        <Button @click="uploadOpen = false">Cancel</Button>
        <Button variant="primary" :disabled="!uploadPrinterId" @click="confirmUpload"><PhUploadSimple class="size-4" /> Upload</Button>
      </template>
    </SlideOver>
  </div>
</template>
