# Design Decision: IVM on an Immutable Substrate

## Status

Accepted — Shipped in v0.11.

## Context

SlateDuck stores all data in SlateDB, an LSM-tree key-value store optimised for object storage. The core invariant is that data is **append-only and immutable**: rows are never modified in place; MVCC is achieved by writing new versions at higher snapshots.

Incremental View Maintenance (IVM) is traditionally implemented with mutation-heavy techniques: in-place UPDATE of aggregate accumulators, delete-and-reinsert for MIN/MAX, and trigger-based row change capture. These approaches conflict with SlateDuck's immutable model.

The question was: **how do we implement IVM without violating the immutable-substrate invariant?**

## Decision

We implement IVM using a **Z-difference stream model** (inspired by DBSP) where the IVM worker maintains an in-memory algebraic state machine and periodically **snapshots its output** as a new set of inlined-insert rows in the output table.

Key choices:

### 1. Output table is a regular SlateDuck table

The IVM output is stored as ordinary inlined-insert rows in a catalog-registered output table. This means:

- Readers use the standard `list_inlined_inserts(table_id)` API with snapshot isolation — no special IVM read path.
- Time-travel works for free: read the output table at any historical snapshot.
- The output is durable on the same code path as user data.

### 2. State is held in-process; output is checkpointed

The running aggregation state (`AggState` per group) lives in the worker's heap. Only the *result* is written back to the catalog. This avoids WAL-style in-place mutation of accumulators.

The `MatviewCheckpointRow` records the highest `input_snapshot` and `output_snapshot` processed. On worker restart, the state is rebuilt from the output table (or re-derived from the source — both are equivalent for pure aggregation).

### 3. Leases via CAS prevent concurrent writes

Because the output table is written by the IVM worker, and SlateDuck has a single-writer-per-table constraint, we must ensure only one worker writes the output table at a time. This is enforced by a shard-level lease stored in `slateduck_matview_shard` using `DbTransaction + IsolationLevel::SerializableSnapshot`.

The lease protocol is safe under worker crash because the lease has a `lease_expires_unix_ms`; after the TTL, any other worker may acquire the lease.

### 4. DBSP crate as dependency, circuit.rs as shim

We take a workspace dependency on `dbsp = "0.299.0"` to use its algebraic type system and batch operators. However, `circuit.rs` wraps DBSP in a thin shim rather than adopting DBSP's full execution model. This gives us the incremental correctness guarantees of Z-difference algebra while keeping the integration surface minimal.

The shim approach also makes it easy to swap DBSP versions or replace it with a custom implementation in a future release without affecting the rest of the codebase.

## Alternatives Considered

### A. Materialise via DuckDB

Run the view SQL against DuckDB on every tick over the full base table.

**Rejected**: O(full table) cost per tick; does not scale.

### B. In-place mutation of accumulator rows

Store each group's accumulator as a mutable row in SlateDB, updated on every insert.

**Rejected**: Conflicts with the append-only invariant; requires careful concurrency control for every aggregate type.

### C. Use Differential Dataflow directly

Replace the DBSP shim with a full Differential Dataflow worker (Timely + DD).

**Deferred**: Higher complexity. Consider for v0.13+ when join support requires a richer dataflow model.

### D. Trigger-based CDC

Install database triggers to capture row changes and feed them to a separate materialization service.

**Not applicable**: SlateDuck does not have traditional triggers. CDC is implicit in the append-only snapshot model.

## Consequences

- **MSRV bump**: DBSP 0.299.0 requires Rust ≥ 1.93.0. The workspace `rust-version` was updated from `1.86` to `1.93`.
- **New crate**: `slateduck-ivm` is a new workspace member. It is optional infrastructure; the core protocol crates (`slateduck-core`, `slateduck-catalog`) have no compile-time dependency on it.
- **Output table is read-only for users**: The SQL layer must reject direct DML on output tables. This is enforced by the `matview_output_table_is_not_writable_by_users` integration test (v011_tests.rs).
- **State rebuild on restart**: The current implementation rebuilds in-process state from checkpoint history on worker restart. For high-cardinality GROUP BY this may take several ticks. A persistent state format is planned for v0.13.

## See Also

- [IVM Architecture](../architecture/ivm-plane.md)
- [Concepts: Incremental Views](../concepts/incremental-views.md)
- [Bounded SQL](bounded-sql.md)
