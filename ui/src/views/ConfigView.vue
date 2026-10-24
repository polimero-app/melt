<script setup lang="ts">
import { nextTick, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open as openFileDialog } from '@tauri-apps/plugin-dialog'
import type { Locale } from '../i18n'
import { cameraPlayback, cameraTransportLabels } from '../cameras'
import { validateSlicerDraft } from '../forms'
import {
  PRESET_NAME_MAX_LENGTH,
  defaultPresets,
  presetValidationFailed,
  temperatureMaximums,
  validatePresetDraft,
  type TemperaturePreset,
} from '../temperatures'
import SlideOver from '../components/SlideOver.vue'
import Button from '../components/Button.vue'
import IconButton from '../components/IconButton.vue'
import Switch from '../components/Switch.vue'
import Card from '../components/Card.vue'
import CardHeader from '../components/CardHeader.vue'
import {
  PhArrowCounterClockwise,
  PhCaretDown,
  PhDesktop,
  PhBellRinging,
  PhGitBranch,
  PhInfo,
  PhPlus,
  PhSun,
  PhThermometerSimple,
  PhTrash,
} from '@phosphor-icons/vue'
import type { AppInfo, Diagnostics, NotificationSetting, SlicerSetting, BackendPreferences, ViewActions } from '../app-types'

const {
  active,
  info,
  t,
  message,
  showToast,
  askConfirmation,
} = defineProps<ViewActions & {
  active: boolean
  info: AppInfo | undefined
}>()

const theme = defineModel<'light' | 'dark' | 'system'>('theme', { required: true })
const locale = defineModel<Locale>('locale', { required: true })
const notifications = defineModel<NotificationSetting[]>('notifications', { required: true })
const slicers = defineModel<SlicerSetting[]>('slicers', { required: true })
const publicFirmwareCatalogue = defineModel<boolean>('publicFirmwareCatalogue', { required: true })
const temperaturePresets = defineModel<TemperaturePreset[]>('temperaturePresets', { required: true })
const diagnostics = ref<Diagnostics>()
const diagnosticsError = ref<string>()
const diagnosticsLoading = ref(false)
const slicerAdditionOpen = ref(false)
const slicerDraft = ref({ name: '', path: '' })
const slicerBusy = ref(false)
const slicerError = ref<string>()
const slicerNameError = ref<string>()
const slicerPathError = ref<string>()
const slicerNameInput = ref<HTMLInputElement>()
const slicerPathInput = ref<HTMLInputElement>()

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

async function updatePublicFirmwareCatalogue(enabled: boolean) {
  const previous = publicFirmwareCatalogue.value
  publicFirmwareCatalogue.value = enabled
  try {
    publicFirmwareCatalogue.value = await invoke<boolean>('update_firmware_catalogue_preferences', {
      request: { publicFirmwareCatalogue: enabled },
    })
  } catch (reason) {
    publicFirmwareCatalogue.value = previous
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

function openSlicerAddition() {
  slicerDraft.value = { name: '', path: '' }
  slicerError.value = undefined
  slicerNameError.value = undefined
  slicerPathError.value = undefined
  slicerAdditionOpen.value = true
}

function closeSlicerAddition() {
  if (slicerBusy.value) return
  slicerAdditionOpen.value = false
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
    slicerAdditionOpen.value = false
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

const presetAdditionOpen = ref(false)
const presetDraft = ref({ name: '', nozzleCelsius: '', bedCelsius: '' })
const presetErrors = ref<{ name?: string; nozzle?: string; bed?: string }>({})
const presetNameInput = ref<HTMLInputElement>()

function openPresetAddition() {
  presetDraft.value = { name: '', nozzleCelsius: '', bedCelsius: '' }
  presetErrors.value = {}
  presetAdditionOpen.value = true
}

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
  presetAdditionOpen.value = false
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
  askConfirmation(
    { title: 'settingsView.presetRestoreTitle', description: 'settingsView.presetRestoreDescription', confirm: 'settingsView.restorePresets' },
    {},
    async () => {
      temperaturePresets.value = [...defaultPresets]
    },
  )
}

async function runDiagnostics() {
  diagnosticsLoading.value = true
  diagnostics.value = undefined
  diagnosticsError.value = undefined
  try {
    diagnostics.value = await invoke<Diagnostics>('diagnostics_report', {
      playback: {
        printers: Object.fromEntries(cameraPlayback.printers),
        cameras: Object.fromEntries(cameraPlayback.cameras),
      },
    })
  } catch (reason) {
    diagnosticsError.value = message(reason)
  } finally {
    diagnosticsLoading.value = false
  }
}
</script>

<template>
  <template v-if="active">
    <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
      <div class="min-w-0 flex-1">
        <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ t('nav.settings') }}</h2>
        <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">{{ t('settingsView.description') }}</p>
      </div>
    </div>
    <div class="grid max-w-[1500px] items-start gap-5 lg:grid-cols-2 xl:grid-cols-3">
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
        <CardHeader :title="t('settingsView.firmwareSources')" :icon="PhGitBranch" />
        <label for="public-firmware-catalogue" class="flex cursor-pointer items-center justify-between gap-3 px-4 py-5 hover:bg-gray-50 sm:px-6 dark:hover:bg-white/5">
          <span class="flex grow flex-col">
            <span class="text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('settingsView.publicFirmwareCatalogue') }}</span>
            <span class="text-sm text-gray-500 dark:text-gray-400">{{ t('settingsView.publicFirmwareCatalogueDescription') }}</span>
          </span>
          <Switch id="public-firmware-catalogue" v-model="publicFirmwareCatalogue" :label="t('settingsView.publicFirmwareCatalogue')" @update:model-value="updatePublicFirmwareCatalogue" />
        </label>
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
        <div class="border-t border-gray-200 px-4 py-4 sm:px-6 dark:border-white/10">
          <Button class="w-full" @click="openSlicerAddition"><PhPlus class="size-4" aria-hidden="true" /> {{ t('settingsView.addSlicer') }}</Button>
        </div>
      </Card>
      <Card as="section" class="overflow-hidden">
        <CardHeader :title="t('settingsView.presets')" :icon="PhThermometerSimple">
          <IconButton :title="t('settingsView.restorePresets')" :aria-label="t('settingsView.restorePresets')" @click="restoreDefaultPresets"><PhArrowCounterClockwise class="size-4" aria-hidden="true" /></IconButton>
        </CardHeader>
        <p class="px-4 pt-4 text-xs text-gray-500 sm:px-6 dark:text-gray-400">
          {{ t('settingsView.presetsDescription', { nozzle: temperatureMaximums.nozzle, bed: temperatureMaximums.bed }) }}
        </p>
        <div class="mt-4 divide-y divide-gray-200 dark:divide-white/10">
          <div v-for="preset in temperaturePresets" :key="preset.id" class="flex items-center justify-between gap-3 px-4 py-4 hover:bg-gray-50 sm:px-6 dark:hover:bg-white/5">
            <div class="flex min-w-0 grow flex-col">
              <span class="truncate text-sm/6 font-medium text-gray-900 dark:text-white">{{ preset.name }}</span>
              <span class="font-mono text-xs text-gray-500 dark:text-gray-400">
                {{ t('dashboard.nozzle') }} {{ preset.nozzleCelsius }} °C <span aria-hidden="true">·</span> {{ t('dashboard.bed') }} {{ preset.bedCelsius }} °C
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
        <div class="border-t border-gray-200 px-4 py-4 sm:px-6 dark:border-white/10">
          <Button class="w-full" @click="openPresetAddition"><PhPlus class="size-4" aria-hidden="true" /> {{ t('settingsView.addPreset') }}</Button>
        </div>
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
              <div v-if="printer.camera.playback" class="flex flex-wrap gap-x-1">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.cameraPlayback') }}:</dt>
                <dd class="font-mono">{{ printer.camera.playback.transport ? cameraTransportLabels[printer.camera.playback.transport] : t('settingsView.none') }}</dd>
                <dd v-if="printer.camera.playback.detail" class="basis-full text-amber-700 dark:text-amber-300">{{ printer.camera.playback.detail }}</dd>
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
                <dd>{{ printer.quirks.length ? printer.quirks.map((quirk) => `${quirk.id} (${quirk.active ? t('settingsView.active') : quirk.qualification})`).join(', ') : t('settingsView.none') }}</dd>
              </div>
            </dl>
            <p v-if="printer.camera.rejectedAdvertisement" class="mt-1 text-xs text-amber-700 dark:text-amber-300">{{ printer.camera.rejectedAdvertisement }}</p>
          </section>
          <!-- IP cameras, redacted to camera-N like printers. Stream is the
               Melt↔camera connection; playback is Melt↔this window. -->
          <section
            v-for="camera in diagnostics?.cameras ?? []"
            :key="camera.camera"
            class="mt-4 border-t border-gray-200 pt-3 dark:border-white/10"
          >
            <h4 class="text-sm font-medium text-gray-900 dark:text-white">{{ t('settingsView.ipCamera') }} · {{ camera.camera }}</h4>
            <dl class="mt-2 space-y-1 text-xs text-gray-600 dark:text-gray-300">
              <div class="flex flex-wrap gap-x-1">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.driver') }}:</dt>
                <dd class="font-mono">{{ camera.driver }}</dd>
              </div>
              <div class="flex flex-wrap gap-x-1">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.linkedPrinters') }}:</dt>
                <dd class="font-mono">{{ camera.linkedPrinters }}</dd>
              </div>
              <div class="flex flex-wrap gap-x-1">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.cameraStreams') }}:</dt>
                <dd class="font-mono">{{ camera.streams.length ? camera.streams.map((stream) => t('settingsView.cameraStreamValue', { quality: stream.quality, transport: stream.owner.transport, viewers: stream.owner.subscribers, reconnects: stream.owner.reconnectAttempts })).join(', ') : t('settingsView.none') }}</dd>
              </div>
              <div v-if="camera.playback" class="flex flex-wrap gap-x-1">
                <dt class="text-gray-500 dark:text-gray-400">{{ t('settingsView.cameraPlayback') }}:</dt>
                <dd class="font-mono">{{ camera.playback.transport ? cameraTransportLabels[camera.playback.transport] : t('settingsView.none') }}</dd>
                <dd v-if="camera.playback.detail" class="basis-full text-amber-700 dark:text-amber-300">{{ camera.playback.detail }}</dd>
              </div>
            </dl>
          </section>
          <p v-if="diagnosticsError" class="mt-3 text-xs text-red-600 dark:text-red-400" role="alert">{{ diagnosticsError }}</p>
          <Button class="mt-4 w-full" :disabled="diagnosticsLoading" @click="runDiagnostics"><PhDesktop class="size-4" aria-hidden="true" /> {{ diagnosticsLoading ? t('common.loading') : t('diagnostics.open') }}</Button>
        </div>
      </Card>
    </div>
  </template>

  <SlideOver :open="slicerAdditionOpen" :title="t('settingsView.addSlicer')" :close-label="t('common.closePanel')" @close="closeSlicerAddition">
    <form id="add-slicer-form" autocomplete="off" class="space-y-4" @submit.prevent="addSlicer">
      <div class="min-w-0">
        <label for="slicer-name" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('settingsView.slicerName') }}</label>
        <input
          ref="slicerNameInput"
          id="slicer-name"
          name="slicer-name"
          v-model="slicerDraft.name"
          type="text"
          autocomplete="off"
          :disabled="slicerBusy"
          :aria-invalid="Boolean(slicerNameError)"
          :aria-describedby="[slicerNameError && 'slicer-name-error', slicerError && 'slicer-error'].filter(Boolean).join(' ') || undefined"
          class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
          @input="slicerNameError = undefined"
        />
        <p v-if="slicerNameError" id="slicer-name-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ slicerNameError }}</p>
      </div>
      <div class="min-w-0">
        <label for="slicer-path" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('settingsView.slicerPath') }}</label>
        <div class="mt-2 flex gap-2">
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
            class="block w-full min-w-0 rounded-md bg-white px-3 py-1.5 font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 placeholder:font-sans placeholder:text-gray-500 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:placeholder:text-gray-400"
          />
          <Button class="shrink-0" :disabled="slicerBusy" @click="browseSlicerPath">{{ t('settingsView.browse') }}</Button>
        </div>
        <p v-if="slicerPathError" id="slicer-path-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ slicerPathError }}</p>
      </div>
      <p v-if="slicerError" id="slicer-error" class="text-sm text-red-600 dark:text-red-400" role="alert">{{ slicerError }}</p>
    </form>
    <template #footer>
      <Button :disabled="slicerBusy" @click="closeSlicerAddition">{{ t('common.cancel') }}</Button>
      <Button variant="primary" type="submit" form="add-slicer-form" :disabled="slicerBusy">{{ slicerBusy ? t('common.adding') : t('settingsView.addSlicer') }}</Button>
    </template>
  </SlideOver>

  <SlideOver
    :open="presetAdditionOpen"
    :title="t('settingsView.addPreset')"
    :description="t('settingsView.presetsDescription', { nozzle: temperatureMaximums.nozzle, bed: temperatureMaximums.bed })"
    :close-label="t('common.closePanel')"
    @close="presetAdditionOpen = false"
  >
    <form id="add-preset-form" autocomplete="off" class="space-y-4" @submit.prevent="addPreset">
      <div class="min-w-0">
        <label for="preset-name" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('settingsView.presetName') }}</label>
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
          class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
        />
        <p v-if="presetErrors.name" id="preset-name-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ presetErrors.name }}</p>
      </div>
      <div class="grid grid-cols-2 gap-3">
        <div class="min-w-0">
          <label for="preset-nozzle" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('settingsView.presetNozzle') }}</label>
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
            class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
          />
          <p v-if="presetErrors.nozzle" id="preset-nozzle-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ presetErrors.nozzle }}</p>
        </div>
        <div class="min-w-0">
          <label for="preset-bed" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('settingsView.presetBed') }}</label>
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
            class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 font-mono text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10"
          />
          <p v-if="presetErrors.bed" id="preset-bed-error" class="mt-1 text-xs text-red-600 dark:text-red-400">{{ presetErrors.bed }}</p>
        </div>
      </div>
    </form>
    <template #footer>
      <Button @click="presetAdditionOpen = false">{{ t('common.cancel') }}</Button>
      <Button variant="primary" type="submit" form="add-preset-form">{{ t('settingsView.addPreset') }}</Button>
    </template>
  </SlideOver>
</template>
