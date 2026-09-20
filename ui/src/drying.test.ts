import { describe, expect, it } from 'vitest'
import { dryingBounds, dryingDefaults, dryingFault, dryingRequestValid, dryingStoppable, dryingTargetShown } from './drying'

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

describe('dryingFault', () => {
  it('is true only for the heater fault states', () => {
    expect(dryingFault({ status: 'error' })).toBe(true)
    expect(dryingFault({ status: 'heatOutOfControl' })).toBe(true)
    expect(dryingFault({ status: 'drying' })).toBe(false)
    expect(dryingFault({ status: 'off' })).toBe(false)
    expect(dryingFault({ status: 'stopping' })).toBe(false)
  })
})

describe('dryingStoppable', () => {
  it('offers stop whenever active or stuck in a fault state', () => {
    expect(dryingStoppable({ active: true, status: 'drying' })).toBe(true)
    expect(dryingStoppable({ active: false, status: 'error' })).toBe(true)
    expect(dryingStoppable({ active: false, status: 'heatOutOfControl' })).toBe(true)
    expect(dryingStoppable({ active: false, status: 'off' })).toBe(false)
    expect(dryingStoppable({ active: false, status: 'stopping' })).toBe(false)
  })
})

describe('dryingTargetShown', () => {
  it('hides the target once the displayed reading has reached it', () => {
    expect(dryingTargetShown(44.7, 45)).toBe(false) // shown as 45 °C
    expect(dryingTargetShown(45, 45)).toBe(false)
    expect(dryingTargetShown(38.2, 45)).toBe(true)
    expect(dryingTargetShown(undefined, 45)).toBe(true)
    expect(dryingTargetShown(45, undefined)).toBe(false)
  })
})

describe('dryingRequestValid', () => {
  it('accepts only numeric values inside the unit bounds', () => {
    expect(dryingRequestValid(0, 45, 12)).toBe(true)
    expect(dryingRequestValid(0, 65, 24)).toBe(true)
    expect(dryingRequestValid(0, 66, 12)).toBe(false) // above AMS 2 Pro range
    expect(dryingRequestValid(128, 85, 1)).toBe(true)
    expect(dryingRequestValid(128, 44, 12)).toBe(false)
    expect(dryingRequestValid(254, 45, 12)).toBe(false) // no heater, no bounds
  })

  it('rejects an emptied number input', () => {
    // v-model.number yields '' for an empty field, and Number('') is 0.
    expect(dryingRequestValid(0, '' as unknown as number, 12)).toBe(false)
    expect(dryingRequestValid(0, 45, '' as unknown as number)).toBe(false)
    expect(dryingRequestValid(0, Number.NaN, 12)).toBe(false)
    expect(dryingRequestValid(0, 45, 0)).toBe(false)
    expect(dryingRequestValid(0, 45, 25)).toBe(false)
  })
})
