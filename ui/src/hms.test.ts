import { describe, expect, it } from 'vitest'
import { hmsGuideUrl, hmsSeverity } from './hms'

describe('HMS presentation', () => {
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
