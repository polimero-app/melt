export type HmsSeverity = 'error' | 'warning' | 'info'

const HMS_GUIDE_URL = 'https://wiki.bambulab.com/en/hms/home?search='
const HMS_CODE = /^[0-9A-F]{4}(?:-[0-9A-F]{4}){3}$/

export function normaliseHmsCode(rawCode: string | undefined): string | undefined {
  if (!rawCode) return undefined
  const code = rawCode.toUpperCase()
  if (HMS_CODE.test(code)) return code

  const compact = /^([0-9A-F]{8})-([0-9A-F]{8})$/.exec(code)
  if (!compact) return undefined
  return `${compact[1].slice(0, 4)}-${compact[1].slice(4)}-${compact[2].slice(0, 4)}-${compact[2].slice(4)}`
}

export function hmsSeverity(rawCode: string | undefined): HmsSeverity {
  const code = normaliseHmsCode(rawCode)
  if (!code) return 'error'

  // Bambu reserves this group for the HMS message level: 1 fatal, 2 serious,
  // 3 common, and 4 informational. A cover-open interruption is common, not
  // a hardware failure, so it should not receive the destructive treatment.
  switch (code.split('-')[2]) {
    case '0003': return 'warning'
    case '0004': return 'info'
    default: return 'error'
  }
}

export function hmsGuideUrl(rawCode: string | undefined): string | undefined {
  const code = normaliseHmsCode(rawCode)
  return code ? `${HMS_GUIDE_URL}${encodeURIComponent(code)}` : undefined
}
