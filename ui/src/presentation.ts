import type { MessageKey } from './i18n'
import type { PrinterBadge } from './monitoring'

// system.md: badge colors and labels stay synchronized across every surface.
// Declaring all three as `Record<PrinterBadge, string>` in one file makes a
// newly added badge a build error instead of a silently mismatched color.
//
// Cyan is reserved for actions and selection, so no connectivity state uses it:
// `synchronizing` awaits its first sample and reads as muted gray, same as
// `connecting` and `unknown`.

/** Nav strip dots: a filled circle with a soft halo ring. */
export const badgeDotClasses: Record<PrinterBadge, string> = {
  idle: 'bg-green-500 ring-green-500/10 dark:bg-green-400 dark:ring-green-400/10',
  busy: 'bg-yellow-500 ring-yellow-500/10 dark:bg-yellow-400 dark:ring-yellow-400/10',
  error: 'bg-red-600 ring-red-600/20 dark:bg-red-500 dark:ring-red-500/20',
  reconnecting: 'bg-amber-500 ring-amber-500/10 dark:bg-amber-400 dark:ring-amber-400/10',
  connecting: 'bg-gray-400 ring-gray-400/10 dark:bg-gray-500 dark:ring-gray-500/10',
  synchronizing: 'bg-gray-400 ring-gray-400/10 dark:bg-gray-500 dark:ring-gray-500/10',
  offline: 'bg-red-500 ring-red-500/10 dark:bg-red-400 dark:ring-red-400/10',
  unknown: 'bg-gray-400 ring-gray-400/10 dark:bg-gray-500 dark:ring-gray-500/10',
}

/** StatusBadge surface: background plus label color. */
export const badgeSurfaceClasses: Record<PrinterBadge, string> = {
  idle: 'bg-green-100 text-green-700 dark:bg-green-400/10 dark:text-green-400',
  busy: 'bg-yellow-100 text-yellow-800 dark:bg-yellow-400/10 dark:text-yellow-500',
  error: 'bg-red-600 text-white dark:bg-red-500 dark:text-white',
  reconnecting: 'bg-amber-100 text-amber-800 dark:bg-amber-400/10 dark:text-amber-400',
  connecting: 'bg-gray-100 text-gray-600 dark:bg-gray-400/10 dark:text-gray-400',
  synchronizing: 'bg-gray-100 text-gray-600 dark:bg-gray-400/10 dark:text-gray-400',
  offline: 'bg-red-100 text-red-700 dark:bg-red-400/10 dark:text-red-400',
  unknown: 'bg-gray-100 text-gray-600 dark:bg-gray-400/10 dark:text-gray-400',
}

/** StatusBadge inline dot, drawn as an SVG circle on the surface above. */
export const badgeFillClasses: Record<PrinterBadge, string> = {
  idle: 'fill-green-500 dark:fill-green-400',
  busy: 'fill-yellow-500 dark:fill-yellow-400',
  error: 'fill-white dark:fill-white',
  reconnecting: 'fill-amber-500 dark:fill-amber-400',
  connecting: 'fill-gray-400 dark:fill-gray-500',
  synchronizing: 'fill-gray-400 dark:fill-gray-500',
  offline: 'fill-red-500 dark:fill-red-400',
  unknown: 'fill-gray-400 dark:fill-gray-500',
}

/** Badges that mean "no sample observed yet", not "this printer failed". */
export function awaitsFirstSample(badge: PrinterBadge): boolean {
  return badge === 'connecting' || badge === 'synchronizing' || badge === 'unknown'
}

export type CameraViewState = 'live' | 'snapshot' | 'loading' | 'offline'

export function cameraViewState(input: {
  loading: boolean
  hasPeer: boolean
  hasMedia: boolean
  hasStreamTransport: boolean
}): CameraViewState {
  if (input.hasPeer || (input.hasMedia && input.hasStreamTransport)) return 'live'
  if (input.hasMedia) return 'snapshot'
  return input.loading ? 'loading' : 'offline'
}

const printerStateKeys: Record<string, MessageKey> = {
  idle: 'printerState.idle',
  printing: 'printerState.printing',
  paused: 'printerState.paused',
  error: 'printerState.error',
  unknown: 'printerState.unknown',
}

export function printerStateMessageKey(state: string | undefined): MessageKey {
  return printerStateKeys[state ?? 'unknown'] ?? 'printerState.unknown'
}

export function isActiveJobState(state: string | undefined): boolean {
  return state === 'printing' || state === 'paused'
}

export function materialSystemLabel(driver: string, id: number, externalLabel: string, unitCount = 1): string {
  if (id >= 254) return externalLabel
  if (driver === 'moonraker') return unitCount > 1 ? `CFS ${id + 1}` : 'CFS'
  if (id >= 128 && id < 153) return id === 128 ? 'AMS HT' : `AMS HT ${id - 127}`
  if (id >= 0 && id < 26) return `AMS ${String.fromCharCode('A'.charCodeAt(0) + id)}`
  return `AMS ${id}`
}

export function serialNumberDisplay(serial: string, revealed: boolean): string {
  if (!serial) return '—'
  return revealed ? serial : '••••••••'
}

export function filamentColor(value: string | undefined): string {
  const hex = value?.trim().replace(/^#/, '')
  if (!hex || !/^(?:[\da-f]{6}|[\da-f]{8})$/i.test(hex) || hex.toUpperCase() === '00000000') return '#9CA3AF'
  return `#${hex.slice(0, 6).toUpperCase()}`
}

export function filamentFillPercent(value: number | undefined): number {
  if (value === undefined || !Number.isFinite(value) || value < 0) return 100
  return Math.min(value, 100)
}
