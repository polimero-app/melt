import { describe, expect, it, vi } from 'vitest'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

const { MAX_PREVIEW_CACHE_CHARS, cachedPreview, loadPreview } = await import('./preview')

describe('loadPreview', () => {
  it('serves a repeated request from cache without a second render', async () => {
    invoke.mockResolvedValue({ kind: 'png', data: 'AAAA' })
    const request = { path: '/models/bracket.stl', sizeBytes: 12 }

    expect(cachedPreview(request)).toBeUndefined()
    await loadPreview(request)
    await loadPreview(request)

    expect(invoke).toHaveBeenCalledTimes(1)
    expect(cachedPreview(request)).toEqual({ kind: 'png', data: 'AAAA' })
  })

  // The grid thumbnail and the dialog render are different pixels for the same
  // path, so a shared key would serve the 640x360 one to the dialog forever.
  it('keeps the grid and large renders apart', async () => {
    invoke.mockResolvedValueOnce({ kind: 'png', data: 'grid' })
    invoke.mockResolvedValueOnce({ kind: 'png', data: 'large' })
    const path = '/models/plate.3mf'

    expect(await loadPreview({ path })).toEqual({ kind: 'png', data: 'grid' })
    expect(await loadPreview({ path, large: true })).toEqual({ kind: 'png', data: 'large' })
    expect(invoke).toHaveBeenLastCalledWith('library_file_preview', {
      request: { path, sizeBytes: undefined, modifiedAt: undefined, large: true },
    })
  })

  it('renders again when the file size or modification time changes', async () => {
    invoke.mockResolvedValueOnce({ kind: 'png', data: 'old' })
    invoke.mockResolvedValueOnce({ kind: 'png', data: 'new' })
    const path = '/models/edited.stl'

    expect(await loadPreview({ path, sizeBytes: 10, modifiedAt: '2026-01-01T00:00:00Z' })).toEqual({ kind: 'png', data: 'old' })
    expect(await loadPreview({ path, sizeBytes: 10, modifiedAt: '2026-01-02T00:00:00Z' })).toEqual({ kind: 'png', data: 'new' })
  })

  it('evicts the least recently used preview once the byte budget is exceeded', async () => {
    const half = 'A'.repeat(MAX_PREVIEW_CACHE_CHARS / 2)
    const first = { path: '/models/first.stl' }
    const second = { path: '/models/second.stl' }
    const third = { path: '/models/third.stl' }
    invoke.mockResolvedValue({ kind: 'png', data: half })

    await loadPreview(first)
    await loadPreview(second)
    // Touching the first makes the second the eviction candidate.
    expect(cachedPreview(first)).toBeDefined()
    await loadPreview(third)

    expect(cachedPreview(first)).toBeDefined()
    expect(cachedPreview(second)).toBeUndefined()
    expect(cachedPreview(third)).toBeDefined()
  })

  it('shares one backend call between identical simultaneous requests', async () => {
    invoke.mockClear()
    invoke.mockResolvedValue({ kind: 'png', data: 'shared' })
    const request = { path: '/models/popular.stl', sizeBytes: 5 }

    const previews = await Promise.all(Array.from({ length: 10 }, () => loadPreview(request)))

    expect(invoke).toHaveBeenCalledTimes(1)
    expect(previews.every((preview) => preview.data === 'shared')).toBe(true)
  })

  describe('with both render slots busy', () => {
    // Holds the two running renders open so everything after them queues.
    async function occupySlots() {
      const releases: (() => void)[] = []
      invoke.mockImplementation(() => new Promise((resolve) => {
        releases.push(() => resolve({ kind: 'png', data: 'busy' }))
      }))
      const busy = [loadPreview({ path: `/busy/${Math.random()}` }), loadPreview({ path: `/busy/${Math.random()}` })]
      invoke.mockClear()
      invoke.mockImplementation(async (_command: string, { request }: { request: { path: string } }) => ({ kind: 'png', data: request.path }))
      return async () => {
        releases.forEach((release) => release())
        await Promise.all(busy)
      }
    }

    it('drops queued work once every caller has aborted', async () => {
      const release = await occupySlots()
      const request = { path: '/models/scrolled-away.stl' }
      const first = new AbortController()
      const second = new AbortController()
      const loads = [
        loadPreview(request, { signal: first.signal }),
        loadPreview(request, { signal: second.signal }),
      ]
      first.abort()
      second.abort()
      await expect(Promise.all(loads)).rejects.toThrow()

      await release()
      expect(invoke).not.toHaveBeenCalled()
    })

    it('keeps queued work while any caller still wants it', async () => {
      const release = await occupySlots()
      const request = { path: '/models/still-visible.stl' }
      const aborted = new AbortController()
      const gone = loadPreview(request, { signal: aborted.signal })
      const kept = loadPreview(request, { signal: new AbortController().signal })
      aborted.abort()
      await expect(gone).rejects.toThrow()

      await release()
      expect(await kept).toEqual({ kind: 'png', data: request.path })
      expect(invoke).toHaveBeenCalledTimes(1)
    })

    it('runs a high-priority request ahead of earlier queued ones', async () => {
      const release = await occupySlots()
      const grid = loadPreview({ path: '/models/card.stl' })
      const dialog = loadPreview({ path: '/models/opened.stl', large: true }, { priority: 'high' })

      await release()
      await Promise.all([grid, dialog])
      expect(invoke.mock.calls.map(([, { request }]) => request.path)).toEqual(['/models/opened.stl', '/models/card.stl'])
    })
  })
})
