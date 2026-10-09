<script setup lang="ts">
import { computed, nextTick, onScopeDispose, ref, shallowRef, watch, watchEffect } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { save as saveFileDialog } from '@tauri-apps/plugin-dialog'
import { PhArrowsClockwise, PhCamera, PhCornersOut, PhVideoCamera, PhWarning } from '@phosphor-icons/vue'
import type { MessageKey } from '../i18n'
import { commandDetail, commandMessage, type Translate } from '../errors'
import { cameraViewState } from '../presentation'
import { h264WebCodecsAvailable, startH264Playback, supportsH264WebCodecs, type H264Playback } from '../camera-stream'
import { cameraPlayback, cameraTransportLabels, sourceKey, sourceName, type CameraPlayback, type CameraSource, type CameraTransport, type Quality } from '../cameras'
import Button from './Button.vue'
import Card from './Card.vue'
import CardHeader from './CardHeader.vue'
import IconButton from './IconButton.vue'

type Snapshot = { dataUrl: string }
type Stream = { viewerId: string; url: string }
type H264Stream = Stream & { codec: string }
type WebRtcAnswer = { viewerId: string; type: 'answer'; sdp: string }

const props = withDefaults(defineProps<{
  source?: CameraSource
  quality?: Quality
  /** Whether this viewer may stream now; see `shouldRunCamera`. */
  active: boolean
  canStream: boolean
  canSnapshot: boolean
  title?: string
  t: Translate
}>(), { quality: 'high' })
const emit = defineEmits<{ 'snapshot-saved': [] }>()

const url = ref<string>()
const peer = shallowRef<RTCPeerConnection>()
const mediaStream = shallowRef<MediaStream>()
const video = ref<HTMLVideoElement>()
const canvas = ref<HTMLCanvasElement>()
const stage = ref<HTMLElement>()
const canvasActive = ref(false)
const canvasLive = ref(false)
const transport = ref<CameraTransport>()
const loading = ref(false)
const error = ref<string>()
const viewerIds = new Set<string>()
let request = 0
let h264Playback: H264Playback | undefined

const label = computed(() => props.title ?? props.t('camera.title'))
const supported = computed(() => props.canStream || props.canSnapshot)
const hasMedia = computed(() => Boolean(peer.value || canvasLive.value || url.value))
const state = computed(() => cameraViewState({
  loading: loading.value,
  hasPeer: Boolean(peer.value),
  hasMedia: Boolean(canvasLive.value || url.value),
  hasStreamTransport: transport.value !== undefined,
}))
const stateLabel = computed(() => props.t({
  live: 'camera.live',
  snapshot: 'camera.snapshotLabel',
  loading: 'camera.loading',
  offline: 'status.offlineLabel',
}[state.value] as MessageKey))
// The badge names the transport while live; MJPEG turns amber since it is the
// degraded path. Why a faster one failed lives in Configuration > About.
const badgeLabel = computed(() => state.value === 'live' && transport.value
  ? `${stateLabel.value} · ${cameraTransportLabels[transport.value]}`
  : stateLabel.value)
const badgeTitle = computed(() => transport.value && props.t(`camera.transport.${transport.value}`))
const stateClasses = computed(() => state.value === 'live' && transport.value === 'mjpeg'
  ? 'bg-yellow-100 fill-yellow-500 text-yellow-800 dark:bg-yellow-400/10 dark:fill-yellow-400 dark:text-yellow-500'
  : ({
  live: 'bg-green-100 fill-green-500 text-green-700 dark:bg-green-400/10 dark:fill-green-400 dark:text-green-400',
  snapshot: 'bg-gray-100 fill-gray-400 text-gray-600 dark:bg-white/10 dark:fill-gray-500 dark:text-gray-400',
  loading: 'bg-yellow-100 fill-yellow-500 text-yellow-800 dark:bg-yellow-400/10 dark:fill-yellow-400 dark:text-yellow-500',
  offline: 'bg-gray-100 fill-gray-400 text-gray-600 dark:bg-white/10 dark:fill-gray-500 dark:text-gray-400',
}[state.value]))

watch(() => props.active ? sourceKey(props.source, props.quality) : '', (key) => {
  if (key) void refresh()
  else void stop()
}, { immediate: true })

watchEffect(() => {
  if (video.value) video.value.srcObject = mediaStream.value ?? null
})

onScopeDispose(() => void stop())

defineExpose({ refresh })

function message(reason: unknown) {
  return commandMessage(reason, props.t)
}

function failureDetail(reason: unknown) {
  return commandDetail(reason) ?? (reason instanceof Error ? reason.message : message(reason))
}

function setPlayback(source: CameraSource, playback: CameraPlayback) {
  transport.value = playback.transport
  const feeds = 'camera' in source ? cameraPlayback.cameras : cameraPlayback.printers
  feeds.set(sourceName(source), playback)
}

/** Keeps a viewer for the current request; a stale one is stopped at once. */
function keep(viewerId: string, current: number) {
  if (current === request) {
    viewerIds.add(viewerId)
    return true
  }
  void invoke('camera_viewer_stop', { viewerId }).catch(() => undefined)
  return false
}

async function releaseViewers() {
  const ids = [...viewerIds]
  viewerIds.clear()
  await Promise.all(ids.map((viewerId) => invoke('camera_viewer_stop', { viewerId }).catch(() => undefined)))
}

async function refresh() {
  const source = props.source
  const quality = props.quality
  const key = sourceKey(source, quality)
  if (!source || !props.active) return
  await stop()
  if (!props.active || sourceKey(props.source, props.quality) !== key) return
  const current = ++request
  loading.value = true
  error.value = undefined
  try {
    if (props.canStream) {
      let connection: RTCPeerConnection | undefined
      try {
        const PeerConnection = window.RTCPeerConnection
        if (!PeerConnection) throw new Error('WebRTC is unavailable in the embedded webview')
        connection = new PeerConnection({ iceServers: [] })
        connection.addTransceiver('video', { direction: 'recvonly' })
        const track = waitForVideoTrack(connection)
        const offer = await connection.createOffer()
        await connection.setLocalDescription(offer)
        await waitForIceGathering(connection)
        const answer = await invoke<WebRtcAnswer>('camera_webrtc_offer', {
          source,
          quality,
          offer: connection.localDescription?.sdp ?? offer.sdp,
        })
        if (!keep(answer.viewerId, current)) {
          connection.close()
          return
        }
        await connection.setRemoteDescription({ type: answer.type, sdp: answer.sdp })
        const media = await waitForPeerReady(connection, track)
        if (current !== request) {
          connection.close()
          return
        }
        connection.onconnectionstatechange = () => {
          if (connection?.connectionState === 'failed' && peer.value === connection) {
            void fallback(source, quality, current, 'WebRTC connection failed')
              .catch((fallbackError) => {
                if (current === request) error.value = message(fallbackError)
              })
          }
        }
        mediaStream.value = media
        peer.value = connection
        setPlayback(source, { transport: 'webrtc' })
      } catch (reason) {
        connection?.close()
        if (current === request) await fallback(source, quality, current, failureDetail(reason))
      }
    } else if (props.canSnapshot) {
      const snapshot = await invoke<Snapshot>('camera_snapshot', { source, quality })
      if (current === request) url.value = snapshot.dataUrl
    }
  } catch (reason) {
    if (current === request) {
      url.value = undefined
      error.value = message(reason)
    }
  } finally {
    if (current === request) loading.value = false
  }
}

function waitForVideoTrack(connection: RTCPeerConnection) {
  return new Promise<MediaStream>((resolve) => {
    connection.ontrack = (event) => resolve(event.streams[0] ?? new MediaStream([event.track]))
  })
}

async function waitForPeerReady(connection: RTCPeerConnection, track: Promise<MediaStream>) {
  const connected = new Promise<void>((resolve, reject) => {
    const finish = () => {
      connection.removeEventListener('connectionstatechange', onStateChange)
      if (connection.connectionState === 'connected') resolve()
      else reject(new Error(`WebRTC connection ${connection.connectionState}`))
    }
    const onStateChange = () => {
      if (connection.connectionState === 'connected' || connection.connectionState === 'failed' || connection.connectionState === 'closed') finish()
    }
    if (connection.connectionState === 'connected') resolve()
    else connection.addEventListener('connectionstatechange', onStateChange)
  })
  return await withTimeout(Promise.all([connected, track]).then(([, stream]) => stream), 10_000, 'WebRTC media timed out')
}

async function fallback(source: CameraSource, quality: Quality, current: number, reason: string) {
  if (current !== request) return
  setPlayback(source, { detail: reason })
  peer.value?.close()
  peer.value = undefined
  mediaStream.value?.getTracks().forEach((track) => track.stop())
  mediaStream.value = undefined
  await releaseViewers()
  if (current !== request) return
  if (!h264WebCodecsAvailable()) {
    return fallbackMjpeg(source, quality, current, `${reason}; WebCodecs is unavailable in the embedded webview`)
  }
  try {
    const stream = await invoke<H264Stream>('camera_h264_stream', { source, quality })
    if (!keep(stream.viewerId, current)) return
    if (!await supportsH264WebCodecs(stream.codec)) {
      throw new Error(`WebCodecs does not support ${stream.codec}`)
    }
    if (current !== request) return
    canvasActive.value = true
    canvasLive.value = false
    await nextTick()
    if (!canvas.value) throw new Error('Camera canvas is unavailable')
    const playback = startH264Playback(canvas.value, stream.url)
    h264Playback = playback
    let ready = false
    void playback.done.catch((playbackError) => {
      if (!ready || h264Playback !== playback || current !== request) return
      loading.value = true
      void fallbackMjpeg(source, quality, current, `${reason}; WebCodecs playback failed: ${failureDetail(playbackError)}`)
        .catch((fallbackError) => {
          if (current === request) error.value = message(fallbackError)
        })
        .finally(() => {
          if (current === request) loading.value = false
        })
    })
    await withTimeout(playback.firstFrame, 10_000, 'WebCodecs media timed out')
    if (current !== request) {
      playback.stop()
      return
    }
    ready = true
    canvasLive.value = true
    setPlayback(source, { transport: 'webcodecs', detail: reason })
  } catch (webCodecsError) {
    if (current !== request) return
    await fallbackMjpeg(source, quality, current, `${reason}; WebCodecs: ${failureDetail(webCodecsError)}`)
  }
}

async function fallbackMjpeg(source: CameraSource, quality: Quality, current: number, reason: string) {
  if (current !== request) return
  setPlayback(source, { detail: reason })
  h264Playback?.stop()
  h264Playback = undefined
  canvasActive.value = false
  canvasLive.value = false
  await releaseViewers()
  if (current !== request) return
  try {
    const stream = await invoke<Stream>('camera_mjpeg_stream', { source, quality })
    if (!keep(stream.viewerId, current)) return
    url.value = stream.url
    setPlayback(source, { transport: 'mjpeg', detail: reason })
  } catch (mjpegError) {
    if (current === request) setPlayback(source, { detail: `${reason}; MJPEG: ${failureDetail(mjpegError)}` })
    throw mjpegError
  }
}

async function withTimeout<T>(promise: Promise<T>, timeout: number, reason: string) {
  let timer: number | undefined
  try {
    return await Promise.race([
      promise,
      new Promise<T>((_, reject) => {
        timer = window.setTimeout(() => reject(new Error(reason)), timeout)
      }),
    ])
  } finally {
    if (timer !== undefined) window.clearTimeout(timer)
  }
}

async function waitForIceGathering(connection: RTCPeerConnection) {
  if (connection.iceGatheringState === 'complete') return
  await new Promise<void>((resolve) => {
    const timer = window.setTimeout(resolve, 3000)
    const onStateChange = () => {
      if (connection.iceGatheringState !== 'complete') return
      window.clearTimeout(timer)
      connection.removeEventListener('icegatheringstatechange', onStateChange)
      resolve()
    }
    connection.addEventListener('icegatheringstatechange', onStateChange)
  })
}

async function stop() {
  request += 1
  peer.value?.close()
  peer.value = undefined
  mediaStream.value?.getTracks().forEach((track) => track.stop())
  mediaStream.value = undefined
  h264Playback?.stop()
  h264Playback = undefined
  canvasActive.value = false
  canvasLive.value = false
  url.value = undefined
  transport.value = undefined
  loading.value = false
  await releaseViewers()
}

async function saveSnapshot() {
  const source = props.source
  if (!source || loading.value || !props.canSnapshot) return
  const name = sourceName(source)
  const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, '-')
  const destination = await saveFileDialog({
    defaultPath: `${name.replace(/[^\w.-]+/g, '-')}-${stamp}.jpg`,
    filters: [{ name: 'JPEG', extensions: ['jpg', 'jpeg'] }],
  })
  if (!destination) return
  loading.value = true
  error.value = undefined
  try {
    url.value = (await invoke<Snapshot>('camera_snapshot', { source, quality: props.quality, destination })).dataUrl
    emit('snapshot-saved')
  } catch (reason) {
    error.value = message(reason)
  } finally {
    loading.value = false
  }
}

function toggleFullscreen() {
  if (document.fullscreenElement) document.exitFullscreen()
  else stage.value?.requestFullscreen()
}
</script>

<template>
  <Card class="overflow-hidden">
    <CardHeader :title="label" :icon="PhVideoCamera">
      <template #suffix>
        <span
          class="inline-flex items-center gap-x-1.5 rounded-md px-2 py-1 text-xs font-medium"
          :title="badgeTitle"
          :class="stateClasses"
        >
          <svg class="size-1.5" viewBox="0 0 6 6" aria-hidden="true"><circle cx="3" cy="3" r="3" /></svg>{{ badgeLabel }}
        </span>
      </template>
      <slot name="controls" />
      <IconButton :title="t('camera.refresh')" :aria-label="t('camera.refresh')" :disabled="loading || !supported" @click="refresh"><PhArrowsClockwise class="size-4" :class="loading && 'animate-spin'" aria-hidden="true" /></IconButton>
      <IconButton :title="t('camera.snapshot')" :aria-label="t('camera.snapshot')" :disabled="loading || !canSnapshot" @click="saveSnapshot"><PhCamera class="size-4" aria-hidden="true" /></IconButton>
      <IconButton :title="t('camera.maximize')" :aria-label="t('camera.maximize')" :disabled="!hasMedia" @click="toggleFullscreen"><PhCornersOut class="size-4" aria-hidden="true" /></IconButton>
    </CardHeader>
    <div
      ref="stage"
      class="relative min-h-60 overflow-hidden sm:min-h-77.5"
      :class="hasMedia ? 'bg-black' : 'grid place-items-center bg-gray-100 dark:bg-gray-800'"
    >
      <video v-if="peer" ref="video" :aria-label="label" class="absolute inset-0 size-full object-contain" autoplay muted playsinline />
      <canvas v-else-if="canvasActive" ref="canvas" :aria-label="label" class="absolute inset-0 size-full object-contain" />
      <img v-else-if="url" :src="url" :alt="label" width="1280" height="720" class="absolute inset-0 size-full object-contain" @error="url = undefined" />
      <div v-else class="relative z-10 text-center">
        <PhWarning class="mx-auto size-8 text-yellow-600 dark:text-yellow-400" aria-hidden="true" />
        <p class="mt-3 font-medium text-gray-900 dark:text-white">{{ t('camera.unavailable') }}</p>
        <p class="mt-1 text-xs text-gray-500 dark:text-gray-400">{{ error ?? t('camera.checkConnection') }}</p>
        <Button v-if="supported" class="mt-4" :disabled="loading" @click="refresh"><PhArrowsClockwise class="size-4" :class="loading && 'animate-spin'" aria-hidden="true" /> {{ t('camera.refreshFeed') }}</Button>
      </div>
    </div>
  </Card>
</template>
