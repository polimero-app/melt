import { describe, expect, it } from 'vitest'
import { dryingBounds, dryingDefaults } from './drying'

describe('drying presets', () => {
  it('uses AMS 2 Pro limits for units 0-3 and AMS HT limits for 128-135', () => {
    expect(dryingBounds(0)).toEqual({ min: 45, max: 65 })
    expect(dryingBounds(128)).toEqual({ min: 45, max: 85 })
    expect(dryingBounds(254)).toBeUndefined()
  })

  it('prefills from the loaded filament per unit type', () => {
    expect(dryingDefaults(0, 'PLA')).toEqual({ temperatureC: 45, hours: 12, filament: 'PLA' })
    expect(dryingDefaults(128, 'ABS')).toEqual({ temperatureC: 80, hours: 8, filament: 'ABS' })
    expect(dryingDefaults(0, 'ABS')).toEqual({ temperatureC: 65, hours: 12, filament: 'ABS' })
  })

  it('normalizes variants and falls back to PLA', () => {
    expect(dryingDefaults(128, 'PETG HF')?.temperatureC).toBe(65)
    expect(dryingDefaults(128, 'PA6-CF')?.filament).toBe('PA')
    expect(dryingDefaults(0, undefined)?.filament).toBe('PLA')
    expect(dryingDefaults(0, 'Mystery')?.filament).toBe('PLA')
  })
})
