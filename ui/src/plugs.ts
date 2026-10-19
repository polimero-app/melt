export type PowerState = 'on' | 'off'
export type RelayState = PowerState | 'unknown'
export type Connection = 'unknown' | 'online' | 'unreachable' | 'authFailed' | 'locked' | 'unsupported'

/** Mirrors `plugs::PlugSummary`. `printers` and `cameras` are names. */
export type PlugSummary = {
  name: string
  id: string
  driver: string
  host: string
  username: string
  timeout: string
  printers: string[]
  cameras: string[]
  updated: string
}

export function snapshotPlug(plug: PlugSummary): PlugSummary {
  return { ...plug, printers: [...plug.printers], cameras: [...plug.cameras] }
}

/** Mirrors `PlugObservation`; the map from `plug_observations` is keyed by plug id. */
export type PlugObservation = {
  relay: RelayState
  connection: Connection
  last: { state: PowerState; observedAt: string } | null
  device: { model: string; hardware: string; firmware: string } | null
  /** Latest successful read only; null when stale or not reported. */
  powerWatts: number | null
}

export function plugsForPrinter(printer: string, plugs: PlugSummary[]): PlugSummary[] {
  const name = printer.toLowerCase()
  return plugs.filter((plug) => plug.printers.includes(name))
}

export function plugsForCamera(camera: string, plugs: PlugSummary[]): PlugSummary[] {
  return plugs.filter((plug) => plug.cameras.includes(camera))
}

/** The relay is unknown now, but an earlier reading exists. */
export function isStale(observation: PlugObservation | undefined): boolean {
  return observation?.relay === 'unknown' && Boolean(observation.last)
}

/** Everything switching `plug` off cuts power to. `activity` maps lowercase printer name to a known state; `active` lists printers with one. */
export function offConfirmation(plug: PlugSummary, activity: Record<string, string>) {
  return {
    printers: plug.printers,
    cameras: plug.cameras,
    active: plug.printers.filter((printer) => activity[printer.toLowerCase()]),
  }
}
