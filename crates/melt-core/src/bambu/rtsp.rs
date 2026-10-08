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
    let shutdown = connection
        .get_ref()
        .try_clone()
        .map_err(CameraError::Stream)?;
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
