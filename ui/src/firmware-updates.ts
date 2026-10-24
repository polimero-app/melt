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
