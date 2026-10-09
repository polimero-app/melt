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

use openh264::{
    OpenH264API,
    decoder::{Decoder, DecoderConfig, Flush},
    formats::YUVSource,
};
use thiserror::Error;

/// Bounds how large a buffered-but-unparsed chunk may grow before we give up:
/// real RTSP responses and RTP frames are a few hundred bytes to a few KB, so
/// anything past this is a protocol confusion, not a slow network.
const MAX_BUFFERED: usize = 1 << 20;
/// Upper bound on one access unit's RTP bytes; far above any real frame.
const MAX_ACCESS_UNIT_BYTES: usize = 8 << 20;
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
    /// ignoring RTCP, other tracks, and late control `Response`s (keepalive
    /// replies) unless they say the session is gone.
    fn next_rtp_packet(&mut self, channel: u8) -> Result<Vec<u8>, RtspError> {
        loop {
            match self.next_message()? {
                rtsp_types::Message::Data(data) if data.channel_id() == channel => {
                    return Ok(data.into_body());
                }
                rtsp_types::Message::Response(response)
                    if keepalive_lost_session(response.status()) =>
                {
                    return Err(protocol_error("RTSP session expired"));
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
    /// The handshake's Digest challenge, reused to authorize keepalives.
    challenge: Option<DigestChallenge>,
    session: String,
    channel: u8,
    cseq: u32,
    next_keepalive: Instant,
    decoder: Option<Decoder>,
    decoder_started: bool,
    pacing: PreviewPacing,
    sps: Vec<u8>,
    pps: Vec<u8>,
    started: bool,
    access_unit: Vec<Vec<u8>>,
    fu_buffer: Option<Vec<u8>>,
    deadline: Option<Instant>,
}

/// One complete H.264 picture as received from the camera. RTP packet bytes
/// are retained verbatim for WebRTC forwarding, while an Annex-B copy feeds
/// browser decoders. JPEG output is present only while the bounded software
/// preview decoder is enabled, and is skipped for pictures that arrive while
/// the reader is catching up with the camera.
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

    /// Makes `next_access_unit` fail with `TimedOut` once `deadline` passes,
    /// even while packets keep arriving.
    pub fn set_deadline(&mut self, deadline: Option<Instant>) {
        self.deadline = deadline;
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
            .then(|| preview_decoder().map_err(io::Error::other))
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
        let mut total = 0usize;
        loop {
            if self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(io::ErrorKind::TimedOut.into());
            }
            let packet = self.next_rtp_packet()?;
            let Some((marker, payload)) = parse_rtp(&packet) else {
                continue;
            };
            let timestamp = u32::from_be_bytes([packet[4], packet[5], packet[6], packet[7]]);
            total += packet.len();
            if total > MAX_ACCESS_UNIT_BYTES {
                self.access_unit.clear();
                self.fu_buffer = None;
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "H.264 access unit exceeded the size limit",
                ));
            }
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
                    let encode = self.pacing.admit(Instant::now(), timestamp);
                    match decoder.decode(access_unit) {
                        Ok(Some(yuv)) if encode => Some(encode_jpeg(&yuv)?),
                        Ok(_) => None,
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

    /// Fire-and-forget, like TEARDOWN: a camera whose send buffer is full
    /// (the reader fell behind) may drop the reply, and waiting for it would
    /// discard media forever. `FramedConnection::next_rtp_packet` skips the
    /// late reply, or fails on 454.
    // ponytail: a 401 reply (stale nonce) is skipped too; the session then
    // expires and the stream reconnects.
    fn keepalive_if_due(&mut self) -> io::Result<()> {
        if Instant::now() < self.next_keepalive {
            return Ok(());
        }
        let authorization = self.challenge.as_ref().map(|challenge| {
            digest_header(
                &self.credentials,
                challenge,
                "GET_PARAMETER",
                self.uri.as_str(),
                &cnonce(),
            )
        });
        write_request(
            &mut self.connection,
            rtsp_types::Method::GetParameter,
            &self.uri,
            self.cseq,
            &[(rtsp_types::headers::SESSION, self.session.clone())],
            authorization.as_deref(),
        )
        .map_err(into_io)?;
        self.cseq += 1;
        self.next_keepalive = Instant::now() + KEEPALIVE_INTERVAL;
        Ok(())
    }
}

/// Any request carrying the session refreshes it (RFC 2326), so only
/// `454 Session Not Found` means the session is gone; cameras that do not
/// implement GET_PARAMETER answer 405/501 and are still alive.
fn keepalive_lost_session(status: rtsp_types::StatusCode) -> bool {
    status == rtsp_types::StatusCode::SessionNotFound
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
    let mut challenge = None;
    let describe = authorized_request(
        &mut connection,
        rtsp_types::Method::Describe,
        &uri,
        &mut cseq,
        &credentials,
        &mut challenge,
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
        &mut challenge,
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
        &mut challenge,
        &[
            (rtsp_types::headers::SESSION, session.clone()),
            (rtsp_types::headers::RANGE, "npt=0.000-".to_owned()),
        ],
    )?;
    expect_ok(&play, "PLAY")?;

    let decoder = decode
        .then(preview_decoder)
        .transpose()
        .map_err(protocol_error)?;

    Ok(H264Stream {
        connection,
        shutdown,
        uri,
        credentials,
        challenge,
        session,
        channel,
        cseq,
        next_keepalive: Instant::now() + KEEPALIVE_INTERVAL,
        decoder,
        decoder_started: false,
        pacing: PreviewPacing::default(),
        sps: track.sps,
        pps: track.pps,
        started: false,
        access_unit: Vec::new(),
        fu_buffer: None,
        deadline: None,
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
    challenge: &mut Option<DigestChallenge>,
    extra_headers: &[(rtsp_types::HeaderName, String)],
) -> Result<rtsp_types::Response<Vec<u8>>, RtspError> {
    let response = send_request(connection, method.clone(), uri, *cseq, extra_headers, None)?;
    *cseq += 1;
    if response.status() != rtsp_types::StatusCode::Unauthorized {
        return Ok(response);
    }
    let latest = digest_challenge(&response).ok_or(RtspError::DigestRequired)?;
    let method_name: &str = (&method).into();
    let authorization = digest_header(credentials, &latest, method_name, uri.as_str(), &cnonce());
    *challenge = Some(latest);
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

/// The first MD5 Digest challenge among a 401's `WWW-Authenticate` values.
/// Cameras may send Basic and Digest as separate headers in either order
/// (Tapo sends Basic first).
fn digest_challenge(response: &rtsp_types::Response<Vec<u8>>) -> Option<DigestChallenge> {
    response
        .headers()
        .filter(|(name, _)| **name == rtsp_types::headers::WWW_AUTHENTICATE)
        .find_map(|(_, value)| DigestChallenge::parse(value.as_str()))
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
        } else if let Some(value) = line.strip_prefix("a=fmtp:")
            && let Some(sets_start) = value.find("sprop-parameter-sets=")
        {
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
    if rtpmaps > 0 && !h264 {
        return Err(RtspError::UnsupportedCodec);
    }
    Ok(VideoTrack {
        control: control.ok_or_else(missing)?,
        sps,
        pps,
    })
}

impl H264Stream {
    pub(crate) fn next_jpeg_frame(&mut self) -> io::Result<Vec<u8>> {
        loop {
            if let Some(jpeg) = self.next_access_unit()?.jpeg {
                return Ok(jpeg);
            }
        }
    }

    fn push_nal(&mut self, nal: Vec<u8>) {
        if nal.is_empty() {
            return;
        }
        match nal[0] & 0x1f {
            7 => self.sps = nal.clone(),
            8 => self.pps = nal.clone(),
            _ => {}
        }
        self.access_unit.push(nal);
    }

    /// Reassembles H.264 NAL units from one RTP payload per RFC 6184: single
    /// NAL unit packets (types 1-23) pass through as-is, STAP-A (24)
    /// aggregates several small NALs, and FU-A (28) fragments one large NAL
    /// across packets. Other payload types (STAP-B/MTAP/FU-B) don't appear
    /// in Bambu's stream and are skipped.
    fn depacketize(&mut self, payload: &[u8]) {
        let Some(&first) = payload.first() else {
            return;
        };
        match first & 0x1f {
            1..=23 => self.push_nal(payload.to_vec()),
            24 => {
                let mut rest = &payload[1..];
                while rest.len() >= 2 {
                    let size = u16::from_be_bytes([rest[0], rest[1]]) as usize;
                    rest = &rest[2..];
                    if rest.len() < size {
                        break;
                    }
                    self.push_nal(rest[..size].to_vec());
                    rest = &rest[size..];
                }
            }
            28 => {
                if payload.len() < 2 {
                    return;
                }
                let fu_indicator = payload[0];
                let fu_header = payload[1];
                let start = fu_header & 0x80 != 0;
                let end = fu_header & 0x40 != 0;
                let original_type = fu_header & 0x1f;
                let data = &payload[2..];
                if start {
                    let nal_header = (fu_indicator & 0xe0) | original_type;
                    let mut buffer = Vec::with_capacity(1 + data.len());
                    buffer.push(nal_header);
                    buffer.extend_from_slice(data);
                    self.fu_buffer = Some(buffer);
                } else if let Some(buffer) = &mut self.fu_buffer {
                    buffer.extend_from_slice(data);
                }
                if end && let Some(buffer) = self.fu_buffer.take() {
                    self.push_nal(buffer);
                }
            }
            _ => {}
        }
    }

    /// Waits for the first keyframe and produces Annex-B access units. Cached
    /// SPS/PPS are repeated before every IDR so a decoder joining an existing
    /// owner can start without forcing a second printer connection.
    fn build_access_unit(&mut self) -> Option<(Vec<u8>, bool)> {
        let nals = std::mem::take(&mut self.access_unit);
        let has_idr = nals.iter().any(|nal| !nal.is_empty() && nal[0] & 0x1f == 5);
        if !self.started && !has_idr {
            return None;
        }
        let output: Vec<Vec<u8>> = if has_idr {
            if self.sps.is_empty() || self.pps.is_empty() {
                return None;
            }
            let mut output = Vec::with_capacity(nals.len() + 2);
            output.push(self.sps.clone());
            output.push(self.pps.clone());
            for nal in nals {
                let nal_type = nal.first().map(|byte| byte & 0x1f);
                if matches!(nal_type, Some(7) | Some(8)) {
                    continue;
                }
                output.push(nal);
            }
            self.started = true;
            output
        } else {
            nals
        };

        if output.is_empty() {
            return None;
        }
        let mut data = Vec::new();
        for nal in output {
            data.extend_from_slice(&[0, 0, 0, 1]);
            data.extend_from_slice(&nal);
        }
        Some((data, has_idr))
    }
}

fn parse_rtp(packet: &[u8]) -> Option<(bool, &[u8])> {
    if packet.len() < 12 || packet[0] >> 6 != 2 {
        return None;
    }
    let padding = packet[0] & 0x20 != 0;
    let extension = packet[0] & 0x10 != 0;
    let csrc_count = (packet[0] & 0x0f) as usize;
    let marker = packet[1] & 0x80 != 0;

    let mut offset = 12 + csrc_count * 4;
    if extension {
        if packet.len() < offset + 4 {
            return None;
        }
        let extension_len = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]) as usize;
        offset += 4 + extension_len * 4;
    }
    if offset > packet.len() {
        return None;
    }
    let mut payload = &packet[offset..];
    if padding {
        let pad = *payload.last()? as usize;
        if pad > 0 && pad <= payload.len() {
            payload = &payload[..payload.len() - pad];
        }
    }
    Some((marker, payload))
}

/// The crate's default flush after every decode corrupts OpenH264 on
/// streams whose output it buffers (Tapo C510W: `dsOutOfMemory` on the third
/// P-frame of every GOP). Without it, a picture is at most one frame late.
fn preview_decoder() -> Result<Decoder, String> {
    Decoder::with_api_config(
        OpenH264API::from_source(),
        DecoderConfig::new().flush_after_decode(Flush::NoFlush),
    )
    .map_err(|error| format!("H.264 decoder init failed: {error}"))
}

/// Paces the JPEG preview to the camera using RTP timestamps. Encoding can
/// outlast the frame interval (debug builds encode 2K frames at ~2 fps), and
/// encoding every frame then puts the preview further behind each frame.
/// Late pictures are still decoded, since later P-frames reference them, but
/// they are not encoded until the reader catches up.
#[derive(Default)]
struct PreviewPacing {
    /// When the last picture would have arrived had the reader never fallen
    /// behind, and that picture's RTP timestamp.
    on_time: Option<(Instant, u32)>,
    last_jpeg: Option<Instant>,
}

impl PreviewPacing {
    /// Above usual Wi-Fi jitter and the time to send a large keyframe.
    const MAX_LATENESS: Duration = Duration::from_millis(250);
    /// Keeps a preview alive if a camera's timestamps are broken.
    const MAX_GAP: Duration = Duration::from_secs(1);

    /// Records a picture's arrival; true if it should be encoded.
    fn admit(&mut self, now: Instant, timestamp: u32) -> bool {
        let on_time = match self.on_time {
            Some((at, last)) => {
                // H.264's RTP clock is 90 kHz. A backwards step (a reset, or
                // B-frame reordering) counts as no time.
                let ticks = u64::try_from(timestamp.wrapping_sub(last) as i32).unwrap_or(0);
                let media = Duration::from_micros(ticks * 100 / 9);
                // Allows the camera's clock to run up to 1 ms/s slow.
                (at + media + media / 1000).min(now)
            }
            None => now,
        };
        self.on_time = Some((on_time, timestamp));
        let encode = now - on_time <= Self::MAX_LATENESS
            || self
                .last_jpeg
                .is_none_or(|last| now - last >= Self::MAX_GAP);
        if encode {
            self.last_jpeg = Some(now);
        }
        encode
    }
}

fn encode_jpeg(yuv: &openh264::decoder::DecodedYUV) -> io::Result<Vec<u8>> {
    let (width, height) = yuv.dimensions();
    let mut rgb = vec![0u8; width * height * 3];
    yuv.write_rgb8(&mut rgb);
    let mut jpeg = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut jpeg, 80);
    encoder
        .encode(
            &rgb,
            width as u16,
            height as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .map_err(|error| io::Error::other(format!("JPEG encode failed: {error}")))?;
    Ok(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(100);
    const FRAME_TICKS: u32 = 9_000;

    /// Frames arrive every 100 ms and encoding one takes 400 ms: the reader
    /// skips the pictures queued meanwhile instead of falling further behind.
    #[test]
    fn preview_skips_pictures_that_arrive_late() {
        let start = Instant::now();
        let mut pacing = PreviewPacing::default();
        let mut now = start;
        let mut encoded = Vec::new();
        for frame in 0..20 {
            now = now.max(start + FRAME * frame);
            if pacing.admit(now, frame * FRAME_TICKS) {
                encoded.push(frame);
                now += Duration::from_millis(400);
            } else {
                now += Duration::from_millis(1);
            }
        }
        assert_eq!(encoded, [0, 2, 6, 10, 14, 18]);
        assert!(now - (start + FRAME * 19) < Duration::from_millis(650));
    }

    #[test]
    fn preview_counts_timestamps_across_the_wrap() {
        let start = Instant::now();
        let mut pacing = PreviewPacing::default();
        let mut timestamp = u32::MAX - FRAME_TICKS;
        for frame in 0..20 {
            assert!(pacing.admit(start + FRAME * frame, timestamp), "{frame}");
            timestamp = timestamp.wrapping_add(FRAME_TICKS);
        }
    }

    /// A camera whose timestamps never advance still gets a picture a second.
    #[test]
    fn preview_encodes_a_picture_a_second_while_late() {
        let start = Instant::now();
        let mut pacing = PreviewPacing::default();
        let admitted: Vec<u32> = (0..30)
            .filter(|&frame| pacing.admit(start + FRAME * frame, 0))
            .collect();
        assert_eq!(admitted, [0, 1, 2, 12, 22]);
    }

    #[test]
    fn keepalive_only_454_loses_the_session() {
        for (status, lost) in [
            (rtsp_types::StatusCode::Ok, false),
            (rtsp_types::StatusCode::MethodNotAllowed, false),
            (rtsp_types::StatusCode::NotImplemented, false),
            (rtsp_types::StatusCode::MethodNotValidInThisState, false),
            (rtsp_types::StatusCode::SessionNotFound, true),
        ] {
            assert_eq!(keepalive_lost_session(status), lost, "{status:?}");
        }
    }

    struct Wire {
        input: io::Cursor<Vec<u8>>,
        output: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    }

    impl Read for Wire {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Wire {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.output.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// A playing stream whose keepalive is due, reading `input`.
    fn playing(input: Vec<u8>) -> (H264Stream, std::sync::Arc<std::sync::Mutex<Vec<u8>>>) {
        let output = std::sync::Arc::default();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let shutdown = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let stream = H264Stream {
            connection: FramedConnection {
                connection: Box::new(Wire {
                    input: io::Cursor::new(input),
                    output: std::sync::Arc::clone(&output),
                }),
                buffer: Vec::new(),
            },
            shutdown,
            uri: rtsp_types::Url::parse("rtsp://192.0.2.20:554/stream1").unwrap(),
            credentials: Credentials {
                username: "melt".into(),
                password: "secret".into(),
                realm: None,
            },
            challenge: DigestChallenge::parse(
                "Digest realm=\"TP-Link IP-Camera\", nonce=\"n0nce\"",
            ),
            session: "s1".into(),
            channel: 0,
            cseq: 5,
            next_keepalive: Instant::now(),
            decoder: None,
            decoder_started: false,
            pacing: PreviewPacing::default(),
            sps: Vec::new(),
            pps: Vec::new(),
            started: false,
            access_unit: Vec::new(),
            fu_buffer: None,
            deadline: None,
        };
        (stream, output)
    }

    fn rtp_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![b'$', 0];
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    /// A camera that falls behind may drop the keepalive reply; waiting for
    /// it used to discard media forever (Tapo C510W, high stream).
    #[test]
    fn keepalive_does_not_wait_for_its_reply() {
        let (mut stream, output) = playing(rtp_frame(b"media"));
        assert_eq!(stream.next_rtp_packet().unwrap(), b"media");
        let sent = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(sent.starts_with("GET_PARAMETER rtsp://192.0.2.20:554/stream1 RTSP/1.0"));
        assert!(sent.contains("CSeq: 5"));
        assert!(sent.contains("Session: s1"));
        assert!(sent.contains("Authorization: Digest username=\"melt\""));
        assert!(stream.next_keepalive > Instant::now());
    }

    #[test]
    fn a_late_keepalive_reply_is_skipped_unless_the_session_is_gone() {
        let mut input = b"RTSP/1.0 200 OK\r\nCSeq: 5\r\n\r\n".to_vec();
        input.extend(rtp_frame(b"media"));
        assert_eq!(playing(input).0.next_rtp_packet().unwrap(), b"media");

        let mut input = b"RTSP/1.0 454 Session Not Found\r\nCSeq: 5\r\n\r\n".to_vec();
        input.extend(rtp_frame(b"media"));
        assert!(playing(input).0.next_rtp_packet().is_err());
    }

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
            digest_header(
                &bambu(),
                &challenge,
                "DESCRIBE",
                "rtsps://printer.local:322/s",
                "c0"
            ),
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
            digest_header(
                &credentials,
                &challenge,
                "DESCRIBE",
                "rtsp://192.0.2.20:554/stream1",
                "c0ffee"
            ),
            format!(
                "Digest username=\"melt\", realm=\"TP-Link IP-Camera\", nonce=\"abc\", uri=\"rtsp://192.0.2.20:554/stream1\", response=\"{response}\", qop=auth, nc=00000001, cnonce=\"c0ffee\", opaque=\"op\""
            )
        );
    }

    /// Tapo C510W (hw 1.0, fw 1.3.5) sends Basic and Digest as separate
    /// headers, Basic first; captured 2026-10-08.
    #[test]
    fn digest_is_found_when_basic_is_sent_as_a_separate_earlier_header() {
        let raw = b"RTSP/1.0 401 Unauthorized\r\nCSeq: 2\r\nWWW-Authenticate: Basic realm=\"TP-Link IP-Camera\"\r\nWWW-Authenticate: Digest realm=\"TP-Link IP-Camera\", nonce=\"n0nce\"\r\n\r\n";
        let (message, _) = rtsp_types::Message::<Vec<u8>>::parse(raw).unwrap();
        let rtsp_types::Message::Response(response) = message else {
            panic!("expected a response");
        };
        let challenge = digest_challenge(&response).expect("Digest challenge");
        assert_eq!(challenge.realm, "TP-Link IP-Camera");
        assert_eq!(challenge.nonce, "n0nce");
        assert!(!challenge.qop_auth);
    }

    #[test]
    fn only_md5_digest_challenges_are_accepted() {
        let cases = [
            ("Basic realm=\"cam\"", false),
            (
                "Digest realm=\"cam\", nonce=\"n\", algorithm=SHA-256",
                false,
            ),
            ("Digest realm=\"cam\", nonce=\"n\", algorithm=MD5", true),
            (
                "Basic realm=\"cam\", Digest realm=\"cam\", nonce=\"n\"",
                true,
            ),
            ("Digest realm=\"cam\", cnonce=\"x\"", false),
        ];
        for (header, accepted) in cases {
            assert_eq!(
                DigestChallenge::parse(header).is_some(),
                accepted,
                "{header}"
            );
        }
    }

    #[test]
    fn track_uri_resolves_relative_absolute_and_aggregate_controls() {
        let base = "rtsp://192.0.2.20:554/stream1/";
        let cases = [
            ("track1", "rtsp://192.0.2.20:554/stream1/track1"),
            (
                "rtsp://192.0.2.20:554/stream1/trackID=1",
                "rtsp://192.0.2.20:554/stream1/trackID=1",
            ),
            ("*", "rtsp://192.0.2.20:554/stream1/"),
        ];
        for (control, expected) in cases {
            assert_eq!(
                track_uri(base, control).unwrap().as_str(),
                expected,
                "{control}"
            );
        }
    }

    #[test]
    fn interleaved_channel_follows_the_setup_response() {
        assert_eq!(
            interleaved_channel(Some("RTP/AVP/TCP;unicast;interleaved=2-3")),
            2
        );
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
        assert!(matches!(
            parse_sdp("m=audio 0 RTP/AVP 8\r\n"),
            Err(RtspError::Protocol(_))
        ));
    }

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
}
