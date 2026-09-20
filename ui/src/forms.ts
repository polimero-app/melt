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

export function printerDraftsMatch(left: PrinterDraftFields, right: PrinterDraftFields): boolean {
  return Object.keys(left).every((key) => left[key as keyof PrinterDraftFields] === right[key as keyof PrinterDraftFields])
}

export function validateSlicerDraft(draft: { name: string; path: string }) {
  return {
    nameRequired: !draft.name.trim(),
    pathRequired: !draft.path.trim(),
  }
}
