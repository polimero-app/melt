import { afterEach, describe, expect, it, vi } from 'vitest'
import { h264AvcAccessUnit, H264StreamParser, startH264Playback } from './camera-stream'

function frame(payload: number[], timestamp: bigint, keyframe: boolean) {
  const bytes = new Uint8Array(13 + payload.length)
  const header = new DataView(bytes.buffer)
  header.setUint32(0, payload.length)
  header.setBigUint64(4, timestamp)
  header.setUint8(12, keyframe ? 1 : 0)
  bytes.set(payload, 13)
  return bytes
}

describe('H264StreamParser', () => {
  it('reassembles a frame split across network chunks', () => {
    const parser = new H264StreamParser()
    const bytes = frame([0, 0, 0, 1, 0x65], 42n, true)

    expect(parser.push(bytes.slice(0, 7))).toEqual([])
    expect(parser.push(bytes.slice(7))).toEqual([
      { data: new Uint8Array([0, 0, 0, 1, 0x65]), timestamp: 42, keyframe: true },
    ])
  })

  it('reassembles large frames from many small chunks, back to back', () => {
    const payload = Array.from({ length: 256 * 1024 }, (_, index) => index & 0xff)
    const first = frame(payload, 1n, true)
    const second = frame(payload.slice(0, 1000), 2n, false)
    const bytes = new Uint8Array(first.length + second.length)
    bytes.set(first)
    bytes.set(second, first.length)
    const parser = new H264StreamParser()

    const frames = []
    // An odd chunk size makes frame boundaries fall mid-chunk.
    for (let offset = 0; offset < bytes.length; offset += 4093) {
      frames.push(...parser.push(bytes.subarray(offset, offset + 4093)))
    }

    expect(frames.map((parsed) => parsed.timestamp)).toEqual([1, 2])
    expect(frames[0].data).toEqual(new Uint8Array(payload))
    expect(frames[1].data).toEqual(new Uint8Array(payload.slice(0, 1000)))
  })

  it('parses multiple access units from one network chunk', () => {
    const first = frame([1, 2], 10n, true)
    const second = frame([3], 20n, false)
    const bytes = new Uint8Array(first.length + second.length)
    bytes.set(first)
    bytes.set(second, first.length)

    expect(new H264StreamParser().push(bytes)).toEqual([
      { data: new Uint8Array([1, 2]), timestamp: 10, keyframe: true },
      { data: new Uint8Array([3]), timestamp: 20, keyframe: false },
    ])
  })

  it('rejects empty and unreasonably large access units', () => {
    const empty = new Uint8Array(13)
    expect(() => new H264StreamParser().push(empty)).toThrow('Invalid H.264 access-unit length')

    const large = new Uint8Array(13)
    new DataView(large.buffer).setUint32(0, 8 * 1024 * 1024 + 1)
    expect(() => new H264StreamParser().push(large)).toThrow('Invalid H.264 access-unit length')
  })
})

const sps = [0x67, 0x64, 0x10, 0x29, 0xac]
const pps = [0x68, 0xee, 0x3c]
const idr = [0x65, 0x88, 0, 0, 3, 1]
const annexB = (...nals: number[][]) => nals.flatMap(nal => [0, 0, 0, 1, ...nal])
const keyframe = annexB(sps, pps, idr)
const avcDescription = new Uint8Array([
  1, 0x64, 0x10, 0x29, 0xff, 0xe1,
  0, sps.length, ...sps, 1, 0, pps.length, ...pps,
])

describe('h264AvcAccessUnit', () => {
  it('replaces start codes with NAL lengths in place and builds an AVC configuration', () => {
    const data = new Uint8Array(keyframe)
    const result = h264AvcAccessUnit({ data, timestamp: 42, keyframe: true })
    expect(result.data).toBe(data)
    expect(data).toEqual(new Uint8Array([
      0, 0, 0, sps.length, ...sps,
      0, 0, 0, pps.length, ...pps,
      0, 0, 0, idr.length, ...idr,
    ]))
    expect(result.description).toEqual(avcDescription)
  })

  it('converts delta frames without requiring parameter sets', () => {
    expect(h264AvcAccessUnit({
      data: new Uint8Array(annexB([0x41, 0x12, 0x34])), timestamp: 90, keyframe: false,
    })).toEqual({ data: new Uint8Array([0, 0, 0, 3, 0x41, 0x12, 0x34]) })
  })

  it('encodes NAL and parameter-set lengths in big-endian order beyond one byte', () => {
    const longSps = [...sps, ...Array(256).fill(0x55)]
    const longIdr = [0x65, ...Array(65536).fill(0x55)]
    const { data, description } = h264AvcAccessUnit({
      data: new Uint8Array(annexB(longSps, pps, longIdr)), timestamp: 0, keyframe: true,
    })
    expect(data.slice(0, 4)).toEqual(new Uint8Array([0, 0, 1, 5]))
    expect(description?.slice(6, 8)).toEqual(new Uint8Array([1, 5]))
    expect(new DataView(data.buffer).getUint32(8 + longSps.length + pps.length)).toBe(65537)
  })

  it.each([
    [],
    [0, 0, 1, 0x65],
    annexB([]),
    annexB(sps, [], pps, idr),
    annexB(sps, idr),
    annexB(pps, idr),
    annexB([0x67, 0x64], pps, idr),
    annexB([...sps, ...Array(65536).fill(0x55)], pps, idr),
  ].map(payload => ({ payload })))('rejects malformed input or missing/invalid keyframe configuration (%#)', ({ payload }) => {
    expect(() => h264AvcAccessUnit({
      data: new Uint8Array(payload), timestamp: 0, keyframe: true,
    })).toThrow(/H.264/)
  })
})

describe('startH264Playback', () => {
  afterEach(() => vi.unstubAllGlobals())

  function playback(chunks: Uint8Array[], configureError?: Error) {
    const configure = vi.fn((_: VideoDecoderConfig) => {
      if (configureError) throw configureError
    })
    const decode = vi.fn()
    const close = vi.fn()
    const cancel = vi.fn()
    vi.stubGlobal('VideoDecoder', class {
      state = 'unconfigured'
      decodeQueueSize = 0
      configure = configure
      decode = decode
      close = close
    })
    vi.stubGlobal('EncodedVideoChunk', class {
      constructor(public init: EncodedVideoChunkInit) {}
    })
    vi.stubGlobal('HTMLCanvasElement', class {
      getContext() { return { drawImage: vi.fn() } }
    })
    vi.stubGlobal('fetch', vi.fn(async () => new Response(new ReadableStream({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(chunk)
        if (!configureError) controller.close()
      },
      cancel,
    }))))
    const stream = startH264Playback(new HTMLCanvasElement(), 'http://127.0.0.1/test')
    return { stream, configure, decode, close, cancel }
  }

  it('waits for a keyframe, preserves timestamps, and reconfigures only for changed SPS/PPS', async () => {
    const first = frame(keyframe, 20n, true)
    const changedSps = [0x67, 0x42, 0xe0, 0x1f, 0xad]
    const { stream, configure, decode, close } = playback([
      frame(annexB([0x41, 0x12]), 10n, false),
      first.slice(0, 18), first.slice(18),
      frame(annexB([0x41, 0x12]), 30n, false),
      frame(keyframe, 40n, true),
      frame(annexB(changedSps, pps, idr), 50n, true),
      frame(annexB(changedSps, [0x68, 0xef], idr), 60n, true),
    ])
    await Promise.all([
      expect(stream.firstFrame).rejects.toThrow('H.264 stream ended'),
      expect(stream.done).rejects.toThrow('H.264 stream ended'),
    ])
    expect(configure).toHaveBeenCalledTimes(3)
    expect(configure.mock.calls[0][0]).toEqual({
      codec: 'avc1.641029', description: avcDescription, optimizeForLatency: true,
    })
    expect(configure.mock.calls[1][0].codec).toBe('avc1.42e01f')
    expect(configure.mock.calls[2][0].description).not.toEqual(configure.mock.calls[1][0].description)
    expect(decode.mock.calls.map(([chunk]) => [chunk.init.type, chunk.init.timestamp])).toEqual([
      ['key', 20], ['delta', 30], ['key', 40], ['key', 50], ['key', 60],
    ])
    expect(decode.mock.calls[0][0].init.data).toEqual(
      h264AvcAccessUnit({ data: new Uint8Array(keyframe), timestamp: 20, keyframe: true }).data,
    )
    expect(close).toHaveBeenCalledOnce()
  })

  it('propagates decoder configuration failures and cancels the stream for MJPEG fallback', async () => {
    const { stream, decode, close, cancel } = playback(
      [frame(keyframe, 0n, true)], new Error('Unsupported AVC configuration'),
    )
    await Promise.all([
      expect(stream.firstFrame).rejects.toThrow('Unsupported AVC configuration'),
      expect(stream.done).rejects.toThrow('Unsupported AVC configuration'),
    ])
    expect(decode).not.toHaveBeenCalled()
    expect(close).toHaveBeenCalledOnce()
    expect(cancel).toHaveBeenCalledOnce()
  })
})
