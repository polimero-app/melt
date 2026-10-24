export type PrinterDraftFields = {
  name: string
  driver: string
  host: string
  serial: string
  model: string
  timeout: string
  insecure: boolean
  accessCode: string
}

export function isValidTimeout(value: string): boolean {
  // Keep aligned with melt_core::drivers::parse_timeout, including u64 overflow.
  const match = /^\+?([0-9]+)(ms|s|m|h)$/.exec(value.trim())
  if (!match) return false
  const amount = BigInt(match[1]!)
  const multiplier = match[2] === 'h' ? 3600n : match[2] === 'm' ? 60n : 1n
  return amount > 0n && amount * multiplier <= 18446744073709551615n
}

export function printerDraftsMatch(left: PrinterDraftFields, right: PrinterDraftFields): boolean {
  return Object.keys(left).every((key) => left[key as keyof PrinterDraftFields] === right[key as keyof PrinterDraftFields])
}

export function validateSlicerDraft(draft: { name: string; path: string }) {
  return {
    nameRequired: !draft.name.trim(),
    pathRequired: !draft.path.trim(),
  }
}
