import { afterEach, describe, expect, it, vi } from 'vitest'
import { createSSRApp } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { invoke } from '@tauri-apps/api/core'
import PlugControl from './components/PlugControl.vue'
import { linkChanges } from './cameras'
import { isStale, offConfirmation, plugsForCamera, plugsForPrinter, type PlugObservation, type PlugSummary } from './plugs'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }))

afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

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

describe('power-off confirmation', () => {
  async function control(current: PlugSummary, refresh: () => Promise<PlugSummary[]>) {
    const setup = vi.spyOn(PlugControl, 'setup')
    await renderToString(createSSRApp(PlugControl, {
      plug: current, refresh, activity: {}, locale: 'en', t: (key: string) => key,
    }))
    return setup.mock.results[0]!.value
  }

  it('does not offer confirmation when refreshing attachments fails', async () => {
    const instance = await control(plug('bench'), async () => { throw { code: 'plugConfigUnreadable' } })
    await instance.askOff()
    expect(instance.confirmOpen.value).toBe(false)
    expect(instance.error.value).toBe('errors.plugConfigUnreadable')
    expect(invoke).not.toHaveBeenCalled()
  })

  it('dispatches the refreshed snapshot, not later changes to the displayed plug', async () => {
    const current = plug('bench')
    const refreshed = { ...plug('bench', ['p1s']), updated: 'r2' }
    const instance = await control(current, async () => [refreshed])
    await instance.askOff()
    expect(instance.confirmOpen.value).toBe(true)
    refreshed.printers.push('a1')
    refreshed.updated = 'r3'
    current.name = 'renamed'
    instance.confirmOff()
    await vi.waitFor(() => expect(instance.busy.value).toBe(false))
    expect(invoke).toHaveBeenCalledWith('plug_set_power', {
      expected: { ...plug('bench', ['p1s']), updated: 'r2' }, state: 'off',
    })
  })
})
