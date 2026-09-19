import { describe, expect, it } from 'vitest'
import {
  hasAvailableUpdate,
  hasRequiredUpdate,
  updateEntryFor,
  versionRail,
  type FirmwareUpdateEntry,
} from './firmware-updates'

const entry: FirmwareUpdateEntry = {
  profile: 'Workshop',
  driver: 'bambu-lan',
  checkedAt: '2026-09-19T12:00:00Z',
  stale: false,
  report: {
    availability: 'available',
    source: 'bambuMqtt',
    components: [{
      id: 'ota',
      label: 'Printer',
      kind: 'printerFirmware',
      currentVersion: '1.0',
      availableVersion: '2.0',
      availability: 'available',
      required: true,
    }],
    issues: [],
  },
}

describe('firmware update presentation', () => {
  it('matches profiles case-insensitively and identifies actionable evidence', () => {
    expect(updateEntryFor([entry], 'workshop')).toBe(entry)
    expect(hasAvailableUpdate(entry)).toBe(true)
    expect(hasRequiredUpdate(entry)).toBe(true)
  })

  it('keeps missing versions visibly unknown', () => {
    expect(versionRail({ ...entry.report.components[0], currentVersion: undefined, availableVersion: undefined }))
      .toEqual({ installed: '—', advertised: '—', hasTarget: false })
  })
})
