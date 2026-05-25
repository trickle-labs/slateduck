//! slateduck-ivm: Incremental View Maintenance (IVM) engine for SlateDuck.
//!
//! This crate implements the IVM runtime:
//!   - `IvmPlan`        — parses a view SQL into GROUP BY + aggregation + JOIN plan
//!   - `IvmWorker`      — drives the incremental computation loop
//!   - `IvmTrace`       — maintains aggregate state between checkpoints
//!   - `IvmJoinCircuit` — multi-input join + aggregation circuit (v0.13)
//!   - `volatility`     — function volatility gate (v0.14)
//!   - `dag`            — multi-view DAG with frontier coordination (v0.15)
//!   - `slatedb_trace`  — native SlateDB-backed trace (v0.15)
//!   - `window`         — window functions (v0.16)
//!   - `ordered_trace`  — ordered trace for total-order output (v0.16)
//!   - `top_n`          — LIMIT/OFFSET top-N operator (v0.16)
//!   - `decorrelate`    — correlated subquery decorrelation (v0.16)
//!   - `recursive_cte`  — WITH RECURSIVE fixed-point iteration (v0.16)
//!   - `nondet_capture` — non-deterministic function capture (v0.16)
//!   - `wasm_udf`       — WASM user-defined functions (v0.16)
//!   - `ref_counted`    — reference-counted DISTINCT + adaptive cost (v0.16)
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
//!
//! ## v0.14: Join Correctness & Aggregate Tiers
//! - EC-01 phantom-row fix: asymmetric delta branches for join inserts/deletes
//! - Aggregate tier classification: Algebraic, SemiAlgebraic, GroupRescan
//! - Volatility validation at view creation time
//! - Property-based "differential ≡ full" oracle
//!
//! ## v0.15: IVM Operational Hardening
//! - Multi-view DAG with Kahn's topological sort and diamond detection
//! - Native `SlateDbTrace` with flush coalescing and cost-mode propagation
//! - Cost guardrails: estimation, budgets, freshness degradation
//! - Backpressure protocol with per-shard publication modes
//! - Schema evolution detection (stale/broken view states)
//! - Exactly-once output snapshots via CAS deduplication
//! - REFRESH FULL, per-shard repair, and doctor diagnostics
//! - Delta optimizations: change-buffer compaction, predicate pushdown,
//!   semi-join key pre-filter, append-only fast path
//! - PG-Wire rate limiting (connection + auth failure)
//! - State store backup and restore with compaction pins
//!
//! ## v0.16: IVM Operator Completeness
//! - Window functions (ROW_NUMBER, RANK, DENSE_RANK, LAG, LEAD, aggregates)
//! - ORDER BY in materialized views (total-order output via ordered trace)
//! - LIMIT/OFFSET (top-N) with DBSP-style bounded sorted heap
//! - Correlated subquery decorrelation (EXISTS/IN → semi-join, scalar → left join)
//! - Recursive CTEs via fixed-point iteration loop
//! - Non-deterministic function capture with per-batch seed storage
//! - WASM UDFs (sandboxed, deterministic, per-batch pooled)
//! - Adaptive DIFFERENTIAL/FULL cost mode switching
//! - Reference-counted DISTINCT (correct under partial delete)
//!
//! ## v0.17: IVM Feature Hardening (IVM GA Gate)
//! - WASM UDFs via wasmtime (per-batch pooled instances, fuel + memory sandbox)
//! - matview_udfs catalog table (tag 0x21) with CREATE/DROP/ALTER FUNCTION DDL
//! - Adaptive DIFFERENTIAL/FULL mode with empirical calibration
//! - Per-view rolling statistics (rows_in, rows_out, ms_spent, last_full_cost)
//! - __sd_ref_count auxiliary column for DISTINCT correctness
//! - Reference-counted UNION DISTINCT / INTERSECT / EXCEPT (MAX/MIN/clamp)
//! - Tier 6f WASM UDF + DISTINCT property tests
//! - Tier 8 24-hour soak test (IVM GA gate)

pub mod backpressure;
pub mod backup;
pub mod circuit;
pub mod config;
pub mod cost;
pub mod dag;
pub mod decorrelate;
pub mod delete_files;
pub mod delta_opt;
pub mod exactly_once;
pub mod heartbeat;
pub mod join;
pub mod nondet_capture;
pub mod observability;
pub mod ordered_trace;
pub mod output;
pub mod parquet;
pub mod plan;
pub mod rate_limit;
pub mod recursive_cte;
pub mod ref_counted;
pub mod repair;
pub mod schema_evolution;
pub mod shard_key;
pub mod shutdown;
pub mod slatedb_trace;
pub mod source;
pub mod state_store;
pub mod top_n;
pub mod trace;
pub mod volatility;
pub mod wasm_udf;
pub mod window;
pub mod worker;

pub use backpressure::{BackpressureConfig, BackpressureState, OutputMode};
pub use backup::{BackupConfig, BackupManifest, RestoreResult};
pub use circuit::{IvmCircuit, IvmJoinCircuit, ZDelta};
pub use config::{CostMode, WorkerConfig};
pub use cost::{CostBudget, CostEstimate, CostEstimateParams};
pub use dag::{DiamondApex, FrontierClock, ViewDag};
pub use decorrelate::{
    AntiJoinEvaluator, CorrelatedSubquery, DecorrelatedOp, SemiJoinEvaluator, SubqueryKind,
};
pub use delta_opt::{AppendOnlyDetector, CompactionResult, SortKeyConfig};
pub use exactly_once::{CommitResult, OutputDeduplicator, OutputTag};
pub use heartbeat::{HeartbeatHandle, LeaseRegistry};
pub use join::{
    hash_join_batch, select_strategy, ExchangeBuffer, HashJoinState, JoinClause, JoinStrategy,
    DEFAULT_BROADCAST_THRESHOLD,
};
pub use nondet_capture::{BatchCapture, CaptureVolatility, CapturedValue};
pub use observability::{ViewRollingStats, ViewStatsStore};
pub use ordered_trace::{
    MergeSortedWriterConfig, OrderedTraceConfig, SlateDbOrderedTrace, SortKey,
};
pub use parquet::{CompactionPolicy, ParquetOutputConfig};
pub use plan::{Aggregate, AggregateKind, AggregateTier, IvmPlan};
pub use rate_limit::{RateLimitConfig, RateLimitResult, RateLimiter};
pub use recursive_cte::{IterationState, RecursiveCteConfig, RecursiveCteEvaluator};
pub use ref_counted::{
    AdaptiveCostConfig, ComplexityMultipliers, RefCountedDistinct, RefCountedSetOp, SetOperator,
};
pub use repair::{DoctorIssue, DoctorReport, RebuildState, RepairOperation, RepairRecord};
pub use schema_evolution::{SchemaChange, ViewStatus};
pub use shard_key::{compute_key_ranges, hash_key_value, shard_index_for, ShardKeyRange};
pub use shutdown::ShutdownSignal;
pub use slatedb_trace::{SlateDbTrace, SlateDbTraceConfig};
pub use source::MatviewInputSource;
pub use state_store::ShardStateStore;
pub use top_n::{TopNConfig, TopNOperator, TopNResult};
pub use trace::IvmTrace;
pub use volatility::Volatility;
pub use wasm_udf::{
    MatviewUdfCatalogEntry, UdfDdl, UdfEntry, UdfRegistry, UdfSignature, UdfType,
    WasmBatchExecutor, WasmConfig, MATVIEW_UDFS_TAG,
};
pub use window::{WindowEvaluator, WindowFunction, WindowMode, WindowSpec};
pub use worker::IvmWorker;
