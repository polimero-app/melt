import { invoke } from '@tauri-apps/api/core'

export type PreviewResponse = { kind: 'png' | 'model'; data: string }

export type PreviewRequest = {
  path: string
  sizeBytes?: number
  modifiedAt?: string
  /** Asks the backend for the dialog-sized render instead of the grid thumbnail. */
  large?: boolean
}

// Rendering a preview is CPU-heavy (Rust-side rasterization for models
// without a baked-in preview), so opening a folder with dozens of visible
// cards must not fire that many renders at once.
const MAX_CONCURRENT_PREVIEWS = 2
let activePreviews = 0
const previewQueue: (() => void)[] = []

// Base64 characters, not decoded bytes: that's what the Map holds. A grid
// thumbnail is ~70K of these and a large render a few million, so this keeps
// hundreds of thumbnails but only a handful of large renders.
export const MAX_PREVIEW_CACHE_CHARS = 48 << 20
// A Map iterates in insertion order, so re-inserting on each hit makes the
// first key the least recently used one.
const previewCache = new Map<string, PreviewResponse>()
let previewCacheChars = 0

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

// Size and modification time are part of the key so an edited file misses
// the cache instead of showing its old render. Superseded entries aren't
// hunted down; they just age out of the LRU.
function cacheKey(request: PreviewRequest) {
  return JSON.stringify([request.large ?? false, request.path, request.sizeBytes, request.modifiedAt])
}

function readCache(key: string) {
  const preview = previewCache.get(key)
  if (preview) {
    previewCache.delete(key)
    previewCache.set(key, preview)
  }
  return preview
}

function writeCache(key: string, preview: PreviewResponse) {
  if (preview.data.length > MAX_PREVIEW_CACHE_CHARS) return
  const previous = previewCache.get(key)
  if (previous) {
    previewCacheChars -= previous.data.length
    previewCache.delete(key)
  }
  previewCache.set(key, preview)
  previewCacheChars += preview.data.length
  for (const [oldest, evicted] of previewCache) {
    if (previewCacheChars <= MAX_PREVIEW_CACHE_CHARS) break
    previewCache.delete(oldest)
    previewCacheChars -= evicted.data.length
  }
}

/** The already-resolved preview for a request, if one has been loaded. */
export function cachedPreview(request: PreviewRequest): PreviewResponse | undefined {
  return readCache(cacheKey(request))
}

export async function loadPreview(request: PreviewRequest): Promise<PreviewResponse> {
  const key = cacheKey(request)
  const cached = readCache(key)
  if (cached) return cached
  await acquirePreviewSlot()
  let preview: PreviewResponse
  try {
    preview = await invoke<PreviewResponse>('library_file_preview', {
      request: {
        path: request.path,
        sizeBytes: request.sizeBytes,
        modifiedAt: request.modifiedAt,
        large: request.large ?? false,
      },
    })
  } finally {
    releasePreviewSlot()
  }
  writeCache(key, preview)
  return preview
}

export function decodeBase64(value: string) {
  const binary = atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}
