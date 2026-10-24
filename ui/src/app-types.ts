import type { MessageKey } from './i18n'
import type { ConnectionState } from './monitoring'
import type { AmsDrying } from './drying'
import type { CommandError } from './errors'
import type { CameraPlayback } from './cameras'
import type { HmsSeverity, PrinterError } from './hms'
import type { Translate } from './errors'

export type View = 'control' | 'printers' | 'cameras' | 'plugs' | 'settings' | 'files'

export type ModelTone = 'cyan' | 'amber' | 'rose' | 'violet'

export type AppInfo = {
  version: string
  modes: [string, string]
  license: string
  sourceUrl: string
}

export type Printer = {
  name: string
  driver: string
  host: string
  serial: string
  model: string
  detectedModel: string
  timeout: string
  insecure: boolean
  presence?: {
    lastSeenUnixMs: number
    model: string
    firmware?: string
    suggestedHost?: string
  }
}

export type Driver = {
  name: string
  description: string
}

export type Capabilities = {
  status: boolean
  temperatureRead: boolean
  lightControl: boolean
  speedControl: boolean
  discovery: boolean
  cameraStream: boolean
  cameraSnapshot: boolean
  fileList: boolean
  fileDownload: boolean
  fileUpload: boolean
  fileDelete: boolean
  jobStart: boolean
  jobPause: boolean
  jobResume: boolean
  jobCancel: boolean
  emergencyStop: boolean
  temperatureWrite: boolean
  motionControl: boolean
  fanControl: boolean
  tlsRefresh: boolean
  firmwareUpdateCheck: boolean
  amsDrying?: boolean
}

export type DiscoveredPrinter = {
  driver: string
  host: string
  serial: string
  model: string
  displayModel: string
  name: string
}

export type Temperature = {
  currentCelsius: number
  targetCelsius?: number
}

export type TemperatureControl = {
  kind: 'nozzle' | 'bed' | 'chamber'
  position?: 'left' | 'right'
  currentCelsius?: number
  targetCelsius?: number
  minimumCelsius?: number
  maximumCelsius?: number
  controllable: boolean
}

export type FanControl = {
  kind: 'parts' | 'auxiliary' | 'hotend' | 'exhaust' | 'chamber' | 'mainboard' | 'heat' | 'unknown'
  speedPercent?: number
  mode: 'manual' | 'auto' | 'off'
  minimumPercent?: number
  maximumPercent?: number
  controllable: boolean
}

export type LightControl = {
  kind: 'lamp' | 'work' | 'unknown'
  mode: 'on' | 'off' | 'flashing' | 'unknown'
  controllable: boolean
}

export type AmsTrayStatus = {
  slot: number
  present?: boolean
  trayIndex?: number
  trayInfoId?: string
  settingId?: string
  tagUid?: string
  filamentType?: string
  color?: string
  remainingPercent?: number
  remainingGrams?: number
  nominalWeightGrams?: number
  diameterMm?: number
}

export type AmsUnitStatus = {
  id: number
  kind?: string
  humidityRange?: string
  humidityPercent?: number
  humidityIndex?: number
  humidityLevel?: string
  temperature?: number
  trays: AmsTrayStatus[]
  drying?: AmsDrying
}

export type PrinterStatus = {
  state: 'idle' | 'printing' | 'paused' | 'error' | 'unknown'
  temperatures?: { nozzle?: Temperature; bed?: Temperature; chamber?: Temperature }
  job?: { name: string }
  printMeta?: { fileName: string; fileSize?: number; plateIndex?: number; plateCount?: number; bedType?: string }
  progress?: { percent: number; preparationPercent?: number; currentLayer?: number; totalLayers?: number }
  timeEstimates?: { elapsedSeconds: number; remainingSeconds?: number; totalSeconds?: number }
  stage?: string
  stageCode?: number
  errors: PrinterError[]
  warnings: { code: string; message: string }[]
  fans?: Record<string, number>
  wifi?: { signalDbm: number }
  lights?: Record<string, string>
  controls?: {
    temperatures?: Record<string, TemperatureControl>
    fans?: Record<string, FanControl>
    lights?: Record<string, LightControl>
  }
  extensions?: { 'bambu-lan'?: { ams?: { units: AmsUnitStatus[] } } }
}

export type StatusFault = {
  code: string
  message: string
  rawCode?: string
  recoverable?: boolean
  severity: HmsSeverity
  guideUrl?: string
}

export type MonitorEntry = {
  name: string
  driver: string
  status?: PrinterStatus
  error?: CommandError
  stale: boolean
  connectionState: ConnectionState
  observedAt?: string
}

export type KeychainProbe = {
  available: boolean
  reason?: 'accessCodeMissing' | 'keychainUnavailable'
}

export type NotificationEvent = { kind: 'completion' | 'failure' | 'disconnection'; printer: string }

export type TransferProgress = { transferId: string; bytesTransferred: number; totalBytes?: number; complete: boolean }

export type PrintStageEvent = { stage: string; percent?: number; bytesTransferred?: number; detail?: string }

export type FileEntry = {
  name: string
  root: string
  path: string
  devicePath: string
  type: 'file' | 'directory'
  mediaType: 'model' | 'timelapse' | 'video' | 'other'
  sizeBytes?: number
  modifiedAt?: string
}

export type FileList = {
  entries: FileEntry[]
}

export type LibraryBreadcrumb = {
  name: string
  path: string
}

export type LibraryListResponse = {
  path: string
  parent?: string
  breadcrumbs: LibraryBreadcrumb[]
  entries: FileEntry[]
}

export type Diagnostics = {
  version: string
  platform: string
  configuredProfiles: number
  drivers: Record<string, number>
  identifiersRedacted: boolean
  monitorWorkers: number
  monitorIntervalSeconds: number
  protocolTracesIncluded: boolean
  bambu: BambuCompatibilityDiagnostic[]
  cameras: CameraDiagnostic[]
}

export type CameraOwnerDiagnostic = { transport: string; subscribers: number; reconnectAttempts: number }

export type CameraDiagnostic = {
  camera: string
  driver: string
  linkedPrinters: number
  streams: { quality: string; owner: CameraOwnerDiagnostic }[]
  playback?: CameraPlayback
}

export type BambuCompatibilityDiagnostic = {
  printer: string
  modelRaw: string
  canonicalModel: string
  modelFamily: string
  firmwareModules: { name: string; software: string; hardware?: string }[]
  authorization: { effective: string; conflict: boolean }
  camera: { preferred: string; source: string; rejectedAdvertisement?: string; owner?: CameraOwnerDiagnostic; playback?: CameraPlayback }
  storageTransport: string
  quirks: { id: string; qualification: string; active: boolean }[]
  tlsPinned: boolean
}

export interface MaterialSlot {
  slot: string
  present?: boolean
  type: string
  color: string
  colorLabel: string
  remainingPercent: number
  remainingGrams?: number
}

export interface MaterialSystemView {
  name: string
  unitId: number
  temperature?: number
  humidity?: string
  slots: MaterialSlot[]
  drying?: AmsDrying
  firstFilament?: string
}

export type NotificationSetting = {
  id: 'completion' | 'failure' | 'disconnection'
  label: MessageKey
  description: MessageKey
  enabled: boolean
}

export type SlicerSetting = {
  name: string
  path: string
  enabled: boolean
}

export type BackendPreferences = {
  notifications: Record<NotificationSetting['id'], boolean>
  slicers: SlicerSetting[]
  publicFirmwareCatalogue: boolean
}

export type ViewActions = {
  t: Translate
  message: (reason: unknown) => string
  showToast: (text: string, tone?: 'success' | 'error') => void
  askConfirmation: (
    keys: { title: MessageKey; description: MessageKey; confirm: MessageKey },
    values: Record<string, string>,
    run: () => Promise<void>,
    variant?: 'primary' | 'danger',
  ) => void
}
