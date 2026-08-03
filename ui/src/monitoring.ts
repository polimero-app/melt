export type PrinterBadge = 'idle' | 'busy' | 'reconnecting' | 'offline' | 'unknown'

type MonitorEntry = {
  status?: { state: 'idle' | 'printing' | 'paused' | 'error' | 'unknown' }
  error?: unknown
  stale?: boolean
}

export function monitorBadge(entry?: MonitorEntry): PrinterBadge {
  if (!entry) return 'unknown'
  if (!entry.status) return entry.error ? 'offline' : 'unknown'
  if (entry.status.state === 'error' || entry.status.state === 'unknown') return 'offline'
  if (entry.stale || entry.error) return 'reconnecting'
  return entry.status.state === 'printing' || entry.status.state === 'paused' ? 'busy' : 'idle'
}
