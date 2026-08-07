//! RTSPS/H.264 camera transport for Bambu printers whose local camera only
//! serves RTSP-over-TLS on port 322 (H/X-series). The classic MJPEG-over-6000
//! protocol (`super::open_mjpeg_stream`) is not offered by these models at
//! all: the printer replies to that handshake with a fixed rejection packet
//! regardless of its contents, because it never speaks that protocol.
//!
//! This module does the RTSP handshake (DESCRIBE/SETUP/PLAY, LIVE555-style
//! HTTP Digest auth, TCP-interleaved RTP transport), depacketizes H.264 per
//! RFC 6184, decodes it with `openh264`, and re-encodes each frame as JPEG so
//! it can flow through the same MJPEG multipart pipe the classic camera uses.
//! All of this — port, path, digest realm, redirect quirk, and the printer
//! pushing RTP/RTCP before PLAY even completes — was confirmed against a
//! real H2-series printer rather than guessed from a spec.

use std::{
    io::{self, Read, Write},
    net::TcpStream,
    time::{Duration, Instant},
};

use openh264::{decoder::Decoder, formats::YUVSource};
use openssl::ssl::SslStream;

use super::{CameraError, MQTT_USERNAME, Profile, transport};

const RTSP_PORT: u16 = 322;
const RTSP_REALM: &str = "LIVE555 Streaming Media";
const RTSP_PATH: &str = "/streaming/live/1";
/// Bounds how large a buffered-but-unparsed chunk may grow before we give up:
/// real RTSP responses and RTP frames are a few hundred bytes to a few KB, so
/// anything past this is a protocol confusion, not a slow network.
const MAX_BUFFERED: usize = 1 << 20;
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(25);

/// A TCP+TLS connection to the RTSP control/data channel, with the small
/// amount of buffering `rtsp_types::Message::parse` needs to find message
/// boundaries. This is shared by the handshake (which reads `Response`s) and
/// the streaming phase (which reads `Data` frames): this printer's session
/// is `a=type:broadcast`, so RTP/RTCP `Data` can and does arrive interleaved
/// with — even before — the control responses that set it up, and any
/// mis-framed byte here desyncs every following read for good.
struct FramedConnection {
    connection: SslStream<TcpStream>,
    buffer: Vec<u8>,
}

impl FramedConnection {
    fn new(connection: SslStream<TcpStream>) -> Self {
        Self {
            connection,
            buffer: Vec::new(),
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), CameraError> {
        self.connection
            .write_all(bytes)
            .map_err(CameraError::Stream)
    }

    /// Reads and returns the next complete RTSP message, buffering any bytes
    /// received past its end for the next call.
    fn next_message(&mut self) -> Result<rtsp_types::Message<Vec<u8>>, CameraError> {
        let mut chunk = [0u8; 4096];
        loop {
            match rtsp_types::Message::<Vec<u8>>::parse(&self.buffer) {
                Ok((message, consumed)) => {
                    self.buffer.drain(..consumed);
                    return Ok(message);
                }
                Err(rtsp_types::ParseError::Incomplete(_)) => {
                    if self.buffer.len() > MAX_BUFFERED {
                        return Err(stream_error("RTSP stream exceeded the buffering limit"));
                    }
                    let read = self
                        .connection
                        .read(&mut chunk)
                        .map_err(CameraError::Stream)?;
                    if read == 0 {
                        return Err(stream_error("RTSP connection closed unexpectedly"));
                    }
                    self.buffer.extend_from_slice(&chunk[..read]);
                }
                Err(_) => return Err(stream_error("malformed RTSP message")),
            }
        }
    }

    /// Reads messages until a `Response` arrives, silently dropping any
    /// interleaved `Data` frames seen along the way (harmless media that
    /// arrived early) and failing on an unexpected `Request`.
    fn next_response(&mut self) -> Result<rtsp_types::Response<Vec<u8>>, CameraError> {
        loop {
            match self.next_message()? {
                rtsp_types::Message::Response(response) => return Ok(response),
                rtsp_types::Message::Data(_) => continue,
                rtsp_types::Message::Request(_) => {
                    return Err(stream_error(
                        "RTSP server sent a request, expected a response",
                    ));
                }
            }
        }
    }

    /// Reads messages until an RTP `Data` frame (channel 0) arrives, ignoring
    /// RTCP (channel 1) and any late control `Response`s.
    fn next_rtp_packet(&mut self) -> Result<Vec<u8>, CameraError> {
        loop {
            match self.next_message()? {
                rtsp_types::Message::Data(data) if data.channel_id() == 0 => {
                    return Ok(data.into_body());
                }
                rtsp_types::Message::Data(_) | rtsp_types::Message::Response(_) => continue,
                rtsp_types::Message::Request(_) => {
                    return Err(stream_error(
                        "RTSP server sent an unexpected request mid-stream",
                    ));
                }
            }
        }
    }
}

/// Native H.264/RTP stream from an H/X-series Bambu camera.
///
/// Unlike [`MjpegStream`](super::MjpegStream), this exposes the printer's
/// original RTP payloads so a WebRTC gateway can forward them without
/// decoding and re-encoding every frame.
pub struct H264Stream {
    connection: FramedConnection,
    uri: rtsp_types::Url,
    access_code: String,
    session: String,
    cseq: u32,
    next_keepalive: Instant,
    decoder: Option<Decoder>,
    sps: Vec<u8>,
    pps: Vec<u8>,
    started: bool,
    access_unit: Vec<Vec<u8>>,
    fu_buffer: Option<Vec<u8>>,
}

/// One complete H.264 picture as received from the printer. RTP packet bytes
/// are retained verbatim for WebRTC forwarding; decoded JPEG output is present
/// only when the stream was opened with its bounded preview decoder enabled.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct H264AccessUnit {
    pub rtp_packets: Vec<Vec<u8>>,
    pub jpeg: Option<Vec<u8>>,
}

impl H264Stream {
    pub fn shutdown_handle(&self) -> io::Result<TcpStream> {
        self.connection.connection.get_ref().try_clone()
    }

    pub fn parameter_sets(&self) -> (&[u8], &[u8]) {
        (&self.sps, &self.pps)
    }

    pub fn next_rtp_packet(&mut self) -> io::Result<Vec<u8>> {
        self.keepalive_if_due()
            .and_then(|()| self.connection.next_rtp_packet())
            .map_err(|error| io::Error::other(format!("{error}")))
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
            let jpeg = match (self.build_access_unit(), self.decoder.as_mut()) {
                (Some(access_unit), Some(decoder)) => match decoder.decode(&access_unit) {
                    Ok(Some(yuv)) => Some(encode_jpeg(&yuv)?),
                    Ok(None) => None,
                    Err(error) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("H.264 decode failed: {error}"),
                        ));
                    }
                },
                _ => None,
            };
            return Ok(H264AccessUnit { rtp_packets, jpeg });
        }
    }

    fn keepalive_if_due(&mut self) -> Result<(), CameraError> {
        if Instant::now() < self.next_keepalive {
            return Ok(());
        }
        let response = authorized_request(
            &mut self.connection,
            rtsp_types::Method::GetParameter,
            &self.uri,
            &mut self.cseq,
            &self.access_code,
            &[(rtsp_types::headers::SESSION, self.session.clone())],
        )?;
        if response.status() != rtsp_types::StatusCode::Ok {
            return Err(stream_error("RTSP keepalive was rejected"));
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

pub fn open_h264_stream(
    profile: &Profile,
    access_code: &str,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<H264Stream, CameraError> {
    open_h264_stream_with_decoder(profile, access_code, fingerprint, timeout, false)
}

pub(super) fn open_decoded_h264_stream(
    profile: &Profile,
    access_code: &str,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<H264Stream, CameraError> {
    open_h264_stream_with_decoder(profile, access_code, fingerprint, timeout, true)
}

fn open_h264_stream_with_decoder(
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
    let mut connection = FramedConnection::new(connection);

    let uri_string = format!("rtsps://{}:{RTSP_PORT}{RTSP_PATH}", profile.host());
    let uri =
        rtsp_types::Url::parse(&uri_string).map_err(|_| stream_error("invalid RTSP camera URL"))?;

    let mut cseq = 1u32;
    let describe = authorized_request(
        &mut connection,
        rtsp_types::Method::Describe,
        &uri,
        &mut cseq,
        access_code,
        &[(rtsp_types::headers::ACCEPT, "application/sdp".to_owned())],
    )?;
    if describe.status() != rtsp_types::StatusCode::Ok {
        return Err(stream_error(&format!(
            "RTSP DESCRIBE failed: {}",
            describe.reason_phrase()
        )));
    }
    let content_base = describe
        .header(&rtsp_types::headers::CONTENT_BASE)
        .map(|value| value.as_str().to_owned())
        .unwrap_or(uri_string);
    let sdp = String::from_utf8_lossy(describe.body());
    let (control, sps, pps) =
        parse_sdp(&sdp).ok_or_else(|| stream_error("no usable H.264 track in SDP"))?;
    let setup_uri =
        rtsp_types::Url::parse(&format!("{}/{control}", content_base.trim_end_matches('/')))
            .map_err(|_| stream_error("invalid RTSP track URL"))?;

    let setup = authorized_request(
        &mut connection,
        rtsp_types::Method::Setup,
        &setup_uri,
        &mut cseq,
        access_code,
        &[(
            rtsp_types::headers::TRANSPORT,
            "RTP/AVP/TCP;unicast;interleaved=0-1".to_owned(),
        )],
    )?;
    if setup.status() != rtsp_types::StatusCode::Ok {
        return Err(stream_error(&format!(
            "RTSP SETUP failed: {}",
            setup.reason_phrase()
        )));
    }
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
        .ok_or_else(|| stream_error("RTSP SETUP response missing Session"))?;

    let play = authorized_request(
        &mut connection,
        rtsp_types::Method::Play,
        &uri,
        &mut cseq,
        access_code,
        &[
            (rtsp_types::headers::SESSION, session.clone()),
            (rtsp_types::headers::RANGE, "npt=0.000-".to_owned()),
        ],
    )?;
    if play.status() != rtsp_types::StatusCode::Ok {
        return Err(stream_error(&format!(
            "RTSP PLAY failed: {}",
            play.reason_phrase()
        )));
    }

    let decoder = decode
        .then(|| {
            Decoder::new()
                .map_err(|error| stream_error(&format!("H.264 decoder init failed: {error}")))
        })
        .transpose()?;

    Ok(H264Stream {
        connection,
        uri,
        access_code: access_code.to_owned(),
        session,
        cseq,
        next_keepalive: Instant::now() + KEEPALIVE_INTERVAL,
        decoder,
        sps,
        pps,
        started: false,
        access_unit: Vec::new(),
        fu_buffer: None,
    })
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

/// Sends a request, and if challenged with a LIVE555-style Digest
/// `401 Unauthorized`, retries once with the computed credentials. Rather
/// than trusting a nonce to stay valid across DESCRIBE/SETUP/PLAY, every
/// request goes through this same challenge/response dance independently:
/// one extra round trip per request, but no shared-nonce assumptions to get
/// wrong against a printer's undocumented protocol.
fn authorized_request(
    connection: &mut FramedConnection,
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: &mut u32,
    access_code: &str,
    extra_headers: &[(rtsp_types::HeaderName, String)],
) -> Result<rtsp_types::Response<Vec<u8>>, CameraError> {
    let response = send_request(connection, method.clone(), uri, *cseq, extra_headers, None)?;
    *cseq += 1;
    if response.status() != rtsp_types::StatusCode::Unauthorized {
        return Ok(response);
    }
    let nonce = response
        .header(&rtsp_types::headers::WWW_AUTHENTICATE)
        .and_then(|value| extract_quoted(value.as_str(), "nonce"))
        .ok_or_else(|| stream_error("RTSP server did not provide a digest nonce"))?;
    let method_name: &str = (&method).into();
    let authorization = digest_header(access_code, method_name, uri.as_str(), &nonce);
    let response = send_request(
        connection,
        method,
        uri,
        *cseq,
        extra_headers,
        Some(&authorization),
    )?;
    *cseq += 1;
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
) -> Result<Vec<u8>, CameraError> {
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
        .map_err(|_| stream_error("failed to serialize RTSP request"))?;
    Ok(bytes)
}

fn write_request(
    connection: &mut FramedConnection,
    method: rtsp_types::Method,
    uri: &rtsp_types::Url,
    cseq: u32,
    extra_headers: &[(rtsp_types::HeaderName, String)],
    authorization: Option<&str>,
) -> Result<(), CameraError> {
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
) -> Result<rtsp_types::Response<Vec<u8>>, CameraError> {
    write_request(connection, method, uri, cseq, extra_headers, authorization)?;
    connection.next_response()
}

fn extract_quoted(header: &str, key: &str) -> Option<String> {
    let marker = format!("{key}=\"");
    let start = header.find(&marker)? + marker.len();
    let end = header[start..].find('"')?;
    Some(header[start..start + end].to_owned())
}

fn md5_hex(input: &str) -> String {
    let digest = openssl::hash::hash(openssl::hash::MessageDigest::md5(), input.as_bytes())
        .expect("md5 is always available");
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn digest_header(access_code: &str, method: &str, uri: &str, nonce: &str) -> String {
    let ha1 = md5_hex(&format!("{MQTT_USERNAME}:{RTSP_REALM}:{access_code}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    let response = md5_hex(&format!("{ha1}:{nonce}:{ha2}"));
    format!(
        "Digest username=\"{MQTT_USERNAME}\", realm=\"{RTSP_REALM}\", nonce=\"{nonce}\", uri=\"{uri}\", response=\"{response}\""
    )
}

/// Extracts the `a=control:` and `sprop-parameter-sets` values for the first
/// H.264 video track in an SDP body. Bambu's SDP is small and flat enough
/// that a real SDP parser would be pure overhead for what we need.
fn parse_sdp(sdp: &str) -> Option<(String, Vec<u8>, Vec<u8>)> {
    let video_start = sdp.find("m=video")?;
    let video_section = &sdp[video_start..];
    let video_section = video_section
        .find("\nm=")
        .map(|next| &video_section[..next + 1])
        .unwrap_or(video_section);

    let mut control = None;
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    for line in video_section.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("a=control:") {
            control = Some(value.trim().to_owned());
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
    Some((control?, sps, pps))
}

impl H264Stream {
    pub(super) fn next_jpeg_frame(&mut self) -> io::Result<Vec<u8>> {
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
                if end {
                    if let Some(buffer) = self.fu_buffer.take() {
                        self.push_nal(buffer);
                    }
                }
            }
            _ => {}
        }
    }

    /// Prepends the cached SPS/PPS to the first access unit that actually
    /// contains a keyframe (the decoder cannot start mid-GOP without them),
    /// then streams subsequent access units through unchanged — matching
    /// the approach the reference Go implementation uses for this printer.
    fn build_access_unit(&mut self) -> Option<Vec<u8>> {
        let nals = std::mem::take(&mut self.access_unit);
        let has_idr = nals.iter().any(|nal| !nal.is_empty() && nal[0] & 0x1f == 5);

        let output: Vec<Vec<u8>> = if self.started {
            nals
        } else {
            if !has_idr || self.sps.is_empty() || self.pps.is_empty() {
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
        };

        if output.is_empty() {
            return None;
        }
        let mut data = Vec::new();
        for nal in output {
            data.extend_from_slice(&[0, 0, 0, 1]);
            data.extend_from_slice(&nal);
        }
        Some(data)
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

    #[test]
    fn teardown_is_serialized_without_waiting_for_a_response() {
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
    fn digest_response_matches_the_live555_scheme() {
        let header = digest_header("12345678", "DESCRIBE", "rtsps://printer.local:322/s", "n0nce");
        assert!(header.contains("username=\"bblp\""));
        assert!(header.contains("realm=\"LIVE555 Streaming Media\""));
        assert!(header.contains("nonce=\"n0nce\""));
        assert_eq!(extract_quoted(&header, "nonce").as_deref(), Some("n0nce"));
    }
}
