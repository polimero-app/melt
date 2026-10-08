import { describe, expect, it } from 'vitest'
import { linkChanges, linkedCameras, printerSource, sourceKey, type CameraSummary } from './cameras'

function camera(name: string, printers: string[] = [], selectedBy: string[] = []): CameraSummary {
  return { name, id: `${name}-id`, driver: 'tapo', host: '192.0.2.20', username: 'melt', rtspPort: 554, timeout: '10s', printers, selectedBy }
}

describe('printer feeds', () => {
  const cameras = [camera('desk', ['p1s']), camera('door', ['p1s', 'a1'], ['a1']), camera('spare')]

  it('lists only cameras linked to the printer', () => {
    expect(linkedCameras('p1s', cameras).map((entry) => entry.name)).toEqual(['desk', 'door'])
    expect(linkedCameras('A1', cameras).map((entry) => entry.name)).toEqual(['door'])
    expect(linkedCameras('x1c', cameras)).toEqual([])
  })

  it('uses the selected camera, else the built-in feed', () => {
    expect(printerSource('x1c', cameras)).toEqual({ printer: 'x1c' })
    expect(printerSource('p1s', cameras)).toEqual({ printer: 'p1s' })
    expect(printerSource('a1', cameras)).toEqual({ camera: 'door' })
  })
})

describe('linkChanges', () => {
  it('turns checkbox state into link and unlink calls', () => {
    expect(linkChanges(['a1', 'p1s'], ['p1s', 'x1c'])).toEqual({ link: ['x1c'], unlink: ['a1'] })
    expect(linkChanges(['a1'], ['a1'])).toEqual({ link: [], unlink: [] })
    expect(linkChanges([], [])).toEqual({ link: [], unlink: [] })
  })
})

describe('sourceKey', () => {
  it('changes with the source kind, name, and camera quality', () => {
    const keys = [
      sourceKey({ printer: 'desk' }, 'high'),
      sourceKey({ camera: 'desk' }, 'high'),
      sourceKey({ camera: 'desk' }, 'low'),
      sourceKey({ camera: 'door' }, 'high'),
    ]
    expect(new Set(keys).size).toBe(keys.length)
    expect(sourceKey({ printer: 'desk' }, 'low')).toBe(sourceKey({ printer: 'desk' }, 'high'))
    expect(sourceKey(undefined, 'high')).toBe('')
  })
})
