import { invoke } from '@tauri-apps/api/core'

export type PreviewResponse = { kind: 'png' | 'model'; data: string }

export type PreviewRequest = {
  path: string
  sizeBytes?: number
  modifiedAt?: string
  /** Asks the backend for the dialog-sized render instead of the grid thumbnail. */
  large?: boolean
}

export type PreviewOptions = {
  /** Aborting drops this caller; queued work nobody waits on is cancelled. */
  signal?: AbortSignal
  /** Jumps the queue, for the preview the user explicitly opened. */
  priority?: 'high' | 'normal'
}

// Rendering a preview is CPU-heavy (Rust-side rasterization for models
// without a baked-in preview), so opening a folder with dozens of visible
// cards must not fire that many renders at once.
const MAX_CONCURRENT_PREVIEWS = 2
let activePreviews = 0

// One job per cache key, shared by every caller asking for it at the same
// time. A job that has started can't be cancelled (invoke has no abort), so
// it runs to completion and fills the cache; only queued jobs are dropped.
type PreviewJob = {
  key: string
  request: PreviewRequest
  promise: Promise<PreviewResponse>
  resolve: (preview: PreviewResponse) => void
  reject: (error: unknown) => void
  waiters: number
  started: boolean
}
const pendingPreviews = new Map<string, PreviewJob>()
const previewQueue: PreviewJob[] = []

// Base64 characters, not decoded bytes: that's what the Map holds. A grid
// thumbnail is ~70K of these and a large render a few million, so this keeps
// hundreds of thumbnails but only a handful of large renders.
export const MAX_PREVIEW_CACHE_CHARS = 48 << 20
// A Map iterates in insertion order, so re-inserting on each hit makes the
// first key the least recently used one.
const previewCache = new Map<string, PreviewResponse>()
let previewCacheChars = 0

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

export async function loadPreview(request: PreviewRequest, options: PreviewOptions = {}): Promise<PreviewResponse> {
  const key = cacheKey(request)
  const cached = readCache(key)
  if (cached) return cached
  const { signal, priority = 'normal' } = options
  signal?.throwIfAborted()
  let job = pendingPreviews.get(key)
  if (!job) {
    let resolve!: PreviewJob['resolve']
    let reject!: PreviewJob['reject']
    const promise = new Promise<PreviewResponse>((res, rej) => {
      resolve = res
      reject = rej
    })
    job = { key, request, promise, resolve, reject, waiters: 0, started: false }
    pendingPreviews.set(key, job)
    previewQueue.push(job)
  }
  if (priority === 'high' && !job.started) {
    previewQueue.splice(previewQueue.indexOf(job), 1)
    previewQueue.unshift(job)
  }
  job.waiters += 1
  pumpPreviewQueue()
  if (!signal) return job.promise
  return waitForJob(job, signal)
}

function waitForJob(job: PreviewJob, signal: AbortSignal) {
  return new Promise<PreviewResponse>((resolve, reject) => {
    const abort = () => {
      reject(signal.reason)
      leaveJob(job)
    }
    signal.addEventListener('abort', abort, { once: true })
    job.promise.then(resolve, reject).finally(() => signal.removeEventListener('abort', abort))
  })
}

function leaveJob(job: PreviewJob) {
  job.waiters -= 1
  if (job.waiters > 0 || job.started) return
  // Nobody holds the job's own promise any more (signal-less callers never
  // leave), so it can simply be dropped unsettled.
  previewQueue.splice(previewQueue.indexOf(job), 1)
  pendingPreviews.delete(job.key)
}

function pumpPreviewQueue() {
  while (activePreviews < MAX_CONCURRENT_PREVIEWS) {
    const job = previewQueue.shift()
    if (!job) return
    void runJob(job)
  }
}

async function runJob(job: PreviewJob) {
  job.started = true
  activePreviews += 1
  try {
    const preview = await invoke<PreviewResponse>('library_file_preview', {
      request: {
        path: job.request.path,
        sizeBytes: job.request.sizeBytes,
        modifiedAt: job.request.modifiedAt,
        large: job.request.large ?? false,
      },
    })
    writeCache(job.key, preview)
    job.resolve(preview)
  } catch (error) {
    job.reject(error)
  } finally {
    pendingPreviews.delete(job.key)
    activePreviews -= 1
    pumpPreviewQueue()
  }
}

export function decodeBase64(value: string) {
  const binary = atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}
