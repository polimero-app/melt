use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use base64::Engine;
use melt_core::bambu::{CameraFrame, CameraSubscription};
use tokio::runtime::Builder;
use tokio::sync::watch;
use webrtc::{
    api::{
        APIBuilder,
        interceptor_registry::register_default_interceptors,
        media_engine::{MIME_TYPE_H264, MediaEngine},
    },
    interceptor::registry::Registry,
    peer_connection::{
        configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription,
    },
    rtp::packet::Packet,
    rtp_transceiver::rtp_codec::{RTCRtpCodecCapability, RTCRtpCodecParameters},
    track::track_local::{
        TrackLocal, TrackLocalWriter, track_local_static_rtp::TrackLocalStaticRTP,
    },
    util::Unmarshal,
};

const SETUP_TIMEOUT: Duration = Duration::from_secs(15);

pub struct Session {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start(
    subscription: CameraSubscription,
    offer_sdp: String,
) -> Result<(String, Session), String> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (answer_tx, answer_rx) = mpsc::sync_channel(1);
    let thread = thread::spawn(move || {
        let runtime = match Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = answer_tx.send(Err(format!("WebRTC runtime: {error}")));
                return;
            }
        };
        let failure_tx = answer_tx.clone();
        let result = runtime.block_on(serve(
            subscription,
            offer_sdp,
            Arc::clone(&thread_stop),
            answer_tx,
        ));
        if let Err(error) = result {
            let _ = failure_tx.send(Err(error));
        }
    });

    let session = Session {
        stop,
        thread: Some(thread),
    };
    await_answer(answer_rx, session, SETUP_TIMEOUT)
}

fn await_answer(
    answer_rx: mpsc::Receiver<Result<String, String>>,
    session: Session,
    timeout: Duration,
) -> Result<(String, Session), String> {
    let answer = answer_rx
        .recv_timeout(timeout)
        .map_err(|_| "WebRTC negotiation timed out".to_owned())??;
    Ok((answer, session))
}

async fn serve(
    subscription: CameraSubscription,
    offer_sdp: String,
    stop: Arc<AtomicBool>,
    answer_tx: mpsc::SyncSender<Result<String, String>>,
) -> Result<(), String> {
    let parameters = subscription
        .h264_parameters()
        .ok_or_else(|| "camera owner is not an H.264 source".to_owned())?;
    let sps = parameters.sps.to_vec();
    let pps = parameters.pps.to_vec();
    let mut timestamp_repair = RtpTimestampRepair::new(subscription.repair_rtp_timestamps());
    let (codec, payload_type) = codec_capability(&sps, &pps, &offer_sdp)?;
    let mut media_engine = MediaEngine::default();
    media_engine
        .register_default_codecs()
        .map_err(|error| format!("WebRTC codecs: {error}"))?;
    let registry = register_default_interceptors(Registry::new(), &mut media_engine)
        .map_err(|error| format!("WebRTC interceptors: {error}"))?;
    let api = APIBuilder::new()
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .build();
    let peer = Arc::new(
        api.new_peer_connection(RTCConfiguration::default())
            .await
            .map_err(|error| format!("WebRTC peer: {error}"))?,
    );
    let track = Arc::new(TrackLocalStaticRTP::new(
        codec.clone(),
        "video".to_owned(),
        "bambu-camera".to_owned(),
    ));
    let sender = peer
        .add_track(Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .map_err(|error| format!("WebRTC track: {error}"))?;
    let transceiver = peer
        .get_transceivers()
        .await
        .into_iter()
        .last()
        .ok_or_else(|| "WebRTC video transceiver missing".to_owned())?;
    transceiver
        .set_codec_preferences(vec![RTCRtpCodecParameters {
            capability: codec,
            payload_type,
            ..Default::default()
        }])
        .await
        .map_err(|error| format!("WebRTC H.264 profile: {error}"))?;
    let rtcp_stop = Arc::clone(&stop);
    let rtcp_task = tokio::spawn(async move {
        while !rtcp_stop.load(Ordering::Acquire) {
            match tokio::time::timeout(Duration::from_secs(1), sender.read_rtcp()).await {
                Ok(Ok(_)) | Err(_) => {}
                Ok(Err(_)) => break,
            }
        }
    });

    let offer = RTCSessionDescription::offer(offer_sdp)
        .map_err(|error| format!("WebRTC offer: {error}"))?;
    peer.set_remote_description(offer)
        .await
        .map_err(|error| format!("WebRTC remote description: {error}"))?;
    let answer = peer
        .create_answer(None)
        .await
        .map_err(|error| format!("WebRTC answer: {error}"))?;
    let mut gathering_complete = peer.gathering_complete_promise().await;
    peer.set_local_description(answer)
        .await
        .map_err(|error| format!("WebRTC local description: {error}"))?;
    tokio::time::timeout(SETUP_TIMEOUT, gathering_complete.recv())
        .await
        .map_err(|_| "WebRTC ICE gathering timed out".to_owned())?
        .ok_or_else(|| "WebRTC ICE gathering canceled".to_owned())?;
    let answer = peer
        .local_description()
        .await
        .ok_or_else(|| "WebRTC local description missing".to_owned())?;

    answer_tx
        .send(Ok(answer.sdp))
        .map_err(|_| "WebRTC signaling request canceled".to_owned())?;

    let (access_unit_tx, mut access_unit_rx) = watch::channel(None);
    let reader_stop = Arc::clone(&stop);
    tokio::task::spawn_blocking(move || {
        while !reader_stop.load(Ordering::Acquire) {
            let frame = match subscription.recv_timeout(Duration::from_secs(1)) {
                Ok(frame) => frame,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if let CameraFrame::H264(access_unit) = frame.as_ref() {
                if access_unit_tx.send(Some(Arc::clone(access_unit))).is_err() {
                    break;
                }
            }
        }
    });

    let mut started = false;
    while access_unit_rx.changed().await.is_ok() {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let Some(access_unit) = access_unit_rx.borrow_and_update().clone() else {
            continue;
        };
        let mut packets = access_unit
            .rtp_packets
            .iter()
            .map(|raw| {
                let mut raw = &raw[..];
                Packet::unmarshal(&mut raw).map_err(|error| format!("RTP packet: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(source_timestamp) = packets.first().map(|packet| packet.header.timestamp) {
            let timestamp = timestamp_repair.timestamp(source_timestamp, Instant::now());
            for packet in &mut packets {
                packet.header.timestamp = timestamp;
            }
        }
        for packet in packets {
            if !started {
                if !is_idr_start(&packet.payload) {
                    continue;
                }
                for (sequence_offset, parameter_set) in [(2, &sps), (1, &pps)] {
                    let mut configuration = packet.clone();
                    configuration.header.marker = false;
                    configuration.header.sequence_number =
                        packet.header.sequence_number.wrapping_sub(sequence_offset);
                    configuration.payload = parameter_set.clone().into();
                    track
                        .write_rtp(&configuration)
                        .await
                        .map_err(|error| format!("WebRTC H.264 configuration: {error}"))?;
                }
                started = true;
            }
            track
                .write_rtp(&packet)
                .await
                .map_err(|error| format!("WebRTC RTP: {error}"))?;
        }
    }
    rtcp_task.abort();
    peer.close()
        .await
        .map_err(|error| format!("WebRTC close: {error}"))?;
    Ok(())
}

struct RtpTimestampRepair {
    enabled: bool,
    last_source: Option<u32>,
    last_output: Option<u32>,
    last_arrival: Option<Instant>,
}

impl RtpTimestampRepair {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_source: None,
            last_output: None,
            last_arrival: None,
        }
    }

    fn timestamp(&mut self, source: u32, arrival: Instant) -> u32 {
        if !self.enabled {
            return source;
        }
        let output = match (self.last_source, self.last_output, self.last_arrival) {
            (Some(last_source), Some(last_output), Some(last_arrival)) => {
                let source_delta = source.wrapping_sub(last_source);
                let ticks = if source_delta > 0 && source_delta < (1 << 31) {
                    source_delta
                } else {
                    let arrival_delta = arrival.saturating_duration_since(last_arrival);
                    ((arrival_delta.as_nanos().saturating_mul(90_000) / 1_000_000_000)
                        .clamp(1, 90_000)) as u32
                };
                last_output.wrapping_add(ticks)
            }
            _ => source,
        };
        self.last_source = Some(source);
        self.last_output = Some(output);
        self.last_arrival = Some(arrival);
        output
    }
}

fn codec_capability(
    sps: &[u8],
    pps: &[u8],
    offer_sdp: &str,
) -> Result<(RTCRtpCodecCapability, u8), String> {
    let camera_profile = sps
        .get(1..3)
        .ok_or_else(|| "camera H.264 SPS is missing its profile".to_owned())?;
    if pps.is_empty() {
        return Err("camera H.264 PPS is missing".to_owned());
    }
    let (payload_type, profile) = compatible_offer_profile(offer_sdp, camera_profile)
        .ok_or_else(|| "webview does not offer the camera H.264 profile".to_owned())?;
    let fmtp = format!(
        "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id={:02x}{:02x}{:02x};sprop-parameter-sets={},{}",
        profile[0],
        profile[1],
        profile[2],
        base64::engine::general_purpose::STANDARD.encode(sps),
        base64::engine::general_purpose::STANDARD.encode(pps),
    );
    Ok((
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            clock_rate: 90_000,
            sdp_fmtp_line: fmtp,
            ..Default::default()
        },
        payload_type,
    ))
}

fn compatible_offer_profile(offer_sdp: &str, camera_profile: &[u8]) -> Option<(u8, [u8; 3])> {
    offer_sdp.lines().find_map(|line| {
        let (payload_type, params) = line.strip_prefix("a=fmtp:")?.split_once(' ')?;
        let payload_type = payload_type.parse().ok()?;
        let packetization_mode = params.split(';').find_map(|param| {
            let (key, value) = param.trim().split_once('=')?;
            (key == "packetization-mode").then_some(value)
        })?;
        if packetization_mode != "1" {
            return None;
        }
        let profile = params.split(';').find_map(|param| {
            let (key, value) = param.trim().split_once('=')?;
            (key == "profile-level-id").then_some(value)
        })?;
        let bytes = decode_profile(profile)?;
        (bytes.len() == 3 && bytes[0] == camera_profile[0] && bytes[1] == camera_profile[1])
            .then_some((payload_type, [bytes[0], bytes[1], bytes[2]]))
    })
}

fn is_idr_start(payload: &[u8]) -> bool {
    let Some(&first) = payload.first() else {
        return false;
    };
    match first & 0x1f {
        5 => true,
        24 => {
            let mut rest = &payload[1..];
            while rest.len() >= 2 {
                let size = u16::from_be_bytes([rest[0], rest[1]]) as usize;
                rest = &rest[2..];
                if rest.len() < size {
                    return false;
                }
                if rest.first().is_some_and(|byte| byte & 0x1f == 5) {
                    return true;
                }
                rest = &rest[size..];
            }
            false
        }
        28 => payload
            .get(1)
            .is_some_and(|header| header & 0x80 != 0 && header & 0x1f == 5),
        _ => false,
    }
}

fn decode_profile(value: &str) -> Option<[u8; 3]> {
    if value.len() != 6 {
        return None;
    }
    let mut bytes = [0; 3];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::{Duration, Instant},
    };

    use super::{
        RtpTimestampRepair, Session, await_answer, compatible_offer_profile, decode_profile,
        is_idr_start,
    };

    #[test]
    fn selects_browser_profile_compatible_with_camera() {
        let offer = "a=fmtp:126 profile-level-id=640c1f;packetization-mode=1\r\na=fmtp:127 profile-level-id=42e01f;packetization-mode=1\r\n";
        assert_eq!(
            compatible_offer_profile(offer, &[0x64, 0x0c]),
            Some((126, [0x64, 0x0c, 0x1f]))
        );
        assert_eq!(
            compatible_offer_profile(offer, &[0x42, 0xe0]),
            Some((127, [0x42, 0xe0, 0x1f]))
        );
        assert_eq!(compatible_offer_profile(offer, &[0x4d, 0x40]), None);
    }

    #[test]
    fn rejects_malformed_profile_ids() {
        assert_eq!(decode_profile("640c"), None);
        assert_eq!(decode_profile("zz0c1f"), None);
        assert_eq!(decode_profile("640c1f"), Some([0x64, 0x0c, 0x1f]));
    }

    #[test]
    fn timeout_stops_and_joins_pending_session() {
        let stop = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_exited = Arc::clone(&exited);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                thread::yield_now();
            }
            thread_exited.store(true, Ordering::Release);
        });
        let (_answer_tx, answer_rx) = mpsc::channel();
        let session = Session {
            stop,
            thread: Some(thread),
        };

        let result = await_answer(answer_rx, session, Duration::from_millis(10));

        assert!(matches!(result, Err(error) if error == "WebRTC negotiation timed out"));
        assert!(exited.load(Ordering::Acquire));
    }

    #[test]
    fn detects_idr_starts_in_supported_h264_packetizations() {
        assert!(is_idr_start(&[0x65, 0x01]));
        assert!(is_idr_start(&[0x7c, 0x85, 0x01]));
        assert!(!is_idr_start(&[0x7c, 0x45, 0x01]));
        assert!(is_idr_start(&[0x78, 0x00, 0x02, 0x65, 0x01]));
        assert!(!is_idr_start(&[0x61, 0x01]));
    }

    #[test]
    fn timestamp_repair_is_inert_without_a_qualified_quirk() {
        let now = Instant::now();
        let mut repair = RtpTimestampRepair::new(false);
        assert_eq!(repair.timestamp(42, now), 42);
        assert_eq!(repair.timestamp(42, now + Duration::from_millis(40)), 42);
    }

    #[test]
    fn timestamp_repair_preserves_valid_deltas_and_repairs_frozen_values() {
        let now = Instant::now();
        let mut repair = RtpTimestampRepair::new(true);
        assert_eq!(repair.timestamp(90_000, now), 90_000);
        assert_eq!(
            repair.timestamp(93_000, now + Duration::from_millis(33)),
            93_000
        );
        assert_eq!(
            repair.timestamp(93_000, now + Duration::from_millis(66)),
            95_970
        );
    }

    #[test]
    fn timestamp_repair_handles_regression_and_source_wraparound() {
        let now = Instant::now();
        let mut regression = RtpTimestampRepair::new(true);
        assert_eq!(regression.timestamp(10_000, now), 10_000);
        assert_eq!(
            regression.timestamp(9_000, now + Duration::from_millis(40)),
            13_600
        );

        let mut wrap = RtpTimestampRepair::new(true);
        assert_eq!(wrap.timestamp(u32::MAX - 10, now), u32::MAX - 10);
        assert_eq!(wrap.timestamp(20, now + Duration::from_millis(1)), 20);
    }
}
