export type PrinterBadge = 'idle' | 'busy' | 'connecting' | 'synchronizing' | 'reconnecting' | 'offline' | 'unknown'

export type ConnectionState = 'connecting' | 'synchronizing' | 'live' | 'recovering' | 'offline'

type MonitorEntry = {
  status?: { state: 'idle' | 'printing' | 'paused' | 'error' | 'unknown' }
  error?: unknown
  stale?: boolean
  connectionState?: ConnectionState
}

export function monitorBadge(entry?: MonitorEntry): PrinterBadge {
  if (!entry) return 'unknown'
  if (entry.connectionState === 'connecting') return 'connecting'
  if (entry.connectionState === 'synchronizing') return 'synchronizing'
  if (entry.connectionState === 'recovering') return entry.status ? 'reconnecting' : 'synchronizing'
  if (entry.connectionState === 'offline') return 'offline'
  if (!entry.status) return entry.error ? 'offline' : 'unknown'
  if (entry.status.state === 'error' || entry.status.state === 'unknown') return 'offline'
  if (entry.stale || entry.error) return 'reconnecting'
  return entry.status.state === 'printing' || entry.status.state === 'paused' ? 'busy' : 'idle'
}
