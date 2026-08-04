import { describe, expect, it, vi } from 'vitest'

import { commandDetail, commandMessage } from './errors'
import type { MessageKey } from './i18n'

const translate = (key: MessageKey, values: Record<string, string | number> = {}) =>
  [key, values.operation, values.state].filter(Boolean).join('|')

describe('commandMessage', () => {
  it('maps structured command errors through localization', () => {
    expect(commandMessage({ code: 'printerWrongState', operation: 'jobStart', state: 'printing' }, translate)).toBe(
      'errors.printerWrongState|operations.jobStart|printerState.printing',
    )
  })

  it('hides unexpected exception details from the user-facing message', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined)

    expect(commandMessage(new Error("Cannot read properties of undefined (reading 'invoke')"), translate)).toBe(
      'errors.unknown',
    )
    expect(consoleError).toHaveBeenCalledOnce()
    consoleError.mockRestore()
  })
})

describe('commandDetail', () => {
  it('returns backend diagnostic detail only when explicitly provided', () => {
    expect(commandDetail({ code: 'unknown', detail: 'diagnostic context' })).toBe('diagnostic context')
    expect(commandDetail(new Error('private runtime detail'))).toBeUndefined()
  })
})
