//! slateduck-ivm: Incremental View Maintenance (IVM) engine for SlateDuck.
//!
//! This crate implements the IVM runtime:
//!   - `IvmPlan`   — parses a view SQL into GROUP BY + aggregation plan
//!   - `IvmWorker` — drives the incremental computation loop
//!   - `IvmTrace`  — maintains aggregate state between checkpoints
//!   - CLI binary  — `slateduck-ivm serve`
//!
//! ## Architecture
//! The IVM computation uses a pure-Rust incremental GROUP BY engine inspired
//! by DBSP (Feldera) semantics. The DBSP crate is listed as a workspace
//! dependency and provides the foundational algebraic model; this crate
//! implements a lightweight compatibility shim in `circuit.rs` that adapts
//! SlateDuck's append-only CDC stream to the DBSP Zset/Z-difference model.

pub mod circuit;
pub mod config;
pub mod observability;
pub mod output;
pub mod plan;
pub mod source;
pub mod trace;
pub mod worker;

pub use circuit::{IvmCircuit, ZDelta};
pub use config::WorkerConfig;
pub use plan::{Aggregate, AggregateKind, IvmPlan};
pub use source::MatviewInputSource;
pub use trace::IvmTrace;
pub use worker::IvmWorker;
