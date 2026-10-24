import { describe, expect, it } from 'vitest'
import { hmsGuideUrl, hmsSeverity, presentPrinterError } from './hms'

describe('HMS presentation', () => {
  it('localizes core errors and preserves raw snake-case identifiers from the API', () => {
    const entry = {
      code: 'hardware_error',
      message: 'Nozzle heater short circuit.',
      raw_code: '0300-0200-0001-0001',
      translations: { 'pt-BR': 'Curto-circuito no aquecedor do bico.' },
      guide_url: 'https://wiki.bambulab.com/en/h2/troubleshooting/hmscode/0300_0200_0001_0001',
    }
    expect(presentPrinterError(entry, 'pt-BR')).toMatchObject({
      message: entry.translations['pt-BR'],
      rawCode: entry.raw_code,
      guideUrl: entry.guide_url,
    })
    expect(presentPrinterError(entry, 'en').message).toBe(entry.message)
    expect(presentPrinterError({ code: 'printer_error', message: 'Unknown error' }, 'pt-BR').message).toBe('Unknown error')
  })

  it('uses model-specific official wiki paths and rejects foreign catalogue links', () => {
    const guide = 'https://wiki.bambulab.com/en/p2s/troubleshooting/hmscode/0300_0100_0001_0007'
    expect(hmsGuideUrl('0300-0100-0001-0007', guide)).toBe(guide)
    expect(hmsGuideUrl('not-an-hms-code', 'https://wiki.bambulab.com.evil.test/en/')).toBeUndefined()
  })

  it('uses the Bambu HMS level to distinguish a common interruption from a fault', () => {
    expect(hmsSeverity('0300-9700-0003-0001')).toBe('warning')
    expect(hmsSeverity('0300-9700-0001-0001')).toBe('error')
    expect(hmsSeverity('0300-9700-0004-0001')).toBe('info')
    expect(hmsSeverity(undefined)).toBe('error')
  })

  it('links valid HMS codes to the official searchable guide', () => {
    expect(hmsGuideUrl('0300-9700-0003-0001')).toBe(
      'https://wiki.bambulab.com/en/hms/home?search=0300-9700-0003-0001',
    )
    expect(hmsGuideUrl('03009700-00030001')).toBe(
      'https://wiki.bambulab.com/en/hms/home?search=0300-9700-0003-0001',
    )
    expect(hmsGuideUrl('not-an-hms-code')).toBeUndefined()
  })
})
