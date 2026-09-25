<script setup lang="ts">
import { Dialog, DialogPanel, DialogTitle, TransitionChild, TransitionRoot } from '@headlessui/vue'
import { PhX } from '@phosphor-icons/vue'
import { ref, watch } from 'vue'
import Button from './Button.vue'
import IconButton from './IconButton.vue'
import type { ActionMenuItem } from './ActionMenu.vue'
import { cachedPreview, loadPreview } from '../preview'

const props = defineProps<{
  file?: { name: string; devicePath: string; sizeBytes?: number; modifiedAt?: string }
  meta: string
  items: ActionMenuItem[]
  closeLabel: string
  unavailableLabel: string
  loadingLabel: string
}>()

const emit = defineEmits<{ close: [] }>()

const source = ref('')
const failed = ref(false)
const loading = ref(false)

function pngUrl(data: string) {
  return `data:image/png;base64,${data}`
}

// Escape, backdrop clicks, the focus trap and focus restore all come from
// HeadlessUI's Dialog rather than a keydown listener of our own.
watch(
  () => props.file,
  async (file, _previous, onCleanup) => {
    source.value = ''
    failed.value = false
    loading.value = false
    if (!file) return
    const request = { path: file.devicePath, sizeBytes: file.sizeBytes, modifiedAt: file.modifiedAt }
    // The card that was just clicked already rendered the grid thumbnail, so
    // showing it costs nothing and the dialog opens with no perceived wait.
    const thumbnail = cachedPreview(request)
    if (thumbnail) source.value = pngUrl(thumbnail.data)
    // Closing the dialog or moving to another file cancels this render if
    // it's still queued, and a slower one must not replace what's shown now.
    const controller = new AbortController()
    onCleanup(() => controller.abort())
    loading.value = true
    try {
      // The user is looking at this one, so it goes ahead of grid thumbnails.
      const large = await loadPreview({ ...request, large: true }, { signal: controller.signal, priority: 'high' })
      if (!controller.signal.aborted) source.value = pngUrl(large.data)
    } catch {
      // A failed large render still leaves the thumbnail worth looking at.
      if (!controller.signal.aborted) failed.value = !source.value
    } finally {
      if (!controller.signal.aborted) loading.value = false
    }
  },
  { immediate: true },
)

// Delete opens a ConfirmDialog, and nesting one modal inside another makes
// Escape ambiguous, so the preview steps aside before any action runs.
function select(item: ActionMenuItem) {
  emit('close')
  item.onSelect()
}
</script>

<template>
  <TransitionRoot as="template" :show="!!file">
    <Dialog class="relative z-50" @close="emit('close')">
      <TransitionChild as="template" enter="ease-out duration-300" enter-from="opacity-0" enter-to="opacity-100" leave="ease-in duration-200" leave-from="opacity-100" leave-to="opacity-0">
        <div class="fixed inset-0 bg-gray-500/75 transition-opacity dark:bg-gray-950/70" />
      </TransitionChild>

      <div class="fixed inset-0 overflow-y-auto overscroll-contain">
        <div class="flex min-h-full items-center justify-center p-4 pb-[calc(1rem+env(safe-area-inset-bottom))]">
          <TransitionChild
            as="template"
            enter="ease-out duration-300"
            enter-from="opacity-0 translate-y-2 scale-95"
            enter-to="opacity-100 translate-y-0 scale-100"
            leave="ease-in duration-200"
            leave-from="opacity-100 translate-y-0 scale-100"
            leave-to="opacity-0 translate-y-2 scale-95"
          >
            <DialogPanel class="w-full max-w-4xl transform overflow-hidden rounded-lg bg-white text-left shadow-xl outline outline-gray-200 transition dark:bg-gray-800 dark:shadow-none dark:outline-white/10">
              <div class="flex items-start gap-3 px-4 py-4 sm:px-6">
                <div class="min-w-0 flex-1">
                  <DialogTitle class="truncate text-base font-semibold text-gray-900 dark:text-white" :title="file?.name" translate="no">{{ file?.name }}</DialogTitle>
                  <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ meta }}</p>
                </div>
                <IconButton class="-mt-2 -mr-2" :title="closeLabel" :aria-label="closeLabel" @click="emit('close')"><PhX class="size-4" aria-hidden="true" /></IconButton>
              </div>

              <div class="grid min-h-60 place-items-center bg-cyan-950/5 dark:bg-gray-950/40">
                <!-- No `w-full`: a slicer's embedded thumbnail can be a few
                     hundred pixels wide, and stretching it to the panel just
                     magnifies its own blur. It renders at its natural size,
                     shrinking only when it outgrows the dialog. -->
                <img
                  v-if="source"
                  :src="source"
                  :alt="file?.name"
                  class="max-h-[70vh] max-w-full object-contain transition-opacity"
                />
                <p v-else-if="loading" class="px-4 py-16 text-sm text-cyan-900/60 dark:text-cyan-100/60">{{ loadingLabel }}</p>
                <p v-else class="px-4 py-16 text-sm text-cyan-900/60 dark:text-cyan-100/60">{{ failed ? unavailableLabel : '' }}</p>
              </div>

              <div v-if="items.length" class="flex flex-wrap justify-end gap-2 px-4 py-4 sm:px-6">
                <Button
                  v-for="item in items"
                  :key="item.label"
                  :variant="item.danger ? 'danger' : 'secondary'"
                  :disabled="item.disabled"
                  @click="select(item)"
                ><component :is="item.icon" class="size-4" aria-hidden="true" /> {{ item.label }}</Button>
              </div>
            </DialogPanel>
          </TransitionChild>
        </div>
      </div>
    </Dialog>
  </TransitionRoot>
</template>
