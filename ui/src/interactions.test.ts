import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { createSSRApp } from 'vue'
import { renderToString } from 'vue/server-renderer'
import { invoke } from '@tauri-apps/api/core'
import App from './App.vue'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

beforeEach(() => {
  vi.stubGlobal('localStorage', { getItem: () => null, setItem: () => undefined })
  vi.stubGlobal('navigator', { language: 'en' })
  vi.stubGlobal('document', {
    visibilityState: 'visible',
    documentElement: { classList: { toggle: () => undefined } },
    querySelector: () => null,
  })
})

afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
  vi.unstubAllGlobals()
})

async function app() {
  const setup = vi.spyOn(App, 'setup')
  await renderToString(createSSRApp(App))
  return setup.mock.results[0]!.value
}

it('keeps removal destructive while allowing neutral confirmations', async () => {
  const instance = await app()
  instance.removePrinter('workshop')
  expect(instance.pendingConfirm.value.variant).toBe('danger')
  instance.askConfirmation(
    { title: 'drying.startTitle', description: 'drying.startDescription', confirm: 'drying.startConfirm' },
    {},
    async () => undefined,
    'primary',
  )
  expect(instance.pendingConfirm.value.variant).toBe('primary')
})

it.each(['printer', 'publicCatalogue'] as const)('tracks only the requested firmware source (%s) and clears it after failure', async (source) => {
  const instance = await app()
  let fail!: (reason: unknown) => void
  vi.mocked(invoke).mockReturnValueOnce(new Promise((_, reject) => { fail = reject }))
  const pending = source === 'printer'
    ? instance.loadFirmwareUpdates('workshop', true)
    : instance.loadPublicFirmwareCatalogue('workshop')
  expect(instance.firmwareRefreshSource.value).toBe(source)
  expect(instance.firmwareRefreshing.value).toBe(true)

  await instance.loadFirmwareUpdates('workshop', true)
  await instance.loadPublicFirmwareCatalogue('workshop')
  expect(invoke).toHaveBeenCalledOnce()
  fail({ code: 'printerUnreachable' })
  await pending
  expect(instance.firmwareRefreshSource.value).toBeUndefined()
  expect(instance.firmwareRefreshing.value).toBe(false)
  expect(instance.firmwareError.value).toBeTruthy()
})
