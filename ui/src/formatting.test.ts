import { describe, expect, it } from 'vitest'
import { formatDuration, relativeAge } from './formatting'

describe('relative age', () => {
  const now = Date.parse('2026-08-04T12:00:30.000Z')

  it('buckets a fresh reading as live', () => {
    expect(relativeAge('2026-08-04T12:00:25.000Z', now)).toEqual({ seconds: 5, bucket: 'live' })
  })

  it('buckets a lagging reading as recent and an old one as stale', () => {
    expect(relativeAge('2026-08-04T11:59:50.000Z', now)?.bucket).toBe('recent')
    expect(relativeAge('2026-08-04T11:55:00.000Z', now)?.bucket).toBe('stale')
  })

  it('returns undefined for missing or unparseable timestamps', () => {
    expect(relativeAge(undefined, now)).toBeUndefined()
    expect(relativeAge('not a date', now)).toBeUndefined()
  })

  it('never reports a negative age when the clocks disagree', () => {
    expect(relativeAge('2026-08-04T12:00:45.000Z', now)?.seconds).toBe(0)
  })
})

describe('duration formatting', () => {
  it('renders hours and minutes above an hour', () => {
    expect(formatDuration(15120)).toBe('4h 12m')
  })

  it('renders minutes alone below an hour', () => {
    expect(formatDuration(720)).toBe('12m')
  })

  it('collapses sub-minute and negative values', () => {
    expect(formatDuration(45)).toBe('< 1m')
    expect(formatDuration(0)).toBe('< 1m')
    expect(formatDuration(-10)).toBe('< 1m')
  })

  it('drops a zero minute component', () => {
    expect(formatDuration(7200)).toBe('2h')
  })
})
