export type FirmwareUpdateAvailability = 'available' | 'current' | 'unknown' | 'unsupported'
export type FirmwareUpdateComponentKind = 'printerFirmware' | 'accessoryFirmware' | 'printerSoftware'
export type FirmwareEvidenceSource = 'bambuLanInventory' | 'bambuLanAdvertisement' | 'bambuLanHistory' | 'bambuPublicCatalogue' | 'moonrakerUpdateManager'
export type FirmwareEvidenceRole = 'installed' | 'printerAdvertised' | 'deviceCatalogue' | 'publicStable' | 'upstreamCurrent'
export type FirmwareUpdateAssessment = 'required' | 'confirmedAvailable' | 'deviceCatalogueNewer' | 'publicReleaseNewer' | 'current' | 'conflict' | 'unknown' | 'unsupported'

export type FirmwareVersionEvidence = {
  source: FirmwareEvidenceSource
  role: FirmwareEvidenceRole
  version?: string
  required: boolean
}
export type FirmwareSourceCheck = {
  source: FirmwareEvidenceSource
  checkedAt: string
  outcome: 'success' | 'empty' | 'failed' | 'disabled' | 'unsupported'
  stale: boolean
  error?: { code: string; detail?: string }
}

export type FirmwareUpdateComponent = {
  id: string
  label: string
  kind: FirmwareUpdateComponentKind
  currentVersion?: string
  availableVersion?: string
  availability: FirmwareUpdateAvailability
  required: boolean
  serial?: string
  evidence?: FirmwareVersionEvidence[]
}

export type FirmwareUpdateEntry = {
  profile: string
  driver: string
  checkedAt: string
  stale: boolean
  sources?: FirmwareSourceCheck[]
  report: {
    availability: FirmwareUpdateAvailability
    assessment: FirmwareUpdateAssessment
    source: 'bambuMqtt' | 'moonrakerUpdateManager'
    components: FirmwareUpdateComponent[]
    issues: { code: string; message: string }[]
    evidenceSources?: FirmwareEvidenceSource[]
  }
  error?: { code: string; detail?: string }
}

export function firmwareAssessmentClasses(assessment: FirmwareUpdateAssessment): string {
  if (assessment === 'current') return 'bg-emerald-100 text-emerald-700 dark:bg-emerald-400/10 dark:text-emerald-300'
  if (assessment === 'unknown' || assessment === 'unsupported') return 'bg-gray-100 text-gray-600 dark:bg-white/10 dark:text-gray-300'
  return 'bg-amber-100 text-amber-800 dark:bg-amber-400/10 dark:text-amber-300'
}

export function hasAvailableUpdate(entry: FirmwareUpdateEntry | undefined) {
  return entry?.report.availability === 'available'
}

export function hasRequiredUpdate(entry: FirmwareUpdateEntry | undefined) {
  return entry?.report.components.some((component) => component.availability === 'available' && component.required) ?? false
}

export function updateEntryFor(entries: FirmwareUpdateEntry[], profile: string | undefined) {
  if (!profile) return undefined
  return entries.find((entry) => entry.profile.toLocaleLowerCase() === profile.toLocaleLowerCase())
}

export function versionRail(component: FirmwareUpdateComponent) {
  const evidence = component.evidence ?? []
  const versionFor = (role: FirmwareEvidenceRole) => evidence.find((item) => item.role === role)?.version
  return {
    installed: component.currentVersion || '—',
    advertised: component.availableVersion || '—',
    deviceCatalogue: versionFor('deviceCatalogue') || '—',
    publicStable: versionFor('publicStable') || '—',
    hasTarget: Boolean(component.availableVersion),
  }
}

export type FirmwareModuleKey =
  | 'printer' | 'mainBoard' | 'motionController' | 'motionSubController' | 'toolhead'
  | 'extensionBoard' | 'amsHub' | 'hotend' | 'rackHotend' | 'ams' | 'amsLite' | 'ams2Pro' | 'amsHt'

// Bambu `get_version` module codes. Most boards send an empty product_name, so
// the code is all there is. Evidence: BambuStudio DevNozzleSystem (`wtm`
// toolhead hotend, `wtm*/N` rack slot N-0x10), UpgradePanel (`ext` "Extension
// Board"), AMS product names (`n3f/1` = "AMS 2 Pro (2)", `n3s/128` = "AMS HT
// (1)"). ponytail: `tm`, `smc`, `ixmj`, `xm` are unverified, so they keep their
// raw code; add them here once their role is confirmed.
const FIXED_MODULES: Record<string, FirmwareModuleKey> = {
  ota: 'printer', ap: 'mainBoard', ap2: 'mainBoard', rv1126: 'mainBoard',
  mc: 'motionController', 'mc-sub': 'motionSubController', th: 'toolhead',
  ext: 'extensionBoard', ahb: 'amsHub', wtm: 'hotend',
}
// family → [key, first unit id]; the printer numbers units from 1.
const UNIT_MODULES: Record<string, [FirmwareModuleKey, number]> = {
  ams: ['ams', 0], ams_f1: ['amsLite', 0], n3f: ['ams2Pro', 0], n3s: ['amsHt', 128],
}

export function firmwareModuleName(id: string): { key: FirmwareModuleKey; unit?: number } | undefined {
  const code = id.toLowerCase()
  if (FIXED_MODULES[code]) return { key: FIXED_MODULES[code], unit: undefined }
  const match = /^([a-z0-9_]+)\/(\d+)$/.exec(code)
  if (!match) return undefined
  const [, family, raw] = match
  const index = Number(raw)
  if (/^wtm\d*$/.test(family)) return index >= 0x10 ? { key: 'rackHotend', unit: index - 0x0f } : undefined
  const unit = UNIT_MODULES[family]
  return unit && index >= unit[1] ? { key: unit[0], unit: index - unit[1] + 1 } : undefined
}
