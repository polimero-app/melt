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
