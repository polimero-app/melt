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
})
