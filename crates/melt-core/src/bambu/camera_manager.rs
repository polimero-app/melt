use std::{
    collections::BTreeMap,
    net::{Shutdown, TcpStream},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak,
        atomic::{AtomicU32, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{
    CameraError, CameraSelection, CameraSelectionSource, CameraTransport, H264AccessUnit,
    H264Stream, MjpegStream, Profile, QuirkEffect, RuntimeCapabilities, applicable_quirks,
    open_classic_mjpeg_stream, open_h264_stream, select_camera_transport,
};
use crate::cameras::{self, ResolvedCamera, StreamQuality};

#[derive(Clone, Debug)]
pub enum CameraFrame {
    Jpeg(Arc<[u8]>),
    H264(Arc<H264AccessUnit>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CameraFrameKind {
    Jpeg,
    H264,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct H264Parameters {
    pub sps: Arc<[u8]>,
    pub pps: Arc<[u8]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CameraOwnerTransport {
    MjpegTls,
    RtspsH264,
    /// Plain RTSP from an external camera.
    RtspH264,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraOwnerStatus {
    pub printer_key: String,
    pub transport: CameraOwnerTransport,
    pub subscribers: usize,
    pub generation: u64,
    pub has_jpeg: bool,
    pub reconnect_attempts: u32,
    pub selection_source: CameraSelectionSource,
    pub rejected_advertisement: Option<String>,
}

enum Source {
    Mjpeg(MjpegStream),
    H264(Box<H264Stream>),
}

impl Source {
    fn shutdown_handle(&self) -> std::io::Result<TcpStream> {
        match self {
            Self::Mjpeg(stream) => stream.shutdown_handle(),
            Self::H264(stream) => stream.shutdown_handle(),
        }
    }
}

struct OwnerState {
    next_subscriber: u64,
    subscribers: BTreeMap<u64, (CameraFrameKind, Arc<FrameMailbox>)>,
    latest_jpeg: Option<(Arc<[u8]>, Instant)>,
    stopped: bool,
}

#[derive(Default)]
struct FrameMailboxState {
    latest: Option<Arc<CameraFrame>>,
    closed: bool,
}

#[derive(Default)]
struct FrameMailbox {
    state: Mutex<FrameMailboxState>,
    available: Condvar,
}

impl FrameMailbox {
    fn publish(&self, frame: Arc<CameraFrame>) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return false;
        }
        state.latest = Some(frame);
        self.available.notify_one();
        true
    }

    fn recv_timeout(&self, timeout: Duration) -> Result<Arc<CameraFrame>, mpsc::RecvTimeoutError> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            if let Some(frame) = state.latest.take() {
                return Ok(frame);
            }
            if state.closed {
                return Err(mpsc::RecvTimeoutError::Disconnected);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(mpsc::RecvTimeoutError::Timeout);
            };
            let (next, _) = self
                .available
                .wait_timeout(state, remaining)
                .unwrap_or_else(|error| error.into_inner());
            state = next;
        }
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        self.available.notify_all();
    }
}

struct Owner {
    printer_key: String,
    revision: [u8; 32],
    generation: u64,
    transport: CameraOwnerTransport,
    parameters: Option<H264Parameters>,
    selection: CameraSelection,
    repair_rtp_timestamps: bool,
    reconnect_attempts: AtomicU32,
    shutdown: Mutex<Option<TcpStream>>,
    state: Mutex<OwnerState>,
}

impl Owner {
    fn subscribe(self: &Arc<Self>, kind: CameraFrameKind) -> CameraSubscription {
        self.try_subscribe(kind)
            .expect("a new camera owner must accept its initial subscriber")
    }

    fn try_subscribe(self: &Arc<Self>, kind: CameraFrameKind) -> Option<CameraSubscription> {
        let receiver = Arc::new(FrameMailbox::default());
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.stopped {
            return None;
        }
        let id = state.next_subscriber;
        state.next_subscriber = state.next_subscriber.wrapping_add(1);
        state.subscribers.insert(id, (kind, Arc::clone(&receiver)));
        Some(CameraSubscription {
            owner: Arc::clone(self),
            id,
            receiver,
        })
    }

    fn publish(&self, frame: CameraFrame) {
        let frame = Arc::new(frame);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let CameraFrame::Jpeg(jpeg) = frame.as_ref() {
            state.latest_jpeg = Some((Arc::clone(jpeg), Instant::now()));
        }
        let kind = match frame.as_ref() {
            CameraFrame::Jpeg(_) => CameraFrameKind::Jpeg,
            CameraFrame::H264(_) => CameraFrameKind::H264,
        };
        state.subscribers.retain(|_, (interest, subscriber)| {
            *interest != kind || subscriber.publish(Arc::clone(&frame))
        });
    }

    fn unsubscribe(&self, id: u64) {
        let should_stop = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.subscribers.remove(&id);
            state.subscribers.is_empty()
        };
        if should_stop {
            self.stop();
        }
    }

    fn stop(&self) {
        let already_stopped = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            std::mem::replace(&mut state.stopped, true)
        };
        if !already_stopped
            && let Some(socket) = self
                .shutdown
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }

    fn finish(&self) {
        self.stop();
        let subscribers = std::mem::take(
            &mut self
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .subscribers,
        );
        for (_, (_, subscriber)) in subscribers {
            subscriber.close();
        }
    }

    fn has_subscribers(&self) -> bool {
        !self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .subscribers
            .is_empty()
    }

    fn has_subscribers_for(&self, kind: CameraFrameKind) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .subscribers
            .values()
            .any(|(interest, _)| *interest == kind)
    }

    fn status(&self) -> CameraOwnerStatus {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        CameraOwnerStatus {
            printer_key: self.printer_key.clone(),
            transport: self.transport,
            subscribers: state.subscribers.len(),
            generation: self.generation,
            has_jpeg: state.latest_jpeg.is_some(),
            reconnect_attempts: self.reconnect_attempts.load(Ordering::Relaxed),
            selection_source: self.selection.source,
            rejected_advertisement: self.selection.rejected_advertisement.clone(),
        }
    }
}

pub struct CameraSubscription {
    owner: Arc<Owner>,
    id: u64,
    receiver: Arc<FrameMailbox>,
}

impl CameraSubscription {
    pub fn transport(&self) -> CameraOwnerTransport {
        self.owner.transport
    }
    pub fn h264_parameters(&self) -> Option<&H264Parameters> {
        self.owner.parameters.as_ref()
    }
    pub fn recv_timeout(
        &self,
        timeout: Duration,
    ) -> Result<Arc<CameraFrame>, mpsc::RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }
    /// The latest decoded frame if it is at most `max_age` old. Frames are
    /// dropped when the upstream fails, so one never outlives its connection.
    pub fn fresh_jpeg(&self, max_age: Duration) -> Option<Arc<[u8]>> {
        self.owner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .latest_jpeg
            .as_ref()
            .filter(|(_, at)| at.elapsed() <= max_age)
            .map(|(jpeg, _)| Arc::clone(jpeg))
    }
    pub fn repair_rtp_timestamps(&self) -> bool {
        self.owner.repair_rtp_timestamps
    }
}

impl Drop for CameraSubscription {
    fn drop(&mut self) {
        self.owner.unsubscribe(self.id);
    }
}

#[derive(Default)]
struct ManagerState {
    generation: u64,
    owners: BTreeMap<String, Weak<Owner>>,
    /// Serializes startups per source so a slow upstream blocks only its
    /// own callers. ponytail: never pruned; bounded by printers + cameras × 2.
    startups: BTreeMap<String, Arc<Mutex<()>>>,
}

#[derive(Clone, Default)]
pub struct CameraManager {
    state: Arc<Mutex<ManagerState>>,
}

enum Reopen {
    Retry,
    Fatal,
}

type Reopener = Box<dyn FnMut() -> Result<Source, Reopen> + Send>;

struct Opened {
    source: Source,
    selection: CameraSelection,
    transport: CameraOwnerTransport,
    repair_rtp_timestamps: bool,
    reopen: Reopener,
}

impl CameraManager {
    pub fn subscribe(
        &self,
        profile: &Profile,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        timeout: Duration,
        kind: CameraFrameKind,
        capabilities: Option<&RuntimeCapabilities>,
    ) -> Result<CameraSubscription, CameraError> {
        let printer_key = format!("bambu:{}", profile.serial().to_ascii_uppercase());
        let defaults;
        let capabilities = match capabilities {
            Some(capabilities) => capabilities,
            None => {
                defaults = profile.default_capabilities();
                &defaults
            }
        };
        let selection = select_camera_transport(profile.host(), capabilities);
        let repair_rtp_timestamps =
            applicable_quirks(&capabilities.identity, &capabilities.firmware)
                .iter()
                .any(|entry| entry.effect == QuirkEffect::RepairRtpTimestampsFromArrivalTime);
        let revision = camera_revision(profile, access_code, fingerprint, capabilities, &selection);
        let open = || {
            let (source, selection) =
                open_source(profile, access_code, fingerprint, timeout, selection)?;
            let (transport, owner_transport) = match &source {
                Source::Mjpeg(_) => (CameraTransport::MjpegTls, CameraOwnerTransport::MjpegTls),
                Source::H264(_) => (CameraTransport::RtspsH264, CameraOwnerTransport::RtspsH264),
            };
            let profile = profile.clone();
            let access_code = access_code.map(str::to_owned);
            let fingerprint = fingerprint.map(str::to_owned);
            let reopen: Reopener = Box::new(move || {
                let deadline = Instant::now().checked_add(timeout).ok_or(Reopen::Fatal)?;
                open_transport(
                    &profile,
                    access_code.as_deref(),
                    fingerprint.as_deref(),
                    deadline,
                    transport,
                )
                .map_err(|error| match error {
                    CameraError::Pin(_) | CameraError::Identity => Reopen::Fatal,
                    _ => Reopen::Retry,
                })
            });
            Ok(Opened {
                source,
                selection,
                transport: owner_transport,
                repair_rtp_timestamps,
                reopen,
            })
        };
        self.subscribe_with(
            printer_key,
            revision,
            kind,
            open,
            || CameraError::UnsupportedMedia,
            CameraError::Stream,
        )
    }

    /// Shares one upstream per camera and quality, like printer feeds.
    pub fn subscribe_external(
        &self,
        camera: ResolvedCamera,
        quality: StreamQuality,
        kind: CameraFrameKind,
    ) -> Result<CameraSubscription, cameras::CameraError> {
        let quality_name = match quality {
            StreamQuality::High => "high",
            StreamQuality::Low => "low",
        };
        let key = format!("camera:{}:{quality_name}", camera.profile.id);
        let port = camera
            .profile
            .rtsp_port
            .unwrap_or(cameras::TAPO_RTSP_PORT)
            .to_be_bytes();
        let mut digest = Sha256::new();
        for part in [
            camera.profile.driver.as_bytes(),
            camera.profile.host.as_bytes(),
            &port,
            camera.profile.username.as_bytes(),
            camera.password.as_bytes(),
            quality_name.as_bytes(),
        ] {
            digest.update(part);
            digest.update([0]);
        }
        let open = move || {
            let source = open_external(&camera, quality)?;
            let reopen: Reopener = Box::new(move || {
                open_external(&camera, quality).map_err(|error| match error {
                    cameras::CameraError::Authentication
                    | cameras::CameraError::DigestRequired
                    | cameras::CameraError::UnsupportedCodec
                    | cameras::CameraError::UnknownDriver(_) => Reopen::Fatal,
                    _ => Reopen::Retry,
                })
            });
            Ok(Opened {
                source,
                selection: CameraSelection {
                    preferred: CameraTransport::Unknown,
                    source: CameraSelectionSource::Unknown,
                    rejected_advertisement: None,
                    quirk_ids: Vec::new(),
                },
                transport: CameraOwnerTransport::RtspH264,
                repair_rtp_timestamps: false,
                reopen,
            })
        };
        self.subscribe_with(
            key,
            digest.finalize().into(),
            kind,
            open,
            || cameras::CameraError::UnsupportedCodec,
            cameras::CameraError::Connect,
        )
    }

    fn subscribe_with<E>(
        &self,
        key: String,
        revision: [u8; 32],
        kind: CameraFrameKind,
        open: impl FnOnce() -> Result<Opened, E>,
        unsupported: impl Fn() -> E,
        stream_error: impl FnOnce(std::io::Error) -> E,
    ) -> Result<CameraSubscription, E> {
        let startup = {
            let mut manager = self.lock();
            if let Some(subscription) = reuse(&mut manager, &key, revision, kind, &unsupported)? {
                return Ok(subscription);
            }
            Arc::clone(manager.startups.entry(key.clone()).or_default())
        };
        let _startup = startup.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(subscription) = reuse(&mut self.lock(), &key, revision, kind, &unsupported)? {
            return Ok(subscription);
        }
        let Opened {
            source,
            selection,
            transport,
            repair_rtp_timestamps,
            reopen,
        } = open()?;
        let shutdown = source.shutdown_handle().map_err(stream_error)?;
        let parameters = match &source {
            Source::Mjpeg(_) => None,
            Source::H264(stream) => {
                let (sps, pps) = stream.parameter_sets();
                Some(H264Parameters {
                    sps: Arc::from(sps),
                    pps: Arc::from(pps),
                })
            }
        };
        if kind == CameraFrameKind::H264 && transport == CameraOwnerTransport::MjpegTls {
            return Err(unsupported());
        }
        let mut manager = self.lock();
        manager.generation = manager.generation.wrapping_add(1);
        let owner = Arc::new(Owner {
            printer_key: key.clone(),
            revision,
            generation: manager.generation,
            transport,
            parameters,
            selection,
            repair_rtp_timestamps,
            reconnect_attempts: AtomicU32::new(0),
            shutdown: Mutex::new(Some(shutdown)),
            state: Mutex::new(OwnerState {
                next_subscriber: 1,
                subscribers: BTreeMap::new(),
                latest_jpeg: None,
                stopped: false,
            }),
        });
        let subscription = owner.subscribe(kind);
        manager.owners.insert(key, Arc::downgrade(&owner));
        drop(manager);
        thread::spawn(move || run_owner(owner, source, reopen));
        Ok(subscription)
    }

    fn lock(&self) -> MutexGuard<'_, ManagerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn statuses(&self) -> Vec<CameraOwnerStatus> {
        let mut manager = self.state.lock().unwrap_or_else(|error| error.into_inner());
        manager.owners.retain(|_, owner| owner.strong_count() > 0);
        manager
            .owners
            .values()
            .filter_map(Weak::upgrade)
            .map(|owner| owner.status())
            .collect()
    }

    pub fn invalidate(&self, printer_key: &str) {
        if let Some(owner) = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .owners
            .remove(printer_key)
            .and_then(|owner| owner.upgrade())
        {
            owner.stop();
        }
    }
}

/// Joins a live owner with the same revision. A stale owner is stopped and
/// forgotten so the caller can start a replacement.
fn reuse<E>(
    manager: &mut ManagerState,
    key: &str,
    revision: [u8; 32],
    kind: CameraFrameKind,
    unsupported: &impl Fn() -> E,
) -> Result<Option<CameraSubscription>, E> {
    let Some(owner) = manager.owners.get(key).and_then(Weak::upgrade) else {
        return Ok(None);
    };
    if owner.revision == revision {
        if kind == CameraFrameKind::H264 && owner.transport == CameraOwnerTransport::MjpegTls {
            return Err(unsupported());
        }
        if let Some(subscription) = owner.try_subscribe(kind) {
            return Ok(Some(subscription));
        }
    }
    owner.stop();
    manager.owners.remove(key);
    Ok(None)
}

fn open_external(
    camera: &ResolvedCamera,
    quality: StreamQuality,
) -> Result<Source, cameras::CameraError> {
    cameras::open_stream(camera, quality, false).map(|stream| Source::H264(Box::new(stream)))
}

fn open_source(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    timeout: Duration,
    selection: CameraSelection,
) -> Result<(Source, CameraSelection), CameraError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| CameraError::Connect(std::io::ErrorKind::TimedOut.into()))?;
    let open = |transport| open_transport(profile, access_code, fingerprint, deadline, transport);
    let security_error =
        |error: &CameraError| matches!(error, CameraError::Pin(_) | CameraError::Identity);
    let (first_transport, second_transport) = match selection.preferred {
        CameraTransport::MjpegTls => (CameraTransport::MjpegTls, CameraTransport::RtspsH264),
        CameraTransport::RtspsH264 | CameraTransport::Unknown => {
            (CameraTransport::RtspsH264, CameraTransport::MjpegTls)
        }
    };
    let first = open(first_transport);
    match first {
        Ok(source) => Ok((source, selection)),
        Err(error) if security_error(&error) => Err(error),
        Err(_) => open(second_transport).map(|source| {
            let preferred = match &source {
                Source::Mjpeg(_) => CameraTransport::MjpegTls,
                Source::H264(_) => CameraTransport::RtspsH264,
            };
            (
                source,
                CameraSelection {
                    preferred,
                    source: CameraSelectionSource::SafeProbe,
                    rejected_advertisement: selection.rejected_advertisement,
                    quirk_ids: selection.quirk_ids,
                },
            )
        }),
    }
}

fn open_transport(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    deadline: Instant,
    transport: CameraTransport,
) -> Result<Source, CameraError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| CameraError::Connect(std::io::ErrorKind::TimedOut.into()))?;
    match transport {
        CameraTransport::RtspsH264 => {
            open_h264_stream(profile, access_code, fingerprint, remaining)
                .map(Box::new)
                .map(Source::H264)
        }
        CameraTransport::MjpegTls => {
            open_classic_mjpeg_stream(profile, access_code, fingerprint, remaining)
                .map(Source::Mjpeg)
        }
        CameraTransport::Unknown => unreachable!("unknown is resolved before probing"),
    }
}

fn camera_revision(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    capabilities: &RuntimeCapabilities,
    selection: &CameraSelection,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(profile.connection_identity(access_code, fingerprint));
    digest.update(serde_json::to_vec(&capabilities.firmware).unwrap_or_default());
    digest.update(serde_json::to_vec(selection).unwrap_or_default());
    digest.finalize().into()
}

fn run_owner(owner: Arc<Owner>, mut source: Source, mut reopen: Reopener) {
    let mut consecutive_failures = 0_u32;
    loop {
        let result = match &mut source {
            Source::Mjpeg(stream) => stream.next_frame().map(|jpeg| {
                owner.publish(CameraFrame::Jpeg(Arc::from(jpeg)));
            }),
            Source::H264(stream) => stream
                .set_preview_decode_enabled(owner.has_subscribers_for(CameraFrameKind::Jpeg))
                .and_then(|()| stream.next_access_unit())
                .map(|access_unit| {
                    if let Some(jpeg) = &access_unit.jpeg {
                        owner.publish(CameraFrame::Jpeg(Arc::from(jpeg.clone())));
                    }
                    owner.publish(CameraFrame::H264(Arc::new(access_unit)));
                }),
        };
        if result.is_ok() {
            consecutive_failures = 0;
            continue;
        }
        let stopped = {
            let mut state = owner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.latest_jpeg = None;
            state.stopped
        };
        if stopped || !owner.has_subscribers() {
            break;
        }
        consecutive_failures = consecutive_failures.saturating_add(1);
        owner.reconnect_attempts.fetch_add(1, Ordering::Relaxed);
        if !wait_for_reconnect(&owner, reconnect_delay(consecutive_failures)) {
            break;
        }
        let replacement = match reopen() {
            Ok(replacement) => replacement,
            Err(Reopen::Fatal) => break,
            Err(Reopen::Retry) => continue,
        };
        if !same_h264_parameters(&owner, &replacement) {
            break;
        }
        let Ok(shutdown) = replacement.shutdown_handle() else {
            break;
        };
        *owner
            .shutdown
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(shutdown);
        source = replacement;
    }
    owner.finish();
}

fn same_h264_parameters(owner: &Owner, source: &Source) -> bool {
    match (owner.parameters.as_ref(), source) {
        (None, Source::Mjpeg(_)) => true,
        (Some(expected), Source::H264(stream)) => {
            let (sps, pps) = stream.parameter_sets();
            expected.sps.as_ref() == sps && expected.pps.as_ref() == pps
        }
        _ => false,
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    Duration::from_millis(250_u64.saturating_mul(1_u64 << attempt.saturating_sub(1).min(4)))
}

fn wait_for_reconnect(owner: &Owner, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if !owner.has_subscribers()
            || owner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .stopped
        {
            return false;
        }
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(50)),
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> Arc<Owner> {
        Arc::new(Owner {
            printer_key: "bambu:SN001".into(),
            revision: [0; 32],
            generation: 1,
            transport: CameraOwnerTransport::MjpegTls,
            parameters: None,
            selection: CameraSelection {
                preferred: CameraTransport::MjpegTls,
                source: CameraSelectionSource::SafeProbe,
                rejected_advertisement: None,
                quirk_ids: Vec::new(),
            },
            repair_rtp_timestamps: false,
            reconnect_attempts: AtomicU32::new(0),
            shutdown: Mutex::new(None),
            state: Mutex::new(OwnerState {
                next_subscriber: 1,
                subscribers: BTreeMap::new(),
                latest_jpeg: None,
                stopped: false,
            }),
        })
    }

    #[test]
    fn fanout_keeps_only_the_latest_frame_and_reuses_the_latest_jpeg() {
        let owner = owner();
        let fast = owner.subscribe(CameraFrameKind::Jpeg);
        let slow = owner.subscribe(CameraFrameKind::Jpeg);
        for value in 0..10 {
            owner.publish(CameraFrame::Jpeg(Arc::from(vec![value])));
        }
        assert_eq!(
            fast.fresh_jpeg(Duration::from_secs(2)).unwrap().as_ref(),
            &[9]
        );
        let fast_frame = fast.recv_timeout(Duration::from_millis(1)).unwrap();
        let CameraFrame::Jpeg(fast_frame) = fast_frame.as_ref() else {
            panic!("expected JPEG frame");
        };
        let slow_frame = slow.recv_timeout(Duration::from_millis(1)).unwrap();
        let CameraFrame::Jpeg(slow_frame) = slow_frame.as_ref() else {
            panic!("expected JPEG frame");
        };
        assert_eq!(fast_frame.as_ref(), &[9]);
        assert_eq!(slow_frame.as_ref(), &[9]);
        assert!(matches!(
            fast.recv_timeout(Duration::from_millis(1)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert_eq!(owner.status().subscribers, 2);
    }

    #[test]
    fn last_subscriber_stops_the_owner() {
        let owner = owner();
        let first = owner.subscribe(CameraFrameKind::Jpeg);
        let second = owner.subscribe(CameraFrameKind::Jpeg);
        drop(first);
        assert!(!owner.state.lock().unwrap().stopped);
        drop(second);
        assert!(owner.state.lock().unwrap().stopped);
    }

    #[test]
    fn stopped_owner_cannot_accept_a_replacement_subscriber() {
        let owner = owner();
        let subscription = owner.subscribe(CameraFrameKind::Jpeg);
        drop(subscription);

        assert!(owner.try_subscribe(CameraFrameKind::Jpeg).is_none());
        assert!(owner.state.lock().unwrap().subscribers.is_empty());
    }

    #[test]
    fn subscribers_receive_only_the_requested_media_kind() {
        let owner = owner();
        let h264 = owner.subscribe(CameraFrameKind::H264);
        owner.publish(CameraFrame::Jpeg(Arc::from(vec![1])));
        assert!(h264.recv_timeout(Duration::from_millis(1)).is_err());
        owner.publish(CameraFrame::H264(Arc::new(H264AccessUnit {
            rtp_packets: vec![vec![2]],
            annex_b: Some(vec![0, 0, 0, 1, 5]),
            keyframe: true,
            jpeg: None,
        })));
        assert!(matches!(
            h264.recv_timeout(Duration::from_millis(1))
                .unwrap()
                .as_ref(),
            CameraFrame::H264(_)
        ));
    }

    #[test]
    fn media_interest_tracks_subscriber_lifetime() {
        let owner = owner();
        let h264 = owner.subscribe(CameraFrameKind::H264);
        assert!(owner.has_subscribers_for(CameraFrameKind::H264));
        assert!(!owner.has_subscribers_for(CameraFrameKind::Jpeg));

        let jpeg = owner.subscribe(CameraFrameKind::Jpeg);
        assert!(owner.has_subscribers_for(CameraFrameKind::Jpeg));
        drop(jpeg);
        assert!(!owner.has_subscribers_for(CameraFrameKind::Jpeg));
        drop(h264);
    }

    fn external(id: &str, port: u16) -> ResolvedCamera {
        ResolvedCamera {
            name: id.into(),
            profile: crate::config::CameraProfile {
                id: id.into(),
                driver: cameras::TAPO.into(),
                host: "127.0.0.1".into(),
                username: "melt".into(),
                rtsp_port: Some(port),
                timeout: "3s".into(),
                created: String::new(),
                updated: String::new(),
            },
            password: "secret".into(),
            timeout: Duration::from_secs(3),
        }
    }

    #[test]
    fn an_unreachable_camera_does_not_stall_another() {
        // The backlog completes connect; DESCRIBE then waits out the read timeout.
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let silent_port = silent.local_addr().unwrap().port();
        let refused = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let manager = CameraManager::default();
        let stalled = {
            let manager = manager.clone();
            thread::spawn(move || {
                manager
                    .subscribe_external(
                        external("a", silent_port),
                        StreamQuality::High,
                        CameraFrameKind::H264,
                    )
                    .is_err()
            })
        };
        thread::sleep(Duration::from_millis(200));
        let started = Instant::now();
        assert!(
            manager
                .subscribe_external(
                    external("b", refused),
                    StreamQuality::High,
                    CameraFrameKind::H264
                )
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(stalled.join().unwrap());
    }

    #[test]
    fn fresh_jpeg_rejects_old_frames() {
        let owner = owner();
        let subscription = owner.subscribe(CameraFrameKind::Jpeg);
        owner.publish(CameraFrame::Jpeg(Arc::from(&b"jpeg"[..])));
        assert!(subscription.fresh_jpeg(Duration::from_secs(2)).is_some());
        assert!(subscription.fresh_jpeg(Duration::ZERO).is_none());
    }

    #[test]
    fn reconnect_backoff_is_bounded() {
        assert_eq!(reconnect_delay(1), Duration::from_millis(250));
        assert_eq!(reconnect_delay(2), Duration::from_millis(500));
        assert_eq!(reconnect_delay(10), Duration::from_secs(4));
    }
}
