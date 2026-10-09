import { afterAll, afterEach, beforeEach, expect, it, vi } from 'vitest'
import { createRenderer, h, isRef, ref, ssrContextKey } from 'vue'
import CameraViewer from './components/CameraViewer.vue'
import { cameraPlayback, type CameraSource } from './cameras'
import { h264WebCodecsAvailable, startH264Playback } from './camera-stream'
import type { MessageKey } from './i18n'

const invoke = vi.hoisted(() => vi.fn<(command: string) => Promise<unknown>>())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ save: vi.fn() }))
vi.mock('./camera-stream', () => ({
  h264WebCodecsAvailable: vi.fn(),
  supportsH264WebCodecs: vi.fn(async () => true),
  startH264Playback: vi.fn(),
}))

type Node = { parent: Node | null; children: Node[] }
const node = (): Node => ({ parent: null, children: [] })
const renderer = createRenderer<Node, Node>({
  createElement: node,
  createText: node,
  createComment: node,
  insert(child, parent) { child.parent = parent; parent.children.push(child) },
  remove(child) {
    if (child.parent) child.parent.children = child.parent.children.filter(entry => entry !== child)
  },
  parentNode: child => child.parent,
  nextSibling: () => null,
  patchProp() {},
  setText() {},
  setElementText() {},
})

// Node-mode SFC imports have SSR templates. Run the real setup/lifecycle
// with Vue's native renderer and only replace the template with a canvas.
const setup = CameraViewer.setup
vi.spyOn(CameraViewer, 'setup').mockImplementation((props, context) => {
  const bindings = setup?.(props, context)
  if (!bindings || typeof bindings !== 'object' || !('canvas' in bindings) || !isRef(bindings.canvas)) {
    throw new Error('CameraViewer must expose its canvas binding to the template')
  }
  const canvas = bindings.canvas
  return () => h('canvas', { ref: canvas })
})
afterAll(() => vi.restoreAllMocks())
const flush = () => new Promise<void>(resolve => setTimeout(resolve, 0))
let unmount: (() => void) | undefined

function mount(source: CameraSource = { camera: 'garage' }) {
  const active = ref(true)
  const app = renderer.createApp({
    render: () => h(CameraViewer, {
      source, active: active.value, canStream: true, canSnapshot: true, t: (key: MessageKey) => key,
    }),
  })
  app.provide(ssrContextKey, {})
  app.mount(node())
  unmount = () => app.unmount()
  return active
}

beforeEach(() => {
  vi.clearAllMocks()
  vi.stubGlobal('window', { setTimeout, clearTimeout })
  vi.mocked(h264WebCodecsAvailable).mockReturnValue(true)
  invoke.mockImplementation(async (command) => {
    if (command === 'camera_h264_stream') {
      return { viewerId: 'h264', url: 'http://127.0.0.1/stream', codec: 'avc1.42e01e' }
    }
    if (command === 'camera_mjpeg_stream') {
      throw { code: 'cameraUnreachable', detail: 'MJPEG connection refused' }
    }
    if (command === 'camera_viewer_stop') return
    throw new Error(`Unexpected command ${command}`)
  })
})

afterEach(async () => {
  unmount?.()
  await flush()
  vi.unstubAllGlobals()
  cameraPlayback.cameras.clear()
  cameraPlayback.printers.clear()
})

it.each([true, false])('retains failed negotiation without claiming playback (WebCodecs: %s)', async (webcodecs) => {
  vi.mocked(h264WebCodecsAvailable).mockReturnValue(webcodecs)
  vi.mocked(startH264Playback).mockImplementation(() => ({
    firstFrame: Promise.reject(new Error('H.264 decoding failed before first frame')),
    done: Promise.resolve(),
    stop: vi.fn(),
  }))
  mount(webcodecs ? { camera: 'garage' } : { printer: 'p1s' })
  await flush()
  const playback = webcodecs ? cameraPlayback.cameras.get('garage') : cameraPlayback.printers.get('p1s')
  expect(playback?.transport).toBeUndefined()
  expect(playback?.detail).toContain('WebRTC is unavailable')
  expect(playback?.detail).toContain(webcodecs ? 'H.264 decoding failed' : 'WebCodecs is unavailable')
  expect(playback?.detail).toContain('MJPEG connection refused')
})

it('records WebCodecs only after its first frame and retains it after stopping', async () => {
  let firstFrame!: () => void
  vi.mocked(startH264Playback).mockReturnValue({
    firstFrame: new Promise<void>(resolve => { firstFrame = resolve }),
    done: Promise.resolve(),
    stop: vi.fn(),
  })
  const active = mount()
  await flush()
  expect(startH264Playback).toHaveBeenCalledOnce()
  expect(cameraPlayback.cameras.get('garage')?.transport).toBeUndefined()
  firstFrame()
  await flush()
  const playback = cameraPlayback.cameras.get('garage')
  expect(playback?.transport).toBe('webcodecs')
  active.value = false
  await flush()
  expect(cameraPlayback.cameras.get('garage')).toEqual(playback)
})

it('clears a failed live transport and retains the complete fallback failure', async () => {
  let fail!: (error: Error) => void
  vi.mocked(startH264Playback).mockReturnValue({
    firstFrame: Promise.resolve(),
    done: new Promise<void>((_, reject) => { fail = reject }),
    stop: vi.fn(),
  })
  mount()
  await flush()
  expect(cameraPlayback.cameras.get('garage')?.transport).toBe('webcodecs')
  fail(new Error('decoder stopped'))
  await flush()
  expect(cameraPlayback.cameras.get('garage')?.transport).toBeUndefined()
  expect(cameraPlayback.cameras.get('garage')?.detail).toContain('WebCodecs playback failed: decoder stopped')
  expect(cameraPlayback.cameras.get('garage')?.detail).toContain('MJPEG connection refused')
})
