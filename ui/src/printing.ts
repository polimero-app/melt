import { invoke } from '@tauri-apps/api/core'

export type PrintFile = { devicePath: string; printer?: string }
export type PrintOptions = { plate?: number; displayName?: string }
export type PrintPackage = {
  sourceFileName: string
  sliced: boolean
  plates: {
    index: number
    name?: string
    gcodePath: string
    thumbnailPaths: string[]
    estimatedSeconds?: number
    estimatedGrams?: number
  }[]
  issues: { severity: 'warning' | 'error'; code: string; message: string }[]
}

export function inspectPrintFile(file: PrintFile): Promise<PrintPackage> {
  return file.printer
    ? invoke('inspect_printer_print', { request: { name: file.printer, devicePath: file.devicePath } })
    : invoke('inspect_library_print', { request: { path: file.devicePath } })
}

export function startPrintFile(file: PrintFile, printer: string, options: PrintOptions): Promise<unknown> {
  return file.printer
    ? invoke('printer_job_action', {
      request: { name: file.printer, action: 'start', devicePath: file.devicePath, options, confirmed: true },
    })
    : invoke('print_library_file', { request: { printer, path: file.devicePath, ...options } })
}

export type PrintTargetState = 'empty' | 'ready' | 'busy'

export function printTargetState(printerCount: number, busy: boolean): PrintTargetState {
  if (printerCount === 0) return 'empty'
  return busy ? 'busy' : 'ready'
}
