import { describe, expect, it } from 'vitest'

import { printStageLabel } from './stages'

describe('printStageLabel', () => {
  it('localizes known stages', () => {
    expect(printStageLabel('en', 'preparing_ams', 77)).toBe('Preparing AMS')
    expect(printStageLabel('pt-BR', 'preparing_ams', 77)).toBe('Preparando o AMS')
  })

  it('uses a safe preparing label for unknown active codes', () => {
    expect(printStageLabel('en', undefined, 73)).toBe('Preparing')
    expect(printStageLabel('pt-BR', 'future_stage', 78)).toBe('Preparando')
  })

  it('does not invent a stage when the printer sent none', () => {
    expect(printStageLabel('en')).toBeUndefined()
  })
})
