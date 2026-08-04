import { describe, expect, it } from 'vitest'
import { cameraViewState, isActiveJobState, materialSystemLabel, printerStateMessageKey } from './presentation'

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

describe('isActiveJobState', () => {
  it('only treats printing and paused printers as active jobs', () => {
    expect(isActiveJobState('printing')).toBe(true)
    expect(isActiveJobState('paused')).toBe(true)
    expect(isActiveJobState('idle')).toBe(false)
    expect(isActiveJobState('error')).toBe(false)
    expect(isActiveJobState(undefined)).toBe(false)
  })
})

describe('materialSystemLabel', () => {
  it('matches Bambu Studio labels for regular, HT, and external units', () => {
    expect(materialSystemLabel(0, 'External spool')).toBe('A')
    expect(materialSystemLabel(1, 'External spool')).toBe('B')
    expect(materialSystemLabel(25, 'External spool')).toBe('Z')
    expect(materialSystemLabel(128, 'External spool')).toBe('HT')
    expect(materialSystemLabel(254, 'External spool')).toBe('External spool')
    expect(materialSystemLabel(42, 'External spool')).toBe('AMS 42')
  })
})
