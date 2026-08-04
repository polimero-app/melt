import { describe, expect, it } from 'vitest'
import { monitorBadge } from './monitoring'

describe('monitor badge', () => {
  it('keeps a retained status visible while monitoring reconnects', () => {
    expect(monitorBadge({
      status: { state: 'printing' },
      error: { code: 'printerTimeout' },
      stale: true,
      connectionState: 'recovering',
    })).toBe('reconnecting')
  })

  it('reports offline only when no usable status remains', () => {
    expect(monitorBadge({ error: { code: 'printerTimeout' }, stale: true })).toBe('offline')
    expect(monitorBadge({ status: { state: 'unknown' } })).toBe('offline')
  })

  it('separates a reported fault from an unreachable printer', () => {
    expect(monitorBadge({ status: { state: 'error' } })).toBe('error')
    expect(monitorBadge({ status: { state: 'error' }, connectionState: 'live' })).toBe('error')
    // A fault we can no longer confirm is not a fault we should still assert.
    expect(monitorBadge({ status: { state: 'error' }, connectionState: 'offline' })).toBe('offline')
  })

  it('preserves live idle and busy states', () => {
    expect(monitorBadge({ status: { state: 'idle' }, stale: false, connectionState: 'live' })).toBe('idle')
    expect(monitorBadge({ status: { state: 'paused' }, stale: false, connectionState: 'live' })).toBe('busy')
  })

  it('exposes connection phases and expires retained status offline', () => {
    expect(monitorBadge({ stale: true, connectionState: 'connecting' })).toBe('connecting')
    expect(monitorBadge({ stale: true, connectionState: 'synchronizing' })).toBe('synchronizing')
    expect(monitorBadge({ status: { state: 'printing' }, stale: true, connectionState: 'offline' })).toBe('offline')
  })
})
