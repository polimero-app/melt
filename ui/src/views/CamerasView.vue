<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Locale } from '../i18n'
import { shouldRunCamera } from '../presentation'
import { commandDetail } from '../errors'
import { linkChanges, type CameraSummary, type Quality } from '../cameras'
import { plugsForCamera, type PlugObservation, type PlugSummary } from '../plugs'
import ActionMenu, { type ActionMenuItem } from '../components/ActionMenu.vue'
import SlideOver from '../components/SlideOver.vue'
import TimeoutField from '../components/TimeoutField.vue'
import Button from '../components/Button.vue'
import Card from '../components/Card.vue'
import CameraViewer from '../components/CameraViewer.vue'
import PlugControl from '../components/PlugControl.vue'
import { PhPencilSimple, PhPlus, PhTrash, PhVideoCamera } from '@phosphor-icons/vue'
import type { Printer, ViewActions } from '../app-types'

const {
  active,
  printers,
  cameras,
  camerasError,
  documentVisible,
  plugs,
  plugObservations,
  plugActivity,
  locale,
  loadCameras,
  loadPlugs,
  refreshPlugs,
  t,
  message,
  showToast,
  askConfirmation,
} = defineProps<ViewActions & {
  active: boolean
  printers: Printer[]
  cameras: CameraSummary[]
  camerasError: string | undefined
  documentVisible: boolean
  plugs: PlugSummary[]
  plugObservations: Record<string, PlugObservation>
  plugActivity: Record<string, string>
  locale: Locale
  loadCameras: () => Promise<void>
  loadPlugs: () => Promise<void>
  refreshPlugs: () => Promise<PlugSummary[]>
}>()

const viewedCameraName = ref<string>()
const viewedCamera = computed(() => cameras.find((camera) => camera.name === viewedCameraName.value) ?? cameras[0])
const viewedCameraMayRun = computed(() => shouldRunCamera(active ? 'cameras' : '', 'cameras', documentVisible, true))
const cameraQuality = ref<Quality>('high')
const cameraViewer = ref<InstanceType<typeof CameraViewer>>()
const cameraFormOpen = ref(false)
const editingCamera = ref<string>()
const cameraDraft = ref({ name: '', host: '', username: '', password: '', rtspPort: '', timeout: '10s', printers: [] as string[] })
const cameraSaving = ref(false)
const cameraFormError = ref<string>()

function openCameraForm(camera?: CameraSummary) {
  editingCamera.value = camera?.name
  cameraDraft.value = {
    name: camera?.name ?? '',
    host: camera?.host ?? '',
    username: camera?.username ?? '',
    password: '',
    rtspPort: camera && camera.rtspPort !== 554 ? String(camera.rtspPort) : '',
    timeout: camera?.timeout ?? '10s',
    printers: [...(camera?.printers ?? [])],
  }
  cameraFormError.value = undefined
  cameraFormOpen.value = true
}

async function saveCamera() {
  if (cameraSaving.value) return
  cameraSaving.value = true
  cameraFormError.value = undefined
  const draft = cameraDraft.value
  const request = {
    name: draft.name,
    driver: 'tapo',
    host: draft.host,
    username: draft.username,
    password: draft.password,
    rtspPort: draft.rtspPort ? Number(draft.rtspPort) : null,
    timeout: draft.timeout,
  }
  try {
    const editing = editingCamera.value
    const saved = editing
      ? await invoke<CameraSummary>('camera_edit', { name: editing, request })
      : await invoke<CameraSummary>('camera_add', { request })
    // The profile is saved; a retry after a failed link must edit, not add.
    editingCamera.value = saved.name
    draft.password = ''
    const { link, unlink } = linkChanges(saved.printers, draft.printers)
    for (const printer of link) await invoke('camera_link', { camera: saved.name, printer, select: false })
    for (const printer of unlink) await invoke('camera_unlink', { camera: saved.name, printer })
    cameraFormOpen.value = false
    viewedCameraName.value = saved.name
    await loadCameras()
    void loadPlugs()
    showToast(t('camerasView.saved'))
    // Same name, same source key: restart so the viewer picks up new settings.
    await nextTick()
    void cameraViewer.value?.refresh()
  } catch (reason) {
    cameraFormError.value = [message(reason), commandDetail(reason)].filter(Boolean).join(' ')
  } finally {
    cameraSaving.value = false
  }
}

function cameraActionItems(camera: CameraSummary): ActionMenuItem[] {
  return [
    { label: t('camerasView.edit'), icon: PhPencilSimple, onSelect: () => openCameraForm(camera) },
    { label: t('camerasView.remove'), icon: PhTrash, danger: true, onSelect: () => removeCamera(camera.name) },
  ]
}

function removeCamera(name: string) {
  askConfirmation(
    { title: 'camerasView.removeTitle', description: 'camerasView.removeDescription', confirm: 'camerasView.remove' },
    { name },
    async () => {
      const result = await invoke<{ warnings: unknown[] }>('camera_remove', { name })
      // Reloading drops the camera from every viewer's source, which stops them.
      await loadCameras()
      void loadPlugs()
      if (result.warnings.length) showToast(t('camerasView.passwordNotDeleted'), 'error')
      else showToast(t('camerasView.removed'))
    },
  )
}

function plugChanged(plug: PlugSummary) {
  if (viewedCamera.value && plugsForCamera(viewedCamera.value.name, [plug]).length) void cameraViewer.value?.refresh()
}
</script>

<template>
  <template v-if="active">
    <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
      <div class="min-w-0 flex-1">
        <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ t('nav.cameras') }}</h2>
        <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">{{ t('camerasView.description') }}</p>
      </div>
      <div class="mt-5 flex lg:mt-0 lg:ml-4">
        <Button variant="primary" @click="openCameraForm()"><PhPlus class="size-4" aria-hidden="true" /> {{ t('camerasView.add') }}</Button>
      </div>
    </div>
    <p v-if="camerasError" class="mb-5 px-4 text-sm text-red-600 sm:px-0 dark:text-red-400" role="alert">{{ camerasError }}</p>
    <section v-if="viewedCamera" class="grid grid-cols-1 gap-5 xl:grid-cols-[minmax(16rem,1fr)_minmax(0,2fr)]">
      <Card as="ul" class="divide-y divide-gray-200 self-start dark:divide-white/10">
        <li
          v-for="camera in cameras"
          :key="camera.id"
          class="flex items-center gap-2 py-3 pr-2 pl-4 sm:pl-6"
          :class="viewedCamera.name === camera.name && 'bg-gray-50 dark:bg-white/5'"
        >
          <button type="button" class="min-w-0 flex-1 text-left" :aria-current="viewedCamera.name === camera.name ? 'true' : undefined" @click="viewedCameraName = camera.name">
            <span class="block truncate text-sm font-semibold text-gray-900 dark:text-white">{{ camera.name }}</span>
            <span class="mt-0.5 block truncate font-mono text-xs text-gray-500 dark:text-gray-400">{{ camera.host }}</span>
            <span class="mt-0.5 block truncate text-xs text-gray-500 dark:text-gray-400">{{ camera.printers.length ? camera.printers.join(', ') : t('camerasView.notLinked') }}</span>
          </button>
          <ActionMenu :label="t('camerasView.actions')" :items="cameraActionItems(camera)" />
        </li>
      </Card>
      <div class="space-y-3">
      <CameraViewer
        ref="cameraViewer"
        :source="{ camera: viewedCamera.name }"
        :quality="cameraQuality"
        :active="viewedCameraMayRun"
        can-stream
        can-snapshot
        :title="viewedCamera.name"
        :t="t"
        @snapshot-saved="showToast(t('camera.snapshotSaved'))"
      >
        <template #controls>
          <select v-model="cameraQuality" :aria-label="t('camerasView.quality')" class="rounded-md bg-white py-1.5 pl-2 text-xs font-medium text-gray-700 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-gray-200 dark:outline-white/10 dark:*:bg-gray-800">
            <option value="high">{{ t('camerasView.qualityHigh') }}</option>
            <option value="low">{{ t('camerasView.qualityLow') }}</option>
          </select>
        </template>
      </CameraViewer>
      <div v-if="plugsForCamera(viewedCamera.name, plugs).length" class="space-y-3 rounded-lg border border-gray-200 px-4 py-3 dark:border-white/10">
        <PlugControl v-for="plug in plugsForCamera(viewedCamera.name, plugs)" :key="plug.id" :plug="plug" :observation="plugObservations[plug.id]" :activity="plugActivity" :refresh="refreshPlugs" :t="t" :locale="locale" @changed="plugChanged(plug)" />
      </div>
      </div>
    </section>
    <button
      v-else-if="!camerasError"
      type="button"
      class="relative flex min-h-55 w-full flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 p-12 text-center hover:border-gray-400 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:border-white/15 dark:hover:border-white/25 dark:focus-visible:outline-cyan-500"
      @click="openCameraForm()"
    >
      <PhVideoCamera class="mx-auto size-12 text-gray-500 dark:text-gray-400" aria-hidden="true" />
      <span class="mt-2 block text-sm font-semibold text-gray-900 dark:text-white">{{ t('camerasView.empty') }}</span>
      <span class="mt-1 block text-sm text-gray-500 dark:text-gray-400">{{ t('camerasView.emptyHint') }}</span>
    </button>
  </template>

  <SlideOver :open="cameraFormOpen" :title="editingCamera ? t('camerasView.edit') : t('camerasView.add')" :description="t('camerasView.formDescription')" :close-label="t('common.closePanel')" @close="!cameraSaving && (cameraFormOpen = false)">
    <form id="camera-form" class="space-y-4" @submit.prevent="saveCamera">
      <div>
        <label for="camera-name" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.name') }}</label>
        <input id="camera-name" v-model="cameraDraft.name" required maxlength="64" autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
      </div>
      <div>
        <label for="camera-host" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.host') }}</label>
        <input id="camera-host" v-model="cameraDraft.host" required autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
      </div>
      <div>
        <label for="camera-username" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('camerasView.username') }}</label>
        <input id="camera-username" v-model="cameraDraft.username" required autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
      </div>
      <div>
        <label for="camera-password" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ editingCamera ? t('camerasView.passwordKeep') : t('camerasView.password') }}</label>
        <input id="camera-password" v-model="cameraDraft.password" type="password" :required="!editingCamera" autocomplete="new-password" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
      </div>
      <div class="grid grid-cols-2 gap-4">
        <div>
          <label for="camera-rtsp-port" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('camerasView.rtspPort') }}</label>
          <input id="camera-rtsp-port" v-model="cameraDraft.rtspPort" type="number" min="1" max="65535" placeholder="554" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
        </div>
        <TimeoutField id="camera-timeout" v-model="cameraDraft.timeout" :t="t" />
      </div>
      <fieldset v-if="printers.length">
        <legend class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('camerasView.printers') }}</legend>
        <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ t('camerasView.printersHint') }}</p>
        <div class="mt-2 space-y-2">
          <label v-for="printer in printers" :key="printer.name" class="flex items-center gap-2 text-sm text-gray-900 dark:text-white">
            <input v-model="cameraDraft.printers" type="checkbox" :value="printer.name" class="size-4 accent-cyan-600" />
            {{ printer.name }}
          </label>
        </div>
      </fieldset>
      <p class="text-xs text-gray-500 dark:text-gray-400">{{ t('camerasView.connectionHint') }}</p>
      <p v-if="cameraFormError" class="text-sm text-red-600 dark:text-red-400" role="alert">{{ cameraFormError }}</p>
    </form>
    <template #footer>
      <Button :disabled="cameraSaving" @click="cameraFormOpen = false">{{ t('common.cancel') }}</Button>
      <Button variant="primary" type="submit" form="camera-form" :disabled="cameraSaving">{{ editingCamera ? (cameraSaving ? t('common.saving') : t('profiles.save')) : (cameraSaving ? t('common.adding') : t('camerasView.add')) }}</Button>
    </template>
  </SlideOver>
</template>
