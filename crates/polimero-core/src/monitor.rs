use std::time::Duration;

use thiserror::Error;

pub const DEFAULT_WORKERS: u8 = 8;
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
pub const MIN_WORKERS: u8 = 1;
pub const MAX_WORKERS: u8 = 32;
pub const MIN_INTERVAL: Duration = Duration::from_secs(2);
pub const MAX_INTERVAL: Duration = Duration::from_secs(120);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Settings {
    pub workers: u8,
    pub interval: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            workers: DEFAULT_WORKERS,
            interval: DEFAULT_INTERVAL,
        }
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SettingsError {
    #[error("monitor workers must be between {MIN_WORKERS} and {MAX_WORKERS}")]
    Workers,
    #[error("monitor interval must be between 2 and 120 seconds")]
    Interval,
}

impl Settings {
    pub fn new(workers: u8, interval: Duration) -> Result<Self, SettingsError> {
        if !(MIN_WORKERS..=MAX_WORKERS).contains(&workers) {
            return Err(SettingsError::Workers);
        }
        if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&interval) {
            return Err(SettingsError::Interval);
        }
        Ok(Self { workers, interval })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Backoff {
    interval: Duration,
    failures: u8,
}

impl Backoff {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            failures: 0,
        }
    }

    pub fn after_result(&mut self, succeeded: bool) -> Duration {
        if succeeded {
            self.failures = 0;
            return self.interval;
        }
        self.failures = self.failures.saturating_add(1);
        self.interval
            .checked_mul(1_u32 << self.failures.min(6))
            .unwrap_or(Duration::from_secs(60))
            .min(Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_enforce_the_product_monitoring_bounds() {
        assert_eq!(
            Settings::new(0, DEFAULT_INTERVAL),
            Err(SettingsError::Workers)
        );
        assert_eq!(
            Settings::new(DEFAULT_WORKERS, Duration::from_secs(1)),
            Err(SettingsError::Interval)
        );
        assert_eq!(
            Settings::new(MAX_WORKERS, MAX_INTERVAL),
            Ok(Settings {
                workers: MAX_WORKERS,
                interval: MAX_INTERVAL
            })
        );
    }

    #[test]
    fn backoff_doubles_failures_and_resets_after_success() {
        let mut backoff = Backoff::new(DEFAULT_INTERVAL);

        assert_eq!(backoff.after_result(false), Duration::from_secs(10));
        assert_eq!(backoff.after_result(false), Duration::from_secs(20));
        assert_eq!(backoff.after_result(false), Duration::from_secs(40));
        assert_eq!(backoff.after_result(false), Duration::from_secs(60));
        assert_eq!(backoff.after_result(true), DEFAULT_INTERVAL);
        assert_eq!(backoff.after_result(false), Duration::from_secs(10));
    }
}
