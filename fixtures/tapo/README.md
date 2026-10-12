# Tapo RTSP evidence

Last-updated: 2026-10-12

The current physical RTSP scope is a Tapo C510W, hardware 1.0, firmware 1.3.5,
captured 2026-10-08. The local address is replaced with documentation address
`192.0.2.20`. These are sanitized SDP descriptions, not media recordings or
complete device/desktop qualification reports.

| Fixture | Stream | H.264 profile-level-id |
| --- | --- | --- |
| `c510w-hw1.0-fw1.3.5-stream1.sdp` | `/stream1` (High) | `4D0034` |
| `c510w-hw1.0-fw1.3.5-stream2.sdp` | `/stream2` (Low) | `4D001F` |

Both descriptions advertise H.264 on a 90 kHz RTP clock, packetization mode 1,
SPS/PPS parameter sets and a separate PCMA audio track. Melt selects video and
does not implement audio playback. The observed camera sends Basic and Digest
as separate challenge headers; the RTSP parser selects Digest and rejects a
Basic-only camera. Credentials, nonces/responses, serials and images are not
stored in these fixture files.

Core unit tests in `src/rtsp.rs` parse both captured descriptions and cover
split authentication headers. The loopback simulated suite separately covers
Digest authentication, the selected interleaved channel, video after an audio
track, native keyframe passthrough, JPEG decoding, both stream paths, wrong
passwords, Basic refusal and silent/markerless timeouts:

```sh
cargo test -p melt-core --locked
cargo test -p melt-core --test tapo_simulated --locked
```

These tests do not establish ONVIF/PTZ support, every Tapo model/firmware,
long-running live behavior or every OS/webview's native playback. ONVIF
pan/tilt and discovery remain future phases. New physical evidence should
record the exact model/hardware/firmware/date and preserve only sanitized
protocol facts. These files do not use the Bambu manifest schema.

See [IP camera usage](../../docs/ip-cameras.md) and
[the camera design](../../docs/superpowers/specs/2026-10-08-ip-cameras-design.md)
for current behavior and planned boundaries.
