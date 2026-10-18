import { describe, expect, it } from 'vitest'
import { linkChanges } from './cameras'
import { isStale, offConfirmation, plugsForCamera, plugsForPrinter, type PlugObservation, type PlugSummary } from './plugs'

function plug(name: string, printers: string[] = [], cameras: string[] = []): PlugSummary {
  return { name, id: `${name}-id`, driver: 'tapo', host: '192.0.2.30', username: 'a@example.invalid', timeout: '10s', printers, cameras, updated: '2026-10-18T12:00:00Z' }
}

function observation(relay: PlugObservation['relay'], last: PlugObservation['last']): PlugObservation {
  return { relay, connection: 'online', last, device: null }
}

describe('plug attachments', () => {
  const plugs = [plug('bench', ['p1s']), plug('shelf', ['p1s', 'a1'], ['door']), plug('spare')]

  it('plugsForPrinter matches case-insensitively', () => {
    expect(plugsForPrinter('P1S', plugs).map((entry) => entry.name)).toEqual(['bench', 'shelf'])
    expect(plugsForPrinter('x1c', plugs)).toEqual([])
  })

  it('plugsForCamera matches by name', () => {
    expect(plugsForCamera('door', plugs).map((entry) => entry.name)).toEqual(['shelf'])
    expect(plugsForCamera('Door', plugs)).toEqual([])
  })

  it('link deltas cover printers and cameras independently', () => {
    expect(linkChanges(['p1s'], ['a1'])).toEqual({ link: ['a1'], unlink: ['p1s'] })
    expect(linkChanges([], ['door'])).toEqual({ link: ['door'], unlink: [] })
  })
})

describe('isStale', () => {
  it('is true only when unknown with a previous observation', () => {
    const last = { state: 'on' as const, observedAt: '2026-10-18T12:00:00Z' }
    expect(isStale(observation('unknown', last))).toBe(true)
    expect(isStale(observation('unknown', null))).toBe(false)
    expect(isStale(observation('on', last))).toBe(false)
    expect(isStale(undefined)).toBe(false)
  })
})

describe('offConfirmation', () => {
  it('offConfirmation lists both kinds and known activity', () => {
    const result = offConfirmation(plug('shelf', ['p1s', 'a1'], ['door']), { p1s: 'printing' })
    expect(result).toEqual({ printers: ['p1s', 'a1'], cameras: ['door'], active: ['p1s'] })
  })

  it('offConfirmation keeps unknown printers without blocking', () => {
    const result = offConfirmation(plug('bench', ['x1c']), {})
    expect(result).toEqual({ printers: ['x1c'], cameras: [], active: [] })
  })
})
