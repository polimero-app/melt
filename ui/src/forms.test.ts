import { describe, expect, it } from 'vitest'
import { printerDraftsMatch, validateSlicerDraft, type PrinterDraftFields } from './forms'

const printerDraft: PrinterDraftFields = {
  name: 'Workshop',
  driver: 'moonraker',
  host: 'printer.local',
  serial: '',
  timeout: '10s',
  insecure: false,
  accessCode: '',
}

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
