<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open as openFileDialog } from '@tauri-apps/plugin-dialog'
import type { Locale } from '../i18n'
import type { PrinterBadge } from '../monitoring'
import { formatSize, formatDate } from '../formatting'
import { inspectPrintFile, printTargetState, startPrintFile, type PrintFile, type PrintPackage } from '../printing'
import { badgeDotClasses } from '../presentation'
import { naturallyDescending, nextSort, shouldDeferLibraryCards, sortedBy, type FileSort, fileTypeLabel } from '../library'
import ActionMenu, { type ActionMenuItem } from '../components/ActionMenu.vue'
import SlideOver from '../components/SlideOver.vue'
import Button from '../components/Button.vue'
import IconButton from '../components/IconButton.vue'
import MiddleEllipsis from '../components/MiddleEllipsis.vue'
import Card from '../components/Card.vue'
import ModelThumbnail from '../components/ModelThumbnail.vue'
import FilePreviewDialog from '../components/FilePreviewDialog.vue'
import {
  PhArrowsClockwise,
  PhArrowDown,
  PhArrowUp,
  PhCaretDown,
  PhCaretRight,
  PhDesktop,
  PhFolder,
  PhFolderOpen,
  PhMagnifyingGlass,
  PhMagnifyingGlassPlus,
  PhPlay,
  PhTrash,
  PhUploadSimple,
  PhX,
} from '@phosphor-icons/vue'
import type {
  ModelTone,
  Printer,
  PrintStageEvent,
  FileEntry,
  LibraryBreadcrumb,
  LibraryListResponse,
  SlicerSetting,
  ViewActions,
} from '../app-types'

const {
  active,
  printers,
  slicers,
  locale,
  badgeFor,
  isReachable,
  refreshMonitoring,
  t,
  message,
  showToast,
  askConfirmation,
} = defineProps<ViewActions & {
  active: boolean
  printers: Printer[]
  slicers: SlicerSetting[]
  locale: Locale
  badgeFor: (name: string) => PrinterBadge
  isReachable: (badge: PrinterBadge) => boolean
  refreshMonitoring: () => Promise<void>
}>()

const libraryFiles = defineModel<FileEntry[]>('libraryFiles', { required: true })

const modelTones: Record<ModelTone, { preview: string; shape: string }> = {
  cyan: {
    preview: 'bg-linear-135 from-cyan-100 to-slate-200 dark:from-cyan-950 dark:to-slate-900',
    shape: 'h-20 w-25 bg-cyan-400/75 shadow-[18px_18px_0_var(--color-cyan-800)]',
  },
  amber: {
    preview: 'bg-linear-135 from-amber-100 to-slate-200 dark:from-amber-950 dark:to-slate-900',
    shape: 'h-20 w-25 rounded-l-[40%] rounded-r-[12%] bg-amber-400/75 shadow-[18px_18px_0_var(--color-amber-800)]',
  },
  slate: {
    preview: 'bg-linear-135 from-slate-200 to-slate-300 dark:from-slate-800 dark:to-slate-900',
    shape: 'h-25 w-17.5 rounded-t-[50%] rounded-b-[12%] bg-slate-400/75 shadow-[15px_17px_0_var(--color-slate-600)]',
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
  other: 'slate',
}

const libraryFilesLoading = ref(false)
const libraryFilesError = ref<string>()
const libraryPath = ref<string>()
const libraryParent = ref<string>()
const libraryBreadcrumbs = ref<LibraryBreadcrumb[]>([])
const printTarget = ref<FileEntry & PrintFile>()
const printPrinters = computed(() => printTarget.value?.printer
  ? printers.filter((printer) => printer.name === printTarget.value?.printer)
  : printers)
const printPreparing = ref(false)
const printBusy = ref(false)
const currentPrintTargetState = computed(() => printTargetState(printPrinters.value.length, printBusy.value))
const printStage = ref<PrintStageEvent>()
const printPackage = ref<PrintPackage>()
const printPlate = ref<number>()
const printDisplayName = ref('')
let printStageUnlisten: UnlistenFn | undefined
let libraryChangeTimer: number | undefined
let libraryChangeUnlisten: UnlistenFn | undefined
let libraryRequest = 0
const searchTerm = ref('')
const librarySort = ref<FileSort>({ key: 'name', descending: false })
const uploadBusy = ref(false)

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
// returning to this view instead.
function handleLibraryChange(path: string) {
  if (libraryChangeTimer !== undefined) window.clearTimeout(libraryChangeTimer)
  libraryChangeTimer = window.setTimeout(() => {
    if (active && path === libraryPath.value) void loadLibraryFiles(path)
  }, 300)
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

function fileToneFor(file: FileEntry) {
  return modelTones[mediaTones[file.mediaType]]
}

const normalizedSearchTerm = computed(() => searchTerm.value.trim().toLocaleLowerCase(locale))
const matchesSearch = (file: FileEntry) => !normalizedSearchTerm.value || file.name.toLocaleLowerCase(locale).includes(normalizedSearchTerm.value)

// The backend lists the whole directory, so search is applied only here.
const visibleDirectories = computed(() => libraryFiles.value.filter((file) => file.type === 'directory' && matchesSearch(file)))

const visibleFiles = computed(() => sortedBy(
  libraryFiles.value.filter((file) => file.type === 'file' && matchesSearch(file)),
  librarySort.value,
))

const deferLibraryCards = computed(() => shouldDeferLibraryCards(visibleFiles.value.length))
const enabledSlicers = computed(() => slicers.filter((slicer) => slicer.enabled))
const isModelFile = (file: FileEntry) => /\.(3mf|stl|obj|zip)$/i.test(file.name)
const previewFile = ref<FileEntry>()

function fileActionItems(file: FileEntry): ActionMenuItem[] {
  const items: ActionMenuItem[] = [
    {
      label: t('dashboard.print'),
      icon: PhPlay,
      disabled: !printers.length || printPreparing.value || printBusy.value,
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

async function requestPrint(file: FileEntry, printer?: string) {
  if (printPreparing.value || printBusy.value || printTarget.value) return
  const target = { ...file, printer }
  printPreparing.value = true
  printPackage.value = undefined
  printPlate.value = undefined
  printDisplayName.value = file.name.replace(/\.gcode\.3mf$|\.3mf$|\.gcode$/i, '')
  try {
    if (/\.3mf$/i.test(file.name)) {
      const inspected = await inspectPrintFile(target)
      if (!inspected.sliced) throw new Error(t('errors.jobFileInvalid'))
      printPackage.value = inspected
      printPlate.value = inspected.plates.find((plate) => plate.gcodePath)?.index
    }
    printTarget.value = target
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    printPreparing.value = false
  }
}

async function printToPrinter(name: string) {
  const file = printTarget.value
  if (!file || printBusy.value) return
  printBusy.value = true
  printStage.value = { stage: 'inspect', percent: 0 }
  try {
    await startPrintFile(file, name, {
      plate: printPlate.value,
      displayName: printDisplayName.value || undefined,
    })
    printTarget.value = undefined
    showToast(t('filesView.sentToPrinter', { name: file.name }))
    void refreshMonitoring()
  } catch (reason) {
    showToast(message(reason), 'error')
  } finally {
    printBusy.value = false
    printStage.value = undefined
  }
}

onMounted(() => {
  if (active) void loadLibraryFiles(libraryPath.value)
  void listen<string>('library-changed', (event) => handleLibraryChange(event.payload)).then((unlisten) => {
    libraryChangeUnlisten = unlisten
  })
  void listen<PrintStageEvent>('print-stage', (event) => {
    printStage.value = event.payload
  }).then((unlisten) => {
    printStageUnlisten = unlisten
  })
})

onUnmounted(() => {
  libraryChangeUnlisten?.()
  printStageUnlisten?.()
  if (libraryChangeTimer !== undefined) window.clearTimeout(libraryChangeTimer)
})

defineExpose({ refresh: () => loadLibraryFiles(libraryPath.value), requestPrint, printPreparing, printBusy })

</script>

<template>
  <template v-if="active">
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
            <PhCaretRight v-if="index > 0" class="mr-4 size-5 shrink-0 text-gray-500 dark:text-gray-400" aria-hidden="true" />
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
          v-model="librarySort.key"
          @change="librarySort.descending = naturallyDescending[librarySort.key]"
          :aria-label="t('filesView.sortBy')"
          class="col-start-1 row-start-1 w-auto appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-sm text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800"
        >
          <option value="name">{{ t('filesView.sortName') }}</option>
          <option value="size">{{ t('filesView.sortSize') }}</option>
          <option value="modified">{{ t('filesView.sortModified') }}</option>
        </select>
        <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-4 self-center justify-self-end text-gray-500 dark:text-gray-400" aria-hidden="true" />
      </div>
      <IconButton
        :title="t(librarySort.descending ? 'filesView.sortAscending' : 'filesView.sortDescending')"
        :aria-label="t(librarySort.descending ? 'filesView.sortAscending' : 'filesView.sortDescending')"
        @click="librarySort = nextSort(librarySort, librarySort.key)"
      >
        <PhArrowDown v-if="librarySort.descending" class="size-4" aria-hidden="true" />
        <PhArrowUp v-else class="size-4" aria-hidden="true" />
      </IconButton>
      <!-- Input with leading icon -->
      <div class="grid grid-cols-1">
        <input
          name="file-search"
          v-model="searchTerm"
          autocomplete="off"
          class="col-start-1 row-start-1 block w-40 rounded-md bg-white py-1.5 pr-3 pl-10 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 placeholder:text-gray-500 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:w-64 sm:pl-9 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:placeholder:text-gray-400"
          :aria-label="t('filesView.search')"
          :placeholder="t('filesView.search') + '…'"
          type="search"
        />
        <PhMagnifyingGlass class="pointer-events-none col-start-1 row-start-1 ml-3 size-5 self-center text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
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

    <div v-if="libraryFilesError && libraryFiles.length" class="mb-5 flex items-center justify-between gap-3 rounded-md border border-amber-300/70 bg-amber-50 px-4 py-2 text-xs text-amber-900 dark:border-amber-400/20 dark:bg-amber-400/10 dark:text-amber-200" role="alert">
      <span class="min-w-0">{{ t('common.refreshFailed', { reason: libraryFilesError }) }}</span>
      <Button :disabled="libraryFilesLoading" @click="loadLibraryFiles(libraryPath)">{{ t('common.retry') }}</Button>
    </div>

    <div v-if="visibleDirectories.length" class="mb-5 grid gap-4 grid-cols-[repeat(auto-fill,minmax(min(100%,220px),1fr))]">
      <button
        v-for="directory in visibleDirectories"
        :key="directory.devicePath"
        type="button"
        class="min-h-28 bg-white px-4 py-5 text-left shadow-sm transition hover:bg-cyan-50 hover:shadow-md sm:rounded-lg sm:p-6 dark:bg-gray-800/50 dark:shadow-none dark:outline dark:-outline-offset-1 dark:outline-white/10 dark:hover:bg-cyan-400/5 dark:hover:outline-cyan-400/30"
        @click="navigateToDirectory(directory.devicePath)"
      >
        <PhFolder class="size-7 text-cyan-600 dark:text-cyan-400" aria-hidden="true" />
        <MiddleEllipsis class="mt-3 text-sm font-medium text-gray-900 dark:text-white" :text="directory.name" :title="directory.name" translate="no" />
        <span class="mt-1 block text-xs text-gray-500 dark:text-gray-400">{{ t('filesView.folder') }}</span>
        <span v-if="directory.modifiedAt" class="mt-1 block text-xs whitespace-nowrap text-gray-500 dark:text-gray-400">{{ formatDate(directory.modifiedAt, locale) }}</span>
      </button>
    </div>

    <!-- Grid list -->
    <div class="grid gap-4 grid-cols-[repeat(auto-fill,minmax(min(100%,220px),1fr))]">
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
        <div class="px-4 py-5 sm:p-6">
          <div class="flex items-start gap-3">
            <h3 class="min-w-0 flex-1 line-clamp-2 wrap-anywhere text-sm font-medium text-gray-900 dark:text-white" :title="file.name" translate="no">{{ file.name }}</h3>
            <ActionMenu v-if="fileActionItems(file).length" class="-mt-2 -mr-2" :label="t('filesView.moreActions')" :items="fileActionItems(file)" />
          </div>
          <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">
            <span class="block">{{ formatSize(file.sizeBytes, locale) }}</span>
            <span class="mt-1 block whitespace-nowrap">{{ formatDate(file.modifiedAt, locale) }}</span>
          </p>
        </div>
      </Card>
      <div v-if="!visibleDirectories.length && !visibleFiles.length" class="col-span-full mx-4 flex min-h-65 flex-col items-center justify-center rounded-lg border-2 border-dashed border-gray-300 text-center sm:mx-0 dark:border-white/15">
        <PhFolder class="mx-auto size-12 text-gray-500 dark:text-gray-400" aria-hidden="true" />
        <h3 class="mt-2 text-sm font-semibold text-gray-900 dark:text-white">{{ t('filesView.emptyTitle') }}</h3>
        <p class="mt-1 text-sm text-gray-500 dark:text-gray-400">{{ libraryFilesLoading ? t('common.loading') : libraryFilesError ?? t('filesView.emptyDescription') }}</p>
      </div>
    </div>
  </template>

  <SlideOver
    :open="printTarget !== undefined"
    :title="t('filesView.printer')"
    :description="printTarget?.name"
    :close-label="t('common.closePanel')"
    @close="!printBusy && (printTarget = undefined)"
  >
    <ul v-if="printPrinters.length" class="divide-y divide-gray-200 dark:divide-white/10">
      <li v-for="printer in printPrinters" :key="printer.name" class="flex items-center justify-between gap-3 py-3">
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
        <input v-model="printDisplayName" name="print-display-name" maxlength="99" autocomplete="off" class="mt-2 block w-full rounded-md bg-white px-3 py-1.5 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10" />
      </label>
      <label class="block text-sm font-medium text-gray-700 dark:text-gray-200">
        {{ t('filesView.plate') }}
        <div class="mt-2 grid grid-cols-1">
          <select v-model="printPlate" name="print-plate" class="col-start-1 row-start-1 w-full appearance-none rounded-md bg-white py-1.5 pr-8 pl-3 text-base text-gray-900 outline-1 -outline-offset-1 outline-gray-300 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-cyan-600 sm:text-sm/6 dark:bg-white/5 dark:text-white dark:outline-white/10 dark:*:bg-gray-800">
            <option v-for="plate in printPackage.plates.filter((candidate) => candidate.gcodePath)" :key="plate.index" :value="plate.index">
              {{ plate.name || `${t('filesView.plate')} ${plate.index}` }}
            </option>
          </select>
          <PhCaretDown class="pointer-events-none col-start-1 row-start-1 mr-2 size-5 self-center justify-self-end text-gray-500 sm:size-4 dark:text-gray-400" aria-hidden="true" />
        </div>
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
    :meta="previewFile ? `${formatSize(previewFile.sizeBytes, locale)} · ${formatDate(previewFile.modifiedAt, locale)}` : ''"
    :items="previewFile ? fileActionItems(previewFile) : []"
    :close-label="t('common.close')"
    :unavailable-label="t('filesView.previewUnavailable')"
    :loading-label="t('filesView.previewLoading')"
    @close="previewFile = undefined"
  />
</template>
