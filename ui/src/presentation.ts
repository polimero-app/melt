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
