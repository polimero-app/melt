<script setup lang="ts">
import { computed, onMounted, provide, onUnmounted, ref, watchEffect, type Component } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { locales, preferredLocale, translate, type Locale, type MessageKey } from './i18n'
import { monitorBadge, type PrinterBadge } from './monitoring'
import { badgeDotClasses } from './presentation'
import { commandMessage } from './errors'
import type { CameraSummary } from './cameras'
import type { PlugObservation, PlugSummary } from './plugs'
import type { FirmwareUpdateEntry } from './firmware-updates'
import { parseStoredPresets, type TemperaturePreset } from './temperatures'
import { Listbox, ListboxButton, ListboxOption, ListboxOptions } from '@headlessui/vue'
import ConfirmDialog from './components/ConfirmDialog.vue'
import {
  PhCaretDown,
  PhCheckCircle,
  PhFolder,
  PhGearSix,
  PhPrinter,
  PhPlug,
  PhVideoCamera,
  PhWarningCircle,
  PhX,
} from '@phosphor-icons/vue'
import type {
  View,
  AppInfo,
  Printer,
  Driver,
  Capabilities,
  MonitorEntry,
  NotificationEvent,
  FileEntry,
  NotificationSetting,
  SlicerSetting,
  BackendPreferences,
} from './app-types'
import ControlView from './views/ControlView.vue'
import PrintersView from './views/PrintersView.vue'
import CamerasView from './views/CamerasView.vue'
import PlugsView from './views/PlugsView.vue'
import ConfigView from './views/ConfigView.vue'
import FilesView from './views/FilesView.vue'

const info = ref<AppInfo>()
const printers = ref<Printer[]>([])
const drivers = ref<Driver[]>([])
const monitoring = ref<MonitorEntry[]>([])
const firmwareUpdates = ref<FirmwareUpdateEntry[]>([])
const firmwareRefreshSource = ref<'printer' | 'publicCatalogue'>()
const firmwareRefreshing = computed(() => firmwareRefreshSource.value !== undefined)
const firmwareError = ref<string>()
const capabilities = ref<Capabilities>()
const loading = ref(true)
const refreshing = ref(false)
const libraryFiles = ref<FileEntry[]>([])
const loadError = ref<string>()
const profileError = ref<string>()
const pendingConfirm = ref<{ title: string; description: string; confirm: string; variant: 'primary' | 'danger'; run: () => Promise<void> }>()
const confirming = ref(false)
const documentVisible = ref(document.visibilityState === 'visible')
let monitorUnlisten: UnlistenFn | undefined
let firmwareUpdatesUnlisten: UnlistenFn | undefined
let presenceUnlisten: UnlistenFn | undefined
let notificationUnlisten: UnlistenFn | undefined
let plugsUnlisten: UnlistenFn | undefined
let selectionRequest = 0
let toastTimer: number | undefined

// Restored across launches like theme and locale: a desktop app that stays
// open for hours should reopen where it was left. A stored view that is no
// longer valid falls back to the control view.
const views: View[] = ['control', 'printers', 'cameras', 'plugs', 'settings', 'files']
const activeView = ref<View>(views.find((view) => view === localStorage.getItem('activeView')) ?? 'control')

const sectionTabs = computed<{ view: View; label: string; icon: Component }[]>(() => [
  { view: 'printers', label: t('nav.printers'), icon: PhPrinter },
  { view: 'cameras', label: t('nav.cameras'), icon: PhVideoCamera },
  { view: 'plugs', label: t('nav.plugs'), icon: PhPlug },
  { view: 'settings', label: t('nav.settings'), icon: PhGearSix },
  { view: 'files', label: t('nav.files'), icon: PhFolder },
])

// Below this the tab strip reads better than a dropdown; above it the strip
// starts scrolling and the active printer can end up off-screen.
const PRINTER_TAB_LIMIT = 5
const usePrinterMenu = computed(() => printers.value.length > PRINTER_TAB_LIMIT)
const activePrinterId = ref(localStorage.getItem('activePrinterId') ?? '')
const firmwarePrinterId = ref<string>()

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

const errorToast = ref<string>()
const successToast = ref<string>()

const toasts = computed(() => [
  ...(errorToast.value ? [{ key: 'error' as const, text: errorToast.value, tone: 'error' as const }] : []),
  ...(successToast.value ? [{ key: 'success' as const, text: successToast.value, tone: 'success' as const }] : []),
])

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
const publicFirmwareCatalogue = ref(false)
const activePrinter = computed(() => printers.value.find((printer) => printer.name === activePrinterId.value) ?? printers.value[0])

// Dense forms and lists read badly stretched; media-heavy views use the width.
const capped = computed(() => activeView.value === 'printers' || activeView.value === 'settings')
const compactNavValue = computed({
  get: () => {
    if (activeView.value !== 'control') return `view:${activeView.value}`
    return activePrinter.value ? `printer:${activePrinter.value.name}` : 'view:printers'
  },
  set: navigateCompact,
})

const hasPrinters = computed(() => printers.value.length > 0)

function badgeFor(name: string): PrinterBadge {
  const entry = monitoring.value.find((candidate) => candidate.name === name)
  return monitorBadge(entry)
}

const activeBadge = computed<PrinterBadge>(() => (activePrinter.value ? badgeFor(activePrinter.value.name) : 'offline'))
const isReachable = (badge: PrinterBadge) => badge === 'idle' || badge === 'busy'

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
const cameras = ref<CameraSummary[]>([])
const camerasError = ref<string>()

// Plug state lives here for the Smart plugs view; the on/off control reads it too.
const plugs = ref<PlugSummary[]>([])
const plugsError = ref<string>()
const plugObservations = ref<Record<string, PlugObservation>>({})

// Commands reject with stable codes; unexpected runtime failures stay in logs
// instead of leaking implementation details into user-facing surfaces.
function message(reason: unknown) {
  return commandMessage(reason, t)
}

function askConfirmation(
  keys: { title: MessageKey; description: MessageKey; confirm: MessageKey },
  values: Record<string, string>,
  run: () => Promise<void>,
  variant: 'primary' | 'danger' = 'danger',
) {
  pendingConfirm.value = {
    title: t(keys.title, values),
    description: t(keys.description, values),
    confirm: t(keys.confirm),
    variant,
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
  publicFirmwareCatalogue.value = preferences.publicFirmwareCatalogue
}

async function loadPreferences() {
  try {
    applyPreferences(await invoke<BackendPreferences>('get_preferences'))
  } catch (reason) {
    showToast(message(reason), 'error')
  }
}

async function load() {
  loading.value = true
  loadError.value = undefined
  profileError.value = undefined
  void loadPreferences()

  void loadCameras()
  void loadPlugs()
  void loadPlugObservations()

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
  activePrinterId.value = ''
  capabilities.value = undefined
  monitoring.value = []
  firmwareError.value = undefined
}

async function loadFirmwareUpdates(name: string, refresh = false) {
  if (firmwareRefreshing.value && refresh) return
  if (refresh) firmwareRefreshSource.value = 'printer'
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
    if (refresh) firmwareRefreshSource.value = undefined
  }
}

async function loadPublicFirmwareCatalogue(name: string) {
  if (firmwareRefreshing.value) return
  firmwareRefreshSource.value = 'publicCatalogue'
  firmwareError.value = undefined
  try {
    const entry = await invoke<FirmwareUpdateEntry>('printer_public_firmware_catalogue', { name })
    const index = firmwareUpdates.value.findIndex((candidate) => candidate.profile === entry.profile)
    if (index === -1) firmwareUpdates.value.push(entry)
    else firmwareUpdates.value.splice(index, 1, entry)
  } catch (reason) {
    firmwareError.value = message(reason)
    showToast(firmwareError.value ?? t('firmware.checkFailed'), 'error')
  } finally {
    firmwareRefreshSource.value = undefined
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

// focus defaults to refresh: user-initiated selections jump to the control
// view, background re-selections (load, removal fallback) keep the current view.
async function selectPrinter(name: string, refresh = true, focus = refresh) {
  const request = ++selectionRequest
  activePrinterId.value = name
  if (focus) {
    firmwarePrinterId.value = undefined
    activeView.value = 'control'
  }
  capabilities.value = undefined
  if (request !== selectionRequest || activePrinterId.value !== name) return
  try {
    const result = await invoke<{ capabilities: Capabilities }>('printer_capabilities', { name })
    if (request !== selectionRequest || activePrinterId.value !== name) return
    capabilities.value = result.capabilities
    if (result.capabilities.firmwareUpdateCheck) void loadFirmwareUpdates(name)
  } catch (reason) {
    showToast(message(reason), 'error')
  }
  if (refresh) await refreshMonitoring()
}

function goTo(view: View) {
  firmwarePrinterId.value = undefined
  activeView.value = view
  if (view === 'files') void filesView.value?.refresh()
}

function navigateCompact(value: string) {
  const separator = value.indexOf(':')
  const kind = value.slice(0, separator)
  const destination = value.slice(separator + 1)
  if (kind === 'printer') void selectPrinter(destination)
  else if (kind === 'view') goTo(destination as View)
}

async function loadCameras() {
  try {
    cameras.value = await invoke<CameraSummary[]>('camera_list')
    camerasError.value = undefined
  } catch (reason) {
    camerasError.value = message(reason)
  }
}

async function loadPlugs() {
  try {
    await refreshPlugs()
  } catch (reason) {
    plugsError.value = message(reason)
  }
}

async function refreshPlugs() {
  const current = await invoke<PlugSummary[]>('plug_list')
  plugs.value = current
  plugsError.value = undefined
  return current
}

async function loadPlugObservations() {
  try {
    plugObservations.value = await invoke<Record<string, PlugObservation>>('plug_observations')
  } catch {
    // The next plugs-updated event carries the same data.
  }
}

/** Known printer activity by lowercase name, from monitoring already loaded; absent when unknown. */
const plugActivity = computed(() => {
  const activity: Record<string, string> = {}
  for (const entry of monitoring.value) {
    const status = entry.status
    if (!status) continue
    const units = status.extensions?.['bambu-lan']?.ams?.units ?? []
    const targets = Object.values(status.temperatures ?? {}).some((temperature) => (temperature?.targetCelsius ?? 0) > 0)
    const value = status.state === 'printing' || status.state === 'paused' ? status.state : units.some((unit) => unit.drying?.active) ? 'drying' : targets ? 'heating' : undefined
    if (value) activity[entry.name.toLowerCase()] = value
  }
  return activity
})

const temperaturePresets = ref<TemperaturePreset[]>(parseStoredPresets(localStorage.getItem('temperaturePresets')))

watchEffect(() => {
  localStorage.setItem('temperaturePresets', JSON.stringify(temperaturePresets.value))
})

onMounted(() => {
  // A webview reload leaves the previous page's viewers running.
  void invoke('camera_viewer_stop').catch(() => undefined)
  document.addEventListener('visibilitychange', handleDocumentVisibility)
  systemThemeQuery = window.matchMedia('(prefers-color-scheme: dark)')
  systemPrefersDark.value = systemThemeQuery.matches
  systemThemeQuery.addEventListener('change', handleSystemThemeChange)
  void load()
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
  void listen<Record<string, PlugObservation>>('plugs-updated', (event) => {
    plugObservations.value = event.payload
  }).then((unlisten) => {
    plugsUnlisten = unlisten
  })
})

onUnmounted(() => {
  document.removeEventListener('visibilitychange', handleDocumentVisibility)
  systemThemeQuery?.removeEventListener('change', handleSystemThemeChange)
  monitorUnlisten?.()
  firmwareUpdatesUnlisten?.()
  presenceUnlisten?.()
  notificationUnlisten?.()
  plugsUnlisten?.()
  if (toastTimer !== undefined) window.clearTimeout(toastTimer)
})

const printersView = ref<InstanceType<typeof PrintersView>>()
const filesView = ref<InstanceType<typeof FilesView>>()
const plugEditRequest = ref<string>()
provide('editPlugCredentials', (id: string) => {
  plugEditRequest.value = id
  goTo('plugs')
})
const viewActions = { t, message, showToast, askConfirmation }

function openEdit(printer: Printer) {
  printersView.value?.openEdit(printer)
}

function requestPrint(file: FileEntry, printer?: string) {
  void filesView.value?.requestPrint(file, printer)
}

</script>

<template>
  <div class="min-h-full min-w-0 overflow-x-clip scheme-light dark:scheme-dark">
    <a href="#main-content" class="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-white focus:px-3 focus:py-2 focus:text-sm focus:font-semibold focus:text-gray-900 dark:focus:bg-gray-800 dark:focus:text-white">Skip to content</a>
    <!-- Tabs with underline -->
    <header class="sticky top-0 z-20 bg-white/95 pt-[env(safe-area-inset-top)] backdrop-blur dark:bg-gray-900/95">
      <!-- Scrolling one axis clips the other, which would cut off the printer
           menu. Only the compact layout below `lg` needs to scroll, and it
           uses a native select that the clip cannot reach. -->
      <div :class="capped && 'max-w-[1500px]'" class="mx-auto overflow-x-auto px-4 sm:px-8 lg:overflow-x-visible lg:px-10">
        <nav class="flex min-w-0 items-stretch border-b border-gray-200 sm:min-w-max dark:border-white/10" :aria-label="t('nav.ariaLabel')">
          <div class="grid w-full grid-cols-1 py-2 lg:hidden">
            <select
              name="primary-navigation"
              v-model="compactNavValue"
              :aria-label="t('nav.ariaLabel')"
              class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-2 pr-9 pl-3 text-sm font-medium text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-gray-900 dark:text-white dark:outline-white/15 dark:*:bg-gray-800"
            >
              <optgroup v-if="printers.length" :label="t('nav.selectPrinter')">
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
          <div v-if="printers.length && !usePrinterMenu" class="hidden items-center space-x-8 pr-8 lg:flex">
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
              <span class="size-1.5 shrink-0 rounded-full ring-3" :class="badgeDotClasses[badgeFor(printer.name)]" aria-hidden="true"></span>{{ printer.name }}
              <span class="sr-only">{{ statusLabel(badgeFor(printer.name)) }}</span>
            </button>
          </div>
          <div
            v-if="printers.length"
            class="hidden items-center pr-8 lg:flex"
            :class="!usePrinterMenu && 'lg:hidden'"
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
                  : 'text-gray-500 group-hover:text-gray-600 dark:text-gray-400 dark:group-hover:text-gray-300'"
                aria-hidden="true"
              />
              <span>{{ tab.label }}</span>
            </button>
          </div>
        </nav>
      </div>
    </header>

    <main id="main-content" tabindex="-1" :class="capped && 'max-w-[1500px]'" class="mx-auto py-7 outline-none sm:px-8 lg:px-10">
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
                  <PhCheckCircle v-else class="size-6 text-green-600 dark:text-green-400" aria-hidden="true" />
                </div>
                <div class="ml-3 w-0 flex-1 pt-0.5">
                  <p class="text-sm font-medium text-gray-900 dark:text-white">{{ item.text }}</p>
                </div>
                <div class="ml-4 flex shrink-0">
                  <button
                    type="button"
                    class="inline-flex rounded-md text-gray-500 hover:text-gray-600 dark:text-gray-400 dark:hover:text-white"
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

      <!-- Keep view state and in-flight dialogs alive; each view gates its visible content. -->
      <ControlView
        :active="activeView === 'control'"
        v-bind="viewActions"
        :active-printer="activePrinter"
        :active-badge="activeBadge"
        :monitoring="monitoring"
        :capabilities="capabilities"
        :cameras="cameras"
        :document-visible="documentVisible"
        :plugs="plugs"
        :plug-observations="plugObservations"
        :plug-activity="plugActivity"
        :locale="locale"
        :library-files="libraryFiles"
        :temperature-presets="temperaturePresets"
        :refreshing="refreshing"
        :refresh-monitoring="refreshMonitoring"
        :refresh-plugs="refreshPlugs"
        :load-cameras="loadCameras"
        :open-edit="openEdit"
        :status-label="statusLabel"
        :is-reachable="isReachable"
        :print-preparing="filesView?.printPreparing ?? false"
        :print-busy="filesView?.printBusy ?? false"
        :request-print="requestPrint"
      />
      <PrintersView
        ref="printersView"
        :active="activeView === 'printers' || (!hasPrinters && activeView === 'control')"
        v-bind="viewActions"
        :drivers="drivers"
        :loading="loading"
        :load-error="loadError"
        :profile-error="profileError"
        :firmware-updates="firmwareUpdates"
        :firmware-refreshing="firmwareRefreshing"
        :firmware-refresh-source="firmwareRefreshSource"
        :firmware-error="firmwareError"
        :locale="locale"
        :has-printers="hasPrinters"
        :badge-for="badgeFor"
        :is-reachable="isReachable"
        :status-label="statusLabel"
        :select-printer="selectPrinter"
        :clear-selection="clearSelection"
        :go-to="goTo"
        :load="load"
        :load-cameras="loadCameras"
        :load-plugs="loadPlugs"
        :refresh-monitoring="refreshMonitoring"
        :load-firmware-updates="loadFirmwareUpdates"
        :load-public-firmware-catalogue="loadPublicFirmwareCatalogue"
        v-model:printers="printers"
        v-model:monitoring="monitoring"
        v-model:firmware-printer-id="firmwarePrinterId"
      />
      <CamerasView
        :active="activeView === 'cameras'"
        v-bind="viewActions"
        :printers="printers"
        :cameras="cameras"
        :cameras-error="camerasError"
        :document-visible="documentVisible"
        :plugs="plugs"
        :plug-observations="plugObservations"
        :plug-activity="plugActivity"
        :locale="locale"
        :load-cameras="loadCameras"
        :load-plugs="loadPlugs"
        :refresh-plugs="refreshPlugs"
      />
      <PlugsView
        v-model:edit-request="plugEditRequest"
        :active="activeView === 'plugs'"
        v-bind="viewActions"
        :printers="printers"
        :cameras="cameras"
        :plugs="plugs"
        :plugs-error="plugsError"
        :plug-observations="plugObservations"
        :plug-activity="plugActivity"
        :locale="locale"
        :load-plugs="loadPlugs"
        :refresh-plugs="refreshPlugs"
      />
      <ConfigView
        :active="activeView === 'settings'"
        v-bind="viewActions"
        :info="info"
        v-model:theme="theme"
        v-model:locale="locale"
        v-model:notifications="notifications"
        v-model:slicers="slicers"
        v-model:public-firmware-catalogue="publicFirmwareCatalogue"
        v-model:temperature-presets="temperaturePresets"
      />
      <FilesView
        ref="filesView"
        :active="activeView === 'files'"
        v-bind="viewActions"
        :printers="printers"
        :slicers="slicers"
        :locale="locale"
        :badge-for="badgeFor"
        :is-reachable="isReachable"
        :refresh-monitoring="refreshMonitoring"
        v-model:library-files="libraryFiles"
      />
    </main>

    <ConfirmDialog
      :open="pendingConfirm !== undefined"
      :title="pendingConfirm?.title ?? ''"
      :description="pendingConfirm?.description"
      :confirm-label="confirming ? t('common.sending') : pendingConfirm?.confirm ?? ''"
      :cancel-label="t('common.cancel')"
      :confirm-variant="pendingConfirm?.variant"
      :busy="confirming"
      @close="!confirming && (pendingConfirm = undefined)"
      @confirm="runConfirmation"
    />
  </div>
</template>
