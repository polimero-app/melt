//! External IP cameras: profiles, printer links, and media. Drivers form a
//! closed set, like printer drivers; `tapo` is the only one today.

mod profiles;
mod tapo;

pub use profiles::{
    CameraRemoveResult, CameraRequest, CameraSummary, SelectedFeed, create, link, list, profile,
    remove, resolve, select, unlink, update,
};

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
    #[error(
        "camera username must be 1-64 characters without quotes, backslashes, or control characters"
    )]
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

/// Decodes one frame from a new connection. The wait for a frame is bounded
/// by the camera timeout from the start of the call; connect and the RTSP
/// handshake are bounded per read.
pub fn snapshot(camera: &ResolvedCamera, quality: StreamQuality) -> Result<Vec<u8>, CameraError> {
    let deadline = Instant::now() + camera.timeout;
    let mut stream = open_stream(camera, quality, true)?;
    stream.set_deadline(Some(deadline));
    let socket = stream.shutdown_handle().map_err(CameraError::Connect)?;
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(CameraError::Timeout)?;
        socket
            .set_read_timeout(Some(remaining))
            .map_err(CameraError::Connect)?;
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
        io::ErrorKind::InvalidData => {
            CameraError::Stream("camera sent an undecodable frame".into())
        }
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
