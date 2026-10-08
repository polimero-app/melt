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
