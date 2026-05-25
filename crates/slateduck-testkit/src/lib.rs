//! slateduck-testkit: shared test utilities for SlateDuck integration tests.
//!
//! ## Modules
//! - `clock` — `DeterministicClock`: wraps `tokio::time::pause()` for
//!   fully deterministic time-dependent tests without wall-clock sleeps.
//! - `harness` — `IvmWorkerHarness`: drives `IvmWorker` in-process with
//!   helper methods for waiting on lag and asserting output counts.
//! - `duckdb_harness` — `DuckDbHarness`: reference GROUP BY / join engine
//!   for IVM correctness assertions (v0.13).
//!
//! All timing tests in SlateDuck use `DeterministicClock` so that:
//! - Tests run in constant CI time regardless of hardware.
//! - Flaky sleep-based assertions are eliminated.

pub mod clock;
pub mod duckdb_harness;
pub mod harness;

pub use clock::DeterministicClock;
pub use duckdb_harness::DuckDbHarness;
pub use harness::IvmWorkerHarness;
