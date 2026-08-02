//! Protocol tracing: an opt-in JSON Lines record of driver network activity.
//!
//! Only metadata is recorded (transport, direction, a short label, byte
//! counts, outcome) — never request/response bodies or credentials — so a
//! trace file is safe to hand over in a bug report.

use std::{
    fmt,
    fs::File,
    io::Write,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::Serialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub trait ProtocolTracer: fmt::Debug + Send + Sync {
    fn record(&self, event: TraceEvent);
}

pub type SharedTracer = Arc<dyn ProtocolTracer>;

static NEXT_TRACER_GENERATION: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_tracer_generation() -> u64 {
    NEXT_TRACER_GENERATION.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Request,
    Response,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Response => "response",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct TraceEvent {
    pub transport: &'static str,
    pub direction: Option<Direction>,
    pub label: String,
    pub outcome: Option<String>,
    pub bytes: Option<u64>,
    pub elapsed_ms: Option<u64>,
}

impl TraceEvent {
    pub fn request(transport: &'static str, label: impl Into<String>) -> Self {
        Self {
            transport,
            direction: Some(Direction::Request),
            label: label.into(),
            ..Self::default()
        }
    }

    pub fn response(transport: &'static str, label: impl Into<String>) -> Self {
        Self {
            transport,
            direction: Some(Direction::Response),
            label: label.into(),
            ..Self::default()
        }
    }

    pub fn with_outcome(mut self, outcome: impl Into<String>) -> Self {
        self.outcome = Some(outcome.into());
        self
    }

    pub fn with_bytes(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }

    pub fn with_elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = Some(elapsed_ms);
        self
    }
}

#[derive(Serialize)]
struct TraceLine<'a> {
    timestamp: String,
    transport: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    direction: Option<&'static str>,
    label: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    elapsed_ms: Option<u64>,
}

/// Writes one JSON object per line to a file.
///
/// A write failure is swallowed: a printer command must never fail just
/// because its diagnostics could not be written.
pub struct JsonlTracer(Mutex<File>);

impl JsonlTracer {
    pub fn create(path: &Path) -> std::io::Result<Self> {
        Ok(Self(Mutex::new(File::create(path)?)))
    }
}

impl fmt::Debug for JsonlTracer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JsonlTracer")
    }
}

impl ProtocolTracer for JsonlTracer {
    fn record(&self, event: TraceEvent) {
        let line = TraceLine {
            timestamp: OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default(),
            transport: event.transport,
            direction: event.direction.map(Direction::as_str),
            label: &event.label,
            outcome: event.outcome.as_deref(),
            bytes: event.bytes,
            elapsed_ms: event.elapsed_ms,
        };
        let Ok(mut json) = serde_json::to_vec(&line) else {
            return;
        };
        json.push(b'\n');
        if let Ok(mut file) = self.0.lock() {
            let _ = file.write_all(&json);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct RecordingTracer(Mutex<Vec<TraceEvent>>);

    impl ProtocolTracer for RecordingTracer {
        fn record(&self, event: TraceEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    #[test]
    fn jsonl_tracer_writes_one_json_object_per_line() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let tracer = JsonlTracer::create(file.path()).unwrap();
        tracer.record(TraceEvent::request("http", "GET /printer/objects/query").with_bytes(0));
        tracer.record(
            TraceEvent::response("http", "GET /printer/objects/query")
                .with_outcome("200")
                .with_bytes(128),
        );

        let contents = std::fs::read_to_string(file.path()).unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["transport"], "http");
        assert_eq!(first["direction"], "request");
        assert_eq!(first["label"], "GET /printer/objects/query");
        let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(second["direction"], "response");
        assert_eq!(second["outcome"], "200");
        assert_eq!(second["bytes"], 128);
    }

    #[test]
    fn recording_tracer_captures_events_for_tests() {
        let tracer = RecordingTracer::default();
        tracer.record(TraceEvent::request("mqtt", "publish device/SN/request"));
        assert_eq!(tracer.0.lock().unwrap().len(), 1);
    }
}
