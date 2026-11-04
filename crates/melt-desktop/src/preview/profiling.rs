//! Local, opt-in stage timings. Records contain no filenames or model contents.
use std::{cell::RefCell, sync::LazyLock, time::Instant};

#[derive(Clone, Debug, serde::Serialize)]
pub struct StageTiming {
    pub stage: &'static str,
    pub elapsed_us: u64,
}

thread_local! {
    static CAPTURE: RefCell<Option<Vec<StageTiming>>> = const { RefCell::new(None) };
}

pub(super) struct Span {
    stage: &'static str,
    started: Option<Instant>,
}

fn trace_enabled() -> bool {
    static ENABLED: LazyLock<bool> =
        LazyLock::new(|| std::env::var_os("MELT_PREVIEW_PROFILE").is_some());
    *ENABLED
}

pub(super) fn span(stage: &'static str) -> Span {
    let enabled = trace_enabled() || CAPTURE.with(|capture| capture.borrow().is_some());
    Span {
        stage,
        started: enabled.then(Instant::now),
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(started) = self.started else {
            return;
        };
        let timing = StageTiming {
            stage: self.stage,
            elapsed_us: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
        };
        record(timing);
    }
}

pub(super) fn record(timing: StageTiming) {
    if trace_enabled()
        && let Ok(json) = serde_json::to_string(&timing)
    {
        eprintln!("melt-preview {json}");
    }
    append(timing);
}

// Worker spans have already been traced; merge them into the caller's capture.
pub(super) fn append(timing: StageTiming) {
    CAPTURE.with(|capture| {
        if let Some(capture) = capture.borrow_mut().as_mut() {
            capture.push(timing);
        }
    });
}

/// Capture timings on this thread, restoring an outer capture even after panic.
pub fn capture<T>(work: impl FnOnce() -> T) -> (T, Vec<StageTiming>) {
    struct Restore(Option<Vec<StageTiming>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CAPTURE.with(|capture| {
                capture.replace(self.0.take());
            });
        }
    }
    let restore = Restore(CAPTURE.with(|capture| capture.replace(Some(Vec::new()))));
    let result = work();
    let timings = CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default());
    drop(restore);
    (result, timings)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captures_only_the_requested_scope_and_restores_nested_capture() {
        let (value, outer) = capture(|| {
            let _outer = span("outer");
            let (_, inner) = capture(|| {
                let _inner = span("inner");
            });
            assert_eq!(inner.len(), 1);
            42
        });
        assert_eq!(value, 42);
        assert_eq!(outer[0].stage, "outer");
    }
}
