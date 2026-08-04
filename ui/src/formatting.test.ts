import { describe, expect, it } from 'vitest'
import { relativeAge } from './formatting'

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
