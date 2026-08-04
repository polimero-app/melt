use std::{
    collections::BTreeMap,
    net::{Shutdown, TcpStream},
    sync::{
        Arc, Mutex, Weak,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

use serde::Serialize;

use super::{
    CameraError, CameraTransport, H264AccessUnit, H264Stream, MjpegStream, Profile,
    open_decoded_h264_stream, open_mjpeg_stream,
};

const SUBSCRIBER_CAPACITY: usize = 2;

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
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraOwnerStatus {
    pub printer_key: String,
    pub transport: CameraOwnerTransport,
    pub subscribers: usize,
    pub generation: u64,
    pub has_jpeg: bool,
}

enum Source {
    Mjpeg(MjpegStream),
    H264(H264Stream),
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
    subscribers: BTreeMap<u64, (CameraFrameKind, SyncSender<Arc<CameraFrame>>)>,
    latest_jpeg: Option<Arc<[u8]>>,
    stopped: bool,
}

struct Owner {
    printer_key: String,
    revision: [u8; 32],
    generation: u64,
    transport: CameraOwnerTransport,
    parameters: Option<H264Parameters>,
    shutdown: Mutex<Option<TcpStream>>,
    state: Mutex<OwnerState>,
}

impl Owner {
    fn subscribe(self: &Arc<Self>, kind: CameraFrameKind) -> CameraSubscription {
        let (sender, receiver) = mpsc::sync_channel(SUBSCRIBER_CAPACITY);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let id = state.next_subscriber;
        state.next_subscriber = state.next_subscriber.wrapping_add(1);
        state.subscribers.insert(id, (kind, sender));
        CameraSubscription {
            owner: Arc::clone(self),
            id,
            receiver,
        }
    }

    fn publish(&self, frame: CameraFrame) {
        let frame = Arc::new(frame);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let CameraFrame::Jpeg(jpeg) = frame.as_ref() {
            state.latest_jpeg = Some(Arc::clone(jpeg));
        }
        let kind = match frame.as_ref() {
            CameraFrame::Jpeg(_) => CameraFrameKind::Jpeg,
            CameraFrame::H264(_) => CameraFrameKind::H264,
        };
        state.subscribers.retain(|_, (interest, subscriber)| {
            *interest != kind
                || match subscriber.try_send(Arc::clone(&frame)) {
                    Ok(()) | Err(TrySendError::Full(_)) => true,
                    Err(TrySendError::Disconnected(_)) => false,
                }
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

    fn status(&self) -> CameraOwnerStatus {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        CameraOwnerStatus {
            printer_key: self.printer_key.clone(),
            transport: self.transport,
            subscribers: state.subscribers.len(),
            generation: self.generation,
            has_jpeg: state.latest_jpeg.is_some(),
        }
    }
}

pub struct CameraSubscription {
    owner: Arc<Owner>,
    id: u64,
    receiver: Receiver<Arc<CameraFrame>>,
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
    pub fn latest_jpeg(&self) -> Option<Arc<[u8]>> {
        self.owner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .latest_jpeg
            .clone()
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
}

#[derive(Clone, Default)]
pub struct CameraManager {
    state: Arc<Mutex<ManagerState>>,
}

impl CameraManager {
    pub fn subscribe(
        &self,
        profile: &Profile,
        access_code: Option<&str>,
        fingerprint: Option<&str>,
        timeout: Duration,
        kind: CameraFrameKind,
    ) -> Result<CameraSubscription, CameraError> {
        let printer_key = format!("bambu:{}", profile.serial().to_ascii_uppercase());
        let revision = profile.connection_identity(access_code, fingerprint);
        let mut manager = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(owner) = manager.owners.get(&printer_key).and_then(Weak::upgrade) {
            if owner.revision == revision {
                if kind == CameraFrameKind::H264
                    && owner.transport != CameraOwnerTransport::RtspsH264
                {
                    return Err(CameraError::UnsupportedMedia);
                }
                return Ok(owner.subscribe(kind));
            }
            owner.stop();
        }
        let source = open_source(profile, access_code, fingerprint, timeout)?;
        let shutdown = source.shutdown_handle().map_err(CameraError::Stream)?;
        manager.generation = manager.generation.wrapping_add(1);
        let (transport, parameters) = match &source {
            Source::Mjpeg(_) => (CameraOwnerTransport::MjpegTls, None),
            Source::H264(stream) => {
                let (sps, pps) = stream.parameter_sets();
                (
                    CameraOwnerTransport::RtspsH264,
                    Some(H264Parameters {
                        sps: Arc::from(sps),
                        pps: Arc::from(pps),
                    }),
                )
            }
        };
        if kind == CameraFrameKind::H264 && transport != CameraOwnerTransport::RtspsH264 {
            return Err(CameraError::UnsupportedMedia);
        }
        let owner = Arc::new(Owner {
            printer_key: printer_key.clone(),
            revision,
            generation: manager.generation,
            transport,
            parameters,
            shutdown: Mutex::new(Some(shutdown)),
            state: Mutex::new(OwnerState {
                next_subscriber: 1,
                subscribers: BTreeMap::new(),
                latest_jpeg: None,
                stopped: false,
            }),
        });
        let subscription = owner.subscribe(kind);
        manager.owners.insert(printer_key, Arc::downgrade(&owner));
        thread::spawn(move || run_owner(owner, source));
        Ok(subscription)
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

fn open_source(
    profile: &Profile,
    access_code: Option<&str>,
    fingerprint: Option<&str>,
    timeout: Duration,
) -> Result<Source, CameraError> {
    if profile.default_capabilities().camera != CameraTransport::MjpegTls {
        match open_decoded_h264_stream(profile, access_code, fingerprint, timeout) {
            Ok(stream) => return Ok(Source::H264(stream)),
            Err(CameraError::Pin(error)) => return Err(CameraError::Pin(error)),
            Err(_) => {}
        }
    }
    open_mjpeg_stream(profile, access_code, fingerprint, timeout).map(Source::Mjpeg)
}

fn run_owner(owner: Arc<Owner>, mut source: Source) {
    loop {
        let result = match &mut source {
            Source::Mjpeg(stream) => stream.next_frame().map(|jpeg| {
                owner.publish(CameraFrame::Jpeg(Arc::from(jpeg)));
            }),
            Source::H264(stream) => stream.next_access_unit().map(|access_unit| {
                if let Some(jpeg) = &access_unit.jpeg {
                    owner.publish(CameraFrame::Jpeg(Arc::from(jpeg.clone())));
                }
                owner.publish(CameraFrame::H264(Arc::new(access_unit)));
            }),
        };
        if result.is_err()
            || owner
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .stopped
        {
            break;
        }
    }
    owner.stop();
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
    fn fanout_is_bounded_and_latest_jpeg_is_reused() {
        let owner = owner();
        let fast = owner.subscribe(CameraFrameKind::Jpeg);
        let slow = owner.subscribe(CameraFrameKind::Jpeg);
        for value in 0..10 {
            owner.publish(CameraFrame::Jpeg(Arc::from(vec![value])));
        }
        assert_eq!(fast.latest_jpeg().unwrap().as_ref(), &[9]);
        assert!(fast.recv_timeout(Duration::from_millis(1)).is_ok());
        assert!(slow.recv_timeout(Duration::from_millis(1)).is_ok());
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
    fn subscribers_receive_only_the_requested_media_kind() {
        let owner = owner();
        let h264 = owner.subscribe(CameraFrameKind::H264);
        owner.publish(CameraFrame::Jpeg(Arc::from(vec![1])));
        assert!(h264.recv_timeout(Duration::from_millis(1)).is_err());
        owner.publish(CameraFrame::H264(Arc::new(H264AccessUnit {
            rtp_packets: vec![vec![2]],
            jpeg: None,
        })));
        assert!(matches!(
            h264.recv_timeout(Duration::from_millis(1))
                .unwrap()
                .as_ref(),
            CameraFrame::H264(_)
        ));
    }
}
