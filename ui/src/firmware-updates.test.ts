import { describe, expect, it } from 'vitest'
import {
  firmwareAssessmentClasses,
  firmwareModuleName,
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

describe('firmwareModuleName', () => {
  // Module lists captured from real H2C/H2D get_version replies.
  it.each([
    ['ota', 'printer', undefined],
    ['ap', 'mainBoard', undefined],
    ['ap2', 'mainBoard', undefined],
    ['rv1126', 'mainBoard', undefined],
    ['mc', 'motionController', undefined],
    ['mc-sub', 'motionSubController', undefined],
    ['th', 'toolhead', undefined],
    ['ext', 'extensionBoard', undefined],
    ['ahb', 'amsHub', undefined],
    ['wtm', 'hotend', undefined],
    ['wtm/16', 'rackHotend', 1],
    ['wtm/19', 'rackHotend', 4],
    ['wtm2/20', 'rackHotend', 5],
    ['ams/0', 'ams', 1],
    ['ams_f1/0', 'amsLite', 1],
    ['n3f/1', 'ams2Pro', 2],
    ['n3s/128', 'amsHt', 1],
    ['N3S/129', 'amsHt', 2],
  ])('names %s', (id, key, unit) => {
    expect(firmwareModuleName(id)).toEqual({ key, unit })
  })

  it.each(['tm', 'smc', 'ixmj', 'xm', 'pc', 'wtm/x', 'ams/', 'n3s/12'])('leaves %s to the printer label', (id) => {
    expect(firmwareModuleName(id)).toBeUndefined()
  })
})
