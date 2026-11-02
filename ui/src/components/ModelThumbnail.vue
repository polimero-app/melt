<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { loadPreview } from '../preview'

const props = defineProps<{ path: string; sizeBytes?: number; modifiedAt?: string; alt: string; unavailableLabel: string }>()
const container = ref<HTMLDivElement>()
// An <img>, not a <canvas>: the library grid uses content-visibility: auto,
// and WebKit drops a skipped canvas's bitmap with nothing to redraw it from.
// An image re-decodes from its src whenever it scrolls back into view.
const src = ref('')
const failed = ref(false)
const loading = ref(false)
const loadedKey = ref('')
let observer: IntersectionObserver | undefined

// Set while a load is in flight. Aborting it tells the preview queue this
// card no longer wants the render, so work still queued for it is dropped.
let controller: AbortController | undefined

async function load() {
  const key = previewIdentity()
  if (controller || loadedKey.value === key) return
  const current = new AbortController()
  controller = current
  failed.value = false
  loading.value = true
  try {
    const preview = await loadPreview({
      path: props.path,
      sizeBytes: props.sizeBytes,
      modifiedAt: props.modifiedAt,
    }, { signal: current.signal })
    if (preview.kind === 'png') {
      src.value = `data:image/png;base64,${preview.data}`
      loadedKey.value = key
      return
    }
    // All archive parsing and geometry rasterization happen in Rust. The UI
    // only decodes and displays the returned PNG.
    throw new Error('Raster preview unavailable')
  } catch {
    if (!current.signal.aborted) failed.value = true
  } finally {
    if (controller === current) {
      controller = undefined
      loading.value = false
    }
  }
}

function cancelLoad() {
  controller?.abort()
  controller = undefined
  loading.value = false
}

function scheduleLoad() {
  if (!container.value) return
  const bounds = container.value.getBoundingClientRect()
  if (bounds.bottom >= -360 && bounds.top <= window.innerHeight + 360) void load()
}

onMounted(() => {
  if ('IntersectionObserver' in window && container.value) {
    observer = new IntersectionObserver((entries) => {
      if (entries.some((entry) => entry.isIntersecting)) void load()
    }, { rootMargin: '360px' })
    observer.observe(container.value)
  } else {
    void load()
  }
})

onBeforeUnmount(() => {
  observer?.disconnect()
  cancelLoad()
})

// The same path with a new size or mtime is a different file, so it must
// redraw rather than keep showing the old image.
function previewIdentity() {
  return JSON.stringify([props.path, props.sizeBytes, props.modifiedAt])
}

watch(previewIdentity, () => {
  cancelLoad()
  loadedKey.value = ''
  src.value = ''
  failed.value = false
  scheduleLoad()
})
</script>

<template>
  <!-- Once rendering has failed there is no image to describe, so the element
       stops claiming to be one and the fallback text is read instead. -->
  <div
    ref="container"
    class="relative grid aspect-video place-items-center overflow-hidden"
    :role="failed ? undefined : 'img'"
    :aria-label="failed ? undefined : alt"
    :aria-busy="loading"
  >
    <img v-if="src && !failed" :src="src" alt="" class="absolute inset-0 size-full object-cover" aria-hidden="true" @error="failed = true" />
    <span v-if="failed" class="absolute inset-0 grid place-items-center text-xs font-medium text-cyan-900/60 dark:text-cyan-100/60">{{ props.unavailableLabel }}</span>
  </div>
</template>
