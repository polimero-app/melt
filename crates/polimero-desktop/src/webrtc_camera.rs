use std::{
    net::{Shutdown, TcpStream},
    sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc},
    thread,
    time::Duration,
};

use base64::Engine;
use polimero_core::{bambu::H264Stream, drivers};
use tokio::runtime::Builder;
use tokio::sync::mpsc as async_mpsc;
use webrtc::{
    api::{APIBuilder, interceptor_registry::register_default_interceptors, media_engine::{MIME_TYPE_H264, MediaEngine}},
    interceptor::registry::Registry,
    peer_connection::{configuration::RTCConfiguration, sdp::session_description::RTCSessionDescription},
    rtp::packet::Packet,
    rtp_transceiver::rtp_codec::RTCRtpCodecCapability,
    track::track_local::{TrackLocal, TrackLocalWriter, track_local_static_rtp::TrackLocalStaticRTP},
    util::Unmarshal,
};

const SETUP_TIMEOUT: Duration = Duration::from_secs(15);

pub struct Session {
    stop: Arc<AtomicBool>,
    shutdown: Option<TcpStream>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(socket) = self.shutdown.take() {
            let _ = socket.shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn start(
    profile: drivers::Profile,
    access_code: Option<String>,
    fingerprint: Option<String>,
    offer_sdp: String,
) -> Result<(String, Session), String> {
    let stream = drivers::camera_h264_stream(
        &profile,
        access_code.as_deref(),
        fingerprint.as_deref(),
        SETUP_TIMEOUT,
    )
    .map_err(|error| error.to_string())?;
    let shutdown = stream
        .shutdown_handle()
        .map_err(|error| format!("camera shutdown handle: {error}"))?;
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
        let result = runtime.block_on(serve(
            stream,
            offer_sdp,
            Arc::clone(&thread_stop),
            answer_tx,
        ));
        let _ = result;
    });

    let answer = answer_rx
        .recv_timeout(SETUP_TIMEOUT)
        .map_err(|_| "WebRTC negotiation timed out".to_owned())??;
    Ok((
        answer,
        Session {
            stop,
            shutdown: Some(shutdown),
            thread: Some(thread),
        },
    ))
}

async fn serve(
    stream: H264Stream,
    offer_sdp: String,
    stop: Arc<AtomicBool>,
    answer_tx: mpsc::SyncSender<Result<String, String>>,
) -> Result<(), String> {
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
        codec_capability(&stream),
        "video".to_owned(),
        "bambu-camera".to_owned(),
    ));
    peer.add_track(Arc::clone(&track) as Arc<dyn TrackLocal + Send + Sync>)
        .await
        .map_err(|error| format!("WebRTC track: {error}"))?;

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
    let _ = gathering_complete.recv().await;
    let answer = peer
        .local_description()
        .await
        .ok_or_else(|| "WebRTC local description missing".to_owned())?;

    answer_tx
        .send(Ok(answer.sdp))
        .map_err(|_| "WebRTC signaling request canceled".to_owned())?;

    let (packet_tx, mut packet_rx) = async_mpsc::channel(4);
    let reader_stop = Arc::clone(&stop);
    tokio::task::spawn_blocking(move || {
        let mut stream = stream;
        while !reader_stop.load(Ordering::Acquire) {
            let packet = match stream.next_rtp_packet() {
                Ok(packet) => packet,
                Err(_) => break,
            };
            if packet_tx.blocking_send(packet).is_err() {
                break;
            }
        }
    });

    while let Some(raw) = packet_rx.recv().await {
        if stop.load(Ordering::Acquire) {
            break;
        }
        let mut raw = &raw[..];
        let packet = Packet::unmarshal(&mut raw)
            .map_err(|error| format!("RTP packet: {error}"))?;
        track
            .write_rtp(&packet)
            .await
            .map_err(|error| format!("WebRTC RTP: {error}"))?;
    }
    peer.close()
        .await
        .map_err(|error| format!("WebRTC close: {error}"))?;
    Ok(())
}

fn codec_capability(stream: &H264Stream) -> RTCRtpCodecCapability {
    let (sps, pps) = stream.parameter_sets();
    let fmtp = match (sps.get(1..4), (!sps.is_empty() && !pps.is_empty())) {
        (Some(profile), true) => format!(
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id={:02x}{:02x}{:02x};sprop-parameter-sets={},{}",
            profile[0],
            profile[1],
            profile[2],
            base64::engine::general_purpose::STANDARD.encode(sps),
            base64::engine::general_purpose::STANDARD.encode(pps),
        ),
        _ => "level-asymmetry-allowed=1;packetization-mode=1".to_owned(),
    };
    RTCRtpCodecCapability {
        mime_type: MIME_TYPE_H264.to_owned(),
        clock_rate: 90_000,
        sdp_fmtp_line: fmtp,
        ..Default::default()
    }
}
