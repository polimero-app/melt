import type { MessageKey } from './i18n'

export type CommandError = {
  code: string
  operation?: string
  state?: string
  detail?: string
}

type Translate = (key: MessageKey, values?: Record<string, string | number>) => string

export function commandMessage(reason: unknown, translate: Translate): string {
  const error = reason as CommandError | null
  if (!error || typeof error !== 'object' || typeof error.code !== 'string') {
    console.error('Unexpected command failure', reason)
    return translate('errors.unknown')
  }

  return translate(`errors.${error.code}` as MessageKey, {
    operation: translate(`operations.${error.operation ?? 'operation'}` as MessageKey),
    state: translate(`printerState.${error.state ?? 'unknown'}` as MessageKey),
  })
}

export function commandDetail(reason: unknown): string | undefined {
  const error = reason as CommandError | null
  return error && typeof error === 'object' && typeof error.detail === 'string' ? error.detail : undefined
}
