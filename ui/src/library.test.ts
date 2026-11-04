import { describe, expect, it } from 'vitest'
import { supportsPreview, fileTypeLabel, LARGE_LIBRARY_THRESHOLD, nextSort, shouldDeferLibraryCards, sortedBy } from './library'

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
    expect(names({ key: 'size', descending: false })).toEqual(['benchy.3mf', 'anchor.stl', 'cube.gcode'])
    expect(names({ key: 'modified', descending: true })).toEqual(['benchy.3mf', 'anchor.stl', 'cube.gcode'])
    expect(names({ key: 'modified', descending: false })).toEqual(['cube.gcode', 'anchor.stl', 'benchy.3mf'])
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

  it('uses the same file type labels in the printer table and local library', () => {
    expect(fileTypeLabel({ name: 'models', type: 'directory' })).toBe('DIR')
    expect(fileTypeLabel({ name: 'model.gcode.3mf', type: 'file' })).toBe('3MF')
    expect(fileTypeLabel({ name: 'README', type: 'file' })).toBe('FILE')
    expect(fileTypeLabel({ name: '.hidden', type: 'file' })).toBe('FILE')
  })
})

describe('supportsPreview', () => {
  it('recognizes supported preview formats without enabling excluded formats', () => {
    for (const name of ['part.stl', 'part.gcode.3mf', 'part.FCStd', 'part.f3z', 'part.bgcode', 'part.lbrn2', 'part.amf', 'part.off', 'part.ply', 'part.glb', 'part.gltf', 'part.dae', 'part.vrml', 'part.wrl', 'part.x3d', 'part.3ds', 'scan.xyz', 'scan.pts', 'scan.asc', 'scan.pcd', 'scan.las', 'scan.e57', 'part.blend', 'part.blend1', 'part.3dm', 'part.skp', 'part.sldprt', 'part.sldasm', 'part.slddrw', 'part.ipt', 'part.iam', 'part.ipn', 'part.idw']) {
      expect(supportsPreview(name)).toBe(true)
    }
    for (const name of ['part.step', 'part.iges', 'part.brep', 'part.ctb', 'part.sl1', 'part.sl1s', 'part.goo', 'README']) {
      expect(supportsPreview(name)).toBe(false)
    }
  })
})
