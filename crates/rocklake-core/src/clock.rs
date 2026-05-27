//! Clock abstraction for wall-clock-dependent logic.
//!
//! Lease expiry and other time-sensitive operations use this trait rather than
//! calling `SystemTime::now()` directly. This makes them testable without
//! real-time dependencies.
//!
//! # Usage
//!
//! ```rust
//! use rocklake_core::clock::{Clock, SystemClock};
//!
//! let clock = SystemClock;
//! let now = clock.now_secs();
//! assert!(now > 0, "epoch seconds must be positive");
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A source of wall-clock time expressed as Unix epoch seconds.
pub trait Clock: Send + Sync {
    /// Returns the current time as Unix epoch seconds.
    fn now_secs(&self) -> u64;
}

/// The real system clock — delegates to `SystemTime::now()`.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_secs(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

/// A mock clock for tests.  The current time is set atomically and can be
/// advanced by calling `set()`.
#[derive(Clone)]
pub struct MockClock {
    now: Arc<AtomicU64>,
}

impl MockClock {
    /// Create a mock clock initialised to `initial_secs` epoch seconds.
    pub fn new(initial_secs: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(initial_secs)),
        }
    }

    /// Advance (or rewind) the clock to `secs` epoch seconds.
    pub fn set(&self, secs: u64) {
        self.now.store(secs, Ordering::Release);
    }

    /// Advance the clock forward by `delta` seconds.
    pub fn advance(&self, delta: u64) {
        self.now.fetch_add(delta, Ordering::AcqRel);
    }
}

impl Clock for MockClock {
    fn now_secs(&self) -> u64 {
        self.now.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_returns_positive() {
        let clock = SystemClock;
        assert!(clock.now_secs() > 0);
    }

    #[test]
    fn mock_clock_advance_and_set() {
        let clock = MockClock::new(1_000_000);
        assert_eq!(clock.now_secs(), 1_000_000);
        clock.advance(3600);
        assert_eq!(clock.now_secs(), 1_003_600);
        clock.set(500);
        assert_eq!(clock.now_secs(), 500);
    }
}
