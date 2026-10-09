import { reactive } from 'vue'

export type Quality = 'high' | 'low'

/** `{ printer }` is always the printer's built-in feed. */
export type CameraSource = { printer: string } | { camera: string }

/** Mirrors `cameras::CameraSummary`. Printer names are normalized lowercase. */
export type CameraSummary = {
  name: string
  id: string
  driver: string
  host: string
  username: string
  rtspPort: number
  timeout: string
  printers: string[]
  selectedBy: string[]
}

export function linkedCameras(printer: string, cameras: CameraSummary[]): CameraSummary[] {
  const name = printer.toLowerCase()
  return cameras.filter((camera) => camera.printers.includes(name))
}

/** The printer's selected external camera, else its built-in feed. */
export function printerSource(printer: string, cameras: CameraSummary[]): CameraSource {
  const name = printer.toLowerCase()
  const selected = cameras.find((camera) => camera.selectedBy.includes(name))
  return selected ? { camera: selected.name } : { printer }
}

export function linkChanges(before: string[], after: string[]): { link: string[]; unlink: string[] } {
  return {
    link: after.filter((printer) => !before.includes(printer)),
    unlink: before.filter((printer) => !after.includes(printer)),
  }
}

/** A viewer restarts when this changes. Quality applies only to cameras. */
export function sourceKey(source: CameraSource | undefined, quality: Quality): string {
  if (!source) return ''
  return 'camera' in source ? `camera:${source.camera}:${quality}` : `printer:${source.printer}`
}

export type CameraTransport = 'webrtc' | 'webcodecs' | 'mjpeg'

export const cameraTransportLabels: Record<CameraTransport, string> = { webrtc: 'WebRTC', webcodecs: 'H.264', mjpeg: 'MJPEG' }

export type CameraPlayback = { transport?: CameraTransport; detail?: string }

/**
 * Each feed's latest playback or negotiation failure, keyed by printer or
 * camera name. Kept after the viewer stops so diagnostics can report it.
 */
// ponytail: session-only and unbounded; sources are the user's few configured feeds.
export const cameraPlayback = reactive({
  printers: new Map<string, CameraPlayback>(),
  cameras: new Map<string, CameraPlayback>(),
})

export function sourceName(source: CameraSource): string {
  return 'camera' in source ? source.camera : source.printer
}
