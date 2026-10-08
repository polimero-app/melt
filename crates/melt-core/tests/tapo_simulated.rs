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
    spawn_camera_with(challenge, send_picture)
}

fn spawn_camera_with(
    challenge: &'static str,
    play: fn(&mut TcpStream),
) -> (u16, JoinHandle<Vec<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        serve(socket, challenge, play)
    });
    (port, handle)
}

fn serve(socket: TcpStream, challenge: &str, play: fn(&mut TcpStream)) -> Vec<Vec<String>> {
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
                play(&mut writer);
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
            let mut packet = vec![
                0x80,
                marker | 96,
                repeat,
                index as u8,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                1,
            ];
            packet.extend_from_slice(nal);
            interleaved(writer, 2, &packet);
        }
    }
}

/// Streams RTP packets with the marker bit clear until the client goes away.
fn send_markerless(writer: &mut TcpStream) {
    for sequence in 0..u16::MAX {
        let mut packet = vec![
            0x80,
            96,
            (sequence >> 8) as u8,
            sequence as u8,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            1,
        ];
        packet.extend_from_slice(&[0x41, 0x9a, 0x00, 0x00]);
        let mut frame = vec![b'$', 2];
        frame.extend_from_slice(&(packet.len() as u16).to_be_bytes());
        frame.extend_from_slice(&packet);
        if writer.write_all(&frame).is_err() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
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
    let end = nal
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
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
    assert!(
        lines.contains(&format!("SETUP rtsp://127.0.0.1:{port}/stream1/track1 RTSP/1.0").as_str())
    );
}

#[test]
fn undecoded_stream_yields_native_annex_b_starting_at_a_keyframe() {
    let (port, server) = spawn_camera(DIGEST);
    let mut stream = cameras::open_stream(
        &camera(port, "secret", Duration::from_secs(5)),
        StreamQuality::High,
        false,
    )
    .unwrap();
    let unit = stream.next_access_unit().unwrap();
    drop(stream);
    server.join().unwrap();

    assert!(unit.keyframe);
    assert!(unit.jpeg.is_none(), "native passthrough must not decode");
    let annex_b = unit.annex_b.unwrap();
    // Annex-B start code, then the in-band SPS (NAL type 7) the camera sent.
    assert_eq!(&annex_b[..4], &[0, 0, 0, 1]);
    assert_eq!(annex_b[4] & 0x1f, 7);
}

#[test]
fn low_quality_uses_the_second_stream() {
    let (port, server) = spawn_camera(DIGEST);
    cameras::snapshot(
        &camera(port, "secret", Duration::from_secs(5)),
        StreamQuality::Low,
    )
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

#[test]
fn markerless_stream_cannot_outlive_the_deadline() {
    let (port, server) = spawn_camera_with(DIGEST, send_markerless);
    let started = Instant::now();
    let result = cameras::snapshot(
        &camera(port, "secret", Duration::from_secs(1)),
        StreamQuality::High,
    );
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(server);
}
