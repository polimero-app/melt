# IP Cameras Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Users can add a Tapo C510W as an independent camera profile, link it to printers, and take snapshots or stream it from the CLI.

**Architecture:** The Bambu RTSP client moves to a vendor-neutral `melt-core/src/rtsp.rs`. Drivers hand it a connected socket, a URI and credentials. Bambu keeps its pinned TLS, fixed realm and quirks in a thin `bambu/rtsp.rs`. Camera profiles and printer links live in `melt.yaml`, which is written as schema version 2 only when camera data exists. Passwords live in the keychain under `camera:<id>:password`. A closed-set `cameras` domain resolves profiles and opens Tapo streams. The CLI adds `camera add|edit|list|remove|link|unlink|select` and a `--camera` selector on `camera snapshot|stream`.

**Tech Stack:** Rust 2024 (MSRV 1.85), `rtsp-types`, `openh264`, `jpeg-encoder`, OpenSSL, `serde_yaml`. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-10-08-ip-cameras-design.md`

## Global Constraints

- **No new crates.** Phase 1 uses only dependencies already in `crates/melt-core/Cargo.toml`.
- **Bambu wire behavior is unchanged:** `rtsps://<host>:322/streaming/live/1`, username `bblp`, realm `LIVE555 Streaming Media`, a byte-identical Digest header, interleaved channel 0.
- **Digest (MD5) only.** A Basic-only challenge is refused before any credential is sent.
- **Secrets stay out of YAML, fixtures, logs, error messages and `Debug` output.** `ResolvedCamera` and `CameraRequest` derive no `Debug`.
- **Exit codes:** validation and not-found 2, authentication and keychain 3, connection and timeout 4, unsupported codec 5, internal 1. Only existing error codes are used.
- **Frozen contract:** the Go-baseline inventory rows for `camera snapshot`/`camera stream` stay unchanged. New commands go in `meltNativeCommands`.
- **Tests:** stdlib `#[test]`, table-driven where there are several cases, no assertion crates. Network tests bind `127.0.0.1:0` only.
- **Commits:** conventional messages, one logical change each, every commit builds and passes `cargo test --workspace --locked`. End each message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Addresses:** only documentation addresses (`192.0.2.0/24`) appear in fixtures and docs.

## Review Focus

1. **Silent camera:** a camera that accepts TCP but never answers must fail with `timeout` (exit 4) within the profile timeout, not hang. Test: Task 3 `silent_camera_times_out`.
2. **Basic-only camera:** a camera offering only Basic auth must be refused without the password ever leaving the process. Test: Task 3 `basic_only_camera_is_refused_without_sending_credentials`.
3. **Wrong password at add time:** reports `authentication_failed` (exit 3) and saves nothing (no YAML entry, no keychain entry). Tests: Task 4 `failed_verification_saves_nothing`, Task 5 `camera_errors_follow_the_exit_contract`.
4. **Printer rename or removal after linking:** links follow the rename, and removal never leaves dangling references. Tests: Task 2 `printer_rename_moves_links_and_printer_removal_drops_them`, `renaming_a_printer_keeps_its_camera_links`.
5. **Renaming a camera that is offline:** must succeed without contacting it. Test: Task 4 `rename_keeps_password_and_skips_verification`.

---

### Task 1: Vendor-neutral RTSP client

**Files:**
- Move: `crates/melt-core/src/bambu/rtsp.rs` → `crates/melt-core/src/rtsp.rs`
- Create: `crates/melt-core/src/bambu/rtsp.rs` (thin Bambu adapter)
- Modify: `crates/melt-core/src/lib.rs`, `crates/melt-core/src/bambu.rs:71`, `crates/melt-core/src/bambu.rs:603-606`

**Interfaces:**
- Produces (crate-internal):
  - `rtsp::open_h264(connection: Box<dyn Duplex>, shutdown: TcpStream, uri: rtsp_types::Url, credentials: Credentials, decode: bool) -> Result<H264Stream, RtspError>`
  - `rtsp::Credentials { username: String, password: String, realm: Option<&'static str> }`
  - `pub enum rtsp::RtspError { Authentication, DigestRequired, UnsupportedCodec, Protocol(String), Io(io::Error) }`
  - `H264Stream::next_jpeg_frame` becomes `pub(crate)`
- Unchanged public paths: `melt_core::bambu::{H264Stream, H264AccessUnit}` and every `H264Stream` method.

- [ ] **Step 1: Pin Bambu's Digest header before touching it**

Add to the `tests` module in `crates/melt-core/src/bambu/rtsp.rs`:

```rust
    #[test]
    fn digest_header_is_byte_identical_for_live555() {
        let ha1 = md5_hex("bblp:LIVE555 Streaming Media:12345678");
        let ha2 = md5_hex("DESCRIBE:rtsps://printer.local:322/s");
        let response = md5_hex(&format!("{ha1}:n0nce:{ha2}"));
        assert_eq!(
            digest_header("12345678", "DESCRIBE", "rtsps://printer.local:322/s", "n0nce"),
            format!(
                "Digest username=\"bblp\", realm=\"LIVE555 Streaming Media\", nonce=\"n0nce\", uri=\"rtsps://printer.local:322/s\", response=\"{response}\""
            )
        );
    }
```

- [ ] **Step 2: Run it against the current code**

Run: `cargo test -p melt-core --locked bambu::rtsp::tests::digest_header_is_byte_identical_for_live555`
Expected: PASS. This characterises current behavior; the refactor must keep it passing.

- [ ] **Step 3: Move the module**

```bash
git mv crates/melt-core/src/bambu/rtsp.rs crates/melt-core/src/rtsp.rs
```

In `crates/melt-core/src/lib.rs`, add `mod rtsp;` after `pub mod profiles;`.

- [ ] **Step 4: Write the new-behavior tests (red)**

In `crates/melt-core/src/rtsp.rs`, replace the whole `#[cfg(test)] mod tests { … }` block with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn bambu() -> Credentials {
        Credentials {
            username: "bblp".into(),
            password: "12345678".into(),
            realm: Some("LIVE555 Streaming Media"),
        }
    }

    #[test]
    fn teardown_request_serializes_with_session_and_cseq() {
        // ponytail: The fire-and-forget property of Drop (reading no response for
        // TEARDOWN) is verified by inspection; exercising it fully requires a live
        // peer. This test covers only serialization format.
        let uri = rtsp_types::Url::parse("rtsps://printer.local:322/streaming/live/1").unwrap();
        let bytes = request_bytes(
            rtsp_types::Method::Teardown,
            &uri,
            9,
            &[(rtsp_types::headers::SESSION, "abc123".to_owned())],
            None,
        )
        .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("TEARDOWN rtsps://printer.local:322/streaming/live/1"));
        assert!(text.contains("CSeq: 9"));
        assert!(text.contains("Session: abc123"));
    }

    #[test]
    fn digest_header_is_byte_identical_for_live555() {
        let challenge =
            DigestChallenge::parse("Digest realm=\"LIVE555 Streaming Media\", nonce=\"n0nce\"")
                .unwrap();
        let ha1 = md5_hex("bblp:LIVE555 Streaming Media:12345678");
        let ha2 = md5_hex("DESCRIBE:rtsps://printer.local:322/s");
        let response = md5_hex(&format!("{ha1}:n0nce:{ha2}"));
        assert_eq!(
            digest_header(&bambu(), &challenge, "DESCRIBE", "rtsps://printer.local:322/s", "c0"),
            format!(
                "Digest username=\"bblp\", realm=\"LIVE555 Streaming Media\", nonce=\"n0nce\", uri=\"rtsps://printer.local:322/s\", response=\"{response}\""
            )
        );
    }

    #[test]
    fn qop_auth_challenge_uses_the_challenge_realm_and_cnonce() {
        let credentials = Credentials {
            username: "melt".into(),
            password: "secret".into(),
            realm: None,
        };
        let challenge = DigestChallenge::parse(
            "Digest realm=\"TP-Link IP-Camera\", nonce=\"abc\", qop=\"auth,auth-int\", opaque=\"op\"",
        )
        .unwrap();
        let ha1 = md5_hex("melt:TP-Link IP-Camera:secret");
        let ha2 = md5_hex("DESCRIBE:rtsp://192.0.2.20:554/stream1");
        let response = md5_hex(&format!("{ha1}:abc:00000001:c0ffee:auth:{ha2}"));
        assert_eq!(
            digest_header(&credentials, &challenge, "DESCRIBE", "rtsp://192.0.2.20:554/stream1", "c0ffee"),
            format!(
                "Digest username=\"melt\", realm=\"TP-Link IP-Camera\", nonce=\"abc\", uri=\"rtsp://192.0.2.20:554/stream1\", response=\"{response}\", qop=auth, nc=00000001, cnonce=\"c0ffee\", opaque=\"op\""
            )
        );
    }

    #[test]
    fn only_md5_digest_challenges_are_accepted() {
        let cases = [
            ("Basic realm=\"cam\"", false),
            ("Digest realm=\"cam\", nonce=\"n\", algorithm=SHA-256", false),
            ("Digest realm=\"cam\", nonce=\"n\", algorithm=MD5", true),
            ("Basic realm=\"cam\", Digest realm=\"cam\", nonce=\"n\"", true),
            ("Digest realm=\"cam\", cnonce=\"x\"", false),
        ];
        for (header, accepted) in cases {
            assert_eq!(DigestChallenge::parse(header).is_some(), accepted, "{header}");
        }
    }

    #[test]
    fn track_uri_resolves_relative_absolute_and_aggregate_controls() {
        let base = "rtsp://192.0.2.20:554/stream1/";
        let cases = [
            ("track1", "rtsp://192.0.2.20:554/stream1/track1"),
            ("rtsp://192.0.2.20:554/stream1/trackID=1", "rtsp://192.0.2.20:554/stream1/trackID=1"),
            ("*", "rtsp://192.0.2.20:554/stream1/"),
        ];
        for (control, expected) in cases {
            assert_eq!(track_uri(base, control).unwrap().as_str(), expected, "{control}");
        }
    }

    #[test]
    fn interleaved_channel_follows_the_setup_response() {
        assert_eq!(interleaved_channel(Some("RTP/AVP/TCP;unicast;interleaved=2-3")), 2);
        assert_eq!(interleaved_channel(Some("RTP/AVP/TCP;unicast")), 0);
        assert_eq!(interleaved_channel(None), 0);
    }

    #[test]
    fn sdp_selects_the_h264_video_track_and_rejects_other_codecs() {
        let tapo = "v=0\r\nm=audio 0 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\na=control:track2\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1;sprop-parameter-sets=Z0IAHg==,aM4G4g==\r\na=control:track1\r\n";
        let track = parse_sdp(tapo).unwrap();
        assert_eq!(track.control, "track1");
        assert_eq!(track.sps, [0x67, 0x42, 0x00, 0x1e]);
        assert_eq!(track.pps, [0x68, 0xce, 0x06, 0xe2]);

        let no_rtpmap = "m=video 0 RTP/AVP 96\r\na=control:track1\r\n";
        assert_eq!(parse_sdp(no_rtpmap).unwrap().control, "track1");

        let h265 = "m=video 0 RTP/AVP 96\r\na=rtpmap:96 H265/90000\r\na=control:track1\r\n";
        assert!(matches!(parse_sdp(h265), Err(RtspError::UnsupportedCodec)));
        assert!(matches!(parse_sdp("m=audio 0 RTP/AVP 8\r\n"), Err(RtspError::Protocol(_))));
    }
}
```

Run: `cargo test -p melt-core --locked rtsp::tests`
Expected: compile errors (`Credentials`, `DigestChallenge`, `track_uri`, `interleaved_channel`, `RtspError` not found).

- [ ] **Step 5: Generalize the top of `rtsp.rs`**

In `crates/melt-core/src/rtsp.rs`, replace everything from line 1 through the end of `fn parse_sdp` (old lines 1–575) with:

```rust
//! Vendor-neutral RTSP/H.264 client. One connection carries
//! DESCRIBE/SETUP/PLAY, Digest authentication, and TCP-interleaved RTP; H.264
//! is depacketized per RFC 6184 and optionally decoded to JPEG with
//! `openh264`. Drivers own the socket (pinned TLS, plain TCP), the URI, and the
//! credentials. Vendor quirks stay in the driver.

use std::{
    io::{self, Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

use openh264::{decoder::Decoder, formats::YUVSource};
use thiserror::Error;

/// Bounds how large a buffered-but-unparsed chunk may grow before we give up:
/// real RTSP responses and RTP frames are a few hundred bytes to a few KB, so
/// anything past this is a protocol confusion, not a slow network.
const MAX_BUFFERED: usize = 1 << 20;
// ponytail: fixed 25 s keepalive; derive it from the Session timeout if a
// camera advertises less than 30 s.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(25);

/// A connected byte stream: a pinned TLS session or a plain TCP socket.
pub(crate) trait Duplex: Read + Write + Send {}

impl<T: Read + Write + Send> Duplex for T {}

pub(crate) struct Credentials {
    pub username: String,
    pub password: String,
    /// A realm verified against real hardware (Bambu's LIVE555). `None` uses
    /// the realm from each challenge.
    pub realm: Option<&'static str>,
}

#[derive(Debug, Error)]
pub enum RtspError {
    #[error("RTSP authentication failed")]
    Authentication,
    #[error("RTSP server did not offer Digest authentication")]
    DigestRequired,
    #[error("RTSP video track is not H.264")]
    UnsupportedCodec,
    #[error("{0}")]
    Protocol(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn protocol_error(message: impl Into<String>) -> RtspError {
    RtspError::Protocol(message.into())
}

fn into_io(error: RtspError) -> io::Error {
    match error {
        RtspError::Io(error) => error,
        other => io::Error::other(other.to_string()),
    }
}

/// The RTSP control/data channel, with the small amount of buffering
/// `rtsp_types::Message::parse` needs to find message boundaries. The
/// handshake reads `Response`s and the streaming phase reads `Data` frames;
/// RTP/RTCP can arrive interleaved with (even before) the control responses
/// that set it up, and any mis-framed byte desyncs every following read.
struct FramedConnection {
    connection: Box<dyn Duplex>,
    buffer: Vec<u8>,
}

impl FramedConnection {
    fn write_all(&mut self, bytes: &[u8]) -> Result<(), RtspError> {
        Ok(self.connection.write_all(bytes)?)
    }

    /// Reads and returns the next complete RTSP message, buffering any bytes
    /// received past its end for the next call.
    fn next_message(&mut self) -> Result<rtsp_types::Message<Vec<u8>>, RtspError> {
        let mut chunk = [0u8; 4096];
        loop {
            match rtsp_types::Message::<Vec<u8>>::parse(&self.buffer) {
                Ok((message, consumed)) => {
                    self.buffer.drain(..consumed);
                    return Ok(message);
                }
                Err(rtsp_types::ParseError::Incomplete(_)) => {
                    if self.buffer.len() > MAX_BUFFERED {
                        return Err(protocol_error("RTSP stream exceeded the buffering limit"));
                    }
                    let read = self.connection.read(&mut chunk)?;
                    if read == 0 {
                        return Err(protocol_error("RTSP connection closed unexpectedly"));
                    }
                    self.buffer.extend_from_slice(&chunk[..read]);
                }
                Err(_) => return Err(protocol_error("malformed RTSP message")),
            }
        }
    }

    /// Reads messages until a `Response` arrives, dropping interleaved `Data`
    /// frames (harmless media that arrived early) and failing on an
    /// unexpected `Request`.
    fn next_response(&mut self) -> Result<rtsp_types::Response<Vec<u8>>, RtspError> {
        loop {
            match self.next_message()? {
                rtsp_types::Message::Response(response) => return Ok(response),
                rtsp_types::Message::Data(_) => continue,
                rtsp_types::Message::Request(_) => {
                    return Err(protocol_error(
                        "RTSP server sent a request, expected a response",
                    ));
                }
            }
        }
    }

    /// Reads messages until an RTP `Data` frame on the video channel arrives,
    /// ignoring RTCP, other tracks, and late control `Response`s.
    fn next_rtp_packet(&mut self, channel: u8) -> Result<Vec<u8>, RtspError> {
        loop {
            match self.next_message()? {
                rtsp_types::Message::Data(data) if data.channel_id() == channel => {
                    return Ok(data.into_body());
                }
                rtsp_types::Message::Data(_) | rtsp_types::Message::Response(_) => continue,
                rtsp_types::Message::Request(_) => {
                    return Err(protocol_error(
                        "RTSP server sent an unexpected request mid-stream",
                    ));
                }
            }
        }
    }
}

/// Native H.264/RTP camera stream.
///
/// The camera's original RTP payloads are exposed so a WebRTC gateway can
/// forward them without decoding and re-encoding every frame.
pub struct H264Stream {
    connection: FramedConnection,
    shutdown: TcpStream,
    uri: rtsp_types::Url,
    credentials: Credentials,
    session: String,
    channel: u8,
    cseq: u32,
    next_keepalive: Instant,
    decoder: Option<Decoder>,
    decoder_started: bool,
    sps: Vec<u8>,
    pps: Vec<u8>,
    started: bool,
    access_unit: Vec<Vec<u8>>,
    fu_buffer: Option<Vec<u8>>,
}

/// One complete H.264 picture as received from the camera. RTP packet bytes
/// are retained verbatim for WebRTC forwarding, while an Annex-B copy feeds
/// browser decoders. JPEG output is present only while the bounded software
/// preview decoder is enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct H264AccessUnit {
    pub rtp_packets: Vec<Vec<u8>>,
    /// Annex-B byte stream for browser decoders. It starts at an IDR and
    /// includes the current SPS/PPS before every keyframe.
    pub annex_b: Option<Vec<u8>>,
    pub keyframe: bool,
    pub jpeg: Option<Vec<u8>>,
}

impl H264Stream {
    pub fn shutdown_handle(&self) -> io::Result<TcpStream> {
        self.shutdown.try_clone()
    }

    pub fn parameter_sets(&self) -> (&[u8], &[u8]) {
        (&self.sps, &self.pps)
    }

    /// Enables the software preview decoder only while a JPEG consumer is
    /// attached. Starting from a fresh decoder waits for the next IDR; the
    /// normalized access unit already carries the cached SPS/PPS.
    pub fn set_preview_decode_enabled(&mut self, enabled: bool) -> io::Result<()> {
        if enabled == self.decoder.is_some() {
            return Ok(());
        }
        self.decoder = enabled
            .then(|| {
                Decoder::new().map_err(|error| {
                    io::Error::other(format!("H.264 decoder init failed: {error}"))
                })
            })
            .transpose()?;
        self.decoder_started = false;
        self.access_unit.clear();
        self.fu_buffer = None;
        Ok(())
    }

    pub fn next_rtp_packet(&mut self) -> io::Result<Vec<u8>> {
        self.keepalive_if_due()?;
        self.connection
            .next_rtp_packet(self.channel)
            .map_err(into_io)
    }

    pub fn next_access_unit(&mut self) -> io::Result<H264AccessUnit> {
        let mut rtp_packets = Vec::new();
        loop {
            let packet = self.next_rtp_packet()?;
            let Some((marker, payload)) = parse_rtp(&packet) else {
                continue;
            };
            self.depacketize(payload);
            rtp_packets.push(packet);
            if !marker {
                continue;
            }
            let access_unit = self.build_access_unit();
            let keyframe = access_unit.as_ref().is_some_and(|(_, keyframe)| *keyframe);
            let should_decode = self.decoder.is_some()
                && access_unit.is_some()
                && (self.decoder_started || keyframe);
            let jpeg = match (access_unit.as_ref(), self.decoder.as_mut(), should_decode) {
                (Some((access_unit, _)), Some(decoder), true) => {
                    match decoder.decode(access_unit) {
                        Ok(Some(yuv)) => Some(encode_jpeg(&yuv)?),
                        Ok(None) => None,
                        Err(error) => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("H.264 decode failed: {error}"),
                            ));
                        }
                    }
                }
                _ => None,
            };
            if should_decode {
                self.decoder_started = true;
            }
            return Ok(H264AccessUnit {
                rtp_packets,
                annex_b: access_unit.map(|(bytes, _)| bytes),
                keyframe,
                jpeg,
            });
        }
    }

    fn keepalive_if_due(&mut self) -> io::Result<()> {
        if Instant::now() < self.next_keepalive {
            return Ok(());
        }
        let response = authorized_request(
            &mut self.connection,
            rtsp_types::Method::GetParameter,
            &self.uri,
            &mut self.cseq,
            &self.credentials,
            &[(rtsp_types::headers::SESSION, self.session.clone())],
        )
        .map_err(into_io)?;
        if response.status() != rtsp_types::StatusCode::Ok {
            return Err(io::Error::other("RTSP keepalive was rejected"));
        }
        self.next_keepalive = Instant::now() + KEEPALIVE_INTERVAL;
        Ok(())
    }
}

impl Drop for H264Stream {
    fn drop(&mut self) {
        // Fire-and-forget. Waiting for the TEARDOWN reply means draining every
        // queued interleaved media frame first, blocking the dropping thread
        // for the socket's whole idle budget for a response nothing reads.
        let _ = write_request(
            &mut self.connection,
            rtsp_types::Method::Teardown,
            &self.uri,
            self.cseq,
            &[(rtsp_types::headers::SESSION, self.session.clone())],
            None,
        );
    }
}

/// Runs DESCRIBE/SETUP/PLAY for the first H.264 video track at `uri` over an
/// already-connected socket. `shutdown` is a clone of the underlying TCP
/// socket, used to interrupt blocked reads.
pub(crate) fn open_h264(
    connection: Box<dyn Duplex>,
    shutdown: TcpStream,
    uri: rtsp_types::Url,
    credentials: Credentials,
    decode: bool,
) -> Result<H264Stream, RtspError> {
    let mut connection = FramedConnection {
        connection,
        buffer: Vec::new(),
    };
    let mut cseq = 1u32;
    let describe = authorized_request(
        &mut connection,
        rtsp_types::Method::Describe,
        &uri,
        &mut cseq,
        &credentials,
        &[(rtsp_types::headers::ACCEPT, "application/sdp".to_owned())],
    )?;
    expect_ok(&describe, "DESCRIBE")?;
    let content_base = describe
        .header(&rtsp_types::headers::CONTENT_BASE)
        .map(|value| value.as_str().to_owned())
        .unwrap_or_else(|| uri.as_str().to_owned());
    let track = parse_sdp(&String::from_utf8_lossy(describe.body()))?;
    let setup_uri = track_uri(&content_base, &track.control)?;

    let setup = authorized_request(
        &mut connection,
        rtsp_types::Method::Setup,
        &setup_uri,
        &mut cseq,
        &credentials,
        &[(
            rtsp_types::headers::TRANSPORT,
            "RTP/AVP/TCP;unicast;interleaved=0-1".to_owned(),
        )],
    )?;
    expect_ok(&setup, "SETUP")?;
    let session = setup
        .header(&rtsp_types::headers::SESSION)
        .map(|value| {
            value
                .as_str()
                .split(';')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .ok_or_else(|| protocol_error("RTSP SETUP response missing Session"))?;
    let channel = interleaved_channel(
        setup
            .header(&rtsp_types::headers::TRANSPORT)
            .map(|value| value.as_str()),
    );

    let play = authorized_request(
        &mut connection,
        rtsp_types::Method::Play,
        &uri,
        &mut cseq,
        &credentials,
        &[
            (rtsp_types::headers::SESSION, session.clone()),
            (rtsp_types::headers::RANGE, "npt=0.000-".to_owned()),
        ],
    )?;
    expect_ok(&play, "PLAY")?;

    let decoder = decode
        .then(|| {
            Decoder::new()
                .map_err(|error| protocol_error(format!("H.264 decoder init failed: {error}")))
        })
        .transpose()?;

    Ok(H264Stream {
        connection,
        shutdown,
        uri,
        credentials,
        session,
        channel,
        cseq,
        next_keepalive: Instant::now() + KEEPALIVE_INTERVAL,
        decoder,
        decoder_started: false,
        sps: track.sps,
        pps: track.pps,
        started: false,
        access_unit: Vec::new(),
        fu_buffer: None,
    })
}

fn expect_ok(response: &rtsp_types::Response<Vec<u8>>, method: &str) -> Result<(), RtspError> {
    if response.status() == rtsp_types::StatusCode::Ok {
        return Ok(());
    }
    Err(protocol_error(format!(
        "RTSP {method} failed: {}",
        response.reason_phrase()
    )))
}

/// Resolves an SDP `a=control:` value against the presentation base. RTSP
/// requests ride the already-open connection, so an absolute control URI
/// cannot redirect credentials to another host.
fn track_uri(content_base: &str, control: &str) -> Result<rtsp_types::Url, RtspError> {
    let joined = if control == "*" {
        content_base.to_owned()
    } else if control.starts_with("rtsp://") || control.starts_with("rtsps://") {
        control.to_owned()
    } else {
        format!("{}/{control}", content_base.trim_end_matches('/'))
    };
    rtsp_types::Url::parse(&joined).map_err(|_| protocol_error("invalid RTSP track URL"))
}

/// The RTP channel from the SETUP response's `interleaved=a-b`; servers may
/// assign a different pair than requested.
fn interleaved_channel(transport: Option<&str>) -> u8 {
    transport
        .and_then(|value| {
            value
                .split(';')
                .find_map(|part| part.trim().strip_prefix("interleaved="))
        })
        .and_then(|range| range.split('-').next()?.parse().ok())
        .unwrap_or(0)
}

/// Sends a request, and if challenged with Digest `401 Unauthorized`, retries
/// once with computed credentials. Every request runs its own challenge
/// rather than trusting a nonce across DESCRIBE/SETUP/PLAY: one extra round
/// trip, no shared-nonce assumptions. A Basic-only challenge is refused, since
/// Basic would send the password in clear over plain RTSP.
fn authorized_request(
    connection: &mut FramedConnection,
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: &mut u32,
    credentials: &Credentials,
    extra_headers: &[(rtsp_types::HeaderName, String)],
) -> Result<rtsp_types::Response<Vec<u8>>, RtspError> {
    let response = send_request(connection, method.clone(), uri, *cseq, extra_headers, None)?;
    *cseq += 1;
    if response.status() != rtsp_types::StatusCode::Unauthorized {
        return Ok(response);
    }
    // ponytail: reads one WWW-Authenticate value; parse every header if a
    // camera sends Basic and Digest as separate headers with Basic first.
    let challenge = response
        .header(&rtsp_types::headers::WWW_AUTHENTICATE)
        .and_then(|value| DigestChallenge::parse(value.as_str()))
        .ok_or(RtspError::DigestRequired)?;
    let method_name: &str = (&method).into();
    let authorization = digest_header(
        credentials,
        &challenge,
        method_name,
        uri.as_str(),
        &cnonce(),
    );
    let response = send_request(
        connection,
        method,
        uri,
        *cseq,
        extra_headers,
        Some(&authorization),
    )?;
    *cseq += 1;
    if response.status() == rtsp_types::StatusCode::Unauthorized {
        return Err(RtspError::Authentication);
    }
    Ok(response)
}

/// Serializes one RTSP request. Kept separate from the socket so a teardown
/// can be written without also committing to read a reply.
fn request_bytes(
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: u32,
    extra_headers: &[(rtsp_types::HeaderName, String)],
    authorization: Option<&str>,
) -> Result<Vec<u8>, RtspError> {
    let mut builder = rtsp_types::Request::builder(method, rtsp_types::Version::V1_0)
        .request_uri(uri.clone())
        .header(rtsp_types::headers::CSEQ, format!("{cseq}"));
    for (name, value) in extra_headers {
        builder = builder.header(name.clone(), value.clone());
    }
    if let Some(authorization) = authorization {
        builder = builder.header(rtsp_types::headers::AUTHORIZATION, authorization.to_owned());
    }
    let mut bytes = Vec::new();
    builder
        .empty()
        .write(&mut bytes)
        .map_err(|_| protocol_error("failed to serialize RTSP request"))?;
    Ok(bytes)
}

fn write_request(
    connection: &mut FramedConnection,
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: u32,
    extra_headers: &[(rtsp_types::HeaderName, String)],
    authorization: Option<&str>,
) -> Result<(), RtspError> {
    let bytes = request_bytes(method, uri, cseq, extra_headers, authorization)?;
    connection.write_all(&bytes)
}

fn send_request(
    connection: &mut FramedConnection,
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: u32,
    extra_headers: &[(rtsp_types::HeaderName, String)],
    authorization: Option<&str>,
) -> Result<rtsp_types::Response<Vec<u8>>, RtspError> {
    write_request(connection, method, uri, cseq, extra_headers, authorization)?;
    connection.next_response()
}

struct DigestChallenge {
    realm: String,
    nonce: String,
    qop_auth: bool,
    opaque: Option<String>,
}

impl DigestChallenge {
    /// Finds an MD5 Digest challenge in a `WWW-Authenticate` value.
    fn parse(header: &str) -> Option<Self> {
        let params = &header[header.find("Digest ")? + "Digest ".len()..];
        if param(params, "algorithm").is_some_and(|value| !value.eq_ignore_ascii_case("MD5")) {
            return None;
        }
        Some(Self {
            realm: param(params, "realm").unwrap_or_default(),
            nonce: param(params, "nonce")?,
            qop_auth: param(params, "qop")
                .is_some_and(|value| value.split(',').any(|qop| qop.trim() == "auth")),
            opaque: param(params, "opaque"),
        })
    }
}

/// Reads `key="value"` or `key=token` from a comma-separated auth parameter
/// list, matching whole keys only (`nonce` never matches `cnonce`).
fn param(params: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=");
    let mut search = 0;
    while let Some(found) = params[search..].find(&marker) {
        let start = search + found;
        let at_boundary = params[..start]
            .chars()
            .next_back()
            .is_none_or(|before| before == ' ' || before == ',');
        let value = &params[start + marker.len()..];
        if at_boundary {
            return Some(match value.strip_prefix('"') {
                Some(quoted) => quoted.split('"').next()?.to_owned(),
                None => value.split(',').next()?.trim().to_owned(),
            });
        }
        search = start + marker.len();
    }
    None
}

fn md5_hex(input: &str) -> String {
    let digest = openssl::hash::hash(openssl::hash::MessageDigest::md5(), input.as_bytes())
        .expect("md5 is always available");
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn cnonce() -> String {
    let mut bytes = [0u8; 8];
    openssl::rand::rand_bytes(&mut bytes).expect("OpenSSL random source is available");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn digest_header(
    credentials: &Credentials,
    challenge: &DigestChallenge,
    method: &str,
    uri: &str,
    cnonce: &str,
) -> String {
    let username = &credentials.username;
    let realm = credentials.realm.unwrap_or(&challenge.realm);
    let nonce = &challenge.nonce;
    let ha1 = md5_hex(&format!("{username}:{realm}:{}", credentials.password));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let mut header = if challenge.qop_auth {
        let response = md5_hex(&format!("{ha1}:{nonce}:00000001:{cnonce}:auth:{ha2}"));
        format!(
            "Digest username=\"{username}\", realm=\"{realm}\", nonce=\"{nonce}\", uri=\"{uri}\", response=\"{response}\", qop=auth, nc=00000001, cnonce=\"{cnonce}\""
        )
    } else {
        let response = md5_hex(&format!("{ha1}:{nonce}:{ha2}"));
        format!(
            "Digest username=\"{username}\", realm=\"{realm}\", nonce=\"{nonce}\", uri=\"{uri}\", response=\"{response}\""
        )
    };
    if let Some(opaque) = &challenge.opaque {
        header.push_str(&format!(", opaque=\"{opaque}\""));
    }
    header
}

struct VideoTrack {
    control: String,
    sps: Vec<u8>,
    pps: Vec<u8>,
}

/// Extracts the first video section's control URI and `sprop-parameter-sets`;
/// audio and other media sections are ignored. A video section whose
/// `rtpmap`s name no H.264 payload is unsupported. One without `rtpmap` is
/// taken as H.264, as before this module served more than Bambu.
fn parse_sdp(sdp: &str) -> Result<VideoTrack, RtspError> {
    let missing = || protocol_error("no usable H.264 track in SDP");
    let video_start = sdp.find("m=video").ok_or_else(missing)?;
    let video_section = &sdp[video_start..];
    let video_section = video_section
        .find("\nm=")
        .map(|next| &video_section[..next + 1])
        .unwrap_or(video_section);

    let mut control = None;
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    let mut rtpmaps = 0;
    let mut h264 = false;
    for line in video_section.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("a=control:") {
            control = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("a=rtpmap:") {
            rtpmaps += 1;
            h264 |= value.to_ascii_uppercase().contains(" H264/");
        } else if let Some(value) = line.strip_prefix("a=fmtp:") {
            if let Some(sets_start) = value.find("sprop-parameter-sets=") {
                let sets = &value[sets_start + "sprop-parameter-sets=".len()..];
                let sets = sets.split(';').next().unwrap_or(sets);
                let mut parts = sets.split(',');
                if let (Some(sps_b64), Some(pps_b64)) = (parts.next(), parts.next()) {
                    use base64::Engine;
                    sps = base64::engine::general_purpose::STANDARD
                        .decode(sps_b64.trim())
                        .unwrap_or_default();
                    pps = base64::engine::general_purpose::STANDARD
                        .decode(pps_b64.trim())
                        .unwrap_or_default();
                }
            }
        }
    }
    if rtpmaps > 0 && !h264 {
        return Err(RtspError::UnsupportedCodec);
    }
    Ok(VideoTrack {
        control: control.ok_or_else(missing)?,
        sps,
        pps,
    })
}
```

In the remaining `impl H264Stream` block, change `pub(super) fn next_jpeg_frame` to `pub(crate) fn next_jpeg_frame`. Leave `push_nal`, `depacketize`, `build_access_unit`, `parse_rtp` and `encode_jpeg` unchanged.

- [ ] **Step 6: Recreate the Bambu adapter**

Create `crates/melt-core/src/bambu/rtsp.rs`:

```rust
//! RTSPS/H.264 camera endpoint for Bambu printers whose local camera only
//! serves RTSP-over-TLS on port 322 (H/X-series). The classic MJPEG-over-6000
//! protocol (`super::open_mjpeg_stream`) is not offered by these models at
//! all: the printer replies to that handshake with a fixed rejection packet
//! regardless of its contents, because it never speaks that protocol.
//!
//! The port, path, digest realm, and username were confirmed against a real
//! H2-series printer. The handshake itself lives in `crate::rtsp`.

use std::{
    io,
    time::{Duration, Instant},
};

use super::{CameraError, MQTT_USERNAME, Profile, transport};
use crate::rtsp::{self, Credentials, H264Stream, RtspError};

const RTSP_PORT: u16 = 322;
const RTSP_REALM: &str = "LIVE555 Streaming Media";
const RTSP_PATH: &str = "/streaming/live/1";

pub fn open_h264_stream(
    profile: &Profile,
    access_code: &str,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<H264Stream, CameraError> {
    open(profile, access_code, fingerprint, timeout, false)
}

pub(super) fn open_decoded_h264_stream(
    profile: &Profile,
    access_code: &str,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<H264Stream, CameraError> {
    open(profile, access_code, fingerprint, timeout, true)
}

fn open(
    profile: &Profile,
    access_code: &str,
    fingerprint: Option<&str>,
    timeout: Duration,
    decode: bool,
) -> Result<H264Stream, CameraError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| CameraError::Connect(io::Error::from(io::ErrorKind::TimedOut)))?;
    let connector = transport::tls_connector().map_err(|_| CameraError::Tls)?;
    let (connection, _) =
        transport::open_tls(&connector, profile, RTSP_PORT, fingerprint, true, deadline)
            .map_err(map_transport_error)?;
    // The handshake budget must not become the live stream's read deadline: a
    // slow connect (or a failed RTSPS probe ahead of this one) would otherwise
    // leave the camera with a sub-second inter-frame timeout.
    transport::set_socket_idle_timeout(connection.get_ref(), profile.timeout())
        .map_err(map_transport_error)?;
    let shutdown = connection.get_ref().try_clone().map_err(CameraError::Stream)?;
    let uri = rtsp_types::Url::parse(&format!(
        "rtsps://{}:{RTSP_PORT}{RTSP_PATH}",
        profile.host()
    ))
    .map_err(|_| stream_error("invalid RTSP camera URL"))?;
    rtsp::open_h264(
        Box::new(connection),
        shutdown,
        uri,
        Credentials {
            username: MQTT_USERNAME.to_owned(),
            password: access_code.to_owned(),
            realm: Some(RTSP_REALM),
        },
        decode,
    )
    .map_err(camera_error)
}

/// Bambu has always reported every RTSP failure as a stream failure; keep
/// that contract.
fn camera_error(error: RtspError) -> CameraError {
    match error {
        RtspError::Io(error) => CameraError::Stream(error),
        other => stream_error(&other.to_string()),
    }
}

fn map_transport_error(error: transport::Error) -> CameraError {
    match error {
        transport::Error::Pin(error) => CameraError::Pin(error),
        transport::Error::MissingCertificate => CameraError::MissingCertificate,
        transport::Error::CertificateIdentity => CameraError::Identity,
        transport::Error::Tls => CameraError::Tls,
        transport::Error::Timeout => CameraError::Connect(io::Error::from(io::ErrorKind::TimedOut)),
        _ => CameraError::Connect(io::Error::from(io::ErrorKind::ConnectionRefused)),
    }
}

fn stream_error(message: &str) -> CameraError {
    CameraError::Stream(io::Error::other(message.to_owned()))
}
```

In `crates/melt-core/src/bambu.rs`:
- line 71: `pub use rtsp::{H264AccessUnit, H264Stream};` → `pub use crate::rtsp::{H264AccessUnit, H264Stream};`
- in `enum MjpegSource`: `H264(Box<rtsp::H264Stream>),` → `H264(Box<H264Stream>),`
- add to `impl MjpegStream` (used by Task 3):

```rust
    /// Wraps a decoding H.264 stream from any RTSP camera as MJPEG.
    #[allow(dead_code)]
    pub(crate) fn from_h264(stream: H264Stream) -> Self {
        Self {
            source: MjpegSource::H264(Box::new(stream)),
            pending: Vec::new(),
        }
    }
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -p melt-core --locked rtsp::tests`
Expected: PASS (7 tests).

Run: `cargo test --workspace --locked && make bambu-evidence-check`
Expected: PASS, with no change to the Bambu camera tests in `camera_manager.rs`, `melt-desktop` or `bambu_evidence`.

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: no warnings.

- [ ] **Step 8: Commit**

```bash
git add crates/melt-core/src/rtsp.rs crates/melt-core/src/bambu/rtsp.rs crates/melt-core/src/bambu.rs crates/melt-core/src/lib.rs
git commit -m "refactor(camera): extract a vendor-neutral RTSP client

Bambu keeps its TLS, realm, and path in a thin adapter; the shared client
adds challenge realms, qop=auth, Basic refusal, advertised interleaved
channels, and codec detection.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Camera profiles and printer links in `melt.yaml`

**Files:**
- Modify: `crates/melt-core/src/config.rs`
- Modify: `crates/melt-core/src/profiles.rs` (`update`, rename path)

**Interfaces:**
- Produces in `melt_core::config`:
  - `pub struct CameraProfile { pub id: String, pub driver: String, pub host: String, pub username: String, pub rtsp_port: Option<u16>, pub timeout: String, pub created: String, pub updated: String }`
  - `pub struct PrinterCameras { pub cameras: Vec<String>, pub selected: Option<String> }` (IDs; `selected: None` means the built-in feed)
  - `Config::{get_camera, cameras, add_camera, replace_camera, remove_camera, link_camera, unlink_camera, select_camera, printer_cameras, camera_printers, rename_profile}`
  - `ConfigError::{InvalidCameras(&'static str), CameraAlreadyExists, CameraNotFound, CameraNotLinked}`
  - `pub fn is_camera_id(id: &str) -> bool`

- [ ] **Step 1: Write the failing config tests**

Append inside `mod tests` in `crates/melt-core/src/config.rs`:

```rust
    const PORCH: &str = "0123456789abcdef0123456789abcdef";
    const GATE: &str = "fedcba9876543210fedcba9876543210";

    fn camera(id: &str) -> CameraProfile {
        CameraProfile {
            id: id.into(),
            driver: "tapo".into(),
            host: "192.0.2.20".into(),
            username: "melt".into(),
            rtsp_port: None,
            timeout: "10s".into(),
            created: "2026-10-08T12:00:00Z".into(),
            updated: "2026-10-08T12:00:00Z".into(),
        }
    }

    #[test]
    fn printer_only_configs_stay_readable_by_version_1_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.save(dir.path()).unwrap();

        let yaml = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(yaml.starts_with("version: 1\n"), "{yaml}");
        assert!(!yaml.contains("cameras"), "{yaml}");
    }

    #[test]
    fn camera_data_is_written_as_version_2_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        config.link_camera("garage", "porch", true).unwrap();
        config.save(dir.path()).unwrap();

        let yaml = fs::read_to_string(dir.path().join(CONFIG_FILE)).unwrap();
        assert!(yaml.starts_with("version: 2\n"), "{yaml}");
        let reopened = Config::open(dir.path()).unwrap();
        assert_eq!(reopened.get_camera("porch"), Some(&camera(PORCH)));
        assert_eq!(
            reopened.printer_cameras("garage"),
            Some(&PrinterCameras {
                cameras: vec![PORCH.into()],
                selected: Some(PORCH.into()),
            })
        );
        assert_eq!(
            reopened.camera_printers(PORCH),
            (vec!["garage".to_owned()], vec!["garage".to_owned()])
        );
    }

    #[test]
    fn rejects_invalid_camera_sections_instead_of_dropping_them() {
        let printer = "profiles:\n  garage: {driver: moonraker, host: printer.local, timeout: 10s, insecure: false, created: x, updated: x}\n";
        let porch = "cameras:\n  porch: {id: 0123456789abcdef0123456789abcdef, driver: tapo, host: 192.0.2.20, username: melt, timeout: 10s, created: x, updated: x}\n";
        let cases = [
            ("camera data in version 1", format!("version: 1\n{porch}")),
            (
                "link to unknown printer",
                format!("version: 2\n{porch}printer_cameras:\n  garage: {{cameras: [0123456789abcdef0123456789abcdef]}}\n"),
            ),
            (
                "link to unknown camera",
                format!("version: 2\n{printer}{porch}printer_cameras:\n  garage: {{cameras: [fedcba9876543210fedcba9876543210]}}\n"),
            ),
            (
                "selection not linked",
                format!("version: 2\n{printer}{porch}printer_cameras:\n  garage: {{cameras: [], selected: 0123456789abcdef0123456789abcdef}}\n"),
            ),
            (
                "malformed id",
                "version: 2\ncameras:\n  porch: {id: NOT-HEX, driver: tapo, host: 192.0.2.20, username: melt, timeout: 10s, created: x, updated: x}\n".to_owned(),
            ),
        ];
        for (name, yaml) in cases {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join(CONFIG_FILE), yaml).unwrap();
            assert!(
                matches!(Config::open(dir.path()), Err(ConfigError::InvalidCameras(_))),
                "{name}"
            );
        }
    }

    #[test]
    fn future_versions_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CONFIG_FILE), "version: 3\n").unwrap();
        assert!(matches!(
            Config::open(dir.path()),
            Err(ConfigError::UnsupportedVersion { got: 3 })
        ));
    }

    #[test]
    fn printer_rename_moves_links_and_printer_removal_drops_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        config.link_camera("garage", "porch", true).unwrap();

        config.rename_profile("garage", "workshop", profile()).unwrap();
        assert!(config.printer_cameras("garage").is_none());
        assert_eq!(
            config.printer_cameras("workshop").unwrap().selected.as_deref(),
            Some(PORCH)
        );

        config.remove_profile("workshop").unwrap();
        assert!(config.printer_cameras("workshop").is_none());
        assert!(config.get_camera("porch").is_some());
    }

    #[test]
    fn camera_removal_and_unlinking_fall_back_to_the_builtin_feed() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_profile("attic", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();
        config.add_camera("gate", camera(GATE)).unwrap();
        config.link_camera("garage", "porch", true).unwrap();
        config.link_camera("garage", "gate", false).unwrap();
        config.link_camera("attic", "porch", false).unwrap();

        config.remove_camera("porch").unwrap();
        assert_eq!(
            config.printer_cameras("garage"),
            Some(&PrinterCameras {
                cameras: vec![GATE.into()],
                selected: None,
            })
        );
        assert!(config.printer_cameras("attic").is_none());

        config.select_camera("garage", Some("gate")).unwrap();
        config.unlink_camera("garage", "gate").unwrap();
        assert!(config.printer_cameras("garage").is_none());
    }

    #[test]
    fn link_operations_report_missing_targets() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config.add_profile("garage", profile()).unwrap();
        config.add_camera("porch", camera(PORCH)).unwrap();

        assert!(matches!(
            config.link_camera("nope", "porch", false),
            Err(ConfigError::ProfileNotFound)
        ));
        assert!(matches!(
            config.link_camera("garage", "nope", false),
            Err(ConfigError::CameraNotFound)
        ));
        assert!(matches!(
            config.select_camera("garage", Some("porch")),
            Err(ConfigError::CameraNotLinked)
        ));
        assert!(matches!(
            config.unlink_camera("garage", "porch"),
            Err(ConfigError::CameraNotLinked)
        ));
        assert!(matches!(
            config.add_camera("porch", camera(GATE)),
            Err(ConfigError::CameraAlreadyExists)
        ));
    }
```

Run: `cargo test -p melt-core --locked config::tests`
Expected: compile errors (`CameraProfile`, `PrinterCameras`, new methods not found).

- [ ] **Step 2: Implement the schema in `config.rs`**

Change the imports and constant at the top of `config.rs`:

```rust
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};
```

```rust
const CONFIG_FILE: &str = "melt.yaml";
/// Version 2 adds camera profiles and printer links. Files without camera
/// data are still written as version 1 so older binaries keep reading them.
const MAX_VERSION: u8 = 2;
```

Replace `ConfigError` with:

```rust
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("unsupported config schema version: got {got}, want 1 or {MAX_VERSION}")]
    UnsupportedVersion { got: u8 },
    #[error("malformed config file: {0}")]
    Malformed(#[from] serde_yaml::Error),
    #[error("invalid camera configuration: {0}")]
    InvalidCameras(&'static str),
    #[error("config file changed since it was read")]
    ConcurrentUpdate,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("profile already exists")]
    ProfileAlreadyExists,
    #[error("profile not found")]
    ProfileNotFound,
    #[error("camera already exists")]
    CameraAlreadyExists,
    #[error("camera not found")]
    CameraNotFound,
    #[error("camera is not linked to that printer")]
    CameraNotLinked,
}
```

After `NamedProfile`, add:

```rust
/// An external camera. The name is its map key; `id` is immutable and keys
/// both printer links and the keychain password, so renames touch neither.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CameraProfile {
    pub id: String,
    pub driver: String,
    pub host: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtsp_port: Option<u16>,
    pub timeout: String,
    pub created: String,
    pub updated: String,
}

/// Cameras linked to one printer. `selected: None` is the built-in feed.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrinterCameras {
    #[serde(default)]
    pub cameras: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<String>,
}

pub fn is_camera_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}
```

Replace `ConfigFile` and `Config` with:

```rust
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    #[serde(default)]
    version: u8,
    #[serde(default)]
    profiles: BTreeMap<String, Profile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    cameras: BTreeMap<String, CameraProfile>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    printer_cameras: BTreeMap<String, PrinterCameras>,
}

#[derive(Debug)]
pub struct Config {
    profiles: BTreeMap<String, Profile>,
    cameras: BTreeMap<String, CameraProfile>,
    printer_cameras: BTreeMap<String, PrinterCameras>,
    digest: Option<[u8; 32]>,
}
```

In `Config::open`, replace the `bytes.is_empty()` early return and everything after it with:

```rust
        if bytes.is_empty() {
            return Ok(Self {
                digest: Some(digest(&bytes)),
                ..Self::empty()
            });
        }

        let file: ConfigFile = serde_yaml::from_slice(&bytes).map_err(ConfigError::Malformed)?;
        if !(1..=MAX_VERSION).contains(&file.version) {
            return Err(ConfigError::UnsupportedVersion { got: file.version });
        }
        if file.version == 1 && (!file.cameras.is_empty() || !file.printer_cameras.is_empty()) {
            return Err(ConfigError::InvalidCameras(
                "camera data requires config version 2",
            ));
        }
        let config = Self {
            profiles: file.profiles,
            cameras: file.cameras,
            printer_cameras: file.printer_cameras,
            digest: Some(digest(&bytes)),
        };
        config.validate()?;
        Ok(config)
    }
```

Replace `remove_profile` with these two methods:

```rust
    /// Removes a printer and its camera links. Camera profiles stay.
    pub fn remove_profile(&mut self, name: &str) -> Result<Profile, ConfigError> {
        let profile = self
            .profiles
            .remove(name)
            .ok_or(ConfigError::ProfileNotFound)?;
        self.printer_cameras.remove(name);
        Ok(profile)
    }

    /// Renames a printer, carrying its camera links along in the same write.
    pub fn rename_profile(
        &mut self,
        current: &str,
        name: &str,
        profile: Profile,
    ) -> Result<(), ConfigError> {
        if self.profiles.contains_key(name) {
            return Err(ConfigError::ProfileAlreadyExists);
        }
        self.profiles
            .remove(current)
            .ok_or(ConfigError::ProfileNotFound)?;
        self.profiles.insert(name.to_owned(), profile);
        if let Some(links) = self.printer_cameras.remove(current) {
            self.printer_cameras.insert(name.to_owned(), links);
        }
        Ok(())
    }

    pub fn get_camera(&self, name: &str) -> Option<&CameraProfile> {
        self.cameras.get(name)
    }

    pub fn cameras(&self) -> &BTreeMap<String, CameraProfile> {
        &self.cameras
    }

    pub fn add_camera(&mut self, name: &str, camera: CameraProfile) -> Result<(), ConfigError> {
        if self.cameras.contains_key(name) {
            return Err(ConfigError::CameraAlreadyExists);
        }
        self.cameras.insert(name.to_owned(), camera);
        Ok(())
    }

    /// Replaces a camera, renaming it when `name` differs from `current`.
    pub fn replace_camera(
        &mut self,
        current: &str,
        name: &str,
        camera: CameraProfile,
    ) -> Result<(), ConfigError> {
        if name != current && self.cameras.contains_key(name) {
            return Err(ConfigError::CameraAlreadyExists);
        }
        self.cameras
            .remove(current)
            .ok_or(ConfigError::CameraNotFound)?;
        self.cameras.insert(name.to_owned(), camera);
        Ok(())
    }

    /// Removes a camera and every link to it; printers that had it selected
    /// fall back to their built-in feed.
    pub fn remove_camera(&mut self, name: &str) -> Result<CameraProfile, ConfigError> {
        let camera = self
            .cameras
            .remove(name)
            .ok_or(ConfigError::CameraNotFound)?;
        for links in self.printer_cameras.values_mut() {
            links.cameras.retain(|id| *id != camera.id);
            if links.selected.as_deref() == Some(camera.id.as_str()) {
                links.selected = None;
            }
        }
        self.printer_cameras
            .retain(|_, links| !links.cameras.is_empty());
        Ok(camera)
    }

    pub fn link_camera(
        &mut self,
        printer: &str,
        camera: &str,
        select: bool,
    ) -> Result<(), ConfigError> {
        if !self.profiles.contains_key(printer) {
            return Err(ConfigError::ProfileNotFound);
        }
        let id = self
            .cameras
            .get(camera)
            .ok_or(ConfigError::CameraNotFound)?
            .id
            .clone();
        let links = self.printer_cameras.entry(printer.to_owned()).or_default();
        if !links.cameras.contains(&id) {
            links.cameras.push(id.clone());
        }
        if select {
            links.selected = Some(id);
        }
        Ok(())
    }

    pub fn unlink_camera(&mut self, printer: &str, camera: &str) -> Result<(), ConfigError> {
        let id = &self
            .cameras
            .get(camera)
            .ok_or(ConfigError::CameraNotFound)?
            .id;
        let links = self
            .printer_cameras
            .get_mut(printer)
            .filter(|links| links.cameras.contains(id))
            .ok_or(ConfigError::CameraNotLinked)?;
        links.cameras.retain(|linked| linked != id);
        if links.selected.as_ref() == Some(id) {
            links.selected = None;
        }
        if links.cameras.is_empty() {
            self.printer_cameras.remove(printer);
        }
        Ok(())
    }

    /// Selects a linked camera, or the built-in feed when `camera` is `None`.
    pub fn select_camera(&mut self, printer: &str, camera: Option<&str>) -> Result<(), ConfigError> {
        if !self.profiles.contains_key(printer) {
            return Err(ConfigError::ProfileNotFound);
        }
        let Some(camera) = camera else {
            if let Some(links) = self.printer_cameras.get_mut(printer) {
                links.selected = None;
            }
            return Ok(());
        };
        let id = self
            .cameras
            .get(camera)
            .ok_or(ConfigError::CameraNotFound)?
            .id
            .clone();
        let links = self
            .printer_cameras
            .get_mut(printer)
            .filter(|links| links.cameras.contains(&id))
            .ok_or(ConfigError::CameraNotLinked)?;
        links.selected = Some(id);
        Ok(())
    }

    pub fn printer_cameras(&self, printer: &str) -> Option<&PrinterCameras> {
        self.printer_cameras.get(printer)
    }

    /// Printers linked to camera `id`, and the subset that selected it.
    pub fn camera_printers(&self, id: &str) -> (Vec<String>, Vec<String>) {
        let mut linked = Vec::new();
        let mut selected = Vec::new();
        for (printer, links) in &self.printer_cameras {
            if links.cameras.iter().any(|camera| camera == id) {
                linked.push(printer.clone());
            }
            if links.selected.as_deref() == Some(id) {
                selected.push(printer.clone());
            }
        }
        (linked, selected)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        let mut ids = BTreeSet::new();
        for camera in self.cameras.values() {
            if !is_camera_id(&camera.id) || !ids.insert(camera.id.as_str()) {
                return Err(ConfigError::InvalidCameras(
                    "camera IDs must be unique 32-digit lowercase hex values",
                ));
            }
        }
        for (printer, links) in &self.printer_cameras {
            if !self.profiles.contains_key(printer) {
                return Err(ConfigError::InvalidCameras(
                    "a camera link names an unknown printer",
                ));
            }
            let linked: BTreeSet<_> = links.cameras.iter().map(String::as_str).collect();
            if linked.len() != links.cameras.len() || !linked.is_subset(&ids) {
                return Err(ConfigError::InvalidCameras(
                    "a camera link names an unknown or repeated camera",
                ));
            }
            if links
                .selected
                .as_deref()
                .is_some_and(|id| !linked.contains(id))
            {
                return Err(ConfigError::InvalidCameras(
                    "a selected camera is not linked to its printer",
                ));
            }
        }
        Ok(())
    }
```

In `save`, validate first and choose the version:

```rust
    pub fn save(&mut self, dir: impl AsRef<Path>) -> Result<(), ConfigError> {
        let dir = dir.as_ref();
        self.validate()?;
        self.check_unchanged(dir)?;
        fs::create_dir_all(dir)?;
        restrict_dir_permissions(dir)?;

        let has_cameras = !self.cameras.is_empty() || !self.printer_cameras.is_empty();
        let bytes = serde_yaml::to_string(&ConfigFile {
            version: if has_cameras { MAX_VERSION } else { 1 },
            profiles: self.profiles.clone(),
            cameras: self.cameras.clone(),
            printer_cameras: self.printer_cameras.clone(),
        })
        .map_err(ConfigError::Malformed)?
        .into_bytes();
```

(the rest of `save` is unchanged). Update `empty()`:

```rust
    fn empty() -> Self {
        Self {
            profiles: BTreeMap::new(),
            cameras: BTreeMap::new(),
            printer_cameras: BTreeMap::new(),
            digest: None,
        }
    }
```

Run: `cargo test -p melt-core --locked config::tests`
Expected: PASS (existing and 7 new tests).

- [ ] **Step 3: Write the failing printer-rename test**

Append inside `mod tests` in `crates/melt-core/src/profiles.rs`:

```rust
    #[test]
    fn renaming_a_printer_keeps_its_camera_links() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::open(dir.path()).unwrap();
        config
            .add_profile(
                "garage",
                Profile {
                    driver: "moonraker".into(),
                    host: "old-printer.local".into(),
                    serial: String::new(),
                    model: String::new(),
                    timeout: "10s".into(),
                    insecure: false,
                    created: "2026-08-01T12:00:00Z".into(),
                    updated: "2026-08-01T12:00:00Z".into(),
                },
            )
            .unwrap();
        config
            .add_camera(
                "porch",
                crate::config::CameraProfile {
                    id: "0123456789abcdef0123456789abcdef".into(),
                    driver: "tapo".into(),
                    host: "192.0.2.20".into(),
                    username: "melt".into(),
                    rtsp_port: None,
                    timeout: "10s".into(),
                    created: "2026-10-08T12:00:00Z".into(),
                    updated: "2026-10-08T12:00:00Z".into(),
                },
            )
            .unwrap();
        config.link_camera("garage", "porch", true).unwrap();
        config.save(dir.path()).unwrap();
        let store = MemoryStore::default();
        let (host, server) = moonraker_server(None);

        update(
            dir.path(),
            &store,
            "garage",
            CreateRequest {
                name: "workshop".into(),
                driver: "moonraker".into(),
                host,
                serial: String::new(),
                model: String::new(),
                timeout: "10s".into(),
                insecure: false,
                access_code: String::new(),
            },
        )
        .unwrap();
        server.join().unwrap();

        let saved = Config::open(dir.path()).unwrap();
        assert!(saved.printer_cameras("garage").is_none());
        assert_eq!(
            saved.printer_cameras("workshop").unwrap().selected.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }
```

Run: `cargo test -p melt-core --locked profiles::tests::renaming_a_printer_keeps_its_camera_links`
Expected: FAIL at `saved.printer_cameras("workshop").unwrap()`, because `update` renames via `remove_profile` + `add_profile`, and `remove_profile` now drops links.

- [ ] **Step 4: Route printer renames through `rename_profile`**

In `profiles::update`, replace:

```rust
    let config_change = if name == current_name {
        config.set_profile(&name, profile.clone())
    } else {
        config
            .remove_profile(&current_name)
            .and_then(|_| config.add_profile(&name, profile.clone()))
    };
```

with:

```rust
    let config_change = if name == current_name {
        config.set_profile(&name, profile.clone())
    } else {
        config.rename_profile(&current_name, &name, profile.clone())
    };
```

Run: `cargo test --workspace --locked`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/melt-core/src/config.rs crates/melt-core/src/profiles.rs
git commit -m "feat(config): store camera profiles and printer links

Configs gain cameras and printer_cameras sections; files without camera
data are still written as version 1. Printer rename carries links, and
camera removal falls back to the built-in feed.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Tapo video and snapshots

**Files:**
- Create: `crates/melt-core/src/cameras.rs`, `crates/melt-core/src/cameras/tapo.rs`
- Create: `crates/melt-core/tests/tapo_simulated.rs`
- Modify: `crates/melt-core/src/lib.rs` (add `pub mod cameras;`), `crates/melt-core/src/bambu.rs` (drop `#[allow(dead_code)]` on `from_h264`)

**Interfaces:**
- Consumes: `rtsp::{open_h264, Credentials, RtspError}`, `MjpegStream::from_h264`, `config::{CameraProfile, ConfigError}`
- Produces in `melt_core::cameras`:
  - `pub const TAPO: &str = "tapo"; pub const DRIVERS: &[&str]; pub const TAPO_RTSP_PORT: u16 = 554;`
  - `pub enum StreamQuality { High, Low }` (implements `FromStr`, serialized lowercase)
  - `pub struct ResolvedCamera { pub name: String, pub profile: CameraProfile, pub password: String, pub timeout: Duration }`
  - `pub enum CameraError` (full variant list below)
  - `pub fn open_stream(&ResolvedCamera, StreamQuality, decode: bool) -> Result<H264Stream, CameraError>`
  - `pub fn snapshot(&ResolvedCamera, StreamQuality) -> Result<Vec<u8>, CameraError>`
  - `pub fn mjpeg_stream(&ResolvedCamera, StreamQuality) -> Result<MjpegStream, CameraError>`
  - `pub fn verify(&ResolvedCamera) -> Result<(), CameraError>`

- [ ] **Step 1: Write the simulated-camera tests (red)**

Create `crates/melt-core/tests/tapo_simulated.rs`:

```rust
//! A loopback RTSP server standing in for a Tapo camera: Digest with
//! qop=auth, an audio track before the video track, in-band SPS/PPS, and a
//! non-default interleaved channel.

use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use melt_core::{
    cameras::{self, CameraError, ResolvedCamera, StreamQuality},
    config::CameraProfile,
};
use openh264::{encoder::Encoder, formats::YUVBuffer};

const REALM: &str = "TP-Link IP-Camera";
const NONCE: &str = "4f2c";
const DIGEST: &str = "Digest realm=\"TP-Link IP-Camera\", nonce=\"4f2c\", qop=\"auth\"";
const SDP: &str = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=Session\r\nt=0 0\r\na=control:*\r\nm=audio 0 RTP/AVP 8\r\na=rtpmap:8 PCMA/8000\r\na=control:track2\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=fmtp:96 packetization-mode=1\r\na=control:track1\r\n";

fn md5_hex(input: &str) -> String {
    openssl::hash::hash(openssl::hash::MessageDigest::md5(), input.as_bytes())
        .unwrap()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn camera(port: u16, password: &str, timeout: Duration) -> ResolvedCamera {
    ResolvedCamera {
        name: "porch".into(),
        profile: CameraProfile {
            id: "0123456789abcdef0123456789abcdef".into(),
            driver: "tapo".into(),
            host: "127.0.0.1".into(),
            username: "melt".into(),
            rtsp_port: Some(port),
            timeout: "5s".into(),
            created: String::new(),
            updated: String::new(),
        },
        password: password.into(),
        timeout,
    }
}

/// Serves one RTSP connection and returns every request it received.
fn spawn_camera(challenge: &'static str) -> (u16, JoinHandle<Vec<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serve(socket, challenge)
    });
    (port, handle)
}

fn serve(socket: TcpStream, challenge: &str) -> Vec<Vec<String>> {
    let mut writer = socket.try_clone().unwrap();
    let mut reader = BufReader::new(socket);
    let mut seen = Vec::new();
    while let Some(request) = read_request(&mut reader) {
        seen.push(request.clone());
        let mut start = request[0].split(' ');
        let method = start.next().unwrap().to_owned();
        let uri = start.next().unwrap().to_owned();
        let cseq = header(&request, "CSeq").unwrap().to_owned();
        if method == "TEARDOWN" {
            break;
        }
        if !authorized(&request, &method, &uri) {
            write!(
                writer,
                "RTSP/1.0 401 Unauthorized\r\nCSeq: {cseq}\r\nWWW-Authenticate: {challenge}\r\n\r\n"
            )
            .unwrap();
            continue;
        }
        match method.as_str() {
            "DESCRIBE" => write!(
                writer,
                "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Base: {uri}/\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{SDP}",
                SDP.len()
            )
            .unwrap(),
            "SETUP" => write!(
                writer,
                "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 7A3F;timeout=60\r\nTransport: RTP/AVP/TCP;unicast;interleaved=2-3\r\n\r\n"
            )
            .unwrap(),
            "PLAY" => {
                write!(writer, "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: 7A3F\r\n\r\n").unwrap();
                send_picture(&mut writer);
            }
            _ => write!(writer, "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\n\r\n").unwrap(),
        }
    }
    seen
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Vec<String>> {
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end().to_owned();
        if line.is_empty() {
            return Some(lines);
        }
        lines.push(line);
    }
}

fn header<'a>(request: &'a [String], name: &str) -> Option<&'a str> {
    request.iter().find_map(|line| {
        line.split_once(": ")
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value)
    })
}

fn authorized(request: &[String], method: &str, uri: &str) -> bool {
    let Some(value) = header(request, "Authorization") else {
        return false;
    };
    let field = |key: &str| {
        value.split(", ").find_map(|part| {
            part.trim_start_matches("Digest ")
                .strip_prefix(&format!("{key}="))
                .map(|found| found.trim_matches('"').to_owned())
        })
    };
    let (Some(user), Some(digest_uri), Some(nc), Some(cnonce), Some(response)) = (
        field("username"),
        field("uri"),
        field("nc"),
        field("cnonce"),
        field("response"),
    ) else {
        return false;
    };
    let ha1 = md5_hex(&format!("{user}:{REALM}:secret"));
    let ha2 = md5_hex(&format!("{method}:{digest_uri}"));
    user == "melt"
        && digest_uri == uri
        && response == md5_hex(&format!("{ha1}:{NONCE}:{nc}:{cnonce}:auth:{ha2}"))
}

/// Sends audio noise on channel 0, then one encoded IDR picture twice on
/// channel 2, so a decoder that buffers one picture still emits a frame.
fn send_picture(writer: &mut TcpStream) {
    interleaved(writer, 0, &[0xde, 0xad]);
    let mut encoder = Encoder::new().unwrap();
    let picture = YUVBuffer::from_vec(vec![128; 64 * 64 * 3 / 2], 64, 64);
    let nals = split_annex_b(&encoder.encode(&picture).unwrap().to_vec());
    for repeat in 0..2u8 {
        for (index, nal) in nals.iter().enumerate() {
            let marker = if index == nals.len() - 1 { 0x80 } else { 0 };
            let mut packet = vec![0x80, marker | 96, repeat, index as u8, 0, 0, 0, 0, 0, 0, 0, 1];
            packet.extend_from_slice(nal);
            interleaved(writer, 2, &packet);
        }
    }
}

fn interleaved(writer: &mut TcpStream, channel: u8, payload: &[u8]) {
    let mut frame = vec![b'$', channel];
    frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    frame.extend_from_slice(payload);
    writer.write_all(&frame).unwrap();
}

fn split_annex_b(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut nals = Vec::new();
    let mut start = None;
    let mut index = 0;
    while index + 3 <= bytes.len() {
        if bytes[index..index + 3] == [0, 0, 1] {
            if let Some(begin) = start {
                nals.push(trim_trailing_zeros(&bytes[begin..index]));
            }
            index += 3;
            start = Some(index);
        } else {
            index += 1;
        }
    }
    if let Some(begin) = start {
        nals.push(bytes[begin..].to_vec());
    }
    nals.retain(|nal| !nal.is_empty());
    nals
}

fn trim_trailing_zeros(nal: &[u8]) -> Vec<u8> {
    let end = nal.iter().rposition(|byte| *byte != 0).map_or(0, |last| last + 1);
    nal[..end].to_vec()
}

#[test]
fn snapshot_decodes_a_fresh_frame_over_digest_rtsp() {
    let (port, server) = spawn_camera(DIGEST);
    let jpeg = cameras::snapshot(
        &camera(port, "secret", Duration::from_secs(5)),
        StreamQuality::High,
    )
    .unwrap();
    let seen = server.join().unwrap();

    assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
    let lines: Vec<&str> = seen.iter().map(|request| request[0].as_str()).collect();
    assert!(lines.contains(&format!("DESCRIBE rtsp://127.0.0.1:{port}/stream1 RTSP/1.0").as_str()));
    assert!(lines.contains(&format!("SETUP rtsp://127.0.0.1:{port}/stream1/track1 RTSP/1.0").as_str()));
}

#[test]
fn low_quality_uses_the_second_stream() {
    let (port, server) = spawn_camera(DIGEST);
    cameras::snapshot(&camera(port, "secret", Duration::from_secs(5)), StreamQuality::Low)
        .unwrap();
    let seen = server.join().unwrap();
    assert!(seen[0][0].starts_with(&format!("DESCRIBE rtsp://127.0.0.1:{port}/stream2 ")));
}

#[test]
fn wrong_password_is_an_authentication_failure() {
    let (port, server) = spawn_camera(DIGEST);
    let result = cameras::snapshot(
        &camera(port, "wrong", Duration::from_secs(5)),
        StreamQuality::High,
    );
    drop(server);
    assert!(matches!(result, Err(CameraError::Authentication)));
}

#[test]
fn basic_only_camera_is_refused_without_sending_credentials() {
    let (port, server) = spawn_camera("Basic realm=\"TP-Link IP-Camera\"");
    let result = cameras::snapshot(
        &camera(port, "secret", Duration::from_secs(5)),
        StreamQuality::High,
    );
    let seen = server.join().unwrap();

    assert!(matches!(result, Err(CameraError::DigestRequired)));
    assert_eq!(seen.len(), 1);
    assert!(header(&seen[0], "Authorization").is_none());
}

#[test]
fn silent_camera_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let holder = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_secs(2));
        drop(socket);
    });
    let started = Instant::now();
    let result = cameras::snapshot(
        &camera(port, "secret", Duration::from_millis(300)),
        StreamQuality::High,
    );
    assert!(matches!(result, Err(CameraError::Timeout)), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
    holder.join().unwrap();
}
```

Run: `cargo test -p melt-core --locked --test tapo_simulated`
Expected: compile error (`melt_core::cameras` not found).

- [ ] **Step 2: Add the camera domain**

In `crates/melt-core/src/lib.rs`, add `pub mod cameras;` after `pub mod bambu;`.

Create `crates/melt-core/src/cameras.rs`:

```rust
//! External IP cameras: profiles, printer links, and media. Drivers form a
//! closed set, like printer drivers; `tapo` is the only one today.

mod tapo;

use std::{
    io,
    str::FromStr,
    time::{Duration, Instant},
};

use serde::Serialize;
use thiserror::Error;

pub use crate::{bambu::MjpegStream, rtsp::H264Stream};
use crate::{
    config::{CameraProfile, ConfigError},
    keychain::SecretError,
    rtsp::RtspError,
};

pub const TAPO: &str = "tapo";
pub const DRIVERS: &[&str] = &[TAPO];
pub const TAPO_RTSP_PORT: u16 = 554;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamQuality {
    #[default]
    High,
    Low,
}

impl FromStr for StreamQuality {
    type Err = CameraError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "high" => Ok(Self::High),
            "low" => Ok(Self::Low),
            _ => Err(CameraError::InvalidQuality),
        }
    }
}

/// A camera profile with its password, ready to connect. Deliberately not
/// `Debug`: it holds a secret.
pub struct ResolvedCamera {
    pub name: String,
    pub profile: CameraProfile,
    pub password: String,
    pub timeout: Duration,
}

#[derive(Debug, Error)]
pub enum CameraError {
    #[error("camera name is required")]
    MissingName,
    #[error("invalid camera name")]
    InvalidName,
    #[error("unsupported camera driver {0:?}")]
    UnknownDriver(String),
    #[error("invalid camera host")]
    InvalidHost,
    #[error("camera username must be 1-64 characters without quotes, backslashes, or control characters")]
    InvalidUsername,
    #[error("camera password must not be empty or contain control characters")]
    InvalidPassword,
    #[error("RTSP port must be between 1 and 65535")]
    InvalidPort,
    #[error("invalid camera timeout")]
    InvalidTimeout,
    #[error("stream quality must be \"high\" or \"low\"")]
    InvalidQuality,
    #[error("camera {0:?} not found")]
    NotFound(String),
    #[error("camera {0:?} already exists")]
    AlreadyExists(String),
    #[error("printer profile {0:?} not found")]
    PrinterNotFound(String),
    #[error("camera {camera:?} is not linked to printer {printer:?}")]
    NotLinked { printer: String, camera: String },
    #[error("camera authentication failed")]
    Authentication,
    #[error("camera offered only Basic authentication; Melt requires Digest over plain RTSP")]
    DigestRequired,
    #[error("camera connection failed")]
    Connect(#[source] io::Error),
    #[error("camera connection timed out")]
    Timeout,
    #[error("camera stream failed: {0}")]
    Stream(String),
    #[error("camera video is not H.264")]
    UnsupportedCodec,
    #[error("stored camera password is unavailable")]
    MissingSecret,
    #[error("keychain operation failed")]
    Secret(#[source] SecretError),
    #[error("cannot save config")]
    Config(#[source] ConfigError),
    #[error("cannot save config and could not restore the stored camera password")]
    RollbackFailed,
}

/// Opens the live H.264 stream. `decode` enables JPEG previews.
pub fn open_stream(
    camera: &ResolvedCamera,
    quality: StreamQuality,
    decode: bool,
) -> Result<H264Stream, CameraError> {
    match camera.profile.driver.as_str() {
        TAPO => tapo::open(camera, quality, decode),
        other => Err(CameraError::UnknownDriver(other.to_owned())),
    }
}

/// Decodes one frame from a new connection, within the camera timeout.
pub fn snapshot(camera: &ResolvedCamera, quality: StreamQuality) -> Result<Vec<u8>, CameraError> {
    let deadline = Instant::now() + camera.timeout;
    let mut stream = open_stream(camera, quality, true)?;
    loop {
        if Instant::now() >= deadline {
            return Err(CameraError::Timeout);
        }
        let unit = stream.next_access_unit().map_err(stream_io_error)?;
        if let Some(jpeg) = unit.jpeg {
            return Ok(jpeg);
        }
    }
}

pub fn mjpeg_stream(
    camera: &ResolvedCamera,
    quality: StreamQuality,
) -> Result<MjpegStream, CameraError> {
    open_stream(camera, quality, true).map(MjpegStream::from_h264)
}

/// Read-only setup check: authenticate and decode one low-quality frame.
/// Never moves the camera.
pub fn verify(camera: &ResolvedCamera) -> Result<(), CameraError> {
    snapshot(camera, StreamQuality::Low).map(|_| ())
}

fn stream_io_error(error: io::Error) -> CameraError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => CameraError::Timeout,
        io::ErrorKind::InvalidData => CameraError::Stream("camera sent an undecodable frame".into()),
        _ => CameraError::Stream(error.to_string()),
    }
}

fn rtsp_error(error: RtspError) -> CameraError {
    match error {
        RtspError::Authentication => CameraError::Authentication,
        RtspError::DigestRequired => CameraError::DigestRequired,
        RtspError::UnsupportedCodec => CameraError::UnsupportedCodec,
        RtspError::Io(error) => stream_io_error(error),
        RtspError::Protocol(message) => CameraError::Stream(message),
    }
}
```

Create `crates/melt-core/src/cameras/tapo.rs`:

```rust
//! Tapo video: RTSP on the camera's local Camera Account, `/stream1`
//! (high) and `/stream2` (low), per TP-Link's integration guide
//! <https://www.tp-link.com/us/support/faq/2680/>. Plain TCP on the LAN.

use std::{
    io,
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

use super::{CameraError, ResolvedCamera, StreamQuality, TAPO_RTSP_PORT, rtsp_error};
use crate::rtsp::{self, Credentials, H264Stream};

pub(super) fn open(
    camera: &ResolvedCamera,
    quality: StreamQuality,
    decode: bool,
) -> Result<H264Stream, CameraError> {
    let host = &camera.profile.host;
    let port = camera.profile.rtsp_port.unwrap_or(TAPO_RTSP_PORT);
    let socket = connect(host, port, camera.timeout)?;
    socket
        .set_read_timeout(Some(camera.timeout))
        .and_then(|()| socket.set_write_timeout(Some(camera.timeout)))
        .map_err(CameraError::Connect)?;
    let shutdown = socket.try_clone().map_err(CameraError::Connect)?;
    let path = match quality {
        StreamQuality::High => "/stream1",
        StreamQuality::Low => "/stream2",
    };
    let authority = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    let uri = rtsp_types::Url::parse(&format!("rtsp://{authority}:{port}{path}"))
        .map_err(|_| CameraError::InvalidHost)?;
    rtsp::open_h264(
        Box::new(socket),
        shutdown,
        uri,
        Credentials {
            username: camera.profile.username.clone(),
            password: camera.password.clone(),
            realm: None,
        },
        decode,
    )
    .map_err(rtsp_error)
}

/// Connects to the first reachable resolved address within one deadline.
fn connect(host: &str, port: u16, timeout: Duration) -> Result<TcpStream, CameraError> {
    let deadline = Instant::now() + timeout;
    let addresses = (host, port)
        .to_socket_addrs()
        .map_err(CameraError::Connect)?;
    let mut last = io::Error::from(io::ErrorKind::AddrNotAvailable);
    for address in addresses {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(CameraError::Timeout)?;
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(socket) => return Ok(socket),
            Err(error) => last = error,
        }
    }
    Err(match last.kind() {
        io::ErrorKind::TimedOut => CameraError::Timeout,
        _ => CameraError::Connect(last),
    })
}
```

Remove `#[allow(dead_code)]` from `MjpegStream::from_h264` in `bambu.rs`.

- [ ] **Step 3: Run the tests**

Run: `cargo test -p melt-core --locked --test tapo_simulated`
Expected: PASS (5 tests).

If `snapshot_decodes_a_fresh_frame_over_digest_rtsp` fails with `Timeout`, `openh264` emitted no picture from the repeated IDR. Print `nals.len()` and the NAL types (`nal[0] & 0x1f`) in `send_picture`: you should see 7 (SPS), 8 (PPS) and 5 (IDR). Fix the splitter, not the client.

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: no warnings.

- [ ] **Step 4: Commit**

```bash
git add crates/melt-core/src/cameras.rs crates/melt-core/src/cameras/tapo.rs crates/melt-core/src/lib.rs crates/melt-core/src/bambu.rs crates/melt-core/tests/tapo_simulated.rs
git commit -m "feat(camera): stream and snapshot Tapo C510W cameras over RTSP

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Camera profile workflows

**Files:**
- Create: `crates/melt-core/src/cameras/profiles.rs`
- Modify: `crates/melt-core/src/cameras.rs` (`mod profiles;` + re-exports)
- Modify: `crates/melt-core/src/profiles.rs` (make `normalize_name`, `StoredSecret`, `restore` crate-visible; move the test `MemoryStore`)
- Modify: `crates/melt-core/src/keychain.rs` (test-only `MemoryStore`)

**Interfaces:**
- Consumes: Task 2 `Config` camera methods, Task 3 `ResolvedCamera`, `CameraError`, `verify`
- Produces in `melt_core::cameras`:
  - `pub struct CameraRequest { pub name, pub driver, pub host, pub username, pub password: String, pub rtsp_port: Option<u16>, pub timeout: String }` (no `Debug`)
  - `pub struct CameraSummary { name, id, driver, host, username, rtsp_port: u16, timeout, printers: Vec<String>, selected_by: Vec<String> }` (`Serialize`, camelCase)
  - `pub struct CameraRemoveResult { name, id, password_removed: bool, unlinked_printers: Vec<String>, warnings: Vec<RemoveWarning> }`
  - `pub struct SelectedFeed { printer: String, camera: Option<String> }`
  - `pub fn create(dir, store, CameraRequest, verify: impl FnOnce(&ResolvedCamera) -> Result<(), CameraError>) -> Result<CameraSummary, CameraError>`
  - `pub fn update(dir, store, current: &str, CameraRequest, verify) -> Result<CameraSummary, CameraError>`. An empty `password` keeps the stored one, and the camera is contacted only when the host, username, port, driver or password changes.
  - `pub fn remove(dir, store, name) -> Result<CameraRemoveResult, CameraError>`
  - `pub fn list(dir) -> Result<Vec<CameraSummary>, CameraError>`
  - `pub fn profile(dir, name) -> Result<CameraProfile, CameraError>`
  - `pub fn resolve(dir, store, name) -> Result<ResolvedCamera, CameraError>`
  - `pub fn link(dir, printer, camera, select: bool) -> Result<CameraSummary, CameraError>`
  - `pub fn unlink(dir, printer, camera) -> Result<CameraSummary, CameraError>`
  - `pub fn select(dir, printer, camera: Option<&str>) -> Result<SelectedFeed, CameraError>`

- [ ] **Step 1: Share the test secret store**

In `crates/melt-core/src/keychain.rs`, append:

```rust
#[cfg(test)]
pub(crate) mod test_support {
    use std::{collections::BTreeMap, sync::Mutex};

    use super::{SecretError, SecretStore};

    #[derive(Default)]
    pub(crate) struct MemoryStore(pub(crate) Mutex<BTreeMap<String, String>>);

    impl SecretStore for MemoryStore {
        fn get(&self, _: &str, account: &str) -> Result<String, SecretError> {
            self.0
                .lock()
                .unwrap()
                .get(account)
                .cloned()
                .ok_or(SecretError::NotFound)
        }
        fn set(&self, _: &str, account: &str, secret: &str) -> Result<(), SecretError> {
            self.0.lock().unwrap().insert(account.into(), secret.into());
            Ok(())
        }
        fn delete(&self, _: &str, account: &str) -> Result<(), SecretError> {
            self.0
                .lock()
                .unwrap()
                .remove(account)
                .map(|_| ())
                .ok_or(SecretError::NotFound)
        }
    }
}
```

In `crates/melt-core/src/profiles.rs` tests, delete the local `MemoryStore` struct and its `impl`, and add `use crate::keychain::test_support::MemoryStore;`. Drop `BTreeMap` and `Mutex` from that module's `use std::{…}` if nothing else there uses them.

In `crates/melt-core/src/profiles.rs`, make these crate-visible:
- `fn normalize_name` → `pub(crate) fn normalize_name`
- `struct StoredSecret` → `pub(crate) struct StoredSecret`, its fields → `pub(crate)`, and `fn replace`, `fn load` → `pub(crate) fn`
- `fn restore(store: &dyn SecretStore, secrets: &mut [StoredSecret])` → `pub(crate) fn restore`

Run: `cargo test -p melt-core --locked profiles::tests`
Expected: PASS (pure move).

- [ ] **Step 2: Write the failing workflow tests**

Create `crates/melt-core/src/cameras/profiles.rs` with only a test module, and add `mod profiles;` under `mod tapo;` in `cameras.rs`:

```rust
#[cfg(test)]
mod tests {
    use std::{cell::Cell, fs};

    use super::*;
    use crate::{config::Profile, keychain::test_support::MemoryStore};

    fn request(name: &str, password: &str) -> CameraRequest {
        CameraRequest {
            name: name.into(),
            driver: "tapo".into(),
            host: "192.0.2.20".into(),
            username: "melt".into(),
            password: password.into(),
            rtsp_port: None,
            timeout: "10s".into(),
        }
    }

    fn accept(_: &ResolvedCamera) -> Result<(), CameraError> {
        Ok(())
    }

    fn add_printer(dir: &Path, name: &str) {
        let mut config = Config::open(dir).unwrap();
        config
            .add_profile(
                name,
                Profile {
                    driver: "moonraker".into(),
                    host: "printer.local".into(),
                    serial: String::new(),
                    model: String::new(),
                    timeout: "10s".into(),
                    insecure: false,
                    created: "2026-10-08T12:00:00Z".into(),
                    updated: "2026-10-08T12:00:00Z".into(),
                },
            )
            .unwrap();
        config.save(dir).unwrap();
    }

    #[test]
    fn create_verifies_with_the_password_and_keeps_it_out_of_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let seen = Cell::new(false);

        let summary = create(dir.path(), &store, request("Porch", "s3cret"), |camera| {
            seen.set(camera.password == "s3cret" && camera.name == "porch");
            Ok(())
        })
        .unwrap();

        assert!(seen.get());
        assert_eq!(summary.name, "porch");
        assert_eq!(summary.rtsp_port, 554);
        assert!(crate::config::is_camera_id(&summary.id));
        let yaml = fs::read_to_string(dir.path().join("melt.yaml")).unwrap();
        assert!(!yaml.contains("s3cret"));
        assert_eq!(
            store
                .get(SERVICE, &format!("camera:{}:password", summary.id))
                .unwrap(),
            "s3cret"
        );
    }

    #[test]
    fn create_rejects_invalid_input_before_contacting_the_camera() {
        let cases: [(&str, fn(&mut CameraRequest)); 8] = [
            ("name", |request| request.name = "-porch".into()),
            ("driver", |request| request.driver = "onvif".into()),
            ("scheme host", |request| request.host = "rtsp://192.0.2.20".into()),
            ("host with port", |request| request.host = "camera.local:554".into()),
            ("quoted username", |request| request.username = "me\"lt".into()),
            ("empty password", |request| request.password = String::new()),
            ("port zero", |request| request.rtsp_port = Some(0)),
            ("timeout", |request| request.timeout = "soon".into()),
        ];
        for (name, edit) in cases {
            let dir = tempfile::tempdir().unwrap();
            let mut invalid = request("porch", "s3cret");
            edit(&mut invalid);
            let called = Cell::new(false);
            let result = create(dir.path(), &MemoryStore::default(), invalid, |_| {
                called.set(true);
                Ok(())
            });
            assert!(result.is_err(), "{name}");
            assert!(!called.get(), "{name}");
        }
    }

    #[test]
    fn failed_verification_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();

        let result = create(dir.path(), &store, request("porch", "wrong"), |_| {
            Err(CameraError::Authentication)
        });

        assert!(matches!(result, Err(CameraError::Authentication)));
        assert!(Config::open(dir.path()).unwrap().cameras().is_empty());
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[test]
    fn concurrent_config_change_rolls_back_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();

        let result = create(dir.path(), &store, request("porch", "s3cret"), |_| {
            fs::write(dir.path().join("melt.yaml"), "version: 1\nprofiles: {}\n").unwrap();
            Ok(())
        });

        assert!(matches!(
            result,
            Err(CameraError::Config(ConfigError::ConcurrentUpdate))
        ));
        assert!(store.0.lock().unwrap().is_empty());
    }

    #[test]
    fn rename_keeps_password_and_skips_verification() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        let created = create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();

        let renamed = update(dir.path(), &store, "porch", request("front", ""), |_| {
            panic!("an offline rename must not contact the camera")
        })
        .unwrap();

        assert_eq!(renamed.id, created.id);
        assert!(profile(dir.path(), "porch").is_err());
        assert_eq!(resolve(dir.path(), &store, "front").unwrap().password, "s3cret");
    }

    #[test]
    fn new_password_is_verified_before_it_replaces_the_old_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        create(dir.path(), &store, request("porch", "old"), accept).unwrap();

        let result = update(dir.path(), &store, "porch", request("porch", "new"), |camera| {
            assert_eq!(camera.password, "new");
            Err(CameraError::Authentication)
        });

        assert!(matches!(result, Err(CameraError::Authentication)));
        assert_eq!(resolve(dir.path(), &store, "porch").unwrap().password, "old");
    }

    #[test]
    fn remove_unlinks_printers_and_deletes_the_password() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        add_printer(dir.path(), "garage");
        let created = create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();
        link(dir.path(), "garage", "porch", true).unwrap();

        let removed = remove(dir.path(), &store, "porch").unwrap();

        assert!(removed.password_removed);
        assert_eq!(removed.unlinked_printers, ["garage"]);
        assert!(store.0.lock().unwrap().is_empty());
        let config = Config::open(dir.path()).unwrap();
        assert!(config.printer_cameras("garage").is_none());
        assert!(config.camera_printers(&created.id).0.is_empty());
    }

    #[test]
    fn links_and_selection_report_names_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::default();
        add_printer(dir.path(), "garage");
        create(dir.path(), &store, request("porch", "s3cret"), accept).unwrap();

        let linked = link(dir.path(), "Garage", "porch", false).unwrap();
        assert_eq!(linked.printers, ["garage"]);
        assert!(linked.selected_by.is_empty());

        let selected = select(dir.path(), "garage", Some("porch")).unwrap();
        assert_eq!(selected.camera.as_deref(), Some("porch"));
        assert_eq!(list(dir.path()).unwrap()[0].selected_by, ["garage"]);

        assert!(select(dir.path(), "garage", None).unwrap().camera.is_none());
        assert!(matches!(
            link(dir.path(), "attic", "porch", false),
            Err(CameraError::PrinterNotFound(_))
        ));
        unlink(dir.path(), "garage", "porch").unwrap();
        assert!(matches!(
            unlink(dir.path(), "garage", "porch"),
            Err(CameraError::NotLinked { .. })
        ));
    }
}
```

Run: `cargo test -p melt-core --locked cameras::profiles`
Expected: compile errors (`create`, `CameraRequest`, … not found).

- [ ] **Step 3: Implement the workflows**

Prepend to `crates/melt-core/src/cameras/profiles.rs`, above the test module:

```rust
//! Camera profile workflows: validated, verified, keychain-backed CRUD and
//! printer links. Mirrors the printer workflow's secret rollback.

use std::{net::IpAddr, path::Path};

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{CameraError, DRIVERS, ResolvedCamera, TAPO_RTSP_PORT};
use crate::{
    config::{CameraProfile, Config, ConfigError},
    drivers,
    keychain::{SERVICE, SecretError, SecretStore, account},
    profiles::{self, ProfileError, RemoveWarning, StoredSecret},
};

/// Add or edit input. Deliberately not `Debug`: it holds a password. An
/// empty `password` on update keeps the stored one.
pub struct CameraRequest {
    pub name: String,
    pub driver: String,
    pub host: String,
    pub username: String,
    pub password: String,
    pub rtsp_port: Option<u16>,
    pub timeout: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraSummary {
    pub name: String,
    pub id: String,
    pub driver: String,
    pub host: String,
    pub username: String,
    pub rtsp_port: u16,
    pub timeout: String,
    pub printers: Vec<String>,
    pub selected_by: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraRemoveResult {
    pub name: String,
    pub id: String,
    pub password_removed: bool,
    pub unlinked_printers: Vec<String>,
    pub warnings: Vec<RemoveWarning>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedFeed {
    pub printer: String,
    /// `None` is the printer's built-in feed.
    pub camera: Option<String>,
}

pub fn create(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    request: CameraRequest,
    verify: impl FnOnce(&ResolvedCamera) -> Result<(), CameraError>,
) -> Result<CameraSummary, CameraError> {
    let name = camera_name(&request.name)?;
    let timeout = validate(&request)?;
    if request.password.is_empty() {
        return Err(CameraError::InvalidPassword);
    }
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    if config.get_camera(&name).is_some() {
        return Err(CameraError::AlreadyExists(name));
    }
    let now = now();
    let camera = ResolvedCamera {
        name: name.clone(),
        profile: CameraProfile {
            id: new_id(),
            driver: request.driver,
            host: request.host,
            username: request.username,
            rtsp_port: request.rtsp_port,
            timeout: request.timeout,
            created: now.clone(),
            updated: now,
        },
        password: request.password,
        timeout,
    };
    verify(&camera)?;

    let mut secrets = vec![
        StoredSecret::replace(store, password_account(&camera.profile.id), &camera.password)
            .map_err(secret_error)?,
    ];
    if let Err(error) = config
        .add_camera(&name, camera.profile.clone())
        .and_then(|()| config.save(dir))
    {
        profiles::restore(store, &mut secrets).map_err(|_| CameraError::RollbackFailed)?;
        return Err(CameraError::Config(error));
    }
    Ok(summary(&config, &name, &camera.profile))
}

pub fn update(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    current: &str,
    request: CameraRequest,
    verify: impl FnOnce(&ResolvedCamera) -> Result<(), CameraError>,
) -> Result<CameraSummary, CameraError> {
    let current = camera_name(current)?;
    let name = camera_name(&request.name)?;
    let timeout = validate(&request)?;
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    let previous = config
        .get_camera(&current)
        .cloned()
        .ok_or_else(|| CameraError::NotFound(current.clone()))?;
    if name != current && config.get_camera(&name).is_some() {
        return Err(CameraError::AlreadyExists(name));
    }

    let password_changed = !request.password.is_empty();
    let account = password_account(&previous.id);
    let password = if password_changed {
        request.password
    } else {
        StoredSecret::load(store, account.clone())
            .map_err(secret_error)?
            .value
            .ok_or(CameraError::MissingSecret)?
    };
    let camera = ResolvedCamera {
        name: name.clone(),
        profile: CameraProfile {
            id: previous.id.clone(),
            driver: request.driver,
            host: request.host,
            username: request.username,
            rtsp_port: request.rtsp_port,
            timeout: request.timeout,
            created: previous.created.clone(),
            updated: now(),
        },
        password,
        timeout,
    };
    let reaches_camera_differently = password_changed
        || camera.profile.driver != previous.driver
        || camera.profile.host != previous.host
        || camera.profile.username != previous.username
        || camera.profile.rtsp_port != previous.rtsp_port;
    if reaches_camera_differently {
        verify(&camera)?;
    }

    let mut secrets = Vec::new();
    if password_changed {
        secrets.push(
            StoredSecret::replace(store, account, &camera.password).map_err(secret_error)?,
        );
    }
    if let Err(error) = config
        .replace_camera(&current, &name, camera.profile.clone())
        .and_then(|()| config.save(dir))
    {
        profiles::restore(store, &mut secrets).map_err(|_| CameraError::RollbackFailed)?;
        return Err(CameraError::Config(error));
    }
    Ok(summary(&config, &name, &camera.profile))
}

/// Removes the profile and its links first, then the keychain password; a
/// failed password delete is reported as a warning, not a failure.
pub fn remove(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
) -> Result<CameraRemoveResult, CameraError> {
    let name = camera_name(name)?;
    let dir = dir.as_ref();
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    let camera = config
        .get_camera(&name)
        .cloned()
        .ok_or_else(|| CameraError::NotFound(name.clone()))?;
    let (unlinked_printers, _) = config.camera_printers(&camera.id);
    config.remove_camera(&name).map_err(CameraError::Config)?;
    config.save(dir).map_err(CameraError::Config)?;

    let mut warnings = Vec::new();
    let password_removed = match store.delete(SERVICE, &password_account(&camera.id)) {
        Ok(()) => true,
        Err(SecretError::NotFound) => false,
        Err(_) => {
            warnings.push(RemoveWarning {
                code: "password_delete_failed",
                message: "camera was removed, but its stored password could not be deleted from keychain",
            });
            false
        }
    };
    Ok(CameraRemoveResult {
        name,
        id: camera.id,
        password_removed,
        unlinked_printers,
        warnings,
    })
}

pub fn list(dir: impl AsRef<Path>) -> Result<Vec<CameraSummary>, CameraError> {
    let config = Config::open(dir).map_err(CameraError::Config)?;
    Ok(config
        .cameras()
        .iter()
        .map(|(name, camera)| summary(&config, name, camera))
        .collect())
}

pub fn profile(dir: impl AsRef<Path>, name: &str) -> Result<CameraProfile, CameraError> {
    let name = camera_name(name)?;
    Config::open(dir)
        .map_err(CameraError::Config)?
        .get_camera(&name)
        .cloned()
        .ok_or(CameraError::NotFound(name))
}

pub fn resolve(
    dir: impl AsRef<Path>,
    store: &dyn SecretStore,
    name: &str,
) -> Result<ResolvedCamera, CameraError> {
    let profile = profile(dir, name)?;
    let password = match store.get(SERVICE, &password_account(&profile.id)) {
        Ok(password) => password,
        Err(SecretError::NotFound) => return Err(CameraError::MissingSecret),
        Err(error) => return Err(CameraError::Secret(error)),
    };
    Ok(ResolvedCamera {
        name: camera_name(name)?,
        timeout: parse_timeout(&profile.timeout)?,
        profile,
        password,
    })
}

pub fn link(
    dir: impl AsRef<Path>,
    printer: &str,
    camera: &str,
    select: bool,
) -> Result<CameraSummary, CameraError> {
    change_links(dir.as_ref(), printer, camera, |config, printer, camera| {
        config.link_camera(printer, camera, select)
    })
}

pub fn unlink(
    dir: impl AsRef<Path>,
    printer: &str,
    camera: &str,
) -> Result<CameraSummary, CameraError> {
    change_links(dir.as_ref(), printer, camera, Config::unlink_camera)
}

pub fn select(
    dir: impl AsRef<Path>,
    printer: &str,
    camera: Option<&str>,
) -> Result<SelectedFeed, CameraError> {
    let dir = dir.as_ref();
    let printer = camera_name(printer)?;
    let camera = camera.map(camera_name).transpose()?;
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    config
        .select_camera(&printer, camera.as_deref())
        .map_err(|error| link_error(error, &printer, camera.as_deref().unwrap_or_default()))?;
    config.save(dir).map_err(CameraError::Config)?;
    Ok(SelectedFeed { printer, camera })
}

fn change_links(
    dir: &Path,
    printer: &str,
    camera: &str,
    change: impl FnOnce(&mut Config, &str, &str) -> Result<(), ConfigError>,
) -> Result<CameraSummary, CameraError> {
    let printer = camera_name(printer)?;
    let camera = camera_name(camera)?;
    let mut config = Config::open(dir).map_err(CameraError::Config)?;
    change(&mut config, &printer, &camera).map_err(|error| link_error(error, &printer, &camera))?;
    config.save(dir).map_err(CameraError::Config)?;
    let profile = config
        .get_camera(&camera)
        .cloned()
        .ok_or_else(|| CameraError::NotFound(camera.clone()))?;
    Ok(summary(&config, &camera, &profile))
}

fn link_error(error: ConfigError, printer: &str, camera: &str) -> CameraError {
    match error {
        ConfigError::ProfileNotFound => CameraError::PrinterNotFound(printer.to_owned()),
        ConfigError::CameraNotFound => CameraError::NotFound(camera.to_owned()),
        ConfigError::CameraNotLinked => CameraError::NotLinked {
            printer: printer.to_owned(),
            camera: camera.to_owned(),
        },
        other => CameraError::Config(other),
    }
}

fn summary(config: &Config, name: &str, camera: &CameraProfile) -> CameraSummary {
    let (printers, selected_by) = config.camera_printers(&camera.id);
    CameraSummary {
        name: name.to_owned(),
        id: camera.id.clone(),
        driver: camera.driver.clone(),
        host: camera.host.clone(),
        username: camera.username.clone(),
        rtsp_port: camera.rtsp_port.unwrap_or(TAPO_RTSP_PORT),
        timeout: camera.timeout.clone(),
        printers,
        selected_by,
    }
}

/// Camera and printer names share the printer profile name rules.
fn camera_name(name: &str) -> Result<String, CameraError> {
    profiles::normalize_name(name).map_err(|error| match error {
        ProfileError::MissingName => CameraError::MissingName,
        _ => CameraError::InvalidName,
    })
}

/// Validates every field and returns the parsed timeout.
fn validate(request: &CameraRequest) -> Result<std::time::Duration, CameraError> {
    if !DRIVERS.contains(&request.driver.as_str()) {
        return Err(CameraError::UnknownDriver(request.driver.clone()));
    }
    validate_host(&request.host)?;
    let username = &request.username;
    if username.is_empty()
        || username.chars().count() > 64
        || username
            .chars()
            .any(|character| character.is_control() || matches!(character, '"' | '\\'))
    {
        return Err(CameraError::InvalidUsername);
    }
    if request.password.chars().any(char::is_control) {
        return Err(CameraError::InvalidPassword);
    }
    if request.rtsp_port == Some(0) {
        return Err(CameraError::InvalidPort);
    }
    parse_timeout(&request.timeout)
}

/// A bare IP address or hostname: no scheme, port, path, or user info.
fn validate_host(host: &str) -> Result<(), CameraError> {
    if host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    if host.contains(['/', ':', '@']) {
        return Err(CameraError::InvalidHost);
    }
    profiles::validate_host(host).map_err(|_| CameraError::InvalidHost)
}

fn parse_timeout(value: &str) -> Result<std::time::Duration, CameraError> {
    drivers::parse_timeout(value).map_err(|_| CameraError::InvalidTimeout)
}

fn password_account(id: &str) -> String {
    account("camera", id, "password")
}

fn secret_error(error: ProfileError) -> CameraError {
    match error {
        ProfileError::Secret(error) => CameraError::Secret(error),
        _ => CameraError::RollbackFailed,
    }
}

fn new_id() -> String {
    let mut bytes = [0u8; 16];
    openssl::rand::rand_bytes(&mut bytes).expect("OpenSSL random source is available");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("formatting an RFC 3339 timestamp cannot fail")
}
```

In `cameras.rs`, below `mod profiles;`, add:

```rust
pub use profiles::{
    CameraRemoveResult, CameraRequest, CameraSummary, SelectedFeed, create, link, list, profile,
    remove, resolve, select, unlink, update,
};
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p melt-core --locked cameras::`
Expected: PASS (8 workflow tests plus earlier ones).

Run: `cargo test --workspace --locked && cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: PASS, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/melt-core/src/cameras.rs crates/melt-core/src/cameras/profiles.rs crates/melt-core/src/profiles.rs crates/melt-core/src/keychain.rs
git commit -m "feat(camera): add, edit, remove, and link camera profiles

Passwords live in the keychain under camera:<id>:password with the
printer workflow's rollback; renames never contact the camera.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: CLI camera profile and link commands

**Files:**
- Create: `crates/melt-cli/src/cameras.rs`
- Modify: `crates/melt-cli/src/lib.rs` (module, dispatch, help tables, `read_secret_file`, `config_error`)
- Modify: `fixtures/cli-contract/inventory.json`, `fixtures/cli-contract/manifest.json`, `crates/melt-cli/tests/contract_fixtures.rs`
- Create: `fixtures/cli-contract/golden/camera-list-empty.json`, `fixtures/cli-contract/golden/camera-select-missing-source.json`

**Interfaces:**
- Consumes: Task 4 workflow functions, `cameras::verify`
- Produces (crate-private to `melt-cli`): `cameras::{run_profile_command, camera_error}`; `read_secret_file(flag: &str, path: &str) -> Result<String, String>` in `lib.rs`

- [ ] **Step 1: Write the failing CLI tests**

Create `crates/melt-cli/src/cameras.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_errors_follow_the_exit_contract() {
        let cases = [
            (CameraError::InvalidName, 2, "config_error"),
            (CameraError::NotFound("porch".into()), 2, "config_error"),
            (CameraError::Authentication, 3, "authentication_failed"),
            (CameraError::DigestRequired, 3, "authentication_failed"),
            (CameraError::MissingSecret, 3, "secret_not_found"),
            (CameraError::Timeout, 4, "timeout"),
            (CameraError::Stream("RTSP PLAY failed".into()), 4, "connection_failed"),
            (CameraError::UnsupportedCodec, 5, "capability_unsupported"),
            (CameraError::RollbackFailed, 1, "internal_error"),
        ];
        for (error, exit_code, code) in cases {
            let name = format!("{error:?}");
            let mapped = camera_error(error);
            assert_eq!((mapped.exit_code, mapped.code), (exit_code, code), "{name}");
        }
    }
}
```

Add `mod cameras;` after the `use` block in `crates/melt-cli/src/lib.rs`.

In `fixtures/cli-contract/manifest.json`, append to `fixtures`:

```json
    {
      "id": "camera-list-empty-json",
      "command": "camera list",
      "args": ["camera", "list", "--output", "json"],
      "exitCode": 0,
      "stdout": "golden/camera-list-empty.json",
      "stderr": "",
      "ignoreJsonPointers": []
    },
    {
      "id": "camera-select-missing-source-json",
      "command": "camera select",
      "args": ["camera", "select", "--printer", "garage", "--output", "json"],
      "exitCode": 2,
      "stdout": "golden/camera-select-missing-source.json",
      "stderr": "",
      "ignoreJsonPointers": []
    }
```

Create `fixtures/cli-contract/golden/camera-list-empty.json`:

```json
{
  "ok": true,
  "data": {
    "cameras": []
  },
  "error": null,
  "meta": {
    "command": "camera list"
  }
}
```

Create `fixtures/cli-contract/golden/camera-select-missing-source.json`:

```json
{
  "ok": false,
  "data": null,
  "error": {
    "code": "config_error",
    "message": "camera select requires exactly one of --camera or --builtin",
    "details": {}
  },
  "meta": {
    "command": "camera select"
  }
}
```

In `fixtures/cli-contract/inventory.json`, append to `meltNativeCommands`:

```json
    { "path": "camera add", "arguments": "<camera>", "flags": ["--driver <driver>", "--host <host>", "--username <user>", "--password-file <path>", "--rtsp-port <port>", "--timeout <duration>"], "implementation": "implemented" },
    { "path": "camera edit", "arguments": "<camera>", "flags": ["--name <name>", "--host <host>", "--username <user>", "--password-file <path>", "--rtsp-port <port>", "--timeout <duration>"], "implementation": "implemented" },
    { "path": "camera link", "arguments": "<camera>", "flags": ["--printer <printer>", "--select"], "implementation": "implemented" },
    { "path": "camera list", "arguments": "", "flags": [], "implementation": "implemented" },
    { "path": "camera remove", "arguments": "<camera>", "flags": ["--yes"], "implementation": "implemented" },
    { "path": "camera select", "arguments": "", "flags": ["--printer <printer>", "--camera <camera>", "--builtin"], "implementation": "implemented" },
    { "path": "camera unlink", "arguments": "<camera>", "flags": ["--printer <printer>"], "implementation": "implemented" }
```

In `crates/melt-cli/tests/contract_fixtures.rs`, change `assert_eq!(manifest.fixtures.len(), 9);` to `assert_eq!(manifest.fixtures.len(), 11);`.

Run: `cargo test -p melt-cli --locked`
Expected: compile errors (`camera_error`, `CameraError` not in scope).

- [ ] **Step 2: Generalize the secret-file reader**

In `crates/melt-cli/src/lib.rs`, replace `fn read_access_code_file(path: &str)` with:

```rust
/// Reads a one-line secret from a regular, user-only file of at most 4 KiB.
fn read_secret_file(flag: &str, path: &str) -> Result<String, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|error| format!("cannot read {flag}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > 4096 {
        return Err(format!("{flag} must be a regular file no larger than 4 KiB"));
    }
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o077 != 0 {
        return Err(format!("{flag} must not grant group or other access"));
    }
    let mut secret =
        std::fs::read_to_string(path).map_err(|error| format!("cannot read {flag}: {error}"))?;
    if let Some(stripped) = secret
        .strip_suffix("\r\n")
        .or_else(|| secret.strip_suffix('\n'))
    {
        secret = stripped.into();
    }
    Ok(secret)
}
```

Run `grep -n read_access_code_file crates/melt-cli/src/lib.rs` and change each call to `read_secret_file("--access-code-file", <same argument>)`. The printer messages stay byte-identical.

In `fn config_error`, add `InvalidCameras` to the usage arms:

```rust
        ConfigError::InvalidCameras(message) => {
            AppError::usage(format!("invalid camera configuration: {message}"))
        }
```

- [ ] **Step 3: Implement the commands**

Prepend to `crates/melt-cli/src/cameras.rs`, above the test module:

```rust
//! `melt camera` profile, link, and external-camera media commands. In the
//! commands added here a positional argument is always a camera name and a
//! printer is always `--printer`.

use std::io::{IsTerminal, Write};

use melt_core::{
    AppError,
    cameras::{self, CameraError, CameraRequest, CameraSummary},
    config::config_dir,
    keychain::SystemKeychain,
};
use serde::Serialize;

use super::{
    OutputFormat, ParsedOptions, config_error, parse_options, read_secret_file, write_error,
    write_success,
};

pub(crate) fn run_profile_command(
    format: OutputFormat,
    action: &str,
    args: &[&String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let command = format!("camera {action}");
    match action {
        "add" => respond(&command, format, add(args), human_summary("Camera added"), out, err),
        "edit" => respond(&command, format, edit(args), human_summary("Camera updated"), out, err),
        "link" => respond(&command, format, link(args), human_summary("Camera linked"), out, err),
        "unlink" => respond(&command, format, unlink(args), human_summary("Camera unlinked"), out, err),
        "list" => respond(
            &command,
            format,
            list(args),
            |data: &CameraList, out| {
                if data.cameras.is_empty() {
                    return writeln!(out, "No cameras configured.");
                }
                for camera in &data.cameras {
                    writeln!(out, "{}", summary_line(camera))?;
                }
                Ok(())
            },
            out,
            err,
        ),
        "remove" => {
            let result = remove(args, err);
            respond(
                &command,
                format,
                result,
                |removed: &cameras::CameraRemoveResult, out| {
                    writeln!(out, "Camera removed: {}", removed.name)?;
                    for warning in &removed.warnings {
                        writeln!(out, "Warning: {}", warning.message)?;
                    }
                    Ok(())
                },
                out,
                err,
            )
        }
        "select" => respond(
            &command,
            format,
            select(args),
            |selected: &cameras::SelectedFeed, out| match &selected.camera {
                Some(camera) => writeln!(out, "Printer {} now shows camera {camera}", selected.printer),
                None => writeln!(out, "Printer {} now shows its built-in camera", selected.printer),
            },
            out,
            err,
        ),
        _ => unreachable!("dispatch matches only known camera profile commands"),
    }
}

/// Renders human text eagerly so the JSON and human paths share one result.
fn respond<T: Serialize>(
    command: &str,
    format: OutputFormat,
    result: Result<T, AppError>,
    human: impl FnOnce(&T, &mut dyn Write) -> std::io::Result<()>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    match result {
        Ok(data) => {
            let mut text = Vec::new();
            if matches!(format, OutputFormat::Human) && human(&data, &mut text).is_err() {
                return 1;
            }
            write_success(command, format, data, |out| out.write_all(&text), out)
        }
        Err(error) => write_error(command, format, error, out, err),
    }
}

fn human_summary(
    verb: &'static str,
) -> impl FnOnce(&CameraSummary, &mut dyn Write) -> std::io::Result<()> {
    move |camera, out| writeln!(out, "{verb}: {}", summary_line(camera))
}

fn summary_line(camera: &CameraSummary) -> String {
    let printers = if camera.printers.is_empty() {
        "-".to_owned()
    } else {
        camera
            .printers
            .iter()
            .map(|printer| {
                if camera.selected_by.contains(printer) {
                    format!("{printer} (selected)")
                } else {
                    printer.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "{}\t{}\t{}:{}\tprinters: {printers}",
        camera.name, camera.driver, camera.host, camera.rtsp_port
    )
}

#[derive(Serialize)]
struct CameraList {
    cameras: Vec<CameraSummary>,
}

fn add(args: &[&String]) -> Result<CameraSummary, AppError> {
    let (positionals, options) = parse_options(
        args,
        &[],
        &["driver", "host", "username", "password-file", "rtsp-port", "timeout"],
    )
    .map_err(AppError::usage)?;
    let name = one_camera("camera add", &positionals)?;
    let request = CameraRequest {
        name: name.to_owned(),
        driver: required(&options, "driver")?,
        host: required(&options, "host")?,
        username: required(&options, "username")?,
        password: read_secret_file("--password-file", &required(&options, "password-file")?)
            .map_err(AppError::usage)?,
        rtsp_port: rtsp_port(&options)?,
        timeout: options.value("timeout").unwrap_or("10s").to_owned(),
    };
    let dir = config_dir().map_err(config_error)?;
    cameras::create(dir, &SystemKeychain, request, cameras::verify).map_err(camera_error)
}

fn edit(args: &[&String]) -> Result<CameraSummary, AppError> {
    let (positionals, options) = parse_options(
        args,
        &[],
        &["name", "host", "username", "password-file", "rtsp-port", "timeout"],
    )
    .map_err(AppError::usage)?;
    let current = one_camera("camera edit", &positionals)?;
    if options.values.is_empty() {
        return Err(AppError::usage("camera edit requires at least one option to change"));
    }
    let dir = config_dir().map_err(config_error)?;
    let existing = cameras::profile(&dir, current).map_err(camera_error)?;
    let request = CameraRequest {
        name: options.value("name").unwrap_or(current).to_owned(),
        driver: existing.driver,
        host: options.value("host").map_or(existing.host, str::to_owned),
        username: options
            .value("username")
            .map_or(existing.username, str::to_owned),
        password: match options.value("password-file") {
            Some(path) => read_secret_file("--password-file", path).map_err(AppError::usage)?,
            None => String::new(),
        },
        rtsp_port: match options.value("rtsp-port") {
            Some(_) => rtsp_port(&options)?,
            None => existing.rtsp_port,
        },
        timeout: options
            .value("timeout")
            .map_or(existing.timeout, str::to_owned),
    };
    cameras::update(dir, &SystemKeychain, current, request, cameras::verify).map_err(camera_error)
}

fn list(args: &[&String]) -> Result<CameraList, AppError> {
    let (positionals, _) = parse_options(args, &[], &[]).map_err(AppError::usage)?;
    if !positionals.is_empty() {
        return Err(AppError::usage("camera list takes no arguments"));
    }
    let dir = config_dir().map_err(config_error)?;
    Ok(CameraList {
        cameras: cameras::list(dir).map_err(camera_error)?,
    })
}

fn remove(args: &[&String], err: &mut dyn Write) -> Result<cameras::CameraRemoveResult, AppError> {
    let (positionals, options) = parse_options(args, &["yes"], &[]).map_err(AppError::usage)?;
    let name = one_camera("camera remove", &positionals)?;
    if !options.enabled("yes") {
        if !std::io::stdin().is_terminal() {
            return Err(AppError::usage("non-interactive mode requires --yes"));
        }
        let _ = write!(
            err,
            "Remove camera {name}, its printer links, and its stored password? Type 'yes' to continue: "
        );
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|_| AppError {
                exit_code: 1,
                code: "internal_error",
                message: "cannot read confirmation".into(),
            })?;
        if answer.trim_end() != "yes" {
            return Err(AppError::usage("confirmation declined; camera not removed"));
        }
    }
    let dir = config_dir().map_err(config_error)?;
    cameras::remove(dir, &SystemKeychain, name).map_err(camera_error)
}

fn link(args: &[&String]) -> Result<CameraSummary, AppError> {
    let (positionals, options) =
        parse_options(args, &["select"], &["printer"]).map_err(AppError::usage)?;
    let camera = one_camera("camera link", &positionals)?;
    let printer = required(&options, "printer")?;
    let dir = config_dir().map_err(config_error)?;
    cameras::link(dir, &printer, camera, options.enabled("select")).map_err(camera_error)
}

fn unlink(args: &[&String]) -> Result<CameraSummary, AppError> {
    let (positionals, options) = parse_options(args, &[], &["printer"]).map_err(AppError::usage)?;
    let camera = one_camera("camera unlink", &positionals)?;
    let printer = required(&options, "printer")?;
    let dir = config_dir().map_err(config_error)?;
    cameras::unlink(dir, &printer, camera).map_err(camera_error)
}

fn select(args: &[&String]) -> Result<cameras::SelectedFeed, AppError> {
    let (positionals, options) =
        parse_options(args, &["builtin"], &["printer", "camera"]).map_err(AppError::usage)?;
    if !positionals.is_empty() {
        return Err(AppError::usage("camera select takes --printer and a source, not names"));
    }
    let camera = match (options.value("camera"), options.enabled("builtin")) {
        (Some(camera), false) => Some(camera),
        (None, true) => None,
        _ => {
            return Err(AppError::usage(
                "camera select requires exactly one of --camera or --builtin",
            ));
        }
    };
    let printer = required(&options, "printer")?;
    let dir = config_dir().map_err(config_error)?;
    cameras::select(dir, &printer, camera).map_err(camera_error)
}

fn one_camera<'a>(command: &str, positionals: &'a [String]) -> Result<&'a str, AppError> {
    match positionals {
        [name] => Ok(name),
        [] => Err(AppError::usage(format!("{command} requires a camera name"))),
        _ => Err(AppError::usage(format!(
            "{command} expects exactly one camera name"
        ))),
    }
}

fn required(options: &ParsedOptions, name: &str) -> Result<String, AppError> {
    options
        .value(name)
        .map(str::to_owned)
        .ok_or_else(|| AppError::usage(format!("--{name} is required")))
}

fn rtsp_port(options: &ParsedOptions) -> Result<Option<u16>, AppError> {
    options
        .value("rtsp-port")
        .map(|value| {
            value
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| AppError::usage("--rtsp-port must be between 1 and 65535"))
        })
        .transpose()
}

pub(crate) fn camera_error(error: CameraError) -> AppError {
    let message = error.to_string();
    let (exit_code, code) = match error {
        CameraError::Config(error) => return config_error(error),
        CameraError::Authentication | CameraError::DigestRequired => (3, "authentication_failed"),
        CameraError::MissingSecret => (3, "secret_not_found"),
        CameraError::Secret(_) => (3, "secret_store_failed"),
        CameraError::Timeout => (4, "timeout"),
        CameraError::Connect(_) | CameraError::Stream(_) => (4, "connection_failed"),
        CameraError::UnsupportedCodec => (5, "capability_unsupported"),
        CameraError::RollbackFailed => (1, "internal_error"),
        _ => return AppError::usage(message),
    };
    AppError {
        exit_code,
        code,
        message,
    }
}
```

In `lib.rs` `run`, add this arm directly above the existing `[camera, action, rest @ ..]` arm:

```rust
        [camera, action, rest @ ..]
            if camera.as_str() == "camera"
                && matches!(
                    action.as_str(),
                    "add" | "edit" | "link" | "list" | "remove" | "select" | "unlink"
                ) =>
        {
            cameras::run_profile_command(invocation.format, action, rest, out, err)
        }
```

Add these entries to `LEAF_COMMANDS`, before the `["camera", "snapshot"]` entry (keep the table's order):

```rust
    LeafCommand {
        path: &["camera", "add"],
        short: "Add an IP camera profile",
        args: "<camera> [flags]",
        flags: "      --driver string          camera driver (tapo)\n  -h, --help                   help for add\n      --host string            camera IP address or hostname\n      --password-file string   file containing the camera account password\n      --rtsp-port int          RTSP port (default 554)\n      --timeout string         connection timeout (default \"10s\")\n      --username string        camera account username",
    },
    LeafCommand {
        path: &["camera", "edit"],
        short: "Edit an IP camera profile",
        args: "<camera> [flags]",
        flags: "  -h, --help                   help for edit\n      --host string            camera IP address or hostname\n      --name string            rename the camera\n      --password-file string   file containing a new camera account password\n      --rtsp-port int          RTSP port\n      --timeout string         connection timeout\n      --username string        camera account username",
    },
    LeafCommand {
        path: &["camera", "link"],
        short: "Link an IP camera to a printer",
        args: "<camera> [flags]",
        flags: "  -h, --help             help for link\n      --printer string   printer profile to link\n      --select           show this camera by default for the printer",
    },
    LeafCommand {
        path: &["camera", "list"],
        short: "List IP camera profiles and their printer links",
        args: "[flags]",
        flags: "  -h, --help   help for list",
    },
    LeafCommand {
        path: &["camera", "remove"],
        short: "Remove an IP camera profile",
        args: "<camera> [flags]",
        flags: "  -h, --help   help for remove\n      --yes    skip interactive confirmation",
    },
    LeafCommand {
        path: &["camera", "select"],
        short: "Choose the camera a printer shows by default",
        args: "[flags]",
        flags: "      --builtin          show the printer's built-in camera\n      --camera string    show this linked IP camera\n  -h, --help             help for select\n      --printer string   printer profile",
    },
```

and after the `["camera", "stream"]` entry:

```rust
    LeafCommand {
        path: &["camera", "unlink"],
        short: "Unlink an IP camera from a printer",
        args: "<camera> [flags]",
        flags: "  -h, --help             help for unlink\n      --printer string   printer profile to unlink",
    },
```

In `COMMAND_GROUPS`, replace the `["camera"]` group's `commands` with:

```rust
        commands: &[
            ("add", "Add an IP camera profile"),
            ("edit", "Edit an IP camera profile"),
            ("link", "Link an IP camera to a printer"),
            ("list", "List IP camera profiles and their printer links"),
            ("remove", "Remove an IP camera profile"),
            ("select", "Choose the camera a printer shows by default"),
            ("snapshot", "Capture one still image from a printer camera"),
            (
                "stream",
                "Stream camera feed from a printer via a local HTTP server",
            ),
            ("unlink", "Unlink an IP camera from a printer"),
        ],
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p melt-cli --locked && make contract`
Expected: PASS, including `camera_errors_follow_the_exit_contract`, `every_leaf_command_documents_itself` and both new fixtures.

Run: `cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/melt-cli/src/cameras.rs crates/melt-cli/src/lib.rs crates/melt-cli/tests/contract_fixtures.rs fixtures/cli-contract
git commit -m "feat(cli): manage IP camera profiles and printer links

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: `--camera` on snapshot and stream, plus docs

**Files:**
- Modify: `crates/melt-cli/src/lib.rs` (`camera_snapshot`, `camera_stream`, new `serve_mjpeg_stream`, help flags)
- Modify: `crates/melt-cli/src/cameras.rs` (`snapshot`, `stream`)
- Modify: `fixtures/cli-contract/manifest.json`, `crates/melt-cli/tests/contract_fixtures.rs`
- Create: `fixtures/cli-contract/golden/camera-snapshot-printer-and-camera.json`, `fixtures/cli-contract/golden/camera-snapshot-unknown-camera.json`
- Create: `docs/ip-cameras.md`
- Modify: `README.md`, `docs/cli-contract-fixtures.md`

**Interfaces:**
- Consumes: `cameras::{resolve, snapshot, mjpeg_stream, StreamQuality}`, and in `lib.rs` `snapshot_destination`, `validate_snapshot_destination`, `write_snapshot_file`, `mark_network_call`
- Produces: `fn serve_mjpeg_stream<T: Serialize>(format, stream: bambu::MjpegStream, port: u16, duration: Option<Duration>, source: &str, data: impl FnOnce(String) -> T, out, err) -> i32` in `lib.rs`

- [ ] **Step 1: Write the failing fixtures**

Append to `fixtures/cli-contract/manifest.json` `fixtures`:

```json
    {
      "id": "camera-snapshot-printer-and-camera-json",
      "command": "camera snapshot",
      "args": ["camera", "snapshot", "garage", "--camera", "porch", "--output", "json"],
      "exitCode": 2,
      "stdout": "golden/camera-snapshot-printer-and-camera.json",
      "stderr": "",
      "ignoreJsonPointers": []
    },
    {
      "id": "camera-snapshot-unknown-camera-json",
      "command": "camera snapshot",
      "args": ["camera", "snapshot", "--camera", "porch", "--output", "json"],
      "exitCode": 2,
      "stdout": "golden/camera-snapshot-unknown-camera.json",
      "stderr": "",
      "ignoreJsonPointers": []
    }
```

Create `fixtures/cli-contract/golden/camera-snapshot-printer-and-camera.json`:

```json
{
  "ok": false,
  "data": null,
  "error": {
    "code": "config_error",
    "message": "use either a printer name or --camera, not both",
    "details": {}
  },
  "meta": {
    "command": "camera snapshot"
  }
}
```

Create `fixtures/cli-contract/golden/camera-snapshot-unknown-camera.json`:

```json
{
  "ok": false,
  "data": null,
  "error": {
    "code": "config_error",
    "message": "camera \"porch\" not found",
    "details": {}
  },
  "meta": {
    "command": "camera snapshot"
  }
}
```

Change the manifest count assertion in `contract_fixtures.rs` from `11` to `13`.

Run: `make contract`
Expected: FAIL. With today's parser, `--camera` is an unknown option, so the error message differs.

- [ ] **Step 2: Extract the MJPEG server tail**

In `lib.rs` `camera_stream`, replace everything from `let upstream_shutdown = if duration.is_some() {` to the end of the function with:

```rust
    let profile = printer.name.clone();
    serve_mjpeg_stream(
        format,
        stream,
        port,
        duration,
        &profile,
        |url| CameraStreamData {
            profile: profile.clone(),
            url,
            format: "mjpeg",
            port,
        },
        out,
        err,
    )
}

/// Serves an MJPEG stream on a loopback port until the duration elapses or
/// the process is interrupted. Shared by printer and IP camera streams.
#[allow(clippy::too_many_arguments)]
fn serve_mjpeg_stream<T: Serialize>(
    format: OutputFormat,
    stream: melt_core::bambu::MjpegStream,
    port: u16,
    duration: Option<Duration>,
    source: &str,
    data: impl FnOnce(String) -> T,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
```

followed by the removed code moved verbatim, with two edits:
1. the `write_success` call becomes

```rust
    let result = write_success(
        "camera stream",
        format,
        data(url.clone()),
        |out| {
            writeln!(out, "Streaming camera from {source}")?;
            writeln!(out, "Format: MJPEG (open in browser)")?;
            writeln!(out, "URL: {url}\n")?;
            writeln!(out, "Press Ctrl+C to stop.")
        },
        out,
    );
```

2. the old `let profile = printer.name.clone();` line inside the moved code is deleted.

Run: `cargo test -p melt-cli --locked`
Expected: PASS. This is a pure extraction; the existing `camera stream` fixture is unchanged.

- [ ] **Step 3: Route `--camera` to the external handlers**

In `camera_snapshot`, change the `parse_options` call and insert the branch right after it:

```rust
    let (positionals, options) = match parse_options(
        args,
        &["insecure", "overwrite"],
        &["to", "timeout", "protocol-trace", "camera", "quality"],
    ) {
        Ok(value) => value,
        Err(error) => {
            return write_error("camera snapshot", format, AppError::usage(error), out, err);
        }
    };
    if let Some(camera) = options.value("camera") {
        return cameras::snapshot(format, camera, &positionals, &options, out, err);
    }
    if options.value("quality").is_some() {
        return write_error(
            "camera snapshot",
            format,
            AppError::usage("--quality applies only with --camera"),
            out,
            err,
        );
    }
```

In `camera_stream`:
1. add `"camera", "quality"` to its value options;
2. move the `let name = match one_positional("camera stream", &positionals) { … };` block down to just before `let connection = match connection_options(&options) {`;
3. insert, just above that moved block:

```rust
    if let Some(camera) = options.value("camera") {
        return cameras::stream(format, camera, &positionals, &options, port, duration, out, err);
    }
    if options.value("quality").is_some() {
        return write_error(
            "camera stream",
            format,
            AppError::usage("--quality applies only with --camera"),
            out,
            err,
        );
    }
```

Add to `crates/melt-cli/src/cameras.rs` (extend the `use super::{…}` list with `mark_network_call, serve_mjpeg_stream, snapshot_destination, validate_snapshot_destination, write_snapshot_file`, and the `melt_core` import with `cameras::StreamQuality, drivers`, plus `std::time::Duration`):

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalSnapshotData {
    camera: String,
    camera_id: String,
    driver: String,
    quality: StreamQuality,
    path: String,
    size_bytes: u64,
    protocol: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExternalStreamData {
    camera: String,
    camera_id: String,
    driver: String,
    quality: StreamQuality,
    url: String,
    format: &'static str,
    port: u16,
}

pub(crate) fn snapshot(
    format: OutputFormat,
    camera: &str,
    positionals: &[String],
    options: &ParsedOptions,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    respond(
        "camera snapshot",
        format,
        take_snapshot(camera, positionals, options),
        |data: &ExternalSnapshotData, out| {
            writeln!(out, "Snapshot saved to {} ({} bytes).", data.path, data.size_bytes)
        },
        out,
        err,
    )
}

fn take_snapshot(
    camera: &str,
    positionals: &[String],
    options: &ParsedOptions,
) -> Result<ExternalSnapshotData, AppError> {
    let quality = external_options(positionals, options)?;
    let overwrite = options.enabled("overwrite");
    let destination = snapshot_destination(options.value("to"), &camera.to_ascii_lowercase())?;
    validate_snapshot_destination(&destination, overwrite)?;
    let resolved = resolve(camera, options)?;
    mark_network_call();
    let image = cameras::snapshot(&resolved, quality).map_err(camera_error)?;
    let size_bytes = write_snapshot_file(&destination, &image, overwrite)?;
    Ok(ExternalSnapshotData {
        camera: resolved.name,
        camera_id: resolved.profile.id,
        driver: resolved.profile.driver,
        quality,
        path: destination.display().to_string(),
        size_bytes,
        protocol: "rtsp",
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn stream(
    format: OutputFormat,
    camera: &str,
    positionals: &[String],
    options: &ParsedOptions,
    port: u16,
    duration: Option<Duration>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let opened = external_options(positionals, options).and_then(|quality| {
        let resolved = resolve(camera, options)?;
        mark_network_call();
        let stream = cameras::mjpeg_stream(&resolved, quality).map_err(camera_error)?;
        Ok((resolved, quality, stream))
    });
    let (resolved, quality, stream) = match opened {
        Ok(opened) => opened,
        Err(error) => return write_error("camera stream", format, error, out, err),
    };
    let name = resolved.name.clone();
    serve_mjpeg_stream(
        format,
        stream,
        port,
        duration,
        &name,
        |url| ExternalStreamData {
            camera: resolved.name,
            camera_id: resolved.profile.id,
            driver: resolved.profile.driver,
            quality,
            url,
            format: "mjpeg",
            port,
        },
        out,
        err,
    )
}

/// Rejects printer-only inputs and returns the requested stream quality.
fn external_options(
    positionals: &[String],
    options: &ParsedOptions,
) -> Result<StreamQuality, AppError> {
    if !positionals.is_empty() {
        return Err(AppError::usage("use either a printer name or --camera, not both"));
    }
    if options.enabled("insecure") || options.value("protocol-trace").is_some() {
        return Err(AppError::usage(
            "--insecure and --protocol-trace apply only to printer cameras",
        ));
    }
    options
        .value("quality")
        .map_or(Ok(StreamQuality::High), str::parse)
        .map_err(camera_error)
}

fn resolve(camera: &str, options: &ParsedOptions) -> Result<cameras::ResolvedCamera, AppError> {
    let dir = config_dir().map_err(config_error)?;
    let mut resolved = cameras::resolve(dir, &SystemKeychain, camera).map_err(camera_error)?;
    if let Some(timeout) = options.value("timeout") {
        resolved.timeout = drivers::parse_timeout(timeout).map_err(|_| {
            AppError::usage("--timeout must be a duration such as 10s or 1500ms")
        })?;
    }
    Ok(resolved)
}
```

Update the help `flags` strings of the existing `["camera", "snapshot"]` and `["camera", "stream"]` `LeafCommand`s, keeping alphabetical order, with these two lines:
`      --camera string           IP camera profile to use instead of a printer\n`
`      --quality string          IP camera stream quality: high or low (default "high")\n`
Change both `args` to `"(<name> | --camera <camera>) [flags]"`.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace --locked && make contract && cargo clippy --workspace --all-targets --locked -- -D warnings`
Expected: PASS, no warnings. The pre-existing `camera-stream-invalid-format-json` fixture is unchanged.

- [ ] **Step 5: Write the user docs**

Create `docs/ip-cameras.md`:

````markdown
# IP cameras

Melt can show IP cameras next to, or instead of, a printer's built-in camera.
Camera profiles are independent of printers; a camera can be linked to any
number of printers, and each printer picks one default feed.

## Supported cameras

| Driver | Model | Qualified hardware / firmware | Video | Snapshots | Pan/tilt |
|---|---|---|---|---|---|
| `tapo` | TP-Link Tapo C510W | Pending (target: 1.0 / 1.3.5) | H.264 over RTSP | Decoded from video | Planned |

Other models and firmware are unqualified until exercised on hardware.

## Tapo setup

1. In the Tapo app, open the camera → **Settings → Advanced Settings →
   Camera Account** and create a local username and password. This account is
   separate from your TP-Link cloud login; Melt never asks for the cloud
   password. If your firmware shows a **Third-Party Compatibility** option,
   turn it on, and make sure privacy mode is off.
2. Save the password to a file only you can read:

   ```sh
   umask 077 && printf '%s\n' 'camera-account-password' > ~/.config/melt/porch.pw
   ```

3. Add the camera. Melt connects, authenticates, and decodes one frame before
   saving; it never moves the camera during setup.

   ```sh
   melt camera add porch --driver tapo --host 192.0.2.20 \
     --username melt --password-file ~/.config/melt/porch.pw
   ```

The password is stored in the OS keychain; `melt.yaml` holds only the
username, host, and port.

## Everyday use

```sh
melt camera list
melt camera snapshot --camera porch --quality low --to ./porch.jpg
melt camera stream --camera porch            # open http://127.0.0.1:8080/stream
melt camera link porch --printer workshop --select
melt camera select --printer workshop --builtin
melt camera edit porch --host 192.0.2.21     # re-verifies the connection
melt camera edit porch --name front-door     # rename only: no connection needed
melt camera remove porch --yes
```

`high` uses the camera's main stream (`/stream1`), `low` its sub-stream
(`/stream2`). `camera snapshot <printer>` and `camera stream <printer>` keep
their meaning: a positional name is always a printer.

## Security

Video uses plain RTSP on your LAN; it is not encrypted end to end. Melt only
accepts Digest authentication, so the password is never sent in clear; a
camera that offers only Basic authentication is refused. Credentials are only
sent to the configured host.

## Troubleshooting

| Symptom | Likely cause |
|---|---|
| `connection_failed`, connection refused on 554 | Camera Account not created yet, Third-Party Compatibility is off (on firmware that has it), or privacy mode is on (it disables RTSP and ONVIF). |
| `authentication_failed` | Wrong Camera Account username or password (not the cloud login). |
| `authentication_failed` mentioning Basic | The camera offered no Digest challenge; Melt will not send the password. |
| `timeout` | Wrong host, or the camera is busy: Tapo cameras limit simultaneous streams; close other viewers (NVRs, apps). |
| `capability_unsupported` | The stream is not H.264 (H.265 is not supported). |

## Downgrading Melt

Adding a camera writes `melt.yaml` as schema version 2, which older Melt
releases refuse to read. To downgrade, remove the `cameras:` and
`printer_cameras:` sections from `melt.yaml` and set `version: 1`, then
delete the `camera:<id>:password` keychain entries if you no longer need them.
````

In `README.md`:
- In the `## CLI` list, replace `- `camera snapshot|stream`;` with
  `- `camera add|edit|list|remove|link|unlink|select|snapshot|stream` (IP cameras: see [docs/ip-cameras.md](docs/ip-cameras.md));`
- In `## Configuration and security`, after the first paragraph, add:
  `IP camera profiles and printer links live in the same configuration file; camera passwords are stored in the OS keychain under the camera's immutable ID. Camera video uses plain RTSP with Digest authentication on the LAN.`
- After the `--access-code-file` paragraph, add:
  `The same rules apply to `--password-file` for IP cameras.`

In `docs/cli-contract-fixtures.md`, append:

```markdown
## Melt-native camera commands

`camera add|edit|link|list|remove|select|unlink` are listed in
`meltNativeCommands`. `camera snapshot` and `camera stream` also accept the
Melt-native `--camera` and `--quality` flags; like `printer add --model`, these
are not added to the frozen Go-baseline rows, whose positional printer
behavior is unchanged.
```

- [ ] **Step 6: Final gates**

Run: `make ci`
Expected: PASS (license, evidence, lint, tests, build, contract).

- [ ] **Step 7: Commit**

```bash
git add crates/melt-cli fixtures/cli-contract docs/ip-cameras.md README.md docs/cli-contract-fixtures.md
git commit -m "feat(cli): snapshot and stream IP cameras with --camera

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Hardware qualification (attended)

**Attended. Only the camera's owner can do this, at the camera. An executing agent stops after Task 6 and hands this task to them.** No credentials go into chat, fixtures or the shell history.

**Files:**
- Create: `fixtures/tapo/c510w-hw1.0-fw1.3.5-stream1.sdp`, `fixtures/tapo/c510w-hw1.0-fw1.3.5-stream2.sdp`, `fixtures/tapo/README.md`
- Modify: `crates/melt-core/src/rtsp.rs` (fixture test), `docs/ip-cameras.md` (qualification row)

- [ ] **Step 1: Create the Camera Account** in the Tapo app (see `docs/ip-cameras.md`). Make sure privacy mode is off. Look for a **Third-Party Compatibility** option. If it exists, note its state, and run Step 2 with it both off and on. Record which state opens ports 554 and 2020.

- [ ] **Step 2: Check reachability and the challenge format**

```sh
nc -zv <camera-ip> 554
nc -zv <camera-ip> 2020
curl -sS -X POST -H 'Content-Type: application/soap+xml' \
  --data '<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body><GetSystemDateAndTime xmlns="http://www.onvif.org/ver10/device/wsdl"/></s:Body></s:Envelope>' \
  http://<camera-ip>:2020/onvif/device_service
curl -sS -v --rtsp-request OPTIONS rtsp://<camera-ip>:554/stream1 2>&1 | grep -E '^< (RTSP|Public)'
curl -sS -v --rtsp-request DESCRIBE rtsp://<camera-ip>:554/stream1 2>&1 | grep -iE '^< (RTSP|WWW-Authenticate)'
```

Expected: both ports open; the ONVIF call returns a SOAP envelope with the camera's UTC date and time and needs no credentials, which confirms ONVIF is reachable for Phase 3; and the DESCRIBE returns a `401` carrying a `Digest` challenge. Note the realm, whether `qop` is present, and whether a `Basic` challenge also appears. Do not record the nonce.

- [ ] **Step 3: Capture both SDPs.** `-u melt` makes curl prompt for the password; nothing lands in argv.

```sh
for stream in stream1 stream2; do
  curl -sS --digest -u melt --rtsp-request DESCRIBE \
    "rtsp://<camera-ip>:554/$stream" \
    | sed 's/<camera-ip>/192.0.2.20/g' \
    > "fixtures/tapo/c510w-hw1.0-fw1.3.5-$stream.sdp"
done
grep -E 'm=|rtpmap|fmtp|control' fixtures/tapo/*.sdp
```

Remove any serial number or MAC address from the `o=`/`s=` lines. Create `fixtures/tapo/README.md` with one line per file: model, hardware 1.0, firmware 1.3.5, capture date, and "address replaced with 192.0.2.20".

- [ ] **Step 4: Pin the captured SDPs in a test.** Append to `mod tests` in `crates/melt-core/src/rtsp.rs`:

```rust
    #[test]
    fn qualified_tapo_sdps_yield_h264_video_tracks() {
        for sdp in [
            include_str!("../../../fixtures/tapo/c510w-hw1.0-fw1.3.5-stream1.sdp"),
            include_str!("../../../fixtures/tapo/c510w-hw1.0-fw1.3.5-stream2.sdp"),
        ] {
            let track = parse_sdp(sdp).unwrap();
            assert!(!track.control.is_empty());
        }
    }
```

Run: `cargo test -p melt-core --locked rtsp::tests::qualified_tapo_sdps_yield_h264_video_tracks`
Expected: PASS. If it fails, the camera's SDP differs from the assumptions. Fix `parse_sdp` with a regression test before going further.

- [ ] **Step 5: Exercise Melt end to end**

```sh
umask 077 && printf '%s\n' '<password>' > /tmp/porch.pw   # type it in an editor instead if you prefer
cargo run -p melt-cli -- camera add porch --driver tapo --host <camera-ip> --username melt --password-file /tmp/porch.pw
cargo run -p melt-cli -- camera snapshot --camera porch --to /tmp/high.jpg
cargo run -p melt-cli -- camera snapshot --camera porch --quality low --to /tmp/low.jpg
cargo run -p melt-cli -- camera stream --camera porch --duration 60s   # watch it in a browser
cargo run -p melt-cli -- camera edit porch --password-file /tmp/porch.pw
cargo run -p melt-cli -- camera edit porch --name front-door
cargo run -p melt-cli -- camera remove front-door --yes
rm /tmp/porch.pw
```

Expected: every command succeeds, both JPEGs open and show the scene, and the stream plays for 60 s. Check one negative case too: `camera add` with a wrong password exits 3 and `camera list` stays empty.

Also check, and record in the report:

- Run `camera stream --camera porch --duration 90s` once for the default quality and once with `--quality low`; playback must survive past the 25 s keepalive.
- Record the raw `WWW-Authenticate` header lines from Step 2 (scheme case, single or multiple headers, whether Basic is offered), without the nonce.
- Open more simultaneous streams than the camera allows and record the error Melt reports.

- [ ] **Step 6: Record the result.** In `docs/ip-cameras.md`, change the qualified column to `1.0 / 1.3.5 (2026-MM-DD)` and add any limitation you observed: the maximum number of concurrent streams, and whether Third-Party Compatibility must be on.

- [ ] **Step 7: Commit**

```bash
git add fixtures/tapo crates/melt-core/src/rtsp.rs docs/ip-cameras.md
git commit -m "test(camera): qualify Tapo C510W hw 1.0 fw 1.3.5 RTSP

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
