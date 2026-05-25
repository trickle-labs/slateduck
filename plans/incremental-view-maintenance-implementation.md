# Incremental View Maintenance — Implementation Plan

> **Scope.** Engineering plan for delivering incremental view maintenance (IVM) from v0.11 foundations through the current v0.18 DuckLake Catalog Standard Interface and v1.0 GA gate. This document was originally written as a v0.11–v0.14 track; the current alignment addendum below maps that historical plan onto the expanded roadmap. Companion to the architectural design in [plans/slateduck-differential-dataflow.md](slateduck-differential-dataflow.md) and the substrate analysis in [plans/slatedb-differential-dataflow.md](slatedb-differential-dataflow.md). Anchored in the roadmap entries for [v0.11](../ROADMAP.md#v011--incremental-view-maintenance-foundations), [v0.12](../ROADMAP.md#v012--ivm-scale-out-sharding--lease-management), [v0.13](../ROADMAP.md#v013--ivm-joins), and the current v0.14–v0.18 sections in [ROADMAP.md](../ROADMAP.md).
>
> **Status.** v0.11–v0.13 are shipped. v0.14–v0.18 are planning-stage and gated by the architecture decisions documented below.
>
> **Audience.** Contributors and reviewers implementing each phase. This document is intentionally concrete: tag bytes, function signatures, key layouts, file boundaries, test names, failure modes.

> **Current alignment note (May 2026):** This document was originally written for the v0.11–v0.14 IVM track. The roadmap has since expanded and renumbered the post-v0.13 work: v0.14 is now IVM Join Correctness, v0.15 is Operational Hardening, v0.16 is Operator Completeness, v0.17 is Feature Hardening / the IVM GA gate, and v0.18 is the DuckLake Catalog Standard Interface. Historical section text is preserved below, but the **Current Roadmap Alignment** and **Current Architecture Gates** notes in this document are binding for new implementation work.

## Current Roadmap Alignment & Risk Assessment

The implementation plan remains directionally sound, but several original assumptions are no longer true in the current codebase:

- `dbsp = "0.299.0"` is declared in the workspace, but the IVM crate does not call DBSP APIs today. `circuit.rs` is a hand-rolled Z-difference engine that follows DBSP concepts without using DBSP's circuit builder.
- `plan.rs` uses `sqlparser` and emits an ad-hoc `IvmPlan`; the IVM planner does not use DataFusion `LogicalPlan`. The `slateduck-datafusion` crate is read-side integration only.
- `IvmOracle` is planned but not implemented. It must be the first v0.14 deliverable because v0.14–v0.17 correctness gates depend on it.
- The original plan marks window functions, correlated subqueries, and recursive CTEs as post-v1.0. The current roadmap moves them into v0.16, which makes the planner and computation-backend decisions urgent.
- Testkit has `DuckDbHarness`, `IvmWorkerHarness`, and `DeterministicClock`; `MinioHarness`, `CatalogHarness`, and `PgWireHarness` still need explicit implementation before the higher test tiers can be credible.

Current realism: the v0.14–v0.18 roadmap is achievable, but only if the architecture gates are resolved before feature implementation. The likely remaining effort is 80–120 person-weeks, with v0.16 carrying the highest uncertainty. If the project chooses a clean DBSP/DataFusion integration path, the plan stays coherent. If the project keeps the hand-rolled shim, v0.16 must either absorb a much larger custom engine effort or move the hardest operators later.

Current phase mapping:

| Roadmap phase | Current role | Implementation-plan impact |
|---|---|---|
| v0.14 — IVM Join Correctness | Build `IvmOracle`; fix EC-01; classify aggregates; add volatility gate | First implementation phase after this alignment note |
| v0.15 — IVM Operational Hardening | Multi-view DAG, native state persistence, cost/observability/fault injection | Former v0.14 hardening content maps here |
| v0.16 — IVM Operator Completeness | Windows, ORDER BY, LIMIT/OFFSET, correlated subqueries, recursive CTEs, non-det capture | Former v0.15 feature-completeness content is split; planner migration required |
| v0.17 — IVM Feature Hardening | WASM UDFs, adaptive mode, DISTINCT ref-counting, 24 h soak | Former v0.15 runtime extensions and GA gate map here |
| v0.18 — DuckLake Catalog Standard Interface | `table_changes()`, rowid, leases, NOTIFY, extension schema, opaque frontiers | Contract validation phase; pg-trickle is validator, not dependency |

---

## Table of Contents

0. [Current Roadmap Alignment & Risk Assessment](#current-roadmap-alignment--risk-assessment)
1. [Guiding Principles](#1-guiding-principles)
2. [System Architecture](#2-system-architecture)
3. [Catalog Schema (Tags, Rows, Keys)](#3-catalog-schema-tags-rows-keys)
4. [Catalog API Surface](#4-catalog-api-surface)
5. [SQL Surface](#5-sql-surface)
6. [`slateduck-ivm` Crate](#6-slateduck-ivm-crate)
7. [State-Store Layout](#7-state-store-layout)
8. [Worker Lifecycle](#8-worker-lifecycle)
9. [DBSP Integration](#9-dbsp-integration)
10. [Sharding & Partition Discipline](#10-sharding--partition-discipline)
11. [Joins](#11-joins)
12. [Output Plane](#12-output-plane)
13. [Failure Model & Recovery](#13-failure-model--recovery)
14. [Observability](#14-observability)
15. [Testing Strategy](#15-testing-strategy)
16. [Performance Targets](#16-performance-targets)
17. [Security & Multi-Tenancy](#17-security--multi-tenancy)
18. [Cost Model](#18-cost-model)
19. [Documentation Deliverables](#19-documentation-deliverables)
20. [Phased Milestones](#20-phased-milestones)
21. [Open Questions Tracker](#21-open-questions-tracker)

---

## 1. Guiding Principles

These five rules govern every implementation decision in the IVM track. When in doubt, refer back here.

**P1 — Immutability everywhere.** No piece of data, intermediate or final, is ever overwritten. State advances by appending new immutable batches; obsolete data is reclaimed only by retention-bounded compaction.

**P2 — Stateless workers.** A worker is a process that reads catalog + base data + its assigned state-store databases and writes new state and new Parquet. Its only durable identity is its `worker_id` (CLI flag). Killing it loses no progress.

**P3 — Single writer per shard.** Inside a `(matview_id, shard_id)` scope, exactly one worker holds the lease at a time. SlateDB CAS enforces it. No multi-writer protocols, no consensus.

**P4 — Output is just DuckLake.** Every materialized view writes Parquet files that are first-class DuckLake data files, registered through the existing `CatalogWriter`. Readers cannot distinguish a materialized view from a base table. No new read path.

**P5 — Bounded SQL surface.** Same discipline as the rest of SlateDuck: pgwire validates a small, explicit grammar; the inner `<select>` is parsed but stored verbatim in the catalog. Only `slateduck-ivm` interprets it.

---

## 2. System Architecture

Three logically separable planes share a SlateDB substrate. Each runs as a distinct process; no in-process coupling between planes other than the catalog itself.

```
                              ┌───────────────────────────────┐
                              │      CONTROL PLANE            │
                              │  slateduck-pgwire             │
                              │  + new matview SQL grammar    │
                              │  + new catalog writer methods │
                              └─────────────┬─────────────────┘
                                            │ catalog facts (snapshots S_n)
                                            ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                              CATALOG (SlateDB)                              │
│   28 DuckLake tables                                                        │
│   + matviews, matview_deps, matview_checkpoints, matview_shards             │
└─────────────────────────────────────────────────────────────────────────────┘
                ▲                                              │
                │ output snapshots                             │ definitions, watermarks, leases
                │                                              ▼
┌───────────────┴──────────────┐              ┌────────────────────────────────┐
│       OUTPUT PLANE           │              │       COMPUTE PLANE            │
│  Parquet writer:             │◀─arrangement─│  N stateless slateduck-ivm     │
│  materialize current state   │   snapshot   │  worker processes              │
│  → new DuckLake snapshot     │              │  each owning a key-range shard │
└──────────────────────────────┘              └────────────────┬───────────────┘
                                                               │
                                              ┌────────────────▼───────────────┐
                                              │  STATE STORE (SlateDB)         │
                                              │  per (matview, shard) database │
                                              │  arrangements as keyed batches │
                                              └────────────────────────────────┘
```

### 2.1 Plane responsibilities

| Plane | Process | Holds state? | Failure impact |
|---|---|---|---|
| Control | `slateduck-pgwire` | No (durable in catalog) | DDL unavailable; ingestion continues |
| Compute | `slateduck-ivm` (1..N processes) | No (durable in state stores) | Owned shards stop advancing until another worker claims |
| Output | thread in `slateduck-ivm` (default) or dedicated binary | No (durable in catalog) | Views become stale; no data loss |

### 2.2 Why three planes

- **Failure isolation.** A compute crash does not bring down DDL or ingest. A control-plane crash does not stop in-flight compute.
- **Scaling shape.** Each plane scales independently: pgwire scales with client connection count; compute scales with view count × shard count; output scales with view count.
- **Operational clarity.** Operators reason about one plane at a time.

---

## 3. Catalog Schema (Tags, Rows, Keys)

Four new tables added to the existing 28 DuckLake tables. All allocations updated atomically in [crates/slateduck-core/src/tags.rs](../crates/slateduck-core/src/tags.rs).

### 3.1 Tag allocation

| Tag | Name | MVCC behaviour | Notes |
|---|---|---|---|
| `0x1D` | `matviews` | `Versioned` | View definitions; `dropped_at_snapshot` for logical drop |
| `0x1E` | `matview_deps` | `AppendOnly` | One row per (matview, base_table); never updated |
| `0x1F` | `matview_checkpoints` | `AppendOnly` | One row per advance; the watermark log |
| `0x20` | `matview_shards` | `MutableSingleton` per `(matview_id, shard_id)` | Lease state; CAS-updated |

Tags `0x21–0x2F` reserved for future IVM-related tables (e.g. broadcast-input registry, audit log of repair operations).

### 3.2 Key layout

All keys follow the existing `slateduck-core` convention: `<tag_byte> | <fixed-length id columns> | <discriminator>`.

```
matviews              : 0x1D | matview_id(u64 BE) | begin_snapshot(u64 BE)
matview_deps          : 0x1E | matview_id(u64 BE) | base_table_id(u64 BE)
matview_checkpoints   : 0x1F | matview_id(u64 BE) | shard_id(u32 BE) | seq(u64 BE)
matview_shards        : 0x20 | matview_id(u64 BE) | shard_id(u32 BE)
```

Notes:

- All multi-byte integers are big-endian to preserve key ordering.
- `matview_checkpoints.seq` is a per-shard monotone counter; provides total ordering of checkpoints under a single key prefix.
- `matview_shards` is a single key per shard (no `begin_snapshot`); updates are CAS-driven.

### 3.3 Row schemas (Protobuf v1)

```protobuf
// matviews
message MatviewRow {
  uint64 matview_id            = 1;
  string name                  = 2;
  string schema_name           = 3;
  string view_sql              = 4;
  uint64 output_table_id       = 5;
  uint32 shard_count           = 6;
  uint32 freshness_target_ms   = 7;
  string state_uri             = 8;   // object-store prefix
  string shard_key_column      = 9;   // empty = auto-detected
  uint64 created_at_snapshot   = 10;
  uint64 begin_snapshot        = 11;
  uint64 end_snapshot          = 12;  // 0 = open
  uint32 status                = 13;  // 0 = active, 1 = stale, 2 = rebuilding, 3 = dropped
  uint32 encoding_version      = 14;  // = 1
}

// matview_deps
message MatviewDepRow {
  uint64 matview_id            = 1;
  uint64 base_table_id         = 2;
  repeated string columns      = 3;
  bool   is_broadcast          = 4;   // true => replicated to every shard
  uint64 begin_snapshot        = 5;
  uint32 encoding_version      = 6;
}

// matview_checkpoints
message MatviewCheckpointRow {
  uint64 matview_id            = 1;
  uint32 shard_id              = 2;
  uint64 seq                   = 3;
  uint64 last_input_snapshot   = 4;
  uint64 last_output_snapshot  = 5;
  uint64 frontier_time         = 6;
  uint64 durable_at_unix_ms    = 7;
  string worker_id             = 8;
  uint32 encoding_version      = 9;
}

// matview_shards
message MatviewShardRow {
  uint64 matview_id            = 1;
  uint32 shard_id              = 2;
  string owner_worker          = 3;   // empty = unowned
  uint64 lease_expires_unix_ms = 4;
  bytes  key_range_lo          = 5;   // inclusive
  bytes  key_range_hi          = 6;   // exclusive
  uint64 generation            = 7;   // bumped on every CAS update
  uint32 encoding_version      = 8;
}
```

### 3.4 Fixture coverage

- `tests/fixtures/matview/create_view.dat` — single matview creation, no shards
- `tests/fixtures/matview/multi_shard.dat` — view with 8 shards, leases unowned
- `tests/fixtures/matview/lease_acquired.dat` — same with one shard claimed
- `tests/fixtures/matview/checkpoint_history.dat` — 100 checkpoints across 8 shards
- `tests/fixtures/matview/dropped.dat` — view with `end_snapshot != 0`

---

## 4. Catalog API Surface

New methods on `CatalogWriter` (in `slateduck-catalog/src/writer.rs`) and `CatalogReader` (`reader.rs`). All writer methods commit a new catalog snapshot; readers see the result at the next `get_current_snapshot()`.

### 4.1 Writer methods

```rust
impl CatalogWriter {
    pub async fn create_matview(
        &mut self,
        name: &str,
        schema_name: &str,
        view_sql: &str,
        output_table_id: TableId,
        shard_count: u32,
        freshness_target_ms: u32,
        shard_key_column: Option<&str>,
        state_uri: &str,
        deps: &[(TableId, Vec<String>, bool /* broadcast */)],
    ) -> Result<MatviewId>;

    pub async fn drop_matview(&mut self, matview_id: MatviewId) -> Result<()>;

    pub async fn set_matview_status(
        &mut self,
        matview_id: MatviewId,
        status: MatviewStatus,
    ) -> Result<()>;

    pub async fn update_matview_checkpoint(
        &mut self,
        matview_id: MatviewId,
        shard_id: u32,
        last_input_snapshot: SnapshotId,
        last_output_snapshot: SnapshotId,
        frontier_time: u64,
        worker_id: &str,
    ) -> Result<u64 /* new seq */>;

    pub async fn claim_matview_shard(
        &mut self,
        matview_id: MatviewId,
        shard_id: u32,
        worker_id: &str,
        lease_ttl_ms: u64,
    ) -> Result<ClaimOutcome>;

    pub async fn extend_matview_lease(
        &mut self,
        matview_id: MatviewId,
        shard_id: u32,
        worker_id: &str,
        new_expires_unix_ms: u64,
        expected_generation: u64,
    ) -> Result<()>;

    pub async fn release_matview_lease(
        &mut self,
        matview_id: MatviewId,
        shard_id: u32,
        worker_id: &str,
    ) -> Result<()>;
}

pub enum ClaimOutcome {
    Acquired { generation: u64, expires_unix_ms: u64, key_range: (Vec<u8>, Vec<u8>) },
    Contended { current_owner: String, current_generation: u64 },
    AlreadyOwned { generation: u64, expires_unix_ms: u64 },
}

pub enum MatviewStatus { Active, Stale, Rebuilding, Dropped }
```

### 4.2 Reader methods

```rust
impl CatalogReader {
    pub fn list_matviews(&self) -> Result<Vec<MatviewSummary>>;
    pub fn get_matview(&self, id: MatviewId) -> Result<Option<MatviewRow>>;
    pub fn get_matview_by_name(&self, schema: &str, name: &str) -> Result<Option<MatviewRow>>;
    pub fn list_matview_deps(&self, id: MatviewId) -> Result<Vec<MatviewDepRow>>;
    pub fn list_matview_shards(&self, id: MatviewId) -> Result<Vec<MatviewShardRow>>;
    pub fn list_shards_for_worker(&self, worker_id: &str) -> Result<Vec<MatviewShardRow>>;
    pub fn read_checkpoint_history(
        &self,
        id: MatviewId,
        shard_id: u32,
        limit: usize,
    ) -> Result<Vec<MatviewCheckpointRow>>;
    pub fn matview_lag_ms(&self, id: MatviewId) -> Result<u64>;
}
```

### 4.3 Concurrency contracts

- `create_matview` is unique on `(schema_name, name)` among un-dropped matviews; conflict returns `SQLSTATE 42710`.
- `claim_matview_shard` is the only multi-writer-safe operation; uses `DbTransaction::cas_update` on the `matview_shards` key, bumping `generation`.
- `update_matview_checkpoint` is single-writer per shard (the lease holder); append-only via auto-incremented `seq`.
- `extend_matview_lease` requires `expected_generation` matches current; otherwise returns `SQLSTATE 40001`.
- `release_matview_lease` is idempotent; if the lease has already been taken over, it returns `Ok(())` without modification.

---

## 5. SQL Surface

Bounded grammar additions in `slateduck-sql/src/grammar/`. The new statements are validated structurally; the inner `<select>` is parsed by DataFusion's SQL frontend but not executed by pgwire.

### 5.1 Grammar

```
create_imv_stmt ::= 'CREATE' 'INCREMENTAL' 'MATERIALIZED' 'VIEW' [IF NOT EXISTS]
                    qualified_name
                    ['WITH' '(' option_list ')']
                    'AS' select_stmt

drop_imv_stmt   ::= 'DROP' 'INCREMENTAL' 'MATERIALIZED' 'VIEW' [IF EXISTS] qualified_name

alter_imv_stmt  ::= 'ALTER' 'INCREMENTAL' 'MATERIALIZED' 'VIEW' qualified_name
                    'SET' '(' option_list ')'

refresh_imv_stmt::= 'REFRESH' 'INCREMENTAL' 'MATERIALIZED' 'VIEW' qualified_name 'FULL'

show_imv_stmt   ::= 'SHOW' 'MATERIALIZED' 'VIEWS'
                  | 'SHOW' 'MATVIEW' 'SHARDS' 'FOR' qualified_name

explain_imv_stmt::= 'EXPLAIN' 'MATERIALIZED' 'VIEW' qualified_name

option          ::= identifier '=' literal
option_list     ::= option (',' option)*
```

### 5.2 Supported options

| Option | Type | Default | Notes |
|---|---|---|---|
| `shard_count` | `u32` | 1 (v0.11), auto (v0.12+) | Set at create or `ALTER` |
| `shard_key` | `string` | auto-detected from `GROUP BY` | Required for joins not co-partitioned by GROUP BY key |
| `freshness` | duration literal (`'5s'`, `'1m'`) | `'5s'` | Target publish lag |
| `output_mode` | `'consistent'` \| `'per_shard'` | `'consistent'` | historical v0.14 / current v0.15 |
| `join_strategy` | `'broadcast'` \| `'co_partition'` \| `'reshuffle'` \| `'auto'` | `'auto'` | v0.13 |
| `broadcast_threshold` | `u64` rows | `1_000_000` | v0.13 |
| `output_compaction` | duration literal \| `'never'` | `'1h'` | v0.12 |

### 5.3 SQL functions

```sql
matview_lag(name VARCHAR) -> BIGINT      -- max lag across shards, in ms
matview_status(name VARCHAR) -> VARCHAR  -- 'active' | 'stale' | 'rebuilding' | 'dropped'
matview_shard_count(name VARCHAR) -> INT
```

Implemented as catalog reads, surfaced through pgwire's existing scalar-function dispatch.

---

## 6. `slateduck-ivm` Crate

New workspace crate. Targets the same MSRV and lint configuration as the rest of the workspace.

```
crates/slateduck-ivm/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs
│   ├── config.rs        # Worker config & CLI parsing
│   ├── source.rs        # MatviewInputSource
│   ├── circuit.rs       # DBSP circuit compilation
│   ├── plan.rs          # View SQL -> DBSP plan
│   ├── trace.rs         # Persistent trace adapter
│   ├── worker.rs        # Lease + event loop
│   ├── output.rs        # Per-shard Parquet writer
│   ├── exchange.rs      # v0.13: re-shuffle exchange operator
│   ├── observability.rs # Metrics, tracing, doctor
│   └── bin/
│       └── slateduck-ivm.rs
├── tests/
│   ├── integration_tests.rs
│   ├── fault_injection_tests.rs
│   └── tpch_streaming_tests.rs
└── benches/
    └── ingest_throughput.rs
```

### 6.1 Cargo.toml outline

```toml
[package]
name = "slateduck-ivm"
version = "0.1.0"
edition = "2021"
rust-version = { workspace = true }

[dependencies]
slateduck-core    = { path = "../slateduck-core" }
slateduck-catalog = { path = "../slateduck-catalog" }
slateduck-sql     = { path = "../slateduck-sql" }
slatedb           = { workspace = true }
dbsp              = { version = "=<pinned>", default-features = false }
datafusion-sql    = { workspace = true }
arrow             = { workspace = true }
parquet           = { workspace = true }
tokio             = { workspace = true }
tracing           = { workspace = true }
prometheus        = { workspace = true }
clap              = { workspace = true }

[dev-dependencies]
fail              = { workspace = true }
proptest          = { workspace = true }
criterion         = { workspace = true }

[[bin]]
name = "slateduck-ivm"
path = "src/bin/slateduck-ivm.rs"
```

### 6.2 CLI

```
slateduck-ivm serve \
  --catalog-path  s3://bucket/catalogs/warehouse-a \
  --state-prefix  s3://bucket/matview-state/ \
  --worker-id     ivm-0 \
  --shard-limit   16 \
  --lease-ttl-ms  30000 \
  --metrics-addr  0.0.0.0:9100 \
  [--matview-allowlist v1,v2]   # optional; default = all

slateduck-ivm doctor \
  --catalog-path s3://bucket/catalogs/warehouse-a \
  [--matview v]                  # optional; default = all

slateduck-ivm repair \
  --catalog-path s3://bucket/catalogs/warehouse-a \
  --matview v \
  --shard N
```

---

## 7. State-Store Layout

State lives at:

```
{state_prefix}/matviews/{matview_id}/shards/{shard_id}/
```

Each shard owns one SlateDB `Db` at that prefix. Workers open it via the standard `slatedb::Db::open` path, the same way the catalog itself is opened.

### 7.1 Internal key layout

Within a shard's state store, keys are tagged by operator role within the DBSP circuit:

```
0x01 | operator_id(u32 BE) | tuple_key                  -> arrangement batch
0x02 | operator_id(u32 BE) | frontier(u64 BE)           -> per-operator frontier marker
0x10                                                     -> latest durable input snapshot
0x11                                                     -> compaction state
0x20 | broadcast_input_id(u64 BE) | tuple_key           -> broadcast input replicas (v0.13)
```

These tags are local to the shard's state store and do not collide with the catalog's tag space.

### 7.2 Compaction policy

- Per-matview: configurable via `WITH (state_compaction = '...')`
- Default: aggressive — target SST count ≤ 32 per shard
- DBSP frontier advancement drives logical compaction; SlateDB physical compaction follows

### 7.3 Lifecycle

- Created on first lease acquisition for the shard
- Never explicitly deleted while the matview is active
- Garbage collected (whole-prefix delete) when the matview's `dropped_at_snapshot` falls below `retain-from`
- Recreated wholesale on `REFRESH ... FULL`

---

## 8. Worker Lifecycle

The worker is a single-threaded event loop per shard, with N shards multiplexed onto a Tokio runtime.

### 8.1 Worker state machine

```
                       ┌──────────────┐
                       │   Starting   │
                       └──────┬───────┘
                              ▼
                       ┌──────────────┐
       ┌──────────────▶│  Discovering │   poll catalog every N seconds
       │               └──────┬───────┘
       │                      │ shards eligible
       │                      ▼
       │               ┌──────────────┐
       │               │   Claiming   │   CAS on matview_shards
       │               └──────┬───────┘
       │                      │ acquired
       │                      ▼
       │               ┌──────────────┐
       │       ┌──────▶│  Initializing│   open state DB, build circuit
       │       │       └──────┬───────┘
       │       │              ▼
       │       │       ┌──────────────┐
       │       │       │   Running    │   drain input ➔ DBSP ➔ state ➔ checkpoint
       │       │       └──────┬───────┘
       │       │              │ lease lost / view dropped / shutdown
       │       │              ▼
       │       │       ┌──────────────┐
       │       └───────│  Recovering  │   re-acquire or release
       │               └──────┬───────┘
       │                      │ released
       └──────────────────────┘
```

### 8.2 Per-cycle work loop

For each owned shard, per freshness tick:

1. **Heartbeat.** `extend_matview_lease` if `now + heartbeat_interval >= lease_expires`.
2. **Read input snapshot range.** Determine `(last_input_snapshot, latest_snapshot]` and the list of data files added in that range, filtered to the shard's key range.
3. **Drive DBSP.** Push each row as `(key, value, +1)` (or `(key, value, -1)` for delete-file overlays).
4. **Flush.** Persist new arrangement batches to the shard's state store. `await_durable = false` for mid-batch flushes; `await_durable = true` at checkpoint boundary.
5. **Append checkpoint.** Call `update_matview_checkpoint`.
6. **Signal output plane.** Append to an in-process channel; the output plane's per-matview task picks it up.

### 8.3 Concurrency model

- One Tokio task per owned shard
- A small bounded channel from each shard task to the output plane task
- A separate Tokio task for lease heartbeats (per worker, not per shard)
- A separate Tokio task for catalog polling

### 8.4 Graceful shutdown

- `SIGTERM` → stop accepting new work; finish current batch; release leases; exit
- 30-second deadline; on timeout, force-exit (lease expires naturally)

---

## 9. DBSP Integration

> **Current architecture gate:** Treat this section as the Option B target architecture, not a description of the implementation that exists today. The current codebase has a hand-rolled `IvmCircuit` and no `use dbsp` imports. Before v0.14 implementation begins, run the spike described in ROADMAP.md Pre-v0.14 Gate 1 and choose one path:
>
> - **Option A:** keep extending the hand-rolled shim and update this section to say "IVM circuit shim" rather than "DBSP".
> - **Option B:** migrate to DBSP's native circuit API and keep this section as the target design.
> - **Option C:** switch to `differential-dataflow` and rewrite DBSP-specific lowering / trace language.
>
> Until that decision is recorded in `docs/design-decisions/ivm-architecture.md`, no v0.15 `SlateDbTrace`, v0.16 recursive CTE, or v0.16 window-function work should start.

DBSP (Feldera) is consumed as a Rust dependency, pinned to a tested version. A thin adaptation layer in `circuit.rs` insulates the rest of `slateduck-ivm` from DBSP API churn.

### 9.1 SQL → DBSP plan

> **Current planner correction:** The current implementation does not parse with `datafusion-sql` and does not produce a DataFusion `LogicalPlan`. It uses `sqlparser` directly and emits `IvmPlan`. The following steps are therefore either (a) the target design if Option B is chosen, or (b) a migration plan that must be inserted before v0.16 correlated subqueries. DataFusion decorrelation requires `LogicalPlan`; it cannot operate on the current ad-hoc `IvmPlan`.

```rust
pub fn compile_view(
    view_sql: &str,
    base_schemas: &HashMap<TableId, Arc<Schema>>,
    shard_predicate: ShardPredicate,
) -> Result<DbspPlan>;
```

Steps:
1. Parse the inner `<select>` with `datafusion-sql` to obtain a `LogicalPlan`.
2. Validate that every operator has a DBSP equivalent. Reject:
   - Window functions (v0.11–v0.13)
   - `ORDER BY` outside aggregates
   - `LIMIT`/`OFFSET` (matviews are sets, not lists)
   - Non-deterministic functions (`now()`, `random()`, etc.) — block list checked statically
3. Lower the `LogicalPlan` to a DBSP plan (`map`, `filter`, `aggregate`, `join`, `distinct`).
4. Apply the shard predicate at the source operators.

### 9.2 Operator support matrix

> **Current roadmap supplement:** The original table below stopped at the old v0.14 and marked several operators post-v1.0. The current roadmap extends the IVM track through v0.17 and moves those operators earlier. Preserve the historical table for context; use this supplement for implementation planning:
>
> | Operator family | Current roadmap target | Prerequisite |
> |---|---|---|
> | EC-01 join correctness, aggregate tiering, volatility reject gate | v0.14 | `IvmOracle` first |
> | Multi-view DAG, frontier durability, native state persistence | v0.15 | DBSP/shim architecture choice |
> | ORDER BY, LIMIT/OFFSET, windows | v0.16 | Ordered trace over whichever persistence path v0.15 ships |
> | Correlated subqueries | v0.16 | DataFusion `LogicalPlan` migration + decorrelation pass |
> | Recursive CTEs | v0.16 | DBSP `iterate` spike or fixed-point fallback |
> | WASM UDFs, adaptive mode, ref-counted DISTINCT | v0.17 | Full v0.16 operator matrix |

| Operator | v0.11 | v0.12 | v0.13 | v0.14 |
|---|---|---|---|---|
| `SELECT` projection | ✓ | ✓ | ✓ | ✓ |
| `WHERE` filter | ✓ | ✓ | ✓ | ✓ |
| `GROUP BY` + aggregate (count, sum, min, max, avg) | ✓ | ✓ | ✓ | ✓ |
| `HAVING` | ✓ | ✓ | ✓ | ✓ |
| `DISTINCT` | ✓ | ✓ | ✓ | ✓ |
| `JOIN` (broadcast) | — | — | ✓ | ✓ |
| `JOIN` (co-partition) | — | — | ✓ | ✓ |
| `JOIN` (reshuffle) | — | — | ✓ | ✓ |
| Subqueries (uncorrelated) | — | — | ✓ | ✓ |
| Subqueries (correlated) | — | — | — | post-v1.0 |
| `UNION ALL` | ✓ | ✓ | ✓ | ✓ |
| `UNION DISTINCT` | — | ✓ | ✓ | ✓ |
| Window functions | — | — | — | post-v1.0 |
| Recursive CTEs | — | — | — | post-v1.0 |

### 9.3 Persistent trace

> **Current persistence correction:** v0.11 did not ship DBSP-bundled persistence in the actual codebase. It shipped `IvmTrace` checkpoint metadata around the hand-rolled `IvmCircuit`. If Option B is chosen, `SlateDbTrace` implements DBSP traits here. If Option A is chosen, the persistent trace work means durable serialization, compaction, and frontier management for `IvmTrace` without DBSP traits.

`SlateDbTrace` implements DBSP's `Trace`/`Batch`/`Cursor`. v0.11 uses DBSP's bundled object-store-backed trace as a stopgap; v0.14 replaces it with a native implementation that maps DBSP batches one-to-one to SlateDB SSTs and uses SlateDB compaction for frontier advancement.

---

## 10. Sharding & Partition Discipline

### 10.1 Shard key inference

Heuristic at view creation:

1. If `shard_key` option is set, use it.
2. Otherwise, if the view has a `GROUP BY`, the first GROUP BY column is the shard key.
3. Otherwise, if the view is a 1:1 filter+project on a single base table, use that table's clustering column.
4. Otherwise, set `shard_count = 1` and emit a warning.

### 10.2 Key range assignment

At view creation:

- For integer / timestamp shard keys: divide the observed `[min, max]` range from base-table statistics into `shard_count` equal slices.
- For string shard keys: hash to `u64`, then divide the `u64` space.
- For composite keys: hash the tuple to `u64`.

Recorded in `matview_shards.key_range_lo` / `key_range_hi` as raw bytes (the encoded boundary value, or hash bytes).

### 10.3 Source-side filtering

`MatviewInputSource::next_batch`:
1. List data files added since `last_input_snapshot`.
2. For each file, consult statistics: if file's `[min_shard_key, max_shard_key]` does not intersect the shard range, skip.
3. Otherwise, read the file's Parquet row groups; for each row group, prune by statistics; for surviving row groups, read pages and filter rows by shard key.

This is the heart of Principle P1 at the source: a single data file is read by potentially every shard, but each shard reads only its own slice.

### 10.4 Re-sharding (`ALTER ... SET shard_count = M`)

Cannot mutate existing shards in place (P1). Strategy:

1. Allocate a new `matview_id'` with `shard_count = M`.
2. Re-run backfill in parallel with the existing view continuing to update.
3. When new view catches up to current frontier, atomically swap output table pointer in the catalog.
4. Mark old `matview_id` as `Dropped`; old state stores GC'd at retention boundary.

---

## 11. Joins

v0.13. Three strategies, chosen per join either by user option or auto-planner.

### 11.1 Broadcast

Used when one input's row count (from base-table statistics) is below `broadcast_threshold`.

- Broadcast input is replicated to each shard's state store under tag `0x20`.
- Updates to the broadcast input fan out to all shards' state stores.
- Join is a local hash lookup against the broadcast side.

Cost: O(broadcast_size × shard_count) state. Hard cap at `broadcast_threshold` rows.

### 11.2 Co-partition

Both inputs share the same shard key. Each shard reads both inputs filtered to its key range. Local hash join.

Cheapest strategy. Auto-selected when both inputs share a clustering column and the view uses that column as the join key.

### 11.3 Re-shuffle exchange

Used when neither broadcast nor co-partition applies.

- A `MatviewExchangeOperator` writes intermediate state keyed by the join key into a temporary SlateDB region per shard.
- The downstream join operator reads the matching key range from the exchange region.
- One extra round-trip through object storage per join input.

Cost: ~2× the steady-state SST write volume of the input. Documented as the most expensive option.

### 11.4 Join plan selection

```
if user specified join_strategy:
    use it
elif one input < broadcast_threshold:
    use broadcast
elif inputs share shard key (deterministic check):
    use co_partition
else:
    use reshuffle
```

Selection recorded in `matview_deps` per dep row and surfaced in `EXPLAIN MATERIALIZED VIEW`.

---

## 12. Output Plane

### 12.1 Trigger model

The output plane runs per matview, driven by either:

- **Time-based.** Every `freshness_target / 2`, attempt to publish.
- **Event-based.** On signal from any shard that crossed a checkpoint boundary.

### 12.2 Publish protocol

1. Determine target frontier `T` (default: min across all shards' latest checkpoint; per-shard mode: each shard's own latest).
2. For each shard, read its arrangement at frontier `T` from its state store.
3. For each shard, write one Parquet file containing the shard's current state. Parquet files placed under `{warehouse}/data/{output_table_id}/matview-{matview_id}/shard-{shard_id}/snapshot-{T}.parquet`.
4. Open a `CatalogWriter` transaction:
   - Register new data files in the output table.
   - Tombstone superseded data files from the previous publish.
   - Update an output-table metadata key `matview.last_published_frontier = T` (CAS to prevent duplicate publish).
   - Commit a new catalog snapshot.

### 12.3 Exactly-once semantics

The `matview.last_published_frontier` CAS guarantees at most one snapshot per `(matview_id, T)` tuple. Combined with idempotent data-file registration, this delivers exactly-once output snapshots.

### 12.4 Output data file format

Parquet files written by the output plane are indistinguishable from base-table Parquet:

- Same Arrow → Parquet writer settings as the rest of SlateDuck
- Statistics enabled (min/max, bloom for high-cardinality columns)
- Compression: zstd level 3 (same default)
- Row group size: 1 048 576 rows or 128 MiB, whichever is smaller

---

## 13. Failure Model & Recovery

### 13.1 Failure cases & responses

| Failure | Detection | Recovery |
|---|---|---|
| Worker crash (kill -9) | Lease expiry | Another worker acquires after TTL; resumes from last checkpoint |
| Worker hung but lease active | Heartbeat stops; lease expires | Same as above |
| Worker partition (split-brain) | New owner CAS bumps `generation`; old worker's `update_matview_checkpoint` rejected | Old worker observes rejection, releases lease, restarts |
| State-store corruption | SlateDB integrity check on open | Worker fails to acquire shard; operator runs `slateduck-ivm repair` |
| Catalog commit fails after Parquet write | Catalog returns error; orphan Parquet files | Existing orphan-file sweep cleans up after grace period |
| DBSP circuit panic | `catch_unwind` around drive loop | Worker exits; orchestration restarts it; lease eventually re-acquired |
| Output plane crash mid-publish | `matview.last_published_frontier` not advanced | Next attempt re-runs the publish (idempotent) |
| Schema change on base table | Detected at next input read; columns mismatch | Set matview status to `Stale`; operator runs `REFRESH ... FULL` |

### 13.2 Repair operations

- `slateduck-ivm repair --matview v --shard N`: drops shard N's state store and rebuilds from base data; lease must be unowned or held by this command.
- `REFRESH INCREMENTAL MATERIALIZED VIEW v FULL`: drops all state stores for `v`, rebuilds in parallel.
- `slateduck-ivm doctor`: read-only diagnostic; reports stuck shards, expired leases, lagging frontiers, mismatched generations, cost outliers.

### 13.3 Audit trail

Every repair operation appends a row to `matview_checkpoints` with a special `worker_id = '__repair__'` and a structured note in a future audit-log table (reserved tag `0x21`).

---

## 14. Observability

### 14.1 Metrics

Per matview, per shard (Prometheus):

```
slateduck_ivm_lag_ms{matview, shard}
slateduck_ivm_throughput_rows_per_sec{matview, shard}
slateduck_ivm_state_bytes{matview, shard}
slateduck_ivm_s3_put_total{matview, shard}
slateduck_ivm_s3_get_total{matview, shard}
slateduck_ivm_checkpoint_count{matview, shard}
slateduck_ivm_lease_holds_total{matview, shard, worker}
slateduck_ivm_lease_takeovers_total{matview, shard}
slateduck_ivm_circuit_panic_total{matview, shard}
slateduck_ivm_output_publish_total{matview}
slateduck_ivm_output_publish_latency_ms{matview}
```

### 14.2 Tracing

OpenTelemetry spans, named:

- `ivm.discover` — catalog poll
- `ivm.claim` — lease acquisition
- `ivm.read_input` — base data read
- `ivm.drive_circuit` — DBSP execution
- `ivm.flush_state` — state-store write
- `ivm.checkpoint` — catalog checkpoint append
- `ivm.publish` — output plane publish

### 14.3 Logs

Structured (JSON) at `info` for lifecycle events, `warn` for lease loss / stale view, `error` for unrecoverable failures.

### 14.4 `doctor` output

```
$ slateduck-ivm doctor --catalog-path …

Worker count: 3
Matviews: 12 active, 1 stale, 0 rebuilding

Per matview:
  events_by_day           shards=8/8 healthy   lag_p99=1.2s
  user_sessions           shards=8/8 healthy   lag_p99=3.4s
  orders_pivot            shards=4/4 healthy   lag_p99=0.8s
  fraud_signals           shards=2/8 STUCK     lag_p99=>5m   ⚠
    shard=3 owner=ivm-2   lease=expired-7m-ago
    shard=5 owner=ivm-1   lease=active state=fail-to-flush err=…
  …

Recommendations:
  - fraud_signals shard=3: lease expired; will be reclaimed at next poll
  - fraud_signals shard=5: investigate; consider repair
```

---

## 15. Testing Strategy

Five layers:

> **Current testing supplement:** ROADMAP.md now defines a 10-tier test plan and makes `IvmOracle` the first v0.14 deliverable. The five layers below remain useful categories, but implementation work should use the tiered mapping below:
>
> | Tier | Current requirement |
> |---|---|
> | Tier 1–3 | Unit/property/catalog/PG-wire tests on every PR |
> | Tier 4–5 | MinIO object-store and live client compatibility; requires `MinioHarness`, `CatalogHarness`, `PgWireHarness` |
> | Tier 6b–6f | IVM correctness suites: joins, hardening, operators, WASM/DISTINCT |
> | Tier 7 | Fault injection for catalog, IVM worker, and toxiproxy network failures |
> | Tier 8 | 24 h soak and 16-shard scale-out on dedicated EC2 runner |
> | Tier 9 | Credential isolation, TLS/auth, SQL injection guards, rate limiting |
> | Tier 10 | Benchmark regression gates against checked-in baselines |
>
> Before v0.14 EC-01 implementation, add `IvmOracle::assert_equivalent(view_sql, deltas)` to `slateduck-testkit` and run it against v0.11–v0.13 GROUP BY + JOIN cases.

### 15.1 Unit tests

- Each catalog method: happy path, conflict, idempotence
- DBSP plan compilation: each supported SQL pattern, each rejected pattern
- Shard-key inference: each heuristic case
- Key-range assignment: integer, timestamp, string, composite

### 15.2 Property tests

- Lease CAS: never two simultaneous owners under concurrent claim attempts
- Checkpoint sequence: monotone per shard
- Input source filtering: shard predicate ∩ data-file rows = expected
- Re-sharding: union of all new shards = union of all old shards (exact contents)

### 15.3 Integration tests

- `tests/integration_tests.rs` — single-shard, single-table, append-only flow
- `tests/sharded_tests.rs` — multi-shard scale-out
- `tests/join_tests.rs` — each join strategy
- `tests/restart_tests.rs` — kill/restart at every interesting boundary

### 15.4 Fault injection (v0.14)

`fail-parallel` covering:

- Worker death at every code path
- S3 returning 503 / partial / slow
- SlateDB compaction concurrent with checkpoint
- Lease expiry race during heartbeat

### 15.5 Streaming benchmark (continuous)

`benches/tpch_streaming.rs` runs TPC-H Q1, Q3, Q5 against synthetic streaming inputs:

- 100 k rows/sec input rate
- 8 shards
- 5-second freshness target
- Asserts: throughput, lag p99, correctness vs DuckDB single-shot reference

Run nightly in CI. Failure blocks merge.

---

## 16. Performance Targets

| Metric | v0.11 | v0.12 | v0.13 | v0.14 |
|---|---|---|---|---|
| Single-shard ingest throughput (rows/sec, GROUP BY) | ≥ 50 k | ≥ 50 k | ≥ 50 k | ≥ 75 k |
| 8-shard ingest throughput (rows/sec, GROUP BY) | — | ≥ 350 k | ≥ 350 k | ≥ 500 k |
| Freshness lag p99 (LocalFS, 5 s target) | ≤ 5 s | ≤ 5 s | ≤ 5 s | ≤ 3 s |
| Freshness lag p99 (S3 Express, 5 s target) | ≤ 10 s | ≤ 10 s | ≤ 10 s | ≤ 5 s |
| Backfill rate (rows/sec, 8 shards) | — | ≥ 1 M | ≥ 1 M | ≥ 1.5 M |
| Worker restart recovery (single shard) | ≤ 60 s | ≤ 60 s | ≤ 60 s | ≤ 30 s |
| State-store size overhead vs base data | ≤ 2× | ≤ 2× | ≤ 2× | ≤ 1.5× |
| TPC-H Q1 maintained correctly (1 GB streaming) | ✓ | ✓ | ✓ | ✓ |
| TPC-H Q3 maintained correctly (1 GB streaming) | — | — | ✓ | ✓ |
| TPC-H Q5 maintained correctly (1 GB streaming) | — | — | ✓ | ✓ |

All numbers verified on a c6i.4xlarge with S3 Standard in the same region.

---

## 17. Security & Multi-Tenancy

### 17.1 IAM model

- IVM workers need: read on catalog prefix, read+write on state-store prefix, read on base-data prefix, write on output-table-data prefix.
- They do **not** need write on the catalog except through the bounded surface of the four catalog methods. Implementation enforces this by giving the worker a `CatalogWriter` restricted to matview operations.
- Recommended IAM split: a dedicated `slateduck-ivm` role with the minimum policy documented in `docs/deployment/iam-policies.md`.

### 17.2 Tenant isolation

When multiple tenants share a warehouse:

- Matview definitions, deps, checkpoints, shards are all tagged with the catalog's schema namespace.
- A tenant's PG role's grants apply identically: `CREATE INCREMENTAL MATERIALIZED VIEW` requires schema-level `CREATE`.
- State stores live under `{state_prefix}/matviews/{matview_id}/...` and are not accessible through the SQL surface.

### 17.3 View SQL provenance

Stored `view_sql` is the literal user input. It is parsed only by DBSP-targeted compilation. No `EXEC`, no shell-out, no `IMPORT`. Even if a user could construct malicious SQL, the compilation surface is the same one DataFusion provides.

---

## 18. Cost Model

Empirical numbers (target, measured in v0.14):

For a single GROUP BY view with 8 shards, freshness = 5 s, ingest = 50 k rows/sec:

| Component | S3 Standard cost/month | S3 Express cost/month |
|---|---|---|
| Worker S3 PUTs (state) | ~$15 | ~$30 |
| Worker S3 GETs (input) | ~$10 | ~$10 |
| Output S3 PUTs (Parquet) | ~$3 | ~$6 |
| Storage (state + output) | ~$5 | ~$25 |
| **Total per view** | ~$33 | ~$71 |

Compute cost on c6i.large (~$60/month full-time) often dominates at low ingest rates; storage and request costs dominate at high ingest rates. Documented in `docs/performance/ivm-cost-model.md`.

Cost-control knobs (all in v0.14):
- Increase freshness target → fewer flushes → linear PUT savings
- Reduce shard count → less per-shard overhead
- Enable state compaction
- Move state store to S3 Standard even if catalog is on S3 Express

---

## 19. Documentation Deliverables

| Path | Phase | Audience |
|---|---|---|
| `docs/concepts/incremental-views.md` | v0.11 | Users |
| `docs/architecture/ivm-plane.md` | v0.11 | Architects, contributors |
| `docs/operations/incremental-materialized-views.md` | v0.11 → current v0.15 | Operators |
| `docs/reference/sql-ivm.md` | v0.11 → v0.13 | Users |
| `docs/design-decisions/ivm-on-immutable-substrate.md` | v0.11 | Reviewers |
| `docs/deployment/ivm-iam-policies.md` | v0.11 | Operators |
| `docs/performance/ivm-cost-model.md` | current v0.15 | Operators |
| `docs/design-decisions/ivm-retrospective.md` | current v0.17 | Future contributors |

All pages must pass `mkdocs build --strict` and have non-trivial content (no stubs).

---

## 20. Phased Milestones

> **Current phased-plan supplement:** The original milestone labels below are retained for historical context. The implementation sequence now follows ROADMAP.md:
>
> 1. **Pre-v0.14 gates:** DBSP/shim/DD architecture decision; planner migration strategy; `IvmOracle`; testkit harness scope; implementation-plan reconciliation.
> 2. **v0.14:** Join correctness release — EC-01, aggregate tiers, volatility gate, oracle/property tests.
> 3. **v0.15:** Operational hardening — DAG/frontiers, native state persistence, cost model, backup/restore, schema evolution, fault injection, observability.
> 4. **v0.16:** Operator completeness — windows, ORDER BY, LIMIT/OFFSET, correlated subqueries, recursive CTEs, non-deterministic capture.
> 5. **v0.17:** Feature hardening / IVM GA gate — WASM UDFs, adaptive mode, ref-counted DISTINCT, 24 h soak, 16-shard scale benchmark.
> 6. **v0.18:** DuckLake Catalog Standard Interface — `table_changes()`, stable rowid, snapshot leases, NOTIFY, extension schema tables, encryption pass-through, opaque mixed frontiers.

### v0.11 — Foundations (matches ROADMAP §v0.11)
- Catalog schema, SQL surface, single-shard runtime, end-to-end demo

### v0.12 — Sharding (matches ROADMAP §v0.12)
- Lease & heartbeat, per-shard state stores, sharded scale-out demo, re-sharding

### v0.13 — Joins (matches ROADMAP §v0.13)
- Broadcast, co-partition, reshuffle; TPC-H Q3 / Q5

### Historical v0.14 — Operational Hardening (now ROADMAP v0.15)
- Native `SlateDbTrace`, cost optimization, observability, `REFRESH ... FULL`, fault injection

### Historical v0.15 — Feature Completeness (now split across ROADMAP v0.16 and v0.17)

Goal: any SQL view that can be written against a static DuckDB table can be maintained incrementally. Adds the advanced operators deferred from v0.11–v0.14.

**Window functions** require an `SlateDbOrderedTrace` variant that maintains per-partition sorted state. Partition-local windows (PARTITION BY = shard key) are fully parallel. Total-order windows force `shard_count = 1` and route through a merge-sort writer in the output plane. Key module changes:

- `trace.rs`: add `SlateDbOrderedTrace` backed by a B-tree-sorted SST layout
- `output.rs`: add `merge_sorted_parquet_writer` that merges N shard outputs into a single sorted Parquet
- `circuit.rs`: lower window function nodes in the DataFusion logical plan to DBSP `window` operators

**`ORDER BY` and `LIMIT`/`OFFSET`** reuse the ordered trace. Top-N views use DBSP's `top_k` operator for a bounded heap; shard-local top-N merged by the output plane.

**Correlated subqueries** require a decorrelation pass over the DataFusion `LogicalPlan` before lowering to DBSP. DataFusion already provides `PullUpCorrelatedPredicates` and `DecorrelatePredicateSubquery`; we apply them in `plan.rs` before the DBSP lowering step. Any plan that survives decorrelation contains only joins and aggregations.

**Recursive CTEs** map to DBSP's `iterate` operator. The `plan.rs` lowering step detects cycles in the CTE dependency graph and wraps the recursive body in `iterate`. Termination via frontier advancement. `max_iterations` guard for divergent queries.

**Non-deterministic function capture** is a pre-pass in `circuit.rs`: before executing a batch, scan the plan for allow-listed functions (`now()`, `random()`, etc.), sample each once, substitute a `Literal` node, and record the sampled value in the checkpoint row. Repair replays with the stored value.

**WASM UDFs** require:
- New catalog table `matview_udfs` (tag `0x21`) — schema identical to `MatviewRow` pattern
- New `CREATE/DROP/ALTER FUNCTION` DDL in `slateduck-sql`
- `wasmtime` as a workspace dependency in `slateduck-ivm`
- A `WasmExecutor` struct in `circuit.rs` that hydrates a compiled module, bounds fuel + memory, and maps Arrow arrays to/from WASM linear memory
- UDF version pinning at view creation; migration via `ALTER INCREMENTAL MATERIALIZED VIEW ... USING FUNCTION ... VERSION N` (triggers `REFRESH ... FULL`)

**Key module diffs for v0.15:**

| Module | Change |
|---|---|
| `crates/slateduck-core/src/tags.rs` | Add `TAG_MATVIEW_UDFS = 0x21` |
| `crates/slateduck-core/src/rows.rs` | Add `UdfRow` protobuf schema |
| `crates/slateduck-catalog/src/writer.rs` | `create_udf`, `drop_udf`, `replace_udf` |
| `crates/slateduck-catalog/src/reader.rs` | `get_udf`, `list_udfs` |
| `crates/slateduck-sql/src/grammar/` | `CREATE/DROP/ALTER FUNCTION` grammar additions |
| `crates/slateduck-ivm/src/trace.rs` | `SlateDbOrderedTrace` |
| `crates/slateduck-ivm/src/plan.rs` | Decorrelation pass, window lowering, `iterate` lowering, non-det capture pass |
| `crates/slateduck-ivm/src/circuit.rs` | `WasmExecutor`, top-k operator, window operator |
| `crates/slateduck-ivm/src/output.rs` | `merge_sorted_parquet_writer` |

### v1.0 GA gate
- v0.11–v0.17 acceptance tests all green; the IVM GA gate item in `## v1.0` of the roadmap.

### Post-1.0 (out of scope for this plan)
- Continuous integrity-constraint checking as a special case of IVM
- Cross-warehouse views (single-warehouse only through v1.0)
- Raw DD (non-DBSP) for Datalog / graph algorithms beyond `CONNECT BY`

---

## 21. Open Questions Tracker

| # | Question | Status | Owner | Decision deadline |
|---|---|---|---|---|
| 1 | Tag allocation: `0x1D–0x20` vs packed `0xFD` subspace | Open | — | Before v0.11 alpha |
| 2 | Share binary with `slateduck-pgwire` vs separate | Recommended: separate | — | Before v0.11 alpha |
| 3 | Default `shard_count` for new views | Open | — | Before v0.11 GA |
| 4 | View outputs as regular data files vs separate namespace | Recommended: regular data files | — | Before v0.11 alpha |
| 5 | How to expose freshness in SQL (`MATVIEW_LAG()` function vs system view) | Both | — | Before v0.11 GA |
| 6 | DBSP integration: direct crate dep vs vendored fork | Recommended: direct dep, version-pin | — | Before v0.11 alpha |
| 7 | Lease eviction policy details (TTL value, heartbeat interval) | Open: defaults 30 s / 10 s | — | Before v0.12 |
| 8 | Cost model defaults: S3 PUT throttling, compaction cadence | Open; empirical | — | current v0.15 |
| 9 | Schema-evolution UX: auto-stale vs auto-rebuild | Recommended: auto-stale, explicit refresh | — | current v0.15 |
| 10 | Multi-warehouse views: in scope for v1.x or v2.x? | Out of scope for v0.11–v1.0 | — | Post-1.0 |
| 11 | WASM runtime: `wasmtime` vs `wasmi` (interpreter, no JIT) | **Resolved: `wasmtime` 29.x** — chosen for throughput; pinned major version | — | Resolved v0.17 |
| 12 | Window functions in sharded mode: error or auto-downgrade to `shard_count = 1`? | Recommended: error with a clear message | — | Before v0.16 alpha |
| 13 | Recursive CTE `max_iterations` default: 100 or unbounded with cost-based cap? | **Resolved: 100** | — | Resolved v0.16 |
| 14 | Non-deterministic function allow-list: user-extensible or hardcoded? | **Resolved: hardcoded; UDFs cover extension** | — | Resolved v0.16 |
| 15 | Computation backend: extend shim, migrate to DBSP, or switch to `differential-dataflow`? | Open; must choose explicitly | — | Before v0.14 implementation |
| 16 | SQL planner: stay on ad-hoc `IvmPlan` through v0.15, or migrate incrementally to DataFusion `LogicalPlan`? | Open; v0.16 depends on it | — | Before v0.14 sprint planning |
| 17 | `IvmOracle` API shape and ownership: `slateduck-testkit` helper vs `slateduck-ivm` test module? | Recommended: `slateduck-testkit` helper | — | First v0.14 PR |
| 18 | Testkit harness gap closure: implement `MinioHarness`, `CatalogHarness`, `PgWireHarness` now or per-tier? | Recommended: scope in v0.14 | — | Before Tier 4–7 test work |
| 19 | If DBSP traits are not externally implementable, does v0.15 ship `IvmTrace` persistence or `SlateDbBatch` spill adapter? | Open; depends on Gate 1 | — | Before v0.15 |
| 20 | v0.16 recursive CTE fallback: hand-rolled fixed-point loop acceptable for GA, or de-scope if DBSP `iterate` is unavailable? | Open | — | Before v0.16 |
| 21 | v0.18 contract naming/versioning: how is the DuckLake Standard Interface versioned independently of pg-trickle? | Open | — | Before v0.18 |

This tracker is maintained alongside the implementation; resolved questions become design decisions documented in `docs/design-decisions/`.

---

## v0.17 Additions (IVM Feature Hardening)

### WASM UDF Integration (wasmtime 29.x)

- `matview_udfs` catalog table (tag `0x21`) stores compiled WASM modules
- `CREATE FUNCTION / DROP FUNCTION / ALTER FUNCTION ... REPLACE` DDL surface
- Per-batch pooled instance model: one `wasmtime::Instance` per UDF per batch
- Fuel limit: 10M instructions/row; memory limit: 64 MiB/instance
- WASI import validation: disallows all I/O imports for pure function sandboxing
- Determinism enforcement at CREATE time (non-deterministic → SQLSTATE 0A000)
- Version pinning: views pin to `udf_id`; migration triggers REFRESH FULL

### Adaptive DIFFERENTIAL/FULL Mode (CostMode::Adaptive)

- Per-view rolling statistics: rows_in, rows_out, ms_spent, last_full_cost
- Switch formula: `Δ_rows / N_rows × complexity_multiplier > threshold`
- Empirically calibrated multiplier table (TPC-H Q1/Q3/Q5 + TPC-DS Q4/Q47)
- Per-view override: `WITH (cost_mode = 'adaptive', adaptive_threshold = 0.3)`
- Calibration data: `benchmarks/v0.17-adaptive-calibration.json`

### Reference-Counted DISTINCT

- `__sd_ref_count: i64` auxiliary column in `IvmTrace` for DISTINCT views
- INSERT increments, DELETE decrements; visible when count > 0
- Set operators: UNION DISTINCT (MAX), INTERSECT (MIN), EXCEPT (clamp to 0)
- Property-based testing with proptest (500 sequences)

### Testing

- Tier 6f WASM UDF tests: 6 tests covering sandbox, fuel, memory, WASI, versioning
- Tier 6f DISTINCT property tests: proptest-based with reference oracle
- Tier 8 scale tests: TPC-H catalog, IVM streaming, soak (shortened), multi-shard

---

*End of implementation plan.*
