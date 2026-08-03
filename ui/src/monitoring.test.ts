import { describe, expect, it } from 'vitest'
import { monitorBadge } from './monitoring'

describe('monitor badge', () => {
  it('keeps a retained status visible while monitoring reconnects', () => {
    expect(monitorBadge({
      status: { state: 'printing' },
      error: { code: 'printerTimeout' },
      stale: true,
    })).toBe('reconnecting')
  })

  it('reports offline only when no usable status remains', () => {
    expect(monitorBadge({ error: { code: 'printerTimeout' }, stale: true })).toBe('offline')
    expect(monitorBadge({ status: { state: 'error' } })).toBe('offline')
  })

  it('preserves live idle and busy states', () => {
    expect(monitorBadge({ status: { state: 'idle' }, stale: false })).toBe('idle')
    expect(monitorBadge({ status: { state: 'paused' }, stale: false })).toBe('busy')
  })
})
