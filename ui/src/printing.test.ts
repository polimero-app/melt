import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'

import { inspectPrintFile, printTargetState, startPrintFile, type PrintPackage } from './printing'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const mockedInvoke = vi.mocked(invoke)

beforeEach(() => {
  mockedInvoke.mockReset()
})

describe('print dialog dispatch', () => {
  const remote = { devicePath: 'sdcard:/two-plates.gcode.3mf', printer: 'workshop' }
  const options = { plate: 2, displayName: 'Second plate' }
  const inspected: PrintPackage = {
    sourceFileName: 'two-plates.gcode.3mf',
    sliced: true,
    plates: [1, 2].map((index) => ({ index, gcodePath: `Metadata/plate_${index}.gcode`, thumbnailPaths: [] })),
    issues: [],
  }

  it('inspects remote plates without starting, uploading or modifying a print', async () => {
    mockedInvoke.mockResolvedValue(inspected)

    expect(await inspectPrintFile(remote)).toEqual(inspected)
    expect(mockedInvoke.mock.calls).toEqual([
      ['inspect_printer_print', { request: { name: 'workshop', devicePath: remote.devicePath } }],
    ])
  })

  it('sends the confirmed plate and name to the original printer without reuploading', async () => {
    await startPrintFile(remote, 'another-printer', options)

    expect(mockedInvoke.mock.calls).toEqual([
      ['printer_job_action', {
        request: { name: 'workshop', action: 'start', devicePath: remote.devicePath, options, confirmed: true },
      }],
    ])
  })

  it('keeps local inspection and upload on the library workflow', async () => {
    const local = { devicePath: '/models/two-plates.gcode.3mf' }
    mockedInvoke.mockResolvedValue(inspected)

    await inspectPrintFile(local)
    await startPrintFile(local, 'selected-printer', options)

    expect(mockedInvoke.mock.calls).toEqual([
      ['inspect_library_print', { request: { path: local.devicePath } }],
      ['print_library_file', { request: { printer: 'selected-printer', path: local.devicePath, ...options } }],
    ])
  })

  it('propagates inspection and start failures instead of reporting success', async () => {
    mockedInvoke.mockRejectedValue({ code: 'jobFileInvalid' })
    await expect(inspectPrintFile(remote)).rejects.toEqual({ code: 'jobFileInvalid' })
    expect(mockedInvoke).toHaveBeenCalledTimes(1)

    mockedInvoke.mockRejectedValue({ code: 'printerBusy' })
    await expect(startPrintFile(remote, 'workshop', options)).rejects.toEqual({ code: 'printerBusy' })
  })
})

describe('printTargetState', () => {
  it('reports an empty target list only when no printers exist', () => {
    expect(printTargetState(0, false)).toBe('empty')
    expect(printTargetState(1, false)).toBe('ready')
  })

  it('reports transfer progress separately from printer availability', () => {
    expect(printTargetState(1, true)).toBe('busy')
  })
})
