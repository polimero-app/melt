import { describe, expect, it } from 'vitest'
import { LARGE_LIBRARY_THRESHOLD, shouldDeferLibraryCards } from './library'

describe('shouldDeferLibraryCards', () => {
  it('defers rendering only after the large-library threshold', () => {
    expect(shouldDeferLibraryCards(LARGE_LIBRARY_THRESHOLD)).toBe(false)
    expect(shouldDeferLibraryCards(LARGE_LIBRARY_THRESHOLD + 1)).toBe(true)
  })
})
