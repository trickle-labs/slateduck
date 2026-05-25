# Learning from pg-trickle: Lessons for `slateduck-ivm`

**Source studied:** `/Users/geir.gronmo/projects/pg-trickle1` (pg-trickle, a PostgreSQL 18 extension implementing IVM via DBSP-style differential dataflow)

**Audience:** SlateDuck IVM contributors planning v0.13.1+ and beyond.

**Goal:** Distill pg-trickle's hard-won design decisions, identify which translate cleanly to SlateDuck's architecture (DuckDB compute, SlateDB storage, single-writer ingest, multi-reader fanout), and flag where pg-trickle's Postgres-coupled choices do **not** apply to us.

---

## 1. TL;DR — Top Takeaways

| # | Lesson | Priority for slateduck-ivm | Where it applies |
|---|--------|----------------------------|------------------|
| 1 | **Differentiate the operator tree, not the SQL string.** Each relational operator gets a `Δ` rule, composed via the DBSP chain rule. | 🔴 Foundational | `plan.rs`, `circuit.rs` |
| 2 | **EC-01 "phantom row after DELETE" fix** in joins (Part 1a/1b/2/3 expansion) is mandatory for correctness. | 🔴 Foundational | `join.rs` |
| 3 | **Diamond detection + atomic refresh groups** (SAVEPOINT-equivalent) prevent inconsistent reads across convergent views. | 🟠 Important for v0.14+ | new `dag.rs` module |
| 4 | **Algebraic vs. semi-algebraic vs. group-rescan** aggregate classification with auxiliary columns (`__pgt_count`, `sum`, `count`, `M2`, …). | 🔴 Foundational | `trace.rs`, `plan.rs` |
| 5 | **Adaptive DIFFERENTIAL → FULL fallback** based on change-rate × query-complexity cost model. | 🟠 Important | `worker.rs`, `config.rs` |
| 6 | **Predicate pushdown into delta scan** and **semi-join key pre-filter** are cheap, high-impact optimizations. | 🟢 Easy win | `plan.rs` |
| 7 | **Change-buffer compaction** (cancel I/D pairs on same row_id) cuts state by 50–90 % for update-heavy workloads. | 🟢 Easy win | `source.rs` |
| 8 | **Frontier as JSONB / vector clock per upstream source**, persisted in catalog, drives crash-recovery resume point. | 🟠 Important | `state_store.rs` |
| 9 | **Property-based "differential ≡ full" oracle** in tests catches subtle delta bugs. | 🔴 Foundational | new `tests/property/` |
| 10 | **Volatility classification** (immutable / stable / volatile) gates which queries are eligible for incremental mode. | 🟠 Important | `plan.rs` validation |

---

## 2. pg-trickle Architecture in One Picture

```
PostgreSQL backend
├── api/                ← CREATE / ALTER / DROP / REFRESH STREAM TABLE
├── catalog.rs          ← pgt_stream_tables, pgt_dependencies, pgt_refresh_history
├── cdc/                ← trigger-based + WAL-decoder change capture
│   └── buffer.rs       ← pgtrickle_changes.changes_<oid> typed delta tables
├── dvm/                ← Differential View Maintenance core
│   ├── diff.rs         ← DiffContext, per-operator Δ rules
│   └── operators/      ← join.rs, aggregate.rs, distinct.rs, setop.rs, window.rs …
├── refresh/            ← orchestrator, codegen (delta SQL), MERGE executor
├── ivm.rs              ← IMMEDIATE mode (in-transaction synchronous)
├── dag.rs              ← topological sort, diamond detection, SCC for circular deps
└── scheduler/          ← EDF + tiered + event-driven LISTEN/NOTIFY
```

Key design choice: pg-trickle compiles each view-plus-deltas combination to **one self-contained SQL query** with a chain of CTEs (delta_left, delta_right, part1a, part1b, part2, part3, …) and lets Postgres' planner execute it. That keeps the implementation small and inherits Postgres' query optimizer — but the trade-off is per-refresh planning overhead (~30 ms), which pg-trickle mitigates by caching the SQL template with LSN placeholders.

**SlateDuck analogue:** we have DuckDB instead of Postgres. We can borrow the "compile to a single delta query and let the engine run it" model — DuckDB is even better-suited because of its vectorized execution and zero startup cost per query.

---

## 3. The DBSP Operator Differentiation Table

This is the most directly reusable piece. pg-trickle's `dvm/operators/` enumerates a Δ rule per logical operator:

| Operator | Differential Rule | Notes for slateduck |
|----------|------------------|-----------------------|
| `Scan(R)` | `Δ(Scan(R)) = ΔR` | Direct passthrough from CDC stream. |
| `σ_p(R)` (filter) | `Δ(σ_p R) = σ_p(ΔR)` | Push predicate into delta scan. |
| `π_L(R)` (project) | `Δ(π_L R) = π_L(ΔR)` | Linear; commutes with Δ. |
| `R ⋈ S` (inner join) | `(ΔR ⋈ S₁) ∪ (R₀ ⋈ ΔS) − (ΔR ⋈ ΔS)` (bilinear) | 3–8 UNION ALL parts depending on outer-join padding. |
| `R ⋈ᴸ S` (left join) | + NULL-padding transitions | Anti-join detection for first/last-match. |
| `R ⋈ᶠ S` (full join) | 8-part UNION ALL | Symmetric NULL handling. |
| `γ_G(R)` (group-by) | `γ(R'∣affected) − γ(R₀∣affected)` | Algebraic aggregates use auxiliary columns; group-rescan otherwise. |
| `DISTINCT R` | Count-based dedup via `__pgt_count` | Reference-count semantics. |
| `R ∪ S`, `R ∩ S`, `R − S` | Dual-branch merge with multiset semantics | Need integer multiplicities or `__pgt_count`. |
| `EXISTS / NOT EXISTS` | Semi/anti-join with delta-key pre-filter | Cheap to support. |
| Window functions | Partition-scoped recompute | No truly-incremental window state. |
| `ORDER BY … LIMIT k` (Top-K) | Scoped recompute when delta touches top region | Worth supporting in v0.15+. |
| `WITH RECURSIVE` | Semi-naive (insert-only) or DRed (mixed DML) | Target v0.15; requires DBSP `iterate` operator + fixpoint termination. |

**Recommendation for slateduck-ivm:** model the IVM plan as an explicit operator tree (rather than the current free-form `IvmPlan` with hardcoded GROUP BY + JOIN). Each operator implements a `differentiate(input_delta) -> output_delta` method. This makes adding new operators (DISTINCT, EXCEPT, semi-join) mechanical rather than ad hoc.

---

## 4. The EC-01 "Phantom Row After DELETE" Fix (Mandatory for Joins)

**Problem (from `src/dvm/operators/join.rs`):** when computing `Δ(R ⋈ S)` for a join where a row in `S` matching a deleted row in `R` has *also* been deleted, the naive bilinear expansion uses the **post-change** snapshots of `S` and misses the match, leaving stale rows in the materialized view.

**Fix:** split the bilinear expansion into:

- **Part 1a:** `ΔR_insert ⋈ S_post`   (new positives from new R rows)
- **Part 1b:** `ΔR_delete ⋈ S_pre`    (negatives must use the *pre-change* snapshot of S)
- **Part 2 :** `R_post ⋈ ΔS`            (symmetric for ΔS)
- **Part 3 :** `−(ΔR ⋈ ΔS)`            (correction term: subtract double-counted intersections)

`S_pre` is reconstructed as `S_post EXCEPT ALL ΔS_insert UNION ALL ΔS_delete`. To avoid expensive EXCEPT ALL on each refresh, pg-trickle caches `L₀` as a named CTE (DI-1, v0.17.0).

**Why this matters for SlateDuck:** our current `IvmJoinCircuit` (`join.rs`) handles broadcast/co-partitioned/reshuffle joins but does not yet enumerate insert/delete-side asymmetry. v0.13 is shipping; these correctness items are tracked for v0.13.1:

1. Make the join delta computation explicitly enumerate (ΔL_ins, ΔL_del, ΔR_ins, ΔR_del) cases.
2. Track or reconstruct the pre-change right side for the delete branch — or use a "two-phase" approach where the right side's state is timestamped per delta.
3. Add the Part-3 correction term.

Test oracle: insert into both tables, delete from both tables in the *same* refresh window, compare differential output to full recompute.

---

## 5. DAG Management: Topological Sort, Diamonds, Cycles

pg-trickle's `dag.rs` exposes three concepts SlateDuck currently lacks:

### 5.1 Topological refresh ordering

Standard Kahn's algorithm — pop nodes with in-degree 0, enqueue downstream, repeat. Guarantees an upstream view is fully refreshed before its consumers compute their delta.

**SlateDuck gap:** we currently treat each materialized view's worker as independent. Once we support ST-from-ST (a view that reads another view as input), we need this. **Recommended location:** new `slateduck-ivm/src/dag.rs`, owned by the IVM control plane.

### 5.2 Diamond detection (the key insight)

A **diamond** is the pattern `A → {B, C} → D`. If we refresh B then C independently, D may compute its delta against an inconsistent (B_new, C_old) pair.

pg-trickle's solution:

```rust
pub enum DiamondConsistency { None, Atomic }
pub enum DiamondSchedulePolicy { Fastest, Slowest }

pub struct Diamond {
    pub convergence: NodeId,         // D
    pub shared_sources: Vec<NodeId>, // A
    pub intermediates: Vec<NodeId>,  // B, C
}
```

- `Atomic`: wrap all diamond members in one SAVEPOINT; rollback entire group on any failure.
- `Slowest`: only fire D when all members of {B,C} have reached the same frontier.

**Detection:** O(V+E) — during topological sort, for each node record the set of ancestor "roots"; a node whose ancestors include the same root via multiple paths is a diamond apex.

**SlateDuck mapping:** we don't have SAVEPOINTs (SlateDB is append-only LSM), but we have **frontiers/sequence numbers**. Equivalent semantics:

- Each materialized view advertises a "consistent up to frontier F" marker.
- The convergence view D refreshes only when **all** upstreams have advertised the same frontier F (DBSP-style coordination).
- This is naturally the `Slowest` policy and aligns with DBSP's multi-input synchronization.

This is *exactly* the right model for SlateDuck because our single-writer/many-reader topology already serializes ingest frontiers — a downstream view simply waits until each upstream view's `state_store` reports `frontier ≥ F`.

### 5.3 Circular dependencies (Tarjan SCC + fixpoint)

pg-trickle v0.7.0+ supports recursive views via Tarjan SCC detection and semi-naive fixpoint iteration. **Target v0.15 for SlateDuck**: map to DBSP's `iterate` operator; base case is the seed, recursive term is the iterate body, termination detected by frontier convergence (output = input at fixed point). Bounded by `max_iterations` to prevent infinite loops. See ROADMAP.md v0.15 §Recursive CTEs for the full task breakdown.

---

## 6. Aggregate Classification

pg-trickle partitions aggregates into three tiers — this directly informs how much auxiliary state we need per group key.

| Class | Aggregates | State per group | Δ computation |
|-------|------------|-----------------|---------------|
| **Algebraic** | COUNT, SUM, AVG, STDDEV, VAR, CORR, REGR_*, BOOL_AND/OR, BIT_AND/OR/XOR | `sum`, `count`, `M2`, `nonnull_count` auxiliary columns | Fully invertible: merge formula gives new value from old + delta. |
| **Semi-algebraic** | MIN, MAX | Current min/max + (optionally) a small heap of top-k candidates | LEAST/GREATEST on insert; on delete of current extremum, rescan group. |
| **Group-rescan** | STRING_AGG, ARRAY_AGG, JSON_AGG, MODE, PERCENTILE_* | Just current value | Re-aggregate entire affected group on each delta — `O(group size × delta groups)`. |

**SlateDuck mapping:** our `trace.rs` and `plan.rs::AggregateKind` already model COUNT/SUM/AVG. We should:

1. Explicitly classify each `AggregateKind` variant with one of these three tiers in `plan.rs`.
2. For algebraic aggregates, store the auxiliary columns (e.g., `sum_arg`, `count_arg` for AVG) in `IvmTrace` so the result can be recomputed without revisiting source rows.
3. Reject (or fall back to FULL) semi-algebraic deletes that empty a group, until we implement group-rescan.
4. Add a "group-rescan" path that re-reads the affected group keys from the source — this is the fallback for `STRING_AGG` etc.

The auxiliary-column trick is particularly elegant: by storing `sum_arg` and `count_arg` separately, AVG becomes `sum_arg / count_arg`, and a delete of one row just subtracts from both, then re-divides. No floating-point drift, fully invertible.

---

## 7. Adaptive Mode Switching (DIFFERENTIAL ↔ FULL)

pg-trickle classifies queries into complexity buckets:

```rust
pub enum QueryComplexityClass {
    Scan,           // 1.0×
    Filter,         // 1.1×
    Aggregate,      // 1.5×
    Join,           // 2.5×
    JoinAggregate,  // 4.0×
}
```

Switch from DIFFERENTIAL to FULL when `Δ_rows / N_rows × complexity_multiplier > threshold` (default 50 %). The threshold is GUC-tunable (`pg_trickle.adaptive_full_threshold`). Statistics are tracked in `pgt_refresh_history` (avg ms per delta row vs. full refresh time).

**SlateDuck mapping:** our `CostMode` already exists in `config.rs`. Extend it to:

- Track per-view rolling stats (rows-in, rows-out, ms-spent, last-full-cost).
- Add a `CostMode::Adaptive` variant that swaps strategies based on the rolling cost.
- Surface metrics through `observability.rs`.

This is the single most impactful "production polish" item — without it, a large delta tanks throughput far below what a full recompute would cost.

---

## 8. Cheap, High-Impact Optimizations

These are small enough to implement in one or two PRs each.

### 8.1 Predicate pushdown into delta scan
When a `Filter` sits directly above `Scan`, push the predicate into the CDC delta-fetch query so we never materialize unfiltered delta rows. pg-trickle does this in `src/dvm/diff.rs::scan_pushed_predicate` — for UPDATE rows, it applies the predicate to both `old` and `new` column values.

### 8.2 Semi-join key pre-filter
Instead of joining `delta_orders ⋈ customers`, first project `DISTINCT cust_id` from `delta_orders` and use that as the probe side. Turns a sequential scan of `customers` into an index lookup. Trivial to add to our `hash_join_batch`.

### 8.3 Change-buffer compaction
For the same `row_id`, consecutive INSERT/DELETE pairs cancel out. Apply this during CDC buffer flush. pg-trickle reports 50–90 % buffer reduction on update-heavy tables. Maps directly to our SlateDB ingest path — we can compact a delta batch before it ever lands in the trace.

### 8.4 Append-only fast path
Detect INSERT-only views and skip DELETE/UPDATE plumbing entirely; revert on first non-INSERT. ~30 % throughput improvement reported. Easy to add as a `JoinStrategy`-like enum on the trace.

### 8.5 L₀ snapshot caching
Materialize the pre-change input as a named CTE / temp table once per refresh and reuse across all Part-1b/Part-2 references. We get this for free if we structure the delta computation as a DataFusion / DuckDB physical plan with a shared scan.

### 8.6 Auto-indexing on join / group-by keys
pg-trickle v0.16.0+ creates indexes on the stream table's join and group-by columns automatically. For SlateDuck, the equivalent is laying out Parquet files sorted by these keys (or maintaining sort runs in the trace). Worth adding to `parquet.rs::CompactionPolicy`.

---

## 9. Volatility Classification (Correctness Gate)

pg-trickle classifies each function appearing in the view definition:

- **IMMUTABLE** (e.g., arithmetic, string functions): always safe.
- **STABLE** (e.g., `now()`, `current_timestamp`): warning — value may differ between initial load and delta evaluation, causing drift.
- **VOLATILE** (e.g., `random()`, `clock_timestamp()`): hard reject for DIFFERENTIAL mode; allow only in FULL.

**SlateDuck:** DuckDB has the same notion via its function catalog. We should:

1. At `CREATE INCREMENTAL MATERIALIZED VIEW` time, walk the expression tree and look up each function's volatility.
2. Reject VOLATILE in `IvmPlan::compile`.
3. Warn (or downgrade to FULL) on STABLE.

Without this gate, users will silently get wrong results from `WHERE created_at > now() - interval '1 day'`.

---

## 10. Frontiers, Recovery, and the State Store

pg-trickle stores per-view frontier as a JSONB **vector clock**: `{"orders": 1000, "customers": 500}` mapping each upstream source to its high-watermark LSN. On crash, the scheduler resumes from this frontier and skips already-applied changes.

**SlateDuck mapping:** our `state_store.rs::ShardStateStore` already tracks per-shard state. Extend it to:

- Store a `BTreeMap<SourceId, Sequence>` frontier per view per shard.
- On worker start, read the frontier, skip CDC events with `seq ≤ frontier[source]`.
- Use this same vector clock to implement the diamond-consistency "all upstreams at frontier F" coordination from §5.2.

The JSONB choice is a Postgres-specific implementation detail; we'd use a Rust struct serialized via the existing SlateDB value encoding.

---

## 11. Testing: The "Differential ≡ Full" Oracle

pg-trickle's killer-app test is a property-based invariant:

```rust
// proptest harness, run after each random DML mutation:
let stream_table_contents = read_stream_table(view);
let full_recompute        = execute_full_query(view.definition);
assert_eq_multiset!(stream_table_contents, full_recompute);
```

100+ test files exercise this against:
- All 22 TPC-H queries (`e2e_tpch_tests.rs`)
- Nexmark streaming workload
- SQLancer-generated random queries (`e2e_sqlancer_tests.rs`)
- Diamond consistency (`e2e_property_diamond_tests.rs`)
- Z-set multiset semantics (`e2e_property_zset_tests.rs`)

**SlateDuck recommendation (high priority for v0.13.1/v0.14):**

1. Build a `slateduck-testkit` helper that takes a view SQL, applies a random DML sequence to the base tables, runs the IVM worker, then runs the same view SQL via DuckDB ad-hoc as the oracle.
2. Use `proptest` strategies to generate inserts/deletes/updates with realistic key distributions (including the diamond / phantom-delete edge cases).
3. Wire at minimum one TPC-H query (Q1 or Q5) as an end-to-end test.

Without this oracle, the EC-01 phantom-row class of bugs is essentially undetectable in code review.

---

## 12. Things pg-trickle Does That We Should *Not* Copy

| pg-trickle choice | Why it doesn't apply to SlateDuck |
|-------------------|-----------------------------------|
| SQL-string code generation with CTE chains | We have DuckDB's logical/physical plan API — emit `LogicalPlan` directly instead of strings. Avoids the 30 ms per-refresh SQL planning overhead pg-trickle has to cache around. |
| AFTER STATEMENT triggers for IMMEDIATE mode | We're an analytics engine with separate ingest writer; "synchronous IVM in the user transaction" doesn't fit. Stick with the deferred refresh model. |
| WAL decoder via logical replication slot | We have our own CDC stream from SlateDB sequence numbers. Cleaner and lower-latency than logical decoding. |
| Postgres advisory locks for refresh concurrency | We have single-writer semantics already; use the existing lease/fencing in `heartbeat.rs`. |
| `pgt_dependencies` catalog table | Our catalog already lives in `slateduck-catalog`; extend that, don't create a parallel system. |
| Citus integration for distribution | Not relevant — we shard via `shard_key.rs`. |
| `LISTEN/NOTIFY` event scheduler | Our worker model is pull-based on the CDC stream; the equivalent is just a tighter polling loop or a tokio `Notify`. |

---

## 13. Concrete Action Items for slateduck-ivm

Mapped to our roadmap milestones.

### v0.13.1 (Join Correctness Follow-Up)

1. **EC-01 phantom-fix:** implement insert/delete-asymmetric Part 1a/1b/2/3 expansion in `join.rs`. Add a regression test that deletes a matching pair from both sides in the same window.
2. **Aggregate classification:** annotate every `AggregateKind` with a `{Algebraic, SemiAlgebraic, GroupRescan}` tier in `plan.rs`. Add auxiliary columns for AVG/STDDEV.
3. **Volatility validation:** reject VOLATILE functions and warn on STABLE at view-creation time. (1-day task.)
4. **Property-based oracle:** ship at minimum a TPC-H Q1 or Q5 end-to-end "differential ≡ full" test.

### v0.14 (Multi-view DAG) — design now, build then

5. **DAG module:** new `slateduck-ivm/src/dag.rs` with Kahn topo-sort and diamond detection. Persist edges in the catalog.
6. **Frontier vector clocks:** extend `state_store.rs` to track per-source frontiers per view.
7. **Diamond `Slowest` policy:** convergence views wait until all upstreams reach a common frontier before refreshing.

### v0.15 (Production polish)

8. **Adaptive DIFFERENTIAL/FULL switching:** rolling cost statistics + `CostMode::Adaptive`.
9. **Change-buffer compaction:** cancel I/D pairs at ingest.
10. **Predicate pushdown** and **semi-join key pre-filter** in `plan.rs` / `join.rs`.
11. **Append-only fast path** detection on traces.
12. **Auto-indexing / sort-by**: extend `parquet.rs::CompactionPolicy` to lay out files sorted by join/group-by keys.

### v0.15 (Feature Completeness)

13. **Window functions** (partition-scoped recompute).
14. **TopK** (`ORDER BY … LIMIT k`) scoped recompute.
15. **DISTINCT / set ops** with `__pgt_count`-style reference counting.
16. **Recursive views** (Tarjan SCC + semi-naive fixpoint via DBSP `iterate`) — target v0.15 pre-GA.

---

## 14. Key pg-trickle Files to Re-Read When Implementing Each Item

| When working on … | Read … |
|-------------------|--------|
| Join EC-01 fix | `src/dvm/operators/join.rs` |
| Aggregate strategies | `src/dvm/operators/aggregate.rs` |
| Delta SQL codegen patterns | `src/refresh/codegen.rs`, `src/dvm/diff.rs` |
| Diamond detection | `src/dag.rs` |
| MERGE / upsert into stream table | `src/refresh/merge/mod.rs` |
| Adaptive fallback cost model | `src/refresh/mod.rs` (lines ~60–130) |
| Change-buffer compaction | `src/cdc/compact.rs` |
| Frontier persistence | `src/catalog.rs` (`pgt_stream_tables.frontier`) |
| Property test oracle | `tests/e2e_property_tests.rs`, `tests/e2e_property_diamond_tests.rs` |
| Operator coverage matrix | `docs/DVM_OPERATORS.md` |
| Theoretical grounding | `docs/research/DBSP_COMPARISON.md` |

---

## 15. Summary

pg-trickle is a textbook engineering of DBSP differential dataflow inside Postgres. The pieces that translate directly into our roadmap are:

1. The **operator-by-operator differentiation table** (§3) — adopt as the structural backbone of `IvmPlan`.
2. The **EC-01 phantom-row fix** (§4) — non-negotiable for join correctness.
3. **Diamond detection + frontier-coordinated refresh** (§5.2, §10) — the right model for multi-view DAGs in our single-writer/many-reader topology.
4. **Aggregate classification with auxiliary columns** (§6) — already half-there in `trace.rs`; finish it.
5. **Property-based "differential ≡ full" testing** (§11) — the only practical defense against subtle delta bugs.

Skip the Postgres-specific surface area (triggers, IMMEDIATE mode, WAL decoder, SAVEPOINTs, advisory locks). Our equivalents already exist or are simpler in SlateDuck's architecture.
