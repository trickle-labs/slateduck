//! slateduck-ivm: Incremental View Maintenance (IVM) engine for SlateDuck.
//!
//! This crate implements the IVM runtime:
//!   - `IvmPlan`        — parses a view SQL into GROUP BY + aggregation + JOIN plan
//!   - `IvmWorker`      — drives the incremental computation loop
//!   - `IvmTrace`       — maintains aggregate state between checkpoints
//!   - `IvmJoinCircuit` — multi-input join + aggregation circuit (v0.13)
//!   - CLI binary       — `slateduck-ivm serve`
//!
//! ## Architecture
//! The IVM computation uses a pure-Rust incremental GROUP BY engine inspired
//! by DBSP (Feldera) semantics. The DBSP crate is listed as a workspace
//! dependency and provides the foundational algebraic model; this crate
//! implements a lightweight compatibility shim in `circuit.rs` that adapts
//! SlateDuck's append-only CDC stream to the DBSP Zset/Z-difference model.
//!
//! ## v0.13: Joins
//! Three join strategies are supported:
//!   - **Broadcast** (`join::JoinStrategy::Broadcast`) — small dimension table
//!     fully replicated to every shard.
//!   - **CoPartitioned** (`join::JoinStrategy::CoPartitioned`) — both inputs
//!     share the same shard key; join is entirely local.
//!   - **Reshuffle** (`join::JoinStrategy::Reshuffle`) — one side is
//!     repartitioned through a temporary exchange buffer.

pub mod circuit;
pub mod config;
pub mod heartbeat;
pub mod join;
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

pub use circuit::{IvmCircuit, IvmJoinCircuit, ZDelta};
pub use config::{CostMode, WorkerConfig};
pub use heartbeat::{HeartbeatHandle, LeaseRegistry};
pub use join::{
    hash_join_batch, select_strategy, ExchangeBuffer, HashJoinState, JoinClause, JoinStrategy,
    DEFAULT_BROADCAST_THRESHOLD,
};
pub use parquet::CompactionPolicy;
pub use plan::{Aggregate, AggregateKind, IvmPlan};
pub use shard_key::{compute_key_ranges, hash_key_value, shard_index_for, ShardKeyRange};
pub use shutdown::ShutdownSignal;
pub use source::MatviewInputSource;
pub use state_store::ShardStateStore;
pub use trace::IvmTrace;
pub use worker::IvmWorker;
