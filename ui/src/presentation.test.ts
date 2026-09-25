import { describe, expect, it } from 'vitest'
import { awaitsFirstSample, badgeDotClasses, badgeFillClasses, badgeSurfaceClasses, cameraViewState, filamentColor, filamentFillPercent, isActiveJobState, materialSystemLabel, plateLabel, printerStateMessageKey, serialNumberDisplay, shouldRunCamera } from './presentation'
import type { PrinterBadge } from './monitoring'

const allBadges: PrinterBadge[] = ['idle', 'busy', 'error', 'connecting', 'synchronizing', 'reconnecting', 'offline', 'unknown']

describe('badge presentation', () => {
  it('keeps cyan out of connectivity, reserving it for actions and selection', () => {
    for (const map of [badgeDotClasses, badgeSurfaceClasses, badgeFillClasses]) {
      for (const badge of allBadges) expect(map[badge]).not.toMatch(/cyan/)
    }
  })

  it('gives every badge a dark-mode variant so surfaces cannot drift apart', () => {
    for (const badge of allBadges) {
      expect(badgeDotClasses[badge]).toMatch(/dark:/)
      expect(badgeSurfaceClasses[badge]).toMatch(/dark:/)
      expect(badgeFillClasses[badge]).toMatch(/dark:/)
    }
  })

  it('treats only pre-observation states as awaiting a first sample', () => {
    expect(allBadges.filter(awaitsFirstSample)).toEqual(['connecting', 'synchronizing', 'unknown'])
  })

  it('never marks a badge that carries usable telemetry as awaiting', () => {
    for (const badge of ['idle', 'busy', 'reconnecting', 'error', 'offline'] as PrinterBadge[]) {
      expect(awaitsFirstSample(badge)).toBe(false)
    }
  })
})

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

describe('shouldRunCamera', () => {
  it('requires a supported camera on the visible control view', () => {
    expect(shouldRunCamera('control', true, true)).toBe(true)
    expect(shouldRunCamera('files', true, true)).toBe(false)
    expect(shouldRunCamera('control', false, true)).toBe(false)
    expect(shouldRunCamera('control', true, false)).toBe(false)
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
    expect(materialSystemLabel('bambu-lan', 0, 'External spool')).toBe('AMS A')
    expect(materialSystemLabel('bambu-lan', 1, 'External spool')).toBe('AMS B')
    expect(materialSystemLabel('bambu-lan', 25, 'External spool')).toBe('AMS Z')
    expect(materialSystemLabel('bambu-lan', 128, 'External spool')).toBe('AMS HT')
    expect(materialSystemLabel('bambu-lan', 129, 'External spool')).toBe('AMS HT 2')
    expect(materialSystemLabel('bambu-lan', 254, 'External spool')).toBe('External spool')
    expect(materialSystemLabel('bambu-lan', 42, 'External spool')).toBe('AMS 42')
  })

  it('uses Creality CFS naming for Moonraker material systems', () => {
    expect(materialSystemLabel('moonraker', 0, 'External spool')).toBe('CFS')
    expect(materialSystemLabel('moonraker', 0, 'External spool', 2)).toBe('CFS 1')
    expect(materialSystemLabel('moonraker', 1, 'External spool', 2)).toBe('CFS 2')
    expect(materialSystemLabel('moonraker', 254, 'External spool')).toBe('External spool')
  })
})

describe('serialNumberDisplay', () => {
  it('redacts configured serials until explicitly revealed', () => {
    expect(serialNumberDisplay('01P00A123456789', false)).toBe('••••••••')
    expect(serialNumberDisplay('01P00A123456789', true)).toBe('01P00A123456789')
    expect(serialNumberDisplay('', false)).toBe('—')
  })
})

describe('filamentColor', () => {
  it('normalizes Bambu RGB and RGBA colors to opaque CSS colors', () => {
    expect(filamentColor('F6DA5AFF')).toBe('#F6DA5A')
    expect(filamentColor('#7c3aed')).toBe('#7C3AED')
    expect(filamentColor('00000000')).toBe('#9CA3AF')
    expect(filamentColor('not-a-color')).toBe('#9CA3AF')
    expect(filamentColor(undefined)).toBe('#9CA3AF')
  })
})

describe('plateLabel', () => {
  it('shows a fraction only when the total can contain the index', () => {
    expect(plateLabel(2, 3)).toEqual({ key: 'control.plateOf', values: { index: 2, total: 3 } })
    expect(plateLabel(1, 1)).toEqual({ key: 'control.plateOf', values: { index: 1, total: 1 } })
  })

  it('drops a missing or contradictory total instead of inventing one', () => {
    expect(plateLabel(2, 1)).toEqual({ key: 'control.plate', values: { index: 2 } })
    expect(plateLabel(2, undefined)).toEqual({ key: 'control.plate', values: { index: 2 } })
  })
})

describe('filamentFillPercent', () => {
  it('uses a full fill when the remaining percentage is unsupported', () => {
    expect(filamentFillPercent(-1)).toBe(100)
    expect(filamentFillPercent(undefined)).toBe(100)
    expect(filamentFillPercent(Number.NaN)).toBe(100)
  })

  it('preserves supported percentages within the display range', () => {
    expect(filamentFillPercent(0)).toBe(0)
    expect(filamentFillPercent(42)).toBe(42)
    expect(filamentFillPercent(120)).toBe(100)
  })
})
