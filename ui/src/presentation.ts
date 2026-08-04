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

export function materialSystemLabel(id: number, externalLabel: string): string {
  if (id === 128) return 'HT'
  if (id >= 254) return externalLabel
  if (id >= 0 && id < 26) return String.fromCharCode('A'.charCodeAt(0) + id)
  return `AMS ${id}`
}

export function serialNumberDisplay(serial: string, revealed: boolean): string {
  if (!serial) return '—'
  return revealed ? serial : '••••••••'
}
