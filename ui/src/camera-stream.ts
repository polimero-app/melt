export const H264_FRAME_HEADER_BYTES = 13
const MAX_H264_ACCESS_UNIT_BYTES = 8 * 1024 * 1024
const MAX_DECODER_QUEUE = 4

export type H264Chunk = {
  data: Uint8Array<ArrayBuffer>
  timestamp: number
  keyframe: boolean
}

/** Parses the Rust loopback stream without assuming HTTP chunk boundaries. */
export class H264StreamParser {
  private buffered: Uint8Array<ArrayBuffer> = new Uint8Array(0)

  push(chunk: Uint8Array): H264Chunk[] {
    if (chunk.byteLength > 0) {
      const combined = new Uint8Array(this.buffered.byteLength + chunk.byteLength)
      combined.set(this.buffered)
      combined.set(chunk, this.buffered.byteLength)
      this.buffered = combined
    }

    const frames: H264Chunk[] = []
    let offset = 0
    while (this.buffered.byteLength - offset >= H264_FRAME_HEADER_BYTES) {
      const header = new DataView(
        this.buffered.buffer,
        this.buffered.byteOffset + offset,
        H264_FRAME_HEADER_BYTES,
      )
      const length = header.getUint32(0)
      if (length === 0 || length > MAX_H264_ACCESS_UNIT_BYTES) {
        throw new Error(`Invalid H.264 access-unit length: ${length}`)
      }
      const frameEnd = offset + H264_FRAME_HEADER_BYTES + length
      if (frameEnd > this.buffered.byteLength) break
      frames.push({
        data: this.buffered.slice(offset + H264_FRAME_HEADER_BYTES, frameEnd),
        timestamp: Number(header.getBigUint64(4)),
        keyframe: (header.getUint8(12) & 1) !== 0,
      })
      offset = frameEnd
    }
    if (offset > 0) this.buffered = this.buffered.slice(offset)
    return frames
  }
}

export async function supportsH264WebCodecs(codec: string): Promise<boolean> {
  if (typeof VideoDecoder === 'undefined') return false
  try {
    const support = await VideoDecoder.isConfigSupported({ codec, optimizeForLatency: true })
    return support.supported === true
  } catch {
    return false
  }
}

export type H264Playback = {
  firstFrame: Promise<void>
  done: Promise<void>
  stop: () => void
}

export function startH264Playback(canvas: HTMLCanvasElement, url: string, codec: string): H264Playback {
  const context = canvas.getContext('2d', { alpha: false })
  if (!context) throw new Error('Canvas video rendering is unavailable')

  const abort = new AbortController()
  let stopped = false
  let firstFrameSettled = false
  let resolveFirstFrame!: () => void
  let rejectFirstFrame!: (reason: unknown) => void
  const firstFrame = new Promise<void>((resolve, reject) => {
    resolveFirstFrame = resolve
    rejectFirstFrame = reject
  })
  let rejectDecoder!: (reason: unknown) => void
  const decoderFailure = new Promise<never>((_, reject) => {
    rejectDecoder = reject
  })
  const fail = (error: unknown) => {
    if (!firstFrameSettled) {
      firstFrameSettled = true
      rejectFirstFrame(error)
    }
    abort.abort()
    rejectDecoder(error)
  }
  const decoder = new VideoDecoder({
    output(frame) {
      try {
        const width = frame.displayWidth || frame.codedWidth
        const height = frame.displayHeight || frame.codedHeight
        if (canvas.width !== width || canvas.height !== height) {
          canvas.width = width
          canvas.height = height
        }
        context.drawImage(frame, 0, 0, width, height)
        if (!firstFrameSettled) {
          firstFrameSettled = true
          resolveFirstFrame()
        }
      } catch (error) {
        fail(error)
      } finally {
        frame.close()
      }
    },
    error(error) {
      fail(error)
    },
  })
  try {
    decoder.configure({ codec, optimizeForLatency: true })
  } catch (error) {
    decoder.close()
    throw error
  }

  const pump = async () => {
    const response = await fetch(url, { cache: 'no-store', signal: abort.signal })
    if (!response.ok || !response.body) throw new Error(`H.264 stream returned HTTP ${response.status}`)
    const reader = response.body.getReader()
    const parser = new H264StreamParser()
    try {
      while (!stopped) {
        const result = await reader.read()
        if (result.done) throw new Error('H.264 stream ended')
        for (const frame of parser.push(result.value)) {
          while (!stopped && decoder.decodeQueueSize >= MAX_DECODER_QUEUE) {
            await waitForDecoderQueue(decoder, abort.signal)
          }
          if (stopped) break
          decoder.decode(new EncodedVideoChunk({
            type: frame.keyframe ? 'key' : 'delta',
            timestamp: frame.timestamp,
            data: frame.data,
          }))
        }
      }
    } finally {
      void reader.cancel()
    }
  }

  const done = Promise.race([pump(), decoderFailure])
    .catch((error) => {
      if (!firstFrameSettled) {
        firstFrameSettled = true
        rejectFirstFrame(error)
      }
      if (!stopped) throw error
    })
    .finally(() => {
      if (decoder.state !== 'closed') decoder.close()
    })

  return {
    firstFrame,
    done,
    stop() {
      if (stopped) return
      stopped = true
      abort.abort()
    },
  }
}

function waitForDecoderQueue(decoder: VideoDecoder, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    const cleanup = () => {
      decoder.removeEventListener('dequeue', onDequeue)
      signal.removeEventListener('abort', onAbort)
    }
    const onDequeue = () => {
      cleanup()
      resolve()
    }
    const onAbort = () => {
      cleanup()
      reject(signal.reason)
    }
    decoder.addEventListener('dequeue', onDequeue, { once: true })
    signal.addEventListener('abort', onAbort, { once: true })
    if (decoder.decodeQueueSize < MAX_DECODER_QUEUE) {
      cleanup()
      resolve()
    }
  })
}
