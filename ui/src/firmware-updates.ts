export type FirmwareUpdateAvailability = 'available' | 'current' | 'unknown' | 'unsupported'
export type FirmwareUpdateComponentKind = 'printerFirmware' | 'accessoryFirmware' | 'printerSoftware'

export type FirmwareUpdateComponent = {
  id: string
  label: string
  kind: FirmwareUpdateComponentKind
  currentVersion?: string
  availableVersion?: string
  availability: FirmwareUpdateAvailability
  required: boolean
}

export type FirmwareUpdateEntry = {
  profile: string
  driver: string
  checkedAt: string
  stale: boolean
  report: {
    availability: FirmwareUpdateAvailability
    source: 'bambuMqtt' | 'moonrakerUpdateManager'
    components: FirmwareUpdateComponent[]
    issues: { code: string; message: string }[]
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
  return {
    installed: component.currentVersion || '—',
    advertised: component.availableVersion || '—',
    hasTarget: Boolean(component.availableVersion),
  }
}
