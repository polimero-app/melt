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
