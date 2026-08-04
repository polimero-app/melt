import { describe, expect, it } from 'vitest'

import { printTargetState } from './printing'

describe('printTargetState', () => {
  it('reports an empty target list only when no printers exist', () => {
    expect(printTargetState(0, false)).toBe('empty')
    expect(printTargetState(1, false)).toBe('ready')
  })

  it('reports transfer progress separately from printer availability', () => {
    expect(printTargetState(1, true)).toBe('busy')
  })
})
