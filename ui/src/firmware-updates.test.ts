import { describe, expect, it } from 'vitest'
import {
  firmwareAssessmentClasses,
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
    assessment: 'confirmedAvailable',
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
    evidenceSources: [],
  },
}

describe('firmware update presentation', () => {
  it('uses amber for newer releases and warnings, green for current, and gray for unknown or unsupported', () => {
    for (const assessment of ['required', 'confirmedAvailable', 'deviceCatalogueNewer', 'publicReleaseNewer', 'conflict'] as const) {
      expect(firmwareAssessmentClasses(assessment)).toBe('bg-amber-100 text-amber-800 dark:bg-amber-400/10 dark:text-amber-300')
    }
    expect(firmwareAssessmentClasses('current')).toContain('bg-emerald-100')
    expect(firmwareAssessmentClasses('unknown')).toContain('bg-gray-100')
    expect(firmwareAssessmentClasses('unsupported')).toContain('bg-gray-100')
  })

  it('matches profiles case-insensitively and identifies actionable evidence', () => {
    expect(updateEntryFor([entry], 'workshop')).toBe(entry)
    expect(hasAvailableUpdate(entry)).toBe(true)
    expect(hasRequiredUpdate(entry)).toBe(true)
  })

  it('keeps missing versions visibly unknown', () => {
    expect(versionRail({ ...entry.report.components[0], currentVersion: undefined, availableVersion: undefined }))
      .toEqual({ installed: '—', advertised: '—', deviceCatalogue: '—', publicStable: '—', hasTarget: false })
  })
})
