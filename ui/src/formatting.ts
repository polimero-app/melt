import { translate, type Locale } from './i18n'

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

export function formatAmsHumidity(percent: number | undefined, index: number | undefined, locale: Locale): string | undefined {
  if (percent !== undefined && Number.isFinite(percent) && percent >= 0 && percent <= 100) {
    return `${new Intl.NumberFormat(locale).format(percent)}%`
  }
  if (index !== undefined && Number.isInteger(index) && index >= 1 && index <= 5) {
    return translate(locale, 'materials.humidityIndex', { index })
  }
  return undefined
}

export type RemainingTimePresentation =
  | { kind: 'almostComplete' }
  | { kind: 'duration'; duration: string }

export function remainingTimePresentation(seconds: number): RemainingTimePresentation {
  if (seconds <= 0) return { kind: 'almostComplete' }
  return { kind: 'duration', duration: formatDuration(seconds) }
}

export function clampTarget(maximum: number, next: number): number {
  return Math.max(0, Math.min(maximum, Math.round(next / 5) * 5))
}

export function formatSize(sizeBytes: number | undefined, locale: string) {
  if (sizeBytes === undefined || sizeBytes < 0) return '—'
  // Intl spells the byte unit out ("16 byte"), so it is formatted by hand.
  if (sizeBytes < 1024) return `${new Intl.NumberFormat(locale).format(sizeBytes)} B`
  const unit = sizeBytes < 1024 * 1024 ? 'kilobyte' : 'megabyte'
  return new Intl.NumberFormat(locale, {
    style: 'unit',
    unit,
    unitDisplay: 'short',
    maximumFractionDigits: 1,
  }).format(sizeBytes / (unit === 'kilobyte' ? 1024 : 1024 * 1024))
}

export function formatDate(value: string | undefined, locale: string) {
  if (!value) return '—'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'short' }).format(date)
}
