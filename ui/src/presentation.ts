import type { MessageKey } from './i18n'

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
