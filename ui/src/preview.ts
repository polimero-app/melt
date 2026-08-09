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

const previewCache = new Map<string, PreviewResponse>()

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

function cacheKey(request: PreviewRequest) {
  return `${request.large ? 'large' : 'grid'}:${request.path}`
}

/** The already-resolved preview for a request, if one has been loaded. */
export function cachedPreview(request: PreviewRequest): PreviewResponse | undefined {
  return previewCache.get(cacheKey(request))
}

export async function loadPreview(request: PreviewRequest): Promise<PreviewResponse> {
  const key = cacheKey(request)
  const cached = previewCache.get(key)
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
  previewCache.set(key, preview)
  return preview
}

export function decodeBase64(value: string) {
  const binary = atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}
