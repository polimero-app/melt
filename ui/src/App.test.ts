import { afterEach, expect, it, vi } from 'vitest'
import { createSSRApp } from 'vue'
import { renderToString } from 'vue/server-renderer'
import App from './App.vue'
import { defaultPresets } from './temperatures'

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

it('preserves custom presets until restoring defaults is confirmed', async () => {
  const customPresets = [{ id: 'tpu', name: 'TPU', nozzleCelsius: 230, bedCelsius: 40 }]
  const stored: Record<string, string> = {
    locale: 'en',
    temperaturePresets: JSON.stringify(customPresets),
  }
  vi.stubGlobal('localStorage', {
    getItem: (key: string) => stored[key] ?? null,
    setItem: (key: string, value: string) => { stored[key] = value },
  })
  vi.stubGlobal('document', {
    visibilityState: 'visible',
    documentElement: { classList: { toggle: vi.fn() } },
    querySelector: vi.fn(),
  })
  const setup = vi.spyOn(App, 'setup')
  await renderToString(createSSRApp(App))
  const instance = setup.mock.results[0]!.value

  instance.restoreDefaultPresets()
  expect(instance.temperaturePresets.value).toEqual(customPresets)
  expect(instance.pendingConfirm.value).toMatchObject({
    title: 'Restore default presets?',
    description: expect.stringContaining('deletes any custom presets'),
    confirm: 'Restore defaults',
  })

  instance.pendingConfirm.value = undefined
  await instance.runConfirmation()
  expect(instance.temperaturePresets.value).toEqual(customPresets)

  instance.restoreDefaultPresets()
  await instance.runConfirmation()
  expect(instance.temperaturePresets.value).toEqual(defaultPresets)
  expect(instance.pendingConfirm.value).toBeUndefined()
})
