import { describe, expect, it } from 'vitest'
import {
  defaultPresets,
  parseStoredPresets,
  presetValidationFailed,
  temperatureMaximums,
  validatePresetDraft,
} from './temperatures'

describe('parseStoredPresets', () => {
  it('ships the material defaults when nothing is stored yet', () => {
    expect(parseStoredPresets(null)).toEqual(defaultPresets)
    expect(parseStoredPresets(undefined)).toEqual(defaultPresets)
  })

  it('falls back to defaults on unparseable or non-array storage', () => {
    expect(parseStoredPresets('{ not json')).toEqual(defaultPresets)
    expect(parseStoredPresets('{"pla":210}')).toEqual(defaultPresets)
  })

  it('honours a deliberately emptied list rather than resurrecting defaults', () => {
    expect(parseStoredPresets('[]')).toEqual([])
  })

  it('drops entries that could drive an out-of-range temperature command', () => {
    const raw = JSON.stringify([
      { id: 'ok', name: 'PLA', nozzleCelsius: 210, bedCelsius: 60 },
      { id: 'hot', name: 'Molten', nozzleCelsius: 900, bedCelsius: 60 },
      { id: 'cold', name: 'Negative', nozzleCelsius: -5, bedCelsius: 60 },
      { id: 'bed', name: 'BedTooHot', nozzleCelsius: 210, bedCelsius: 400 },
      { id: 'fractional', name: 'Fractional', nozzleCelsius: 210.5, bedCelsius: 60 },
      { id: '', name: 'NoId', nozzleCelsius: 210, bedCelsius: 60 },
      { id: 'blank', name: '   ', nozzleCelsius: 210, bedCelsius: 60 },
      { id: 'stringly', name: 'Stringly', nozzleCelsius: '210', bedCelsius: 60 },
      null,
    ])
    expect(parseStoredPresets(raw)).toEqual([{ id: 'ok', name: 'PLA', nozzleCelsius: 210, bedCelsius: 60 }])
  })

  it('accepts the ceilings and zero as valid targets', () => {
    const raw = JSON.stringify([
      { id: 'max', name: 'Max', nozzleCelsius: temperatureMaximums.nozzle, bedCelsius: temperatureMaximums.bed },
      { id: 'off', name: 'Cold', nozzleCelsius: 0, bedCelsius: 0 },
    ])
    expect(parseStoredPresets(raw)).toHaveLength(2)
  })
})

describe('validatePresetDraft', () => {
  it('accepts a well-formed draft', () => {
    const validation = validatePresetDraft({ name: 'TPU', nozzleCelsius: 230, bedCelsius: 40 })
    expect(presetValidationFailed(validation)).toBe(false)
  })

  it('rejects blank names and values beyond the printer ceilings', () => {
    expect(validatePresetDraft({ name: '   ', nozzleCelsius: 230, bedCelsius: 40 }).nameRequired).toBe(true)
    expect(validatePresetDraft({ name: 'X', nozzleCelsius: 400, bedCelsius: 40 }).nozzleOutOfRange).toBe(true)
    expect(validatePresetDraft({ name: 'X', nozzleCelsius: 230, bedCelsius: 400 }).bedOutOfRange).toBe(true)
    expect(validatePresetDraft({ name: 'X'.repeat(33), nozzleCelsius: 230, bedCelsius: 40 }).nameTooLong).toBe(true)
  })

  it('rejects non-numeric input from the form fields', () => {
    const validation = validatePresetDraft({ name: 'X', nozzleCelsius: 'hot', bedCelsius: '' })
    expect(validation.nozzleOutOfRange).toBe(true)
    // An empty string coerces to 0, which is a legitimate "cold" target.
    expect(validation.bedOutOfRange).toBe(false)
  })
})
