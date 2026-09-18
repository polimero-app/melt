import { describe, expect, it } from 'vitest'
import { H264StreamParser } from './camera-stream'

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
