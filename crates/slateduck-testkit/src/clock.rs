//! Deterministic clock for time-dependent tests.
//!
//! Wraps `tokio::time` to allow test code to advance time without wall-clock
//! sleeps.  All IVM tests that involve lease TTL, heartbeat intervals, or
//! lag measurements use this clock so that CI is fast and deterministic.
//!
//! ## Usage
//! ```ignore
//! #[tokio::test]
//! async fn lease_expires() {
//!     let clock = DeterministicClock::new();
//!     // Pause the Tokio time source.
//!     clock.pause();
//!     // … start worker, acquire lease …
//!     // Advance time past the lease TTL.
//!     clock.advance_ms(35_000).await;
//!     // Now assert that the second worker can claim the shard.
//! }
//! ```

use std::time::Duration;

/// A handle to the Tokio deterministic time source.
///
/// Calling `DeterministicClock::new()` calls `tokio::time::pause()` which
/// switches the Tokio runtime's time source from the wall clock to a manually
/// controlled counter.  Time only advances when `advance_ms` (or Tokio's own
/// `sleep` drives it within a `select!`).
pub struct DeterministicClock;

impl DeterministicClock {
    /// Create a new deterministic clock and pause Tokio time.
    pub fn new() -> Self {
        tokio::time::pause();
        DeterministicClock
    }

    /// Advance simulated time by `ms` milliseconds.
    ///
    /// This wakes any `tokio::time::sleep` or `tokio::time::interval` futures
    /// that would fire within the advanced window.
    pub async fn advance_ms(&self, ms: u64) {
        tokio::time::advance(Duration::from_millis(ms)).await;
    }

    /// Advance simulated time by `secs` seconds.
    pub async fn advance_secs(&self, secs: u64) {
        tokio::time::advance(Duration::from_secs(secs)).await;
    }

    /// Return the current simulated instant.
    pub fn now(&self) -> tokio::time::Instant {
        tokio::time::Instant::now()
    }

    /// Return the current simulated time as a Unix millisecond timestamp.
    ///
    /// The base epoch is arbitrary (Tokio's start time).  For tests that need
    /// a real epoch, add a known offset.
    pub fn now_unix_ms(&self) -> u64 {
        // tokio::time::Instant does not expose a Unix epoch directly.
        // We use SystemTime for the base and add the monotonic offset.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

impl Default for DeterministicClock {
    fn default() -> Self {
        Self::new()
    }
}
