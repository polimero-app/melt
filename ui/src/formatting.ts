// Thresholds are generous: the monitor polls on an interval, so a reading a
// few seconds old is normal and should not look alarming.
const RECENT_AFTER_SECONDS = 15
const STALE_AFTER_SECONDS = 60

export type Freshness = { seconds: number; bucket: 'live' | 'recent' | 'stale' }

export function relativeAge(observedAt: string | undefined, nowMs: number): Freshness | undefined {
  if (!observedAt) return undefined
  const parsed = Date.parse(observedAt)
  if (Number.isNaN(parsed)) return undefined
  const seconds = Math.max(0, Math.round((nowMs - parsed) / 1000))
  if (seconds >= STALE_AFTER_SECONDS) return { seconds, bucket: 'stale' }
  if (seconds >= RECENT_AFTER_SECONDS) return { seconds, bucket: 'recent' }
  return { seconds, bucket: 'live' }
}

export function formatDuration(seconds: number): string {
  if (seconds < 60) return '< 1m'
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  if (!hours) return `${minutes}m`
  return minutes ? `${hours}h ${minutes}m` : `${hours}h`
}
