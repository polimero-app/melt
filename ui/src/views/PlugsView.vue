<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { Locale } from '../i18n'
import { commandDetail } from '../errors'
import { linkChanges, type CameraSummary } from '../cameras'
import { snapshotPlug, type PlugObservation, type PlugSummary } from '../plugs'
import ActionMenu, { type ActionMenuItem } from '../components/ActionMenu.vue'
import SlideOver from '../components/SlideOver.vue'
import TimeoutField from '../components/TimeoutField.vue'
import Button from '../components/Button.vue'
import Card from '../components/Card.vue'
import PlugControl from '../components/PlugControl.vue'
import { PhPencilSimple, PhPlus, PhTrash, PhPlug } from '@phosphor-icons/vue'
import type { Printer, ViewActions } from '../app-types'

const {
  active,
  printers,
  cameras,
  plugs,
  plugsError,
  plugObservations,
  plugActivity,
  locale,
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
  plugs: PlugSummary[]
  plugsError: string | undefined
  plugObservations: Record<string, PlugObservation>
  plugActivity: Record<string, string>
  locale: Locale
  loadPlugs: () => Promise<void>
  refreshPlugs: () => Promise<PlugSummary[]>
}>()

const plugFormOpen = ref(false)
const editingPlug = ref<PlugSummary>()
const plugDraft = ref({ name: '', host: '', username: '', password: '', timeout: '10s', printers: [] as string[], cameras: [] as string[] })
const plugSaving = ref(false)
const plugFormError = ref<string>()

function openPlugForm(plug?: PlugSummary) {
  editingPlug.value = plug && snapshotPlug(plug)
  plugDraft.value = {
    name: plug?.name ?? '',
    host: plug?.host ?? '',
    username: plug?.username ?? '',
    password: '',
    timeout: plug?.timeout ?? '10s',
    printers: [...(plug?.printers ?? [])],
    cameras: [...(plug?.cameras ?? [])],
  }
  plugFormError.value = undefined
  plugFormOpen.value = true
}

async function savePlug() {
  if (plugSaving.value) return
  plugSaving.value = true
  plugFormError.value = undefined
  const draft = plugDraft.value
  const request = { name: draft.name, driver: 'tapo', host: draft.host, username: draft.username, password: draft.password, timeout: draft.timeout }
  let saved: PlugSummary
  try {
    const editing = editingPlug.value
    saved = editing
      ? await invoke<PlugSummary>('plug_edit', { expected: editing, request })
      : await invoke<PlugSummary>('plug_add', { request })
  } catch (reason) {
    plugFormError.value = [message(reason), commandDetail(reason)].filter(Boolean).join(' ')
    await loadPlugs()
    plugSaving.value = false
    return
  }
  // The profile is saved; a retry after a failed link must edit, not add.
  editingPlug.value = snapshotPlug(saved)
  draft.password = ''
  const printerChanges = linkChanges(saved.printers, draft.printers)
  const cameraChanges = linkChanges(saved.cameras, draft.cameras)
  try {
    for (const printer of printerChanges.link) await invoke('plug_link', { plug: saved.name, printer })
    for (const printer of printerChanges.unlink) await invoke('plug_unlink', { plug: saved.name, printer })
    for (const camera of cameraChanges.link) await invoke('plug_link', { plug: saved.name, camera })
    for (const camera of cameraChanges.unlink) await invoke('plug_unlink', { plug: saved.name, camera })
    plugFormOpen.value = false
    showToast(t('plugsView.saved'))
  } catch (reason) {
    plugFormError.value = [t('plugsView.attachmentsFailed'), message(reason), commandDetail(reason)].filter(Boolean).join(' ')
  } finally {
    await loadPlugs()
    plugSaving.value = false
  }
}

function plugActionItems(plug: PlugSummary): ActionMenuItem[] {
  return [
    { label: t('plugsView.edit'), icon: PhPencilSimple, onSelect: () => openPlugForm(plug) },
    { label: t('plugsView.remove'), icon: PhTrash, danger: true, onSelect: () => removePlug(plug.name) },
  ]
}

function removePlug(name: string) {
  askConfirmation(
    { title: 'plugsView.removeTitle', description: 'plugsView.removeDescription', confirm: 'plugsView.remove' },
    { name },
    async () => {
      const result = await invoke<{ warnings: unknown[] }>('plug_remove', { name })
      await loadPlugs()
      if (result.warnings.length) showToast(t('plugsView.passwordNotDeleted'), 'error')
      else showToast(t('plugsView.removed'))
    },
  )
}
</script>

<template>
  <template v-if="active">
    <div class="mb-7 px-4 sm:px-0 lg:flex lg:items-center lg:justify-between">
      <div class="min-w-0 flex-1">
        <h2 class="text-2xl/7 font-bold text-gray-900 sm:truncate sm:text-3xl sm:tracking-tight dark:text-white">{{ t('nav.plugs') }}</h2>
        <p class="mt-2 max-w-4xl text-sm text-gray-500 dark:text-gray-400">{{ t('plugsView.description') }}</p>
      </div>
      <div class="mt-5 flex lg:mt-0 lg:ml-4">
        <Button variant="primary" @click="openPlugForm()"><PhPlus class="size-4" aria-hidden="true" /> {{ t('plugsView.add') }}</Button>
      </div>
    </div>
    <p v-if="plugsError" class="mb-5 px-4 text-sm text-red-600 sm:px-0 dark:text-red-400" role="alert">{{ plugsError }}</p>
    <ul v-if="plugs.length" class="grid grid-cols-1 gap-5 xl:grid-cols-2">
      <Card v-for="plug in plugs" :key="plug.id" as="li" class="flex items-start gap-2 py-4 pr-2 pl-4 sm:pl-6">
        <PhPlug class="mt-0.5 size-5 shrink-0 text-gray-500 dark:text-gray-400" aria-hidden="true" />
        <div class="min-w-0 flex-1">
          <span class="block truncate text-sm font-semibold text-gray-900 dark:text-white">{{ plug.name }}</span>
          <span class="mt-0.5 block truncate font-mono text-xs text-gray-500 dark:text-gray-400">{{ plug.host }}</span>
          <PlugControl class="mt-2" :plug="plug" :observation="plugObservations[plug.id]" :activity="plugActivity" :refresh="refreshPlugs" :t="t" :locale="locale" :show-name="false" />
        </div>
        <ActionMenu :label="t('plugsView.actions')" :items="plugActionItems(plug)" />
      </Card>
    </ul>
    <button
      v-else-if="!plugsError"
      type="button"
      class="relative flex min-h-55 w-full flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 p-12 text-center hover:border-gray-400 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-cyan-600 dark:border-white/15 dark:hover:border-white/25 dark:focus-visible:outline-cyan-500"
      @click="openPlugForm()"
    >
      <PhPlug class="mx-auto size-12 text-gray-500 dark:text-gray-400" aria-hidden="true" />
      <span class="mt-2 block text-sm font-semibold text-gray-900 dark:text-white">{{ t('plugsView.empty') }}</span>
      <span class="mt-1 block text-sm text-gray-500 dark:text-gray-400">{{ t('plugsView.emptyHint') }}</span>
    </button>
  </template>

  <SlideOver :open="plugFormOpen" :title="editingPlug ? t('plugsView.edit') : t('plugsView.add')" :description="t('plugsView.formDescription')" :close-label="t('common.closePanel')" @close="!plugSaving && (plugFormOpen = false)">
    <form id="plug-form" class="space-y-4" @submit.prevent="savePlug">
      <div>
        <label for="plug-name" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('addition.name') }}</label>
        <!-- Mirrors profiles::normalize_name so the browser catches spaces before the round trip. -->
        <input id="plug-name" v-model="plugDraft.name" required maxlength="64" pattern="[A-Za-z0-9][A-Za-z0-9._\-]*" :title="t('plugsView.nameHint')" aria-describedby="plug-name-hint" autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
        <p id="plug-name-hint" class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ t('plugsView.nameHint') }}</p>
      </div>
      <div>
        <label for="plug-host" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('plugsView.host') }}</label>
        <input id="plug-host" v-model="plugDraft.host" required autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
      </div>
      <div>
        <label for="plug-username" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('plugsView.username') }}</label>
        <input id="plug-username" v-model="plugDraft.username" type="email" required autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
      </div>
      <div>
        <label for="plug-password" class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ editingPlug ? t('plugsView.passwordKeep') : t('plugsView.password') }}</label>
        <input id="plug-password" v-model="plugDraft.password" type="password" :required="!editingPlug" autocomplete="new-password" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
      </div>
      <TimeoutField id="plug-timeout" v-model="plugDraft.timeout" :t="t" />
      <fieldset v-if="printers.length">
        <legend class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('plugsView.printers') }}</legend>
        <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ t('plugsView.attachmentsHint') }}</p>
        <div class="mt-2 space-y-2">
          <label v-for="printer in printers" :key="printer.name" class="flex items-center gap-2 text-sm text-gray-900 dark:text-white">
            <input v-model="plugDraft.printers" type="checkbox" :value="printer.name" class="size-4 accent-cyan-600" />
            {{ printer.name }}
          </label>
        </div>
      </fieldset>
      <fieldset v-if="cameras.length">
        <legend class="block text-sm/6 font-medium text-gray-900 dark:text-white">{{ t('plugsView.cameras') }}</legend>
        <div class="mt-2 space-y-2">
          <label v-for="camera in cameras" :key="camera.id" class="flex items-center gap-2 text-sm text-gray-900 dark:text-white">
            <input v-model="plugDraft.cameras" type="checkbox" :value="camera.name" class="size-4 accent-cyan-600" />
            {{ camera.name }}
          </label>
        </div>
      </fieldset>
      <p class="text-xs text-gray-500 dark:text-gray-400">{{ t('plugsView.connectionHint') }}</p>
      <p v-if="plugFormError" class="text-sm text-red-600 dark:text-red-400" role="alert">{{ plugFormError }}</p>
    </form>
    <template #footer>
      <Button :disabled="plugSaving" @click="plugFormOpen = false">{{ t('common.cancel') }}</Button>
      <Button variant="primary" type="submit" form="plug-form" :disabled="plugSaving">{{ editingPlug ? (plugSaving ? t('common.saving') : t('profiles.save')) : (plugSaving ? t('common.adding') : t('plugsView.add')) }}</Button>
    </template>
  </SlideOver>
</template>
