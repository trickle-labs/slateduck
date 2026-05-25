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
pub mod heartbeat;
pub mod observability;
pub mod output;
pub mod parquet;
pub mod plan;
pub mod shard_key;
pub mod shutdown;
pub mod source;
pub mod state_store;
pub mod trace;
pub mod worker;

pub use circuit::{IvmCircuit, ZDelta};
pub use config::{CostMode, WorkerConfig};
pub use heartbeat::{HeartbeatHandle, LeaseRegistry};
pub use parquet::CompactionPolicy;
pub use plan::{Aggregate, AggregateKind, IvmPlan};
pub use shard_key::{compute_key_ranges, hash_key_value, shard_index_for, ShardKeyRange};
pub use shutdown::ShutdownSignal;
pub use source::MatviewInputSource;
pub use state_store::ShardStateStore;
pub use trace::IvmTrace;
pub use worker::IvmWorker;
