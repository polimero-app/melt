import { expect, it, vi } from 'vitest'
import html from '../index.html?raw'

const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1]

it.each([
  [null, false, false],
  [null, true, true],
  ['system', false, false],
  ['system', true, true],
  ['light', true, false],
  ['dark', false, true],
  ['invalid', false, false],
  ['invalid', true, true],
])('applies saved theme %s with system dark=%s before first paint', (theme, systemDark, dark) => {
  if (!script) throw new Error('Missing initial theme script')
  const toggle = vi.fn()
  const setAttribute = vi.fn()
  const querySelector = vi.fn(() => ({ setAttribute }))
  new Function('localStorage', 'matchMedia', 'document', script)(
    { getItem: () => theme },
    () => ({ matches: systemDark }),
    { documentElement: { classList: { toggle } }, querySelector },
  )
  expect(toggle).toHaveBeenCalledWith('dark', dark)
  expect(querySelector).toHaveBeenCalledWith('meta[name="theme-color"]')
  expect(setAttribute).toHaveBeenCalledWith('content', dark ? '#111827' : '#f9fafb')
})
