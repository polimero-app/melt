import { describe, expect, it } from 'vitest'
import { isValidTimeout, printerDraftsMatch, validateSlicerDraft, type PrinterDraftFields } from './forms'

const printerDraft: PrinterDraftFields = {
  name: 'Workshop',
  driver: 'moonraker',
  host: 'printer.local',
  serial: '',
  model: '',
  timeout: '10s',
  insecure: false,
  accessCode: '',
}

describe('isValidTimeout', () => {
  it('matches the backend duration units and positive integer syntax', () => {
    for (const value of ['1500ms', '10s', '2m', '1h', ' 10s ', '+10s', '001ms']) {
      expect(isValidTimeout(value), value).toBe(true)
    }
    for (const value of ['', ' ', '0s', '000ms', '-1s', '1.5s', '10', '10sec', '10S', '1 s', '1m30s', 'NaNs']) {
      expect(isValidTimeout(value), value).toBe(false)
    }
  })

  it('rejects values that overflow the backend u64 amount or minute/hour conversion', () => {
    for (const value of ['18446744073709551615ms', '18446744073709551615s', '307445734561825860m', '5124095576030431h']) {
      expect(isValidTimeout(value), value).toBe(true)
    }
    for (const value of ['18446744073709551616ms', '18446744073709551616s', '307445734561825861m', '5124095576030432h']) {
      expect(isValidTimeout(value), value).toBe(false)
    }
  })
})

describe('printerDraftsMatch', () => {
  it('detects edits to regular and sensitive profile fields', () => {
    expect(printerDraftsMatch(printerDraft, { ...printerDraft })).toBe(true)
    expect(printerDraftsMatch(printerDraft, { ...printerDraft, host: '192.168.1.42' })).toBe(false)
    expect(printerDraftsMatch(printerDraft, { ...printerDraft, accessCode: 'secret' })).toBe(false)
  })
})

describe('validateSlicerDraft', () => {
  it('treats blank and whitespace-only values as missing', () => {
    expect(validateSlicerDraft({ name: ' ', path: '' })).toEqual({ nameRequired: true, pathRequired: true })
    expect(validateSlicerDraft({ name: 'OrcaSlicer', path: '/usr/bin/orca-slicer' })).toEqual({ nameRequired: false, pathRequired: false })
  })
})
