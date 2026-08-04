import { describe, expect, it } from 'vitest'
import { cameraViewState, printerStateMessageKey } from './presentation'

describe('cameraViewState', () => {
  it('distinguishes live transports from retained snapshots', () => {
    expect(cameraViewState({ loading: false, hasPeer: true, hasMedia: false, hasStreamTransport: false })).toBe('live')
    expect(cameraViewState({ loading: false, hasPeer: false, hasMedia: true, hasStreamTransport: true })).toBe('live')
    expect(cameraViewState({ loading: false, hasPeer: false, hasMedia: true, hasStreamTransport: false })).toBe('snapshot')
  })

  it('reports loading only while no camera media is available', () => {
    expect(cameraViewState({ loading: true, hasPeer: false, hasMedia: false, hasStreamTransport: false })).toBe('loading')
    expect(cameraViewState({ loading: false, hasPeer: false, hasMedia: false, hasStreamTransport: false })).toBe('offline')
  })
})

describe('printerStateMessageKey', () => {
  it('maps supported states and safely falls back for new backend values', () => {
    expect(printerStateMessageKey('printing')).toBe('printerState.printing')
    expect(printerStateMessageKey('not-yet-supported')).toBe('printerState.unknown')
    expect(printerStateMessageKey(undefined)).toBe('printerState.unknown')
  })
})
