<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'

type PreviewResponse = { kind: 'png' | 'model'; data: string }

const props = defineProps<{ path: string; sizeBytes?: number; modifiedAt?: string; alt: string; unavailableLabel: string }>()
const container = ref<HTMLDivElement>()
const canvas = ref<HTMLCanvasElement>()
const failed = ref(false)
const loading = ref(false)
const loadedKey = ref('')
let observer: IntersectionObserver | undefined
const previewCache = new Map<string, PreviewResponse>()

// Rendering a thumbnail is CPU-heavy (Rust-side rasterization for models
// without a baked-in preview), so opening a folder with dozens of visible
// cards must not fire that many renders at once. Shared across every
// instance of this component since it's a module-level singleton.
const MAX_CONCURRENT_PREVIEWS = 2
let activePreviews = 0
const previewQueue: (() => void)[] = []

function acquirePreviewSlot(): Promise<void> {
  if (activePreviews < MAX_CONCURRENT_PREVIEWS) {
    activePreviews += 1
    return Promise.resolve()
  }
  return new Promise((resolve) => previewQueue.push(resolve))
}

function releasePreviewSlot() {
  const next = previewQueue.shift()
  if (next) {
    next()
    return
  }
  activePreviews = Math.max(0, activePreviews - 1)
}

function decodeBase64(value: string) {
  const binary = atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}

async function drawPng(bytes: Uint8Array) {
  const element = canvas.value
  if (!element) return false
  const image = new Image()
  const png = new ArrayBuffer(bytes.byteLength)
  new Uint8Array(png).set(bytes)
  const url = URL.createObjectURL(new Blob([png], { type: 'image/png' }))
  try {
    await new Promise<void>((resolve, reject) => {
      image.onload = () => resolve()
      image.onerror = () => reject(new Error('Invalid thumbnail'))
      image.src = url
    })
    const context = element.getContext('2d')
    if (!context) return false
    const width = 640
    const height = 360
    element.width = width
    element.height = height
    context.clearRect(0, 0, width, height)
    const scale = Math.min(width / image.naturalWidth, height / image.naturalHeight)
    const drawWidth = image.naturalWidth * scale
    const drawHeight = image.naturalHeight * scale
    context.drawImage(image, (width - drawWidth) / 2, (height - drawHeight) / 2, drawWidth, drawHeight)
    return true
  } finally {
    URL.revokeObjectURL(url)
  }
}

async function load() {
  const key = props.path
  if (loading.value || loadedKey.value === key) return
  failed.value = false
  loading.value = true
  try {
    let preview = previewCache.get(key)
    if (!preview) {
      await acquirePreviewSlot()
      try {
        preview = await invoke<PreviewResponse>('library_file_preview', {
          request: {
            path: props.path,
            sizeBytes: props.sizeBytes,
            modifiedAt: props.modifiedAt,
          },
        })
      } finally {
        releasePreviewSlot()
      }
      previewCache.set(key, preview)
    }
    const bytes = decodeBase64(preview.data)
    if (preview.kind === 'png') {
      if (!await drawPng(bytes)) throw new Error('Invalid thumbnail')
      loadedKey.value = key
      return
    }
    // All archive parsing and geometry rasterization happen in Rust. The UI
    // only decodes and displays the returned PNG.
    throw new Error('Raster preview unavailable')
  } catch {
    failed.value = true
  } finally {
    loading.value = false
  }
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

onBeforeUnmount(() => observer?.disconnect())

watch(() => props.path, () => {
  loadedKey.value = ''
  failed.value = false
  scheduleLoad()
})
</script>

<template>
  <div ref="container" class="relative grid place-items-center overflow-hidden" :aria-label="alt" :aria-busy="loading" role="img">
    <canvas ref="canvas" class="size-full object-contain" aria-hidden="true" />
    <span v-if="failed" class="absolute inset-0 grid place-items-center text-xs font-medium text-cyan-900/60 dark:text-cyan-100/60">{{ props.unavailableLabel }}</span>
  </div>
</template>
