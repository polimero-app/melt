import { describe, expect, it } from 'vitest'
import { LARGE_LIBRARY_THRESHOLD, nextSort, shouldDeferLibraryCards, sortedBy } from './library'

describe('shouldDeferLibraryCards', () => {
  it('defers rendering only after the large-library threshold', () => {
    expect(shouldDeferLibraryCards(LARGE_LIBRARY_THRESHOLD)).toBe(false)
    expect(shouldDeferLibraryCards(LARGE_LIBRARY_THRESHOLD + 1)).toBe(true)
  })
})

describe('sortedBy', () => {
  const files = [
    { name: 'cube.gcode', sizeBytes: 300, modifiedAt: '2026-08-10T10:00:00Z' },
    { name: 'benchy.3mf', sizeBytes: 100, modifiedAt: '2026-08-12T10:00:00Z' },
    { name: 'anchor.stl', sizeBytes: 200, modifiedAt: '2026-08-11T10:00:00Z' },
  ]
  const names = (sort: Parameters<typeof sortedBy>[1]) => sortedBy(files, sort).map((file) => file.name)

  it('orders by each key in both directions', () => {
    expect(names({ key: 'name', descending: false })).toEqual(['anchor.stl', 'benchy.3mf', 'cube.gcode'])
    expect(names({ key: 'name', descending: true })).toEqual(['cube.gcode', 'benchy.3mf', 'anchor.stl'])
    expect(names({ key: 'size', descending: true })).toEqual(['cube.gcode', 'anchor.stl', 'benchy.3mf'])
    expect(names({ key: 'modified', descending: true })).toEqual(['benchy.3mf', 'anchor.stl', 'cube.gcode'])
  })

  it('leaves the source array untouched', () => {
    sortedBy(files, { key: 'size', descending: true })
    expect(files[0].name).toBe('cube.gcode')
  })

  it('sorts entries without a size or date as the smallest and oldest', () => {
    const partial = [{ name: 'known.3mf', sizeBytes: 10, modifiedAt: '2026-08-10T10:00:00Z' }, { name: 'bare.3mf' }, { name: 'broken.3mf', modifiedAt: 'not a date' }]
    expect(sortedBy(partial, { key: 'size', descending: false })[0].name).not.toBe('known.3mf')
    expect(sortedBy(partial, { key: 'modified', descending: true })[0].name).toBe('known.3mf')
  })
})

describe('nextSort', () => {
  it('flips the active column and starts a new one in its natural direction', () => {
    expect(nextSort({ key: 'name', descending: false }, 'name')).toEqual({ key: 'name', descending: true })
    expect(nextSort({ key: 'name', descending: false }, 'size')).toEqual({ key: 'size', descending: true })
    expect(nextSort({ key: 'size', descending: true }, 'name')).toEqual({ key: 'name', descending: false })
  })
})
