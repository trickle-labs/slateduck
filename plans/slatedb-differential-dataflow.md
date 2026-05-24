# SlateDB × Differential Dataflow: A Research Report

> **Status:** Exploratory research. Nothing here is committed to any roadmap.
> **Audience:** SlateDuck maintainers and contributors evaluating where incremental
> computation could fit into an object-storage-native lakehouse.
> **Scope:** Architecture analysis, prior art, project ideas, and concrete
> SlateDuck/DuckLake-shaped opportunities.

---

## 1. Executive Summary

Differential dataflow (DD) is a Rust framework for *incrementally maintaining*
the result of relational and graph computations as the input collections change.
It has two properties that make it interesting in the SlateDB context:

1. Its data model is a multiset of `(record, time, diff)` triples — i.e. it
   already speaks "log of changes," which is exactly what an LSM tree like
   SlateDB stores natively.
2. Its in-memory state (*arrangements* / *traces*) is layered and immutable in
   the same way SLateDB SSTs are layered and immutable, with a *compaction*
   step that physically forgets history older than a *frontier*. The structural
   analogy to an LSM tree is unusually close.

The combination of (a) SlateDB providing bottomless, multi-reader object
storage and (b) DD providing low-latency incremental joins/aggregations/loops
points at a class of systems that today either don't exist or require expensive
distributed databases (Materialize, ksqlDB, Flink + RocksDB + Kafka): **stateful
streaming computation with an S3-only storage tier and a stateless worker
fleet.**

For SlateDuck specifically, DD opens a credible path to **incremental
materialized views over DuckLake** — i.e. Parquet-backed result tables that
stay continuously fresh as the lakehouse ingests data, maintained by stateless
workers, with the materialized state durably checkpointed to the same bucket as
the rest of the catalog.

The rest of this document develops that idea and several adjacent ones.

---

## 2. Background

### 2.1 What differential dataflow actually is

Differential dataflow ([TimelyDataflow/differential-dataflow](https://github.com/TimelyDataflow/differential-dataflow))
is a Rust library built on top of *timely dataflow* (a low-level distributed
streaming runtime). The programming model is:

- A *collection* is a multiset of records. It is represented physically as a
  stream of update triples `(record, time, diff: isize)`. A diff of `+1`
  means "this record exists once more starting at `time`"; `-1` means "one
  fewer". Counts can exceed 1 and can be negative during intermediate
  computations.
- Operators are the usual relational/functional ones: `map`, `filter`,
  `join`, `reduce`, `count`, `distinct`, `semijoin`, `concat`, plus the
  fixed-point operator `iterate` (which makes DD strictly more expressive
  than SQL — Datalog, graph reachability, k-core, regular path queries, etc.).
- For each output collection the runtime emits the *exact* sequence of
  `(record, time, diff)` triples that describes how the output changes as
  inputs change.

The headline performance number from the upstream README is that a recomputation
that takes ~15 seconds from scratch can absorb an input change and emit the
correct delta in ~200 microseconds. The asymmetry between cold compute and
incremental update is what makes DD interesting for any system where the
input is a continuously growing log.

### 2.2 Arrangements and traces

The mechanism that makes DD efficient is the *arrangement*: an indexed,
shared, append-only data structure that stores the history of a collection
keyed by `K → V`. Internally, an arrangement is a *trace* — a sequence of
immutable *batches*, each batch covering a half-open interval of timestamps
and containing the `(K, V, time, diff)` updates that fell into that interval.

Two operations matter:

- **Insertion** appends a new batch at the leading edge.
- **Merge / compaction** combines older batches and, crucially, *advances
  their compaction frontier*: once no live computation can distinguish times
  before some `frontier`, all updates with `time < frontier` can be
  consolidated by summing their diffs per `(K, V)`. Cancelling updates
  vanish; survivors are coalesced. The trace gets smaller without changing
  any observable answer.

If that description sounds suspiciously like an LSM tree with tombstones and
leveled compaction, that is because **it is the same data structure pattern**
expressed in a different domain. A DD trace is a temporal multiset; an LSM
tree is a keyed map. The merging discipline, the immutable-batch invariant,
and the "compact when you can prove no one cares about the old history"
property are all isomorphic.

### 2.3 What SlateDB provides

SlateDB ([slatedb/slatedb](https://github.com/slatedb/slatedb)) is an LSM-tree
embedded KV store whose SSTs live directly in object storage. Relevant
features for this discussion:

- **Object-storage-native immutable SSTs.** Every flushed SST is an
  immutable object with a stable name and content hash. Compaction produces
  new objects; old ones are GC'd after a retention window.
- **Disaggregated compaction & GC.** Background maintenance runs in a
  separate process from the writer, on cheap spot compute, without touching
  the write or read path.
- **Single writer, many readers.** Writers are fenced via compare-and-set on
  the manifest. Readers tail the manifest and serve queries directly from
  object storage with local block/SST caching.
- **Checkpoints and clones.** Cheap metadata operations create branchable
  point-in-time views of the database.
- **Roadmap CDC.** [Issue #249](https://github.com/slatedb/slatedb/issues/249)
  tracks first-class change-data-capture, i.e. the engine itself producing a
  stream of `(key, old_value, new_value, seqno)` records.
- **Adopters already in the streaming space.** SlateDB's adopter list
  includes [Malstrom](https://github.com/MalstromDevelopers/malstrom) and
  [Volga](https://github.com/volga-project/volga) (stream processing
  frameworks), and [s2](https://github.com/s2-streamstore/s2) (a streaming
  log service), so the SlateDB-as-streaming-state-store thesis is being
  validated in the wild.

---

## 3. The Structural Analogy: DD Traces vs SlateDB SSTs

The single most useful framing for this report is this table:

| Concern                       | Differential Dataflow arrangement              | SlateDB                              |
| ----------------------------- | ---------------------------------------------- | ------------------------------------ |
| Atom of storage               | `Batch<K, V, T, R>`                            | SST                                  |
| Immutability                  | Batches are immutable once sealed              | SSTs are immutable once flushed      |
| Append point                  | New batch at leading edge                      | New SST flushed from MemTable        |
| Reorganization                | Trace merge with frontier advancement          | Leveled / tiered compaction          |
| Retention knob                | Compaction frontier `since`                    | GC retention / manifest checkpoints  |
| Lookup                        | Sorted batch + cursor                          | Bloom filter + block cache + binary search |
| Sharding                      | Per-worker exchange on key                     | Per-key range (future, via clones)   |
| Durability                    | None by default — RAM only                     | Object storage + WAL                 |
| Recovery                      | Replay input from upstream                     | Open manifest, fetch SSTs            |

The shaded last two rows are the gap that this document is about. Stock DD
keeps all arrangement state in process memory; if the worker dies, the trace
must be rebuilt from the source. That is *the* operational pain point of every
production DD deployment (Materialize built an entire subsystem called
`persist` precisely to address it; see §5).

SlateDB plugs exactly into those last two rows.

---

## 4. How to Implement DD on Object Storage

There are at least three layers at which DD and SlateDB can be wired
together, in increasing order of ambition.

### 4.1 Layer 1: Source-of-truth log in SlateDB

The minimal integration: use SlateDB as a durable, replayable input log.
Workers run vanilla in-memory DD; on startup they scan SlateDB from a
checkpoint sequence number forward to rebuild arrangements; periodically they
record "I have processed up to seqno N" back into SlateDB.

- **What it buys you:** durable input, multi-reader replicas (each replica
  is just another DD worker pool reading the same log), zero coordination on
  the input side because SlateDB already gives you a totally-ordered
  single-writer log.
- **What it does not buy you:** recovery is still `O(history)` because the
  arrangements themselves are not persisted. For long-lived joins this is
  a deal-breaker.
- **Effort:** small. This is essentially what a Kafka source connector does,
  just pointed at SlateDB.

This is a good *first* milestone — it validates the wiring without taking on
the hardest design problem.

### 4.2 Layer 2: Persistent arrangements in SlateDB

Treat each DD arrangement as a separate logical SlateDB column family /
key prefix:

```
arrangements/<dataflow_id>/<arrangement_id>/<batch_id>  → serialized batch
arrangements/<dataflow_id>/<arrangement_id>/_frontier   → since/upper bounds
```

A `Batch<K, V, T, R>` is a sorted run of `(K, V, T, R)` tuples, which is
*structurally* an SST. There are two implementation paths:

1. **Naïve:** serialize each DD batch (e.g. with Arrow IPC or columnar
   protobuf) and `put` it as a single SlateDB value. SlateDB will pack it
   into its own SSTs. This works but pays a double-LSM tax: DD's batch
   layering sits on top of SlateDB's SST layering, and DD's frontier
   compaction is independent of SlateDB's leveled compaction.
2. **Native:** implement `differential_dataflow::trace::Trace` directly on
   top of SlateDB scans. Each DD batch is materialized as a *contiguous key
   range* in SlateDB, with the SlateDB sort order matching the DD trace
   sort order on `(K, V, T)`. A trace cursor is then a SlateDB range scan
   with a wrapper that decodes diffs. Frontier-advance compaction becomes a
   SlateDB `compact_range`-style background job that rewrites updates with
   `time < since` into consolidated rows.

The native path is more work but aligns the two compaction loops and lets
SlateDB's existing block cache, bloom filters, and disaggregated compaction
do their job. The most natural deployment is: **one SlateDB database per
dataflow**, with one trace per arrangement encoded as a key prefix.

Key design questions:

- **Serialization format for `(K, V, T, R)`.** Arrow `RecordBatch` is the
  natural fit, especially in the SlateDuck context where downstream consumers
  speak Arrow. Per-batch dictionary encoding gives excellent compression for
  the small-cardinality `K` values typical of join keys.
- **Diff type.** DD supports arbitrary abelian-group diffs (`isize`,
  `(isize, isize)` for sum + count, etc.). The encoding has to be generic
  over `R`. A `Diff` trait with `encode`/`decode` to bytes is the right
  shape; for `isize` it is varint.
- **Time encoding.** For pure stream processing, `T = u64` (sequence number)
  is enough. For richer use cases (event time, hybrid logical clocks), the
  time encoding gets more interesting and you may want a partial-order
  capable time (tuples).
- **Crash recovery.** A persisted trace lets a worker resume from the last
  durable frontier instead of replaying from the source. The trade-off is
  that you must `await_durable` (or accept a bounded staleness window) when
  sealing batches.

### 4.3 Layer 3: Disaggregated stateful streaming runtime

Once arrangements live in SlateDB, the entire DD execution model can adopt
SlateDB's operational pattern:

- **Single writer per dataflow shard.** The writer is the timely worker that
  owns a key range. It is fenced via SlateDB's manifest CAS just like any
  SlateDB writer.
- **Stateless query replicas.** Read-only DD workers can attach to the same
  arrangements and serve `interactive` queries (point lookups, filtered
  scans, ad-hoc joins against the maintained state). This is the equivalent
  of Materialize's "compute clusters" reading from "persist", but without
  the operational complexity of Materialize's storage stack.
- **Spot-compute compaction.** Trace compaction (frontier-advance) can run
  in a separate process on cheap interruptible compute, just like SlateDB's
  disaggregated compaction. The frontier `since` is the only piece of
  coordination state and it advances monotonically.
- **Branch and time-travel.** Because SlateDB clones are O(1), you can
  branch a dataflow's *state* at any point — invaluable for backfills,
  debugging, schema migrations, and A/B testing of dataflow changes.

This is the destination architecture. The rest of the report assumes it is
the target even when discussing smaller, earlier steps.

---

## 5. Prior Art

### 5.1 Materialize's `persist`

[Materialize](https://github.com/MaterializeInc/materialize) is the
commercial home of differential dataflow, and they have spent ~four years
building [`src/persist`](https://github.com/MaterializeInc/materialize/tree/main/src/persist):
a transactional, time-versioned, S3-backed shard store designed specifically
to durably hold DD traces. It is, in effect, a bespoke object-store LSM
optimized for `(K, V, T, R)` updates. Studying `persist` is the single
highest-leverage research activity for anyone serious about this direction;
much of what it solves (sharded writers, garbage collection of unreferenced
batches, frontier advancement) is exactly the surface area SlateDB already
solves.

The interesting observation is that `persist` predates SlateDB and was built
because nothing equivalent existed. SlateDB now does exist, and a sizeable
fraction of `persist`'s code is solving SlateDB-shaped problems. A clean
re-implementation of "persist for DD" on top of SlateDB is plausibly an
order of magnitude smaller than `persist` itself.

### 5.2 Feldera / DBSP

[Feldera](https://www.feldera.com/) is built on
[DBSP](https://github.com/feldera/feldera), a sibling theory to DD developed
by Mihai Budiu and Frank McSherry. DBSP is strictly more SQL-shaped (no
fixed-point iteration in the user-facing language; integer-valued weights
only), which makes it a better fit for "incrementally maintain a SQL view"
than for graph algorithms.

DBSP also has its own persistence story (RocksDB-backed in the open-source
version, with newer object-storage spill support in development). For
SlateDuck, DBSP is arguably a *better* upstream than DD itself because the
target is SQL materialized views, not Datalog. The same SlateDB-as-trace-
storage argument applies almost verbatim.

### 5.3 SlateDB-native streaming projects

- **[Malstrom](https://github.com/MalstromDevelopers/malstrom):** Rust
  stream processing framework explicitly using SlateDB as state backend.
  Worth reading their state-store integration as a reference.
- **[Volga](https://github.com/volga-project/volga):** stream processing on
  SlateDB; covers more of the Flink-shaped surface (windowing, sessions).
- **[s2](https://github.com/s2-streamstore/s2):** an object-storage-native
  log abstraction (effectively Kafka-on-S3 powered by SlateDB). This is the
  natural *input* substrate for any DD-on-SlateDB system.

None of these implement *differential* incremental computation (joins,
aggregations, fixed-point). That niche is open.

### 5.4 Other adjacent work worth tracking

- **Apache Arrow + DataFusion** as the query engine that consumes
  arrangements (SlateDuck already depends on DataFusion).
- **Iceberg/DuckLake table-format CDC.** DuckLake's snapshot model is
  effectively a CDC stream at the table level. This is the obvious input
  side of a SlateDuck-flavored DD system.
- **Pathway** (Python streaming framework, built on DD): demonstrates the
  developer ergonomics that wins users.

---

## 6. Practical Use Cases

Use cases fall into three buckets.

### 6.1 Incremental materialized views over a lakehouse

The flagship use case. Given a DuckLake table that is being continuously
appended to, maintain the result of a SQL query (`SELECT … JOIN … GROUP BY …`)
as a *second* DuckLake table that stays fresh within seconds of the input.
Today this requires Materialize, Flink, or a hand-rolled batch pipeline. On
SlateDB+DD it could be a single binary reading a DuckLake snapshot stream
and writing a derived Parquet dataset.

### 6.2 Operational analytics with cheap fan-out reads

Maintain pre-aggregated counters, top-K leaderboards, sessionized event
streams, or feature-store features in DD arrangements that are durably
stored in SlateDB. Any number of stateless replicas can serve point
lookups against those arrangements at extremely high QPS without coordinating
with the writer. This is the "dashboard backend" use case that Materialize
sells into; with SlateDB the price floor is "an S3 bucket."

### 6.3 Graph and Datalog workloads

DD's killer differentiator over DBSP is `iterate`. This unlocks:

- Continuous graph analytics (PageRank deltas, k-core membership,
  connected-components on streaming edge updates).
- Live IAM/RBAC evaluation (transitive role expansion as a Datalog
  fixed-point).
- Bill-of-materials / dependency-graph services (recompute the closure on
  every change to a node).
- Streaming entity resolution / record linkage.

These are workloads where the cold recomputation is genuinely expensive
and the incremental update is the only viable path.

---

## 7. Project Ideas

In rough order of "smallest interesting demo" → "ambitious platform."

### 7.1 `slatedb-dd-trace`: a `Trace` implementation for SlateDB

A standalone crate that implements
`differential_dataflow::trace::Trace` (or the DBSP equivalent) backed by a
SlateDB instance. Deliverables:

- A `SlateDbTrace<K, V, T, R>` type that serializes batches into a SlateDB
  key prefix.
- A `SlateDbBatch` cursor that wraps a SlateDB range scan.
- A background compactor that performs frontier-advance compaction by
  rewriting key ranges.
- Benchmarks against the in-memory trace and against `Spine` (DD's
  in-memory layered trace).

This is the *enabling technology* for everything else. It is also the
cleanest contribution back to the DD ecosystem and would land SlateDB
adoption with the DD/Materialize crowd.

### 7.2 `dlstream`: a CDC source for DuckLake

A small binary that subscribes to changes in a DuckLake catalog and emits a
DD-shaped `(record, time, diff)` stream. Because SlateDuck's catalog is
already an MVCC log of facts with `begin_snapshot` / `end_snapshot`, this
is mostly a matter of decoding catalog rows. Pairs trivially with `slatedb-dd-trace`
to give you an end-to-end DuckLake → DD pipeline.

### 7.3 `dlmatview`: incremental materialized views for DuckLake

The headline product. SQL in, Parquet out, continuously maintained. Built on:

- `dlstream` for input,
- DBSP (preferred over raw DD for SQL surface) for the IVM engine,
- `slatedb-dd-trace` for state,
- a Parquet writer that periodically materializes the current arrangement
  contents as a new DuckLake snapshot of the *output* table.

This would make SlateDuck the first lakehouse with first-class incremental
materialized views that don't require a separate streaming database.

### 7.4 `slate-graph`: streaming graph database on SlateDB

A graph-shaped front-end (Cypher subset or Datalog) over `slatedb-dd-trace`,
exposing continuous queries like reachability, shortest paths, and
community detection over a graph stored in SlateDB. Closest competitor is
[Differential Datalog](https://github.com/vmware-archive/differential-datalog)
(unmaintained) and [Materialize's Datalog mode](https://materialize.com/docs/).
Strong fit for IAM, security analytics, and SBOM tooling.

### 7.5 `feature-store-lite`: online features over SlateDB+DD

A minimal feature store: define feature transformations as DD dataflows,
serve point lookups from materialized feature arrangements via a tiny HTTP
service. The pitch is "Feast without the operational complexity," with
S3 as the only durable dependency.

### 7.6 `s2-dd`: DD operators over s2 streams

If [s2](https://github.com/s2-streamstore/s2) becomes the de facto
object-storage log, an "s2 + DD" runtime is a natural fit: s2 provides the
input log, DD provides the compute, SlateDB provides the state. Together
they replace the Kafka+Flink+RocksDB stack with three Rust binaries and an
S3 bucket.

### 7.7 Research / paper-shaped: "Disaggregated Differential Dataflow"

The architectural claim — that DD's trace abstraction maps exactly onto an
object-storage LSM, enabling stateless workers with bottomless durable
state — is publishable. A workshop paper (HotOS / CIDR / DBPL) describing
the mapping, the frontier-advancement-as-compaction unification, and
benchmark results against `persist` would be a strong artifact.

---

## 8. SlateDuck/DuckLake-Specific Opportunities

This is the section that matters most for this repository.

### 8.1 Incremental DuckLake materialized views (high value)

DuckLake today is a *static* lakehouse format: snapshots are written by
DuckDB's `ducklake` extension and the catalog (which SlateDuck owns)
records the changes. There is no built-in concept of a derived table that
stays fresh.

A SlateDuck-hosted IVM engine would let users write:

```sql
CREATE INCREMENTAL MATERIALIZED VIEW daily_revenue AS
SELECT date_trunc('day', ts) AS day, sum(amount) AS revenue
FROM orders
GROUP BY 1;
```

…and have the resulting Parquet files in the same bucket stay continuously
correct as `orders` is appended to. The catalog entries for the materialized
view are versioned facts in exactly the same MVCC scheme SlateDuck already
uses. The DD/DBSP state lives in a per-view SlateDB instance under
`catalogs/<warehouse>/matviews/<view_id>/`. **No new operational primitive
is introduced** — it is the same single-writer-many-readers pattern, the
same checkpointing model, the same backup story.

Strategic value: this is a feature DuckLake-as-a-format does not have and
that Iceberg/Delta achieve only with bolted-on external systems (Flink,
Spark Structured Streaming). SlateDuck delivering it natively would be a
genuine differentiator.

### 8.2 Continuously-maintained catalog statistics

DuckLake stores column-level statistics (min, max, null count, NDV) per
data file. Today these are computed at write time and never refined. A
DD pipeline could maintain *table-level* and *partition-level* roll-up
statistics incrementally, giving DuckDB much better cost-based optimizer
input without rescanning data. The same machinery can power live
data-quality metrics (e.g. "fraction of rows where `email IS NULL` by day,
updated within seconds of every snapshot").

### 8.3 Catalog-of-catalogs / cross-warehouse views

A single SlateDuck deployment can host many DuckLake catalogs. A DD layer
on top can maintain *federated* views that join across them: "show me the
union of `events` from every warehouse, with the join against the central
`customers` dimension always up to date." This is hard to do with batch
tools because the cross-warehouse join is expensive; trivially incrementable
with DD.

### 8.4 Real-time lakehouse CDC export

Many users want their lakehouse changes to flow back out as a CDC stream
(into Kafka, into a search index, into a downstream warehouse). SlateDuck
already has the MVCC fact log internally; exposing it as a DD-shaped
`(record, time, diff)` source over a stable wire format (Arrow Flight,
gRPC streaming) is a small project that unlocks integration with the
entire streaming ecosystem.

### 8.5 Snapshot diff service

`describe_snapshot_diff(from, to)` is naturally expressible as a DD
collection difference. Even without a long-running DD runtime, exposing
snapshot diffs as Arrow-encoded `(record, diff)` streams from a stateless
endpoint is a useful feature for backups, replication, and audit.

### 8.6 Time-travel-aware incremental queries

Because DuckLake snapshots and DD timestamps both encode a notion of time,
you can answer queries of the form "what would my materialized view have
contained at snapshot `S`?" by reading the DD trace `as_of` that snapshot
ID. This is genuinely new functionality — neither Materialize nor any
lakehouse engine does this today — and it falls out of the architecture
for free.

### 8.7 What *not* to do in SlateDuck core

A few non-goals worth stating to keep the scope honest:

- **Do not embed DD into `slateduck-pgwire`.** The bounded-SQL surface is a
  load-bearing design promise. IVM should live in a *separate* binary that
  shares the SlateDB substrate but not the pgwire dispatcher.
- **Do not try to ship a general-purpose stream processor.** That is
  Volga/Malstrom/s2's job. SlateDuck's wedge is *lakehouse-shaped*
  incremental computation.
- **Do not invent a new query language.** SQL (via DBSP) is enough for the
  matview use case. Datalog/graph is a separate, later product.

---

## 9. Challenges and Open Questions

Honest list of things that will be hard.

1. **Serialization throughput.** DD batches are produced rapidly; if every
   batch round-trips through Arrow + SlateDB `put`, the engine will be
   I/O bound long before it is compute bound. A staged design where small
   in-memory batches are kept locally and only sealed-and-flushed batches
   hit SlateDB is essential. This is structurally the same problem
   SlateDB's own MemTable solves; the patterns transfer.

2. **Compaction policy coupling.** DD compaction (frontier advance) and
   SlateDB compaction (leveled merge) are doing related but not identical
   work. Running them independently risks redundant rewrites. Best case
   the two are unified into one background process that does both at
   once; worst case there is a measurable double-compaction tax.

3. **Recovery semantics.** Persisted arrangements give `O(checkpoint_age)`
   recovery, not `O(0)`. The exactly-once contract from input log seqno
   to output collection has to be designed carefully (idempotent sink,
   frontier-tagged checkpoints, etc.). This is where Materialize's
   `persist` has its hardest code; expect this to be the dominant
   complexity sink.

4. **Multi-worker sharding.** Single-writer SlateDB is fine for a single
   DD worker. Scaling to multiple workers means either (a) one SlateDB
   database per shard, or (b) extending SlateDB's writer-fencing to
   support range-partitioned writers. Both are workable; (a) is
   pragmatically easier.

5. **DBSP vs DD choice.** Picking DBSP narrows the use cases to SQL but
   gets you a much cleaner SQL surface and an active commercial sponsor.
   Picking DD opens up graph/Datalog but means writing the SQL frontend
   yourself. The honest answer is *both, eventually*, with DBSP first
   because it matches the lakehouse story.

6. **Memory pressure during cold start.** Even with persisted traces,
   `iterate` operators may need to rebuild fixed-points in memory after a
   restart. Bounds on operator state are an unsolved research problem in
   general DD; pragmatic engineering (per-iterate state checkpoints) is
   the workaround.

7. **Object-store API cost.** A naïve trace flush per round at 100ms
   granularity would generate thousands of S3 PUTs per dataflow per
   minute. Batching, write coalescing, and `await_durable=false` for
   non-critical traces are mandatory; the SlateDB community has already
   developed these patterns.

---

## 10. Recommendations

1. **Read `persist`.** Spend a week with Materialize's `src/persist`
   source. Almost every design decision in this report has a corresponding
   battle-tested answer there, and the contrast with SlateDB will sharpen
   the design.

2. **Build the smallest possible trace.** Start with `slatedb-dd-trace` as a
   research crate, targeting DBSP first (smaller API surface than DD's
   `Trace` trait). Benchmark it against the in-memory baseline on a
   realistic IVM workload (TPC-H Q1 incremental).

3. **Pick one DuckLake use case and ship it end-to-end.** The
   `CREATE INCREMENTAL MATERIALIZED VIEW` story from §8.1 is the most
   compelling narrative for SlateDuck users and exercises the entire stack.
   Everything else (graphs, feature stores) can wait.

4. **Keep it out of the v1.0 critical path.** SlateDuck's v0.9.x work on
   correctness, security, and operational safety must land first. IVM is
   a v1.x conversation. But the *architectural seams* (per-warehouse
   subdirectory layout, snapshot-tagged catalog events, MVCC fact log)
   should be checked now to make sure they don't preclude a future IVM
   layer. Specifically: the catalog's change-log shape should be
   consumable as a DD source without re-engineering.

5. **Engage the SlateDB and DBSP communities early.** Both are small,
   responsive, and would welcome a serious adopter doing object-storage-
   native IVM. There is meaningful upstream leverage available.

---

## Appendix A: Glossary

- **Arrangement.** An indexed, shared view of a DD collection's history,
  backed by a *trace*.
- **Batch.** An immutable, sorted run of `(K, V, T, R)` updates covering a
  time interval. The atomic unit of DD storage.
- **DBSP.** Database Stream Processor; a theory and runtime for incremental
  SQL view maintenance closely related to DD.
- **Differential dataflow (DD).** Rust framework for incremental
  computation on changing multisets, built on timely dataflow.
- **Frontier.** A partial-ordered set of timestamps that demarcates
  "completed" from "in-progress" work in a dataflow. `since` and `upper`
  are the two key frontiers of a trace.
- **IVM.** Incremental view maintenance. The general problem of keeping a
  derived dataset fresh as inputs change.
- **LSM tree.** Log-structured merge tree; the storage pattern SlateDB
  implements.
- **Persist.** Materialize's S3-backed shard store for DD traces; the
  closest prior art to "DD on object storage."
- **SlateDB.** Embedded LSM KV store with SSTs in object storage.
- **Timely dataflow.** Low-level distributed dataflow runtime; the
  substrate DD runs on.
- **Trace.** A logical sequence of batches representing the full history of
  a collection.
