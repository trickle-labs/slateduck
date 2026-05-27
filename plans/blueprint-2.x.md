# Rocklake v2.x: A World-Class Fact Store on Object Storage

> Status: **Exploration / Design**. This document is a forward-looking design
> blueprint for the v2.x line. It builds on the architectural compass in
> [`docs/concepts/fact-store-vision.md`](../docs/concepts/fact-store-vision.md)
> and the substrate already shipped in v0.x/v1.x (see
> [`plans/blueprint.md`](blueprint.md) §1.4 and §5.29). Nothing here is
> committed for implementation until promoted into `ROADMAP.md` with concrete
> milestones.

---

## Introduction (for a non-technical audience)

Rocklake v1.x ships a **lakehouse catalog**: a way to store metadata about
data files in cloud object storage. The interesting thing about how it does
that is not the catalog itself — it is the **storage engine underneath the
catalog**. That engine treats every change as an immutable *fact* with a
version number, never overwrites or silently deletes anything, and lets
unlimited readers query any point in history without coordinating with a
writer.

That engine is fundamentally schema-agnostic. The lakehouse catalog is
*one application* of it. The 28 catalog tables defined by the lakehouse spec
take up only about 12 % of the available "tag namespace" in the key encoding;
the other 88 % is deliberately reserved for other schemas.

**v2.x is the release line that opens up the substrate.** It extracts the
generic fact-storage layer into a standalone crate, adds the abstractions
needed to host arbitrary schemas on it, and ships first-class query
interfaces (SQL, typed APIs, and a rule-based query language) plus
horizontally-scaling read replicas.

The end state we are aiming at: **a world-class immutable fact store that
runs on nothing but an object-storage bucket**, scales reads horizontally
to thousands of replicas without coordination, supports time travel as a
native read mode, and costs pennies per gigabyte-month to operate.

---

## 1. Vision and Goals

### 1.1 What "world-class" means here

A world-class fact store on object storage must clear every one of these
bars. The bar order matters: correctness gates everything else.

| # | Bar | Concrete commitment |
|---|-----|---------------------|
| 1 | **Correctness** | Every committed fact is durable, every historical version is readable, the audit trail is tamper-evident, and crash recovery is bit-identical to a clean shutdown. |
| 2 | **Time travel** | Point-in-time queries at any historical version are first-class and have the same correctness guarantees as "current" queries. |
| 3 | **Cost** | $0.02–$0.05 per GB-month of stored facts in steady state; no fixed infrastructure cost when idle. |
| 4 | **Operational simplicity** | A bucket and a binary. No external database, no coordination service, no schema registry server. |
| 5 | **Read scale-out** | Linear throughput scaling to ≥ 100 stateless reader replicas on a single fact store. |
| 6 | **Latency** | p50 < 50 ms, p99 < 200 ms for indexed point lookups on warm caches; range scans bounded by object-storage prefetch. |
| 7 | **Query expressiveness** | At least three interfaces (SQL, typed Rust API, rule-based query language) over the same substrate. |
| 8 | **Schema evolution** | Add attributes, change cardinality, rename, and migrate without rewriting history. |
| 9 | **Federation** | Multiple fact stores in different buckets can be queried together with time-aligned semantics. |
| 10 | **Compliance** | Right-to-be-forgotten erasure via the audited excision path; full provenance for every fact. |

v2.x targets bars 1–7 directly; bars 8–10 are stretch goals that may slip
into v2.5+.

### 1.2 Non-goals

- **Replacing operational OLTP databases.** Object storage latency is
  fundamentally tens of milliseconds per round-trip. v2.x is for workloads
  where time travel, audit, and infinite history are worth that cost.
- **General-purpose graph database.** Rule-based queries are supported, but
  graph-specific optimisations (e.g. native shortest-path) are out of scope
  for v2.x.
- **Streaming pub/sub.** Facts are committed in batches; v2.x does not
  attempt to compete with log brokers on per-event latency.
- **Multi-writer per fact store.** v2.x keeps the single-writer model from
  v1.x. Multi-writer is evaluated as a separate exploration in §10.
- **Bulk analytical processing.** EAV on an LSM tree cannot compete with
  columnar stores for aggregations over millions of rows. The recommended
  answer for analytical workloads is: export to Parquet and query with
  DuckDB — exactly what the v1.x lakehouse catalog already enables.
  v2.x targets **lookup-dominated, audit-grade, temporally-queried
  metadata** workloads, not OLAP.

---

## 2. The Generic Fact Model

### 2.1 Core abstractions

A **fact** is the tuple

```
(entity, attribute, value, version, op)
```

where:

- `entity` — an opaque identifier (u64 or byte-string) for the thing the
  fact is about.
- `attribute` — a named, typed property of the entity (e.g. `user/email`).
- `value` — the asserted value, typed according to the attribute's schema.
- `version` — a monotonically increasing fact-store version (analogous to
  `dl_snapshot_id` in v1.x).
- `op` — `assert` or `retract`. Retraction is a fact too: a new fact at a
  later version that says "this attribute is no longer set on this entity".

A **transaction** is a set of facts committed atomically at one version.
Multiple facts in one transaction share a version and a single audit record.

A **schema** is a set of attribute declarations: name, value type,
cardinality (one vs. many), uniqueness, indexed dimensions, and retention
policy. Schemas are themselves stored as facts in a reserved system
namespace, so schema evolution participates in time travel.

### 2.2 Why entity-attribute-value, not rows

A row-oriented model bakes the column set into the storage key. Adding a
column requires either rewriting old rows or carrying a NULL forest of
absent values. Renaming or splitting a column is even worse.

The entity-attribute-value (EAV) model stores each attribute of each entity
as an independent fact. Adding an attribute is purely additive — no
historical data changes. Renaming is a metadata edit on the schema fact.
Splitting an attribute is a transaction that asserts the new attributes and
retracts the old one. Every change participates in time travel: queries at
older versions still see the old schema and the old data exactly as they
were.

EAV pays real costs that must be acknowledged:

- **Write amplification.** A fact stored across N indexes means N physical
  writes per attribute per transaction. §3.3 addresses this with a tiered
  index model that avoids blanket 4× amplification.
- **Full-entity reads are prefix scans, not block reads.** A columnar store
  returns an entity's 30 attributes in one block fetch; EAVT requires a
  prefix scan over 30 keys. This is acceptable for the target workloads
  (point lookups, audit, metadata) and is addressed by the pull API (§4.4)
  which batches the scan into one round-trip.
- **Analytical aggregations are expensive.** Counting or summing across all
  entities for one attribute requires an AVET scan, not a column read. See
  §1.2 — this is a deliberate non-goal.

These costs are the central engineering challenges §3 and §4 address with
tiered index design, query compilation, and honest workload scoping.

### 2.3 The fact lifecycle

```
        assert(e, a, v, V₁)                retract(e, a, V₂)
            │                                      │
            ▼                                      ▼
[uncommitted]──────► [live: V₁..V₂)─────────► [retracted: V₂..]
                                                   │
                                                   ▼
                                              [excised]
                                          (only via rocklake excise,
                                            audited, irreversible)
```

A fact's `version` (`V₁`) is its **transaction time** — when it was
recorded. The retraction's `version` (`V₂`) marks the upper bound of its
visibility. A query `as_of(V)` with `V₁ ≤ V < V₂` sees the fact as live.

§5 adds an optional second time dimension (**valid time**) for workloads
that need bi-temporal semantics.

---

## 3. Storage Substrate (extracted from `rocklake-core`)

### 3.1 Crate extraction

v2.0 promotes the schema-agnostic primitives from `rocklake-core` into a
new top-level crate, `rocklake-factstore`. The boundary is defined in
[`plans/blueprint.md` §5.29](blueprint.md):

| Moves into `rocklake-factstore`             | Stays in `rocklake-catalog` |
|----------------------------------------------|------------------------------|
| Key encoding utilities                       | 28-table tag allocation      |
| Value header + version byte + Protobuf dispatch | Lakehouse MVCC filter      |
| Counter allocation (transactional RMW)       | Schema-version increment    |
| `retain-from` and visibility advancement     | Inlined-data encoding       |
| Excision primitives + audit log              | Spec-specific operations    |
| Leadership / epoch keys                      | `dl_snapshot_id` semantics  |
| Generic `CatalogStore` with `SnapshotId(u64)`| Lakehouse adapter           |

The lakehouse catalog becomes the **first hosted schema** on the new crate.
v1.x APIs continue to work unchanged; the extraction is purely internal.

### 3.2 Key layout for the generic fact store

The 1-byte tag namespace from v1.x is preserved. Of the 225 still-unused
tags, v2.x reserves a contiguous block for the generic fact-store schema:

```
Tag    Role
----   ----
0x40   Datom: EAVT primary index   (Entity, Attribute, Version, Tx) — always built
0x41   Datom: AEVT opt-in index    (Attribute, Entity, Version, Tx) — declare :attribute-scan
0x42   Datom: AVET index           (Attribute, Value, Entity, Tx)   — declare :indexed or :unique
0x43   Datom: VAET opt-in index    (Value, Attribute, Entity, Tx)   — declare :reverse-ref
0x44   Tx log: per-transaction audit record
0x45   Schema facts (attribute declarations)
0x46   Schema migration log
0x47   Statistics: per-attribute fact counts, cardinality sketches, min/max
0x48–0x4F  Reserved for future fact-store internals
0x50–0xBF  User-allocated tag ranges for application schemas
```

A fact is stored in EAVT (always) plus any declared secondary indexes.
Version ranges are embedded in the key prefix so temporal pruning happens
during the prefix scan itself, not as a post-filter (§3.3).

### 3.3 Tiered index model: mandatory, opt-in, and derived

Not every attribute needs every index. Blanket four-way write amplification
is the wrong default on an LSM tree — it inflates SST count, compaction
pressure, and per-request object-storage costs. Instead, indexes are
declared per attribute:

| Index | When it is built | Access pattern it enables |
|-------|------------------|---------------------------|
| **EAVT** (always) | Every attribute, unconditionally | Point and range lookups on entity; full-entity scan; time travel |
| **AVET** (declare `:indexed` or `:unique`) | Attributes where value lookups matter | "Which entity has attribute A = value X?"; uniqueness enforcement; range scans by value |
| **AEVT** (declare `:attribute-scan`) | Attributes frequently queried across all entities | "Which entities have attribute A set?" without scanning all EAVT keys |
| **VAET** (declare `:reverse-ref`) | Reference attributes where reverse traversal is needed | "What references entity E?" graph reverse-edge queries |

This gives operators control over the write-amplification budget. A simple
configuration fact store might use only EAVT. An entity-graph workload
adds AVET (unique IDs) and VAET (reverse refs) on a handful of attributes.
AEVT is only needed for analytical-flavoured queries across all entities of
a type, which is in the non-goals for v2.x but supported if declared.

**Temporal pruning in the key.** Each EAVT key is
`[0x40][entity][attribute][version_bigendian]`. Because version is encoded
big-endian and facts are appended in version order, a scan `as_of(V)` is
a prefix scan of `[0x40][entity][attribute]` with an upper-bound stop at
`[0x40][entity][attribute][V+1]` — a natural LSM range scan with zero
post-filtering overhead. The same pattern extends to AVET: range scans by
value at a given version are key-range queries, not table scans.

**AVET and VAET as RoaringBitmap posting-list indexes.** For attributes
declaring `:indexed`, `:unique`, or `:reverse-ref`, each distinct
`(attribute, value)` pair maps to a **RoaringBitmap** of entity IDs stored
as a single key-value pair, rather than one physical key per entity. A query
"all entities where `:country` = `"NO"`" becomes a single key lookup
followed by an in-memory bitmap scan. Multi-attribute conditions (`AND`)
become bitmap intersections — far cheaper than two range scans joined after
the fact. RoaringBitmaps provide efficient compression and fast set
operations (intersection, union).

**Merge operators for secondary index writes.** Every AVET or VAET write
would naively require a read-modify-write cycle: read the current posting
list, add the entity ID, write back. With SlateDB's merge operator, the
write path emits a blind `AddEntity(eid)` merge operand and returns
immediately. The LSM merges operands lazily into the RoaringBitmap at
compaction time. This eliminates the read on the secondary-index write path
entirely — the most expensive hidden cost of the tiered index model.

### 3.4 Leveraging SlateDB features

The substrate is designed to make full use of SlateDB capabilities:

**Atomic `WriteBatch`** — All four index writes for one fact, plus the
transaction-log audit fact (tag `0x44`), commit in one batch. Either every
projection is durable or none is. This is the same atomicity v1.x relies on
for catalog correctness.

**`commit_with_options(await_durable)`** — Every transaction commits with
explicit durability semantics. The `Tx` returned to the caller is the one
guaranteed by SlateDB to survive crashes.

**`DbSnapshot` / `DbReader` at checkpoint** — Time travel translates
directly: a query `as_of(V)` opens a reader at the SlateDB checkpoint that
contains version `V`, then filters facts with `tx ≤ V`. No bespoke MVCC
machinery is needed beyond what v1.x already proved.

**Prefix scans** — Every query plan compiles to a (possibly small) set of
prefix scans. SlateDB's tiered storage prefetches SSTs sequentially, which
maps perfectly onto the access pattern of "scan all facts for entity E in
version range V₁..V₂".

**Manifest generations as cache keys** — A reader pinned to a specific
SlateDB manifest generation can be HTTP-cached by the SST's content hash.
This gives us §6 (CDN-friendly read replicas) almost for free.

**Compaction** — Excised facts (the audited deletion path) propagate
through SlateDB compaction the same way `end_snapshot`-marked rows do
today. No new compaction policy is needed.

**Merge operators** — AVET and VAET secondary indexes write `AddEntity(eid)`
merge operands rather than performing read-modify-write cycles. The LSM
merges them lazily at compaction time into the RoaringBitmap posting list
(§3.3).

**Hybrid block cache (memory + NVMe disk tier)** — Readers and the writer
use a two-tier block cache: a fast in-memory tier (default 1–4 GiB) backed
by a larger on-disk NVMe tier (default 50–100 GiB). Decoded SST blocks
are written to disk on first access, so the effective working set that
avoids S3 round-trips is the *disk* size, not RAM. Configuration:

| Parameter | Default | Purpose |
|-----------|---------|------------------|
| `memory_capacity` | 1 GiB | Hot blocks in RAM |
| `disk_capacity` | 90 GiB | Warm blocks on NVMe |
| `disk_path` | `/var/cache/rocklake` | On-disk tier location |
| `write_policy` | `WriteOnInsertion` | When to promote to disk tier |

The disk tier is ephemeral — the system operates correctly without it (pure
S3 fallback). This makes PVCs optional for development and mandatory only
for production latency targets.

**Writer fencing** — The single-writer guarantee on the substrate carries
over; multi-writer exploration (§10) builds *on top* of fencing rather than
replacing it.

### 3.5 Value encoding

Every value carries the v1.x SDKV header (encoding version + type tag +
optional compression flag) and a Protobuf-encoded payload. The payload's
schema is a `Value` union covering the primitive types:

```
oneof value {
  bool      bool_val   = 1;
  int64     i64_val    = 2;
  uint64    u64_val    = 3;
  double    f64_val    = 4;  // canonical: NaN normalised, -0.0 == 0.0
  string    str_val    = 5;
  bytes     bytes_val  = 6;
  bytes     uuid_val   = 7;  // 16-byte raw
  int64     instant_us = 8;  // microseconds since epoch
  uint64    ref_val    = 9;  // entity reference
  Decimal   decimal    = 10; // [scale, mantissa bytes]
  Vector    vector     = 11; // [dtype, dim, packed bytes] for ML use cases
}
```

The format is deliberately small and stable. Application schemas that need
richer types compose them from these primitives (e.g. a `Point2D` is two
`f64_val` facts on the same entity under attributes `geo/x` and `geo/y`).

### 3.6 Counter and ID allocation

Entity IDs are allocated from a per-fact-store monotonic counter under tag
`0xFE`, using the same transactional read-modify-write protocol v1.x uses
for `next_catalog_id`. The counter is bumped atomically inside the same
batch that asserts the entity's first fact, so an ID is never burned
without producing a fact.

For high-throughput ingestion, the writer can reserve a **range** of IDs in
one counter bump (`counter += 1000`) and hand them out from memory. The
range itself is durable; only the in-memory cursor is volatile. After a
crash the unallocated tail of the range is permanently skipped — a price
worth paying for batched allocation.

This is *block-based sequence allocation*: a `SeqBlock` record captures
the allocated range; crash recovery reads the last block and skips forward,
preserving monotonicity with O(1) recovery cost.

### 3.7 Proposed SST enhancements (upstream to SlateDB)

Two SlateDB enhancements would materially improve read performance and are
worth contributing upstream rather than working around:

**Entity-level bloom filters.** Current bloom filters are keyed on the full
composite key (e.g. the complete EAVT tuple). A probe “does entity `E` have
*any* fact in this SST?” cannot answer without enumerating every possible
attribute+version combination. If bloom filters were keyed on the entity-ID
prefix alone, a single probe skips entire SSTs for absent entities. The same
approach works for any structured key model that uses a shared prefix: key
the bloom filter on the logical key, not the full composite key.

**Block-level record counts.** Including a cumulative record count in each
SST block-index entry enables `COUNT(entity)` and cardinality-estimation
queries at the block-index level, without reading block data. This gives
O(1) range counting within a key-prefix range.

Both are *proposed* enhancements, not requirements for Phase 2.0. Tracked
as open questions in §19.

### 3.8 Disaggregated compaction and garbage collection

Because all data lives in object storage, compaction and garbage collection
do not need to run on the same process as the writer. This disaggregation
yields three practical benefits:

1. **No resource contention.** Compaction never competes with ingest or
   queries for CPU, memory, or disk I/O.
2. **Cheaper compute.** Compaction and GC jobs are stateless and
   restartable — they can run on spot or preemptible instances without
   risk of data loss. The writer and readers continue normally regardless
   of whether a compaction job is active.
3. **Independent scheduling.** Compaction intensity can be dialled up
   during off-peak hours and throttled during ingest spikes.

SlateDB already supports this model through its `compactor_options` and
`garbage_collector_options` configuration blocks. Recommended production
slots:

```yaml
compactor_options:
  max_concurrent_compactions: 2
  max_sst_size: 67108864          # 64 MiB
garbage_collector_options:
  manifest_options: { interval: '60s', min_age: '3600s' }
  wal_options:      { interval: '60s', min_age: '60s' }
  compacted_options: { interval: '60s', min_age: '3600s' }
```

The `min_age` guards prevent the GC from deleting SST files that a reader
pinned to a recent manifest generation still needs. Setting it to 1 hour
for manifests and compacted SSTs is conservative but safe; tighten it only
if storage costs are a concern.

---

## 4. Query Layer

### 4.1 Three query interfaces, one substrate

| Interface | When to use it                                          | Crate |
|-----------|---------------------------------------------------------|-------|
| Typed Rust API | Embedded use, hot paths, library consumers         | `rocklake-factstore` |
| SQL (PG-wire) | Existing SQL tooling, BI dashboards, ad-hoc analysis | `rocklake-pgwire` (extended) |
| Rule-based queries | Recursive, graph-shaped, exploratory queries       | `rocklake-rules` (new) |

All three compile to the same physical operators over the four indexes
(§3.3). A rule-based query and a SQL query that express the same logical
result compile to the same scan plan; the difference is only in surface
syntax.

### 4.2 Index selection

The planner chooses indexes by **estimated scan width**, computed from
inexpensive metadata that the writer maintains as it commits:

- Per-attribute fact count (a counter under tag `0x47`)
- Per-attribute cardinality estimate (HyperLogLog sketch, also `0x47`)
- Min/max value summaries for indexed scalar attributes

These are themselves stored as facts (in the system namespace) so they
participate in time travel and are reconstructible from a rebuild.

Selection rules, in order:

1. If the query binds a unique attribute and a value → AVET point lookup.
2. If the query binds an entity → EAVT prefix scan.
3. If the query binds an attribute and asks for entities → AEVT scan.
4. If the query traverses a reference backward → VAET scan.
5. Otherwise, fall through to a full scan with a covering filter.

### 4.3 The pipeline query model

All query interfaces share a common **pipeline** model: queries are built
from small, composable operators that transform a stream of fact tuples.
A query is a *source operator* followed by zero or more *tail operators*.
This model is deliberately explicit and iteratively testable — you can
remove tail operators to inspect intermediate results at any step.

Core source operators:

| Operator | Description |
|----------|-------------|
| `from(namespace, bindings)` | Read facts from the EAVT index. Temporal filter defaults to current version. |
| `from(namespace, bindings, for_version: V)` | Same, pinned to a specific version. |
| `from(namespace, bindings, for_valid_time: T)` | Valid-time filter (bi-temporal, §5.2). |
| `rel(inline_data)` | Inline relation — useful for constants and test fixtures. |

Core tail operators: `where`, `with` (computed columns), `return`
(projection), `without` (column exclusion), `order_by`, `limit`,
`aggregate`, `join`, `left_join`.

Multi-source joins use **unification**: declare a binding variable in two
`from` sources and the planner automatically adds the join condition on
equal values — no explicit `ON` clause needed for the common case.

```rust
// Rust API — composable query:
let q = store.as_of(v)
    .from("user", ["name", "country"])
    .join(
        store.as_of(v).from("order", ["user_id", "total"]),
        on: |u, o| u.entity_id == o["user_id"]
    )
    .where_(|row| row["country"] == "NO")
    .aggregate(["country"], [("total_revenue", sum("total"))])
    .order_by("total_revenue", Desc);
```

The rule-based query engine is a thin layer on top of the pipeline model
that evaluates **recursive rules** via *semi-naïve* bottom-up evaluation:

```
?ancestor(X, Y) :- parent(X, Y).
?ancestor(X, Y) :- parent(X, Z), ancestor(Z, Y).

?- ancestor(Alice, ?who).
```

- **Non-recursive rules** compile directly to pipeline join sequences.
- **Recursive rules** maintain a worklist of newly-derived tuples,
  scan only their neighbourhood at each iteration, terminate when empty.
- **Negation as failure** is supported only for stratified programs
  (no cycles through negation).
- **Aggregates** are pushed into scans when the index supports it.

The compiler emits the same DAG of physical operators (`Scan`, `Filter`,
`Join`, `Project`, `Aggregate`) regardless of which surface syntax was
used. SQL, rules, and the typed API produce the same plan for the same
logical query.

### 4.4 The pull API

Pull is **sugar over subqueries**, not a separate mechanism. Every nested
level in a pull spec compiles to a fully-powered sub-pipeline with the
complete set of tail operators available: ordering, filtering, limiting,
and aggregation are all legal inside a nested pull. This means there is
no "pull can't do X" class of problems — if a sub-query can express it, a
nested pull can express it too.

```rust
// Pull a user with their 10 most-recent orders, each with line items
let user = store.as_of(version).pull(user_id, pull! {
    "user/name",
    "user/email",
    // nested pull is a sub-pipeline — full operator access
    "user/orders": from("order", ["total", "created_at"])
        .order_by("created_at", Desc)
        .limit(10)
        .pull! {
            "order/total",
            "order/created_at",
            "order/items": from("item", ["name", "price"]),
        },
}).await?;
```

Compilation: one EAVT scan for the root entity, one index scan per nested
relationship, batched in one SlateDB scan pipeline. Each nested scan uses
the most selective available index (AVET for unique refs, EAVT for
entity-keyed lookups). The planner has perfect cardinality information
because the spec shape is fixed at compile time.

This eliminates the N+1-query problem that plagues most graph traversal
APIs — every nested level is a single batched scan, not one round-trip per
parent entity.

### 4.5 SQL surface

The existing PG-wire dispatcher gains a synthetic schema where every
attribute becomes a column on a virtual `entity` table per namespace.
This is enough for BI tools to issue standard SQL:

```sql
SELECT u.name, COUNT(o.id) AS order_count
FROM   user u
JOIN   "order" o ON o.user_id = u.id
WHERE  u.created_at > '2026-01-01'
GROUP BY u.name;
```

The SQL planner translates joins and subqueries into pipeline operators
under the hood, so SQL and rule-based queries share the same optimiser and
produce the same physical plans.

**Time travel** is expressed via SQL:2011 temporal syntax:

```sql
-- Transaction-time (system-time) view
SELECT name FROM user FOR SYSTEM_TIME AS OF '2025-06-01';

-- Valid-time view (bi-temporal, §5.2)
SELECT name FROM user FOR VALID_TIME AS OF '2025-01-01';

-- Full history
SELECT name, _system_from, _system_to FROM user FOR ALL SYSTEM_TIME;
```

**Hierarchical result shaping** is available via `NEST_ONE` and
`NEST_MANY` extensions, avoiding the N+1 problem in SQL consumers:

```sql
SELECT
    u._id AS user_id,
    u.name,
    NEST_MANY(
        SELECT total, created_at
        FROM   "order" WHERE user_id = u._id
        ORDER BY created_at DESC
        LIMIT  10
    ) AS recent_orders
FROM user u
WHERE u.country = 'NO';
```

`NEST_ONE` returns a JSON object (or NULL), `NEST_MANY` returns a JSON
array. Both compile to the same batched index scans as the pull API;
they are not client-side post-processing.

### 4.6 Materialised views

Some queries are too expensive to recompute on every request. The system
supports **incrementally-maintained materialised views**: a view is declared
as a SQL query or typed Rust expression, and on every committed transaction
only the *delta* (the changed facts) is processed — never the full dataset.
This makes view maintenance cost proportional to the transaction size, not
the total number of facts.

See §11 for the formal delta computation model (Z-sets / DBSP), concrete
use cases, storage format analysis (Arrow IPC vs. Parquet), and scale-out
architecture. §11 also covers the two-tier storage strategy and the
boostrap/replay path for new views.

---

## 5. Time, Versioning, and Retention

### 5.1 Version vs. timestamp

The fact-store version (`u64`, monotonic) is the canonical time. Wall
clocks are unreliable across nodes and across history. Each transaction
also records a wall-clock timestamp in its audit fact (tag `0x44`), but
only as **metadata** — queries should use versions, not timestamps.

A helper API translates timestamps to versions:

```rust
let version = store.version_at_or_before(timestamp).await?;
let results = store.as_of(version).query(...).await?;
```

The lookup uses a sparse index of `(timestamp, version)` pairs maintained
by the writer (one entry per N transactions, where N is tunable).

### 5.2 Bi-temporal facts (optional)

For workloads where the question "when did this become true in the real
world?" is distinct from "when did we record it?", facts can carry an
optional `valid_from` / `valid_to` pair:

```rust
txn.assert_at(entity, attribute, value, valid_from..valid_to)?;
```

Queries can then ask either:

- `as_of(version)` — transaction-time view (what we knew on that day)
- `valid_at(instant)` — valid-time view (what was true on that day)
- `bi_temporal(version, instant)` — both (what we knew *then* about *then*)

This unlocks audit, regulatory, and historical-correction use cases that
single-time-axis systems cannot model cleanly.

### 5.3 Retention policies

Per-attribute retention policies are stored as system facts:

```
attribute = "user/login_event"
retention = { keep_versions: 90_days, mode: visibility_only }
```

`mode: visibility_only` advances `retain-from` for that attribute, hiding
old facts from default queries but leaving the bytes in place. `mode:
excise` invokes the audited deletion path on a schedule.

The default is `keep: forever, mode: visibility_only`. Operators must opt
in to byte-level deletion, just as in v1.x.

**SlateDB native TTL.** For short-lived data (staging namespaces, rolling
event windows, ephemeral tenants) the SlateDB `default_ttl` setting
applies a storage-level expiry to every key written. Once the TTL elapses
SlateDB marks the key as a tombstone during the next compaction pass —
no application code required. This is a lightweight complement to
per-attribute retention: use TTL for whole-namespace expiry, per-attribute
policies for finer-grained control.

**Segment-based retention.** Encoding a time-window boundary in the key
prefix (e.g. `[tag][hour_bucket][entity][attr][version]`) physically
clusters all facts from that window into a contiguous key range. Expiring
an entire window then becomes a bounded prefix-range delete rather than a
per-key tombstone sweep — O(1) in the number of key ranges, not O(n) in
the number of facts. This is worth considering for high-volume append-only
workloads (event logs, metric facts) where entire time windows age out
together.

### 5.4 Excision

Inherited from v1.x unchanged. `rocklake excise --before V --apply`
remains the only path to byte-level deletion, with the same audit
requirements (operator identity + reason, recorded as an immutable fact
under tag `0xFF`).

v2.x adds **per-entity excision** for compliance use cases (right-to-be-
forgotten): `rocklake excise --entity 12345 --apply` removes all facts
for entity 12345 across all indexes and writes a compliance audit record.

### 5.5 Checkpoints and branches

Because the fact store is an immutable LSM tree on object storage, a
**checkpoint** is a single O(1) metadata operation: it records the current
SlateDB manifest generation and marks the referenced SST files immune from
garbage collection. The checkpoint can be held indefinitely without
blocking writes.

Checkpoints enable three operational patterns that are otherwise expensive
or impossible:

- **Point-in-time restore.** Roll back to a known-good version after a bad
  bulk import or a runaway migration:
  ```
  rocklake checkpoint create --name pre-migration-v42
  rocklake checkpoint restore --name pre-migration-v42   # dry-run by default
  ```
- **Test and staging branches.** Developers take a checkpoint of production
  data, open it in a read-only reader, and run the migration script against
  real data without risking production. No data copy required.
- **Snapshot isolation for long-running analytics.** Pin a multi-hour
  analytical query to a stable manifest generation. The writer continues
  committing at full speed; the pinned reader sees a frozen view.

A checkpoint is purely a manifest reference — it does not copy any SST
files. `rocklake checkpoint list` shows all named checkpoints and their
generation numbers; `rocklake checkpoint drop` releases the GC hold when
the checkpoint is no longer needed.

---

## 6. Horizontal Read Scale-Out

### 6.1 The architectural argument

Because every fact key is immutable once written, a reader at version V
sees a stable view that no concurrent writer can perturb. This means:

- Readers do not need to talk to writers.
- Readers do not need to talk to each other.
- Readers can be cached, replicated, and pinned to specific manifest
  generations indefinitely.
- Readers can be deployed in specific availability zones. An in-zone
  replica serves all reads locally, eliminating cross-zone S3 data-transfer
  costs entirely. At high query volumes cross-zone transfer is a measurable
  line item; zonal replica deployment is the operational lever to remove it.

This is the strongest scale-out story possible: linear throughput by
adding processes, no coordination overhead, no consistency protocol.

### 6.2 The `rocklake reader` binary

A new binary that serves either the lakehouse schema or any registered
application schema, with three deployment modes:

| Mode | Purpose | State |
|------|---------|-------|
| `--mode embedded` | Library use inside another process | None |
| `--mode pod` | Long-running pod behind a load balancer | Warm cache |
| `--mode lambda` | Cold-start serverless function | Cold cache, opens at known manifest |

Mode selection only affects caching and connection pooling; the read path
is identical.

### 6.3 CDN-friendly cache contract

Because keys are content-addressable (the SST that contains them is
identified by content hash), the system can publish recommended HTTP cache
headers:

- `Cache-Control: public, max-age=31536000, immutable` on SST GETs.
- Manifest reads are `max-age=10` (writer-bounded staleness).
- A range-keyed lookup translates to a small number of byte-range GETs
  against immutable URLs — perfect for CDN edge caching.

The CDN guide documents which proxies (CloudFront, Cloudflare, Fastly)
are validated and what request patterns to expect.

### 6.4 Edge / Lambda integration

Documented patterns:

- Open a `DbReader` against a known manifest generation, query, return.
- Cold-start cost dominated by manifest fetch (~50 ms on warm region).
- The manifest URL is published by the writer to a tiny "current-tip"
  pointer; readers tail it.

### 6.5 Read freshness model

Read freshness is measured in object-storage round-trips between a write
being accepted and it becoming visible to a query:

| Write path | Reader | Extra RTs | Freshness |
|------------|--------|-----------|---------------------|
| Direct write to writer | Writer itself | 0 | Immediate (in-memory Delta) |
| Direct write to writer | Read replica | +1 | Manifest poll interval (~1 s) |
| Buffered ingestion (§8.5) | Writer | +1 | Batch flush + consumer poll (~1–2 s) |
| Buffered ingestion (§8.5) | Read replica | +2 | Batch flush + consumer poll + manifest poll |

Each object-storage round-trip adds ~100 ms of latency plus the polling
interval. The polling interval dominates: a reader configured to poll the
manifest every second sees facts within ~1.1 s of commit. For the writer
itself on the direct path, facts are visible immediately because they enter
an in-memory read buffer (the **Delta**) before the SlateDB WAL flush
completes.

### 6.6 Cache warmer

On startup, a cold reader (Lambda, new pod, post-crash restart) pays full
S3 latency on every block until the hybrid cache (§3.4) is warm. A
**startup cache warmer** eliminates this penalty:

```yaml
cache_warmer:
  warm_range: 24h        # how far back to pre-scan
  include_data: true     # warm raw fact blocks, not just indexes
```

The warmer runs once at startup: it scans recent EAVT key ranges through
the storage reader, and the block cache picks up those blocks as a side
effect. First queries after restart see local-cache performance rather
than cold-S3 latency. The warmer exits after the scan completes; it does
not consume ongoing resources.

Disable the warmer for Lambda mode or short-lived readers that only serve
a single request.

### 6.7 Scale targets

v2.x ships with reproducible benchmarks demonstrating:

- Linear read-throughput scaling to ≥ 100 reader pods on a single store.
- p99 < 100 ms for indexed point lookups across the entire reader fleet.
- < 1 % p99 degradation when the writer is concurrently committing at
  100 TPS.

---

## 7. Schema and Evolution

### 7.1 Schema modes: strict and dynamic

v2.x supports two schema modes per namespace, configurable per application
schema:

**Strict mode** (default for declared schemas):
- Attribute declarations are required before facts can be asserted.
- The value type, cardinality, and index set are enforced at write time.
- Schema violations are rejected at the writer's validation pipeline (§8.2).
- Best for: configuration management, compliance records, catalog metadata
  where the shape is known in advance.

**Dynamic mode** (opt-in per namespace):
- Facts can be asserted without a prior attribute declaration.
- Types are inferred from the first assertion and recorded as schema facts
  automatically.
- New attributes appear in queries immediately without a schema migration.
- Conflicting types produce type-widening (e.g. `i64` + `f64` → `f64`),
  recorded as a schema fact.
- Best for: exploratory data, log-like workloads, prototyping, or any case
  where the shape isn't fully known in advance.

Both modes store their schema in the same reserved namespace (tag `0x45`).
A dynamic namespace can be promoted to strict mode at any version by
declaring the inferred schema as authoritative — all future writes are
then validated against it.

### 7.2 Schemas as facts

Attribute declarations are themselves facts in a reserved namespace
(tag `0x45`). A schema change is a transaction; rollback is a query at an
older version; comparing two schemas is a diff between two versions.

This eliminates the perennial "schema registry" problem — there is no
separate service to keep in sync with the data, because the schema *is*
data.

### 7.3 Permitted changes

Without rewriting history:

- Add a new attribute.
- Mark an attribute deprecated (new asserts rejected, old facts still
  queryable).
- Add an index dimension (rebuilt by a background job).
- Add a uniqueness constraint (validated against existing facts before
  commit).
- Rename an attribute (the old name becomes an alias).
- Tighten a value type (e.g. `i64` → `i32`) if all existing facts fit.

With explicit migration:

- Split an attribute into two.
- Merge two attributes into one.
- Change cardinality from `one` to `many` (additive — old facts still
  valid).
- Change cardinality from `many` to `one` (requires conflict resolution).

### 7.4 Migration log

Every migration is recorded as a fact under tag `0x46`. A migration log
entry captures the transformation (as code or as a declarative rule), the
versions it spans, and the operator who authorised it. The log is itself
queryable: "show me every migration that touched `user/email` in the last
year".

---

## 8. Writer Path

### 8.1 The transaction lifecycle

```
caller                writer                  SlateDB                 audit
  │                     │                        │                      │
  │ begin() ────────────►                        │                      │
  │                     │ allocate Tx version ───►                      │
  │ assert(facts) ──────►                        │                      │
  │                     │ validate schema        │                      │
  │                     │ validate uniqueness    │                      │
  │                     │ validate references    │                      │
  │ commit() ───────────►                        │                      │
  │                     │ build WriteBatch       │                      │
  │                     │  ├── EAVT (always)     │                      │
  │                     │  ├── opt-in indexes    │                      │
  │                     │  ├── tx-log fact       │                      │
  │                     │  └── counter bumps     │                      │
  │                     │ commit_with_options ───► durable ─────────────►
  │ ◄─── version (V) ───┤                        │                      │
```

### 8.2 Flexible durability levels

The writer supports three durability levels, selectable per transaction:

| Level | Acknowledges when | Risk window | Latency |
|-------|-------------------|-------------|---------|
| **Buffered** | Data written to in-memory Delta | Writes since last WAL flush (bounded by `wal_flush_interval`, default 100 ms) | Lowest (~1 ms) |
| **WAL-durable** (default) | WAL flushed to object storage | None — survives process crash | Higher (~50–150 ms) |
| **Queue-durable** | Batch flushed to object storage via §8.5 | None — survives writer crash | Highest (~200–500 ms) |

```rust
// Per-transaction durability selection
txn.commit_with(Durability::Buffered).await?;    // fire-and-forget
txn.commit_with(Durability::WalDurable).await?;  // default
txn.commit_with(Durability::QueueDurable).await?; // via buffer front-end
```

`Buffered` mode is appropriate for high-throughput ingestion where losing
the last 100 ms of writes on a crash is acceptable (e.g. metrics, counters).
The default (`WalDurable`) guarantees that a committed version survives
arbitrary process failures.

### 8.3 Backpressure

When writes arrive faster than the writer can flush Deltas to SlateDB, the
system applies backpressure to prevent unbounded memory growth. Once the
number of unflushed in-memory Deltas exceeds a configurable threshold
(default: 4), new `commit()` calls are stalled until a flush completes and
frees memory.

This ensures the system degrades gracefully under load rather than running
out of memory. The backpressure threshold is tuned alongside the Delta size
(`max_unflushed_bytes`, default 128 MiB) and flush interval to balance
write throughput against memory usage.

### 8.4 Validation pipeline

Before a transaction commits, the writer runs three validators in order
of increasing cost:

1. **Type validation** — every asserted value matches its attribute's
   declared type (cheap, in-memory).
2. **Cardinality / uniqueness validation** — uniqueness constraints
   require an AVET probe per asserted value.
3. **Reference integrity** — every `ref_val` points to an entity that
   exists in the EAVT index at the current version.

Each validator can be disabled per-namespace for performance-sensitive
workloads, but the default is "all on".

### 8.5 Batched commits

For high-throughput ingestion, the writer accepts multiple `begin → commit`
calls and groups them into one durable batch. The group commit returns
when SlateDB has flushed the batch; until then, callers see a "pending"
result. Group-commit window is tunable (default 10 ms or 1000 facts,
whichever comes first).

### 8.6 Idempotency

Every transaction carries an optional **idempotency token**. The writer
maintains a small LRU of recently-seen tokens; replays return the original
version without re-committing. This makes the write path safe for clients
behind a load balancer where retries are routine.

### 8.7 Stateless multi-producer ingestion (Buffer pattern)

The direct write path routes all clients through the single Rocklake writer
process. For deployments with application instances spread across
availability zones this means cross-zone transfer costs and write
unavailability during writer restarts.

An optional **Buffer front-end** decouples write availability from writer
availability using an object-storage queue:

```
  ┌────────────────────────────────────────────────────────────┐
  │  App A  |  App B  |  App C    (producers, any AZ)       │
  └───┬────────┬────────┬────────────────────────────────┘
        │          │          │  flush ULID-named batch
        └──────────┴─────────┘
                      │
           ┌──────────▼──────────┐
           │  Object Storage    │  data/ + CAS queue manifest
           └──────────┬──────────┘
                      │  poll
           ┌──────────▼──────────┐
           │  Rocklake Writer  │
           │  BufferConsumer    │
           │  → FactStore.write │
           └────────────────────┘
```

Key properties:

- **Multi-producer, single consumer.** Any number of app instances append
  to the queue manifest concurrently via compare-and-swap. Only the
  Rocklake writer consumes. This preserves the single-writer guarantee
  on the fact store.
- **Epoch-based consumer fencing.** On writer restart, the consumer
  increments the manifest epoch, fencing any zombie consumer from a
  previous instance — the same fencing primitive v1.x already uses for
  writer exclusivity.
- **At-least-once delivery with idempotent replay.** If the consumer
  crashes between processing a batch and acknowledging it, the batch is
  re-processed on restart. Idempotency tokens (§8.6) make replay safe.
- **Exactly-once delivery via atomic sequence persistence.** To upgrade
  from at-least-once to exactly-once, the consumer writes the batch
  payload and the last acknowledged sequence number in the *same*
  `WriteBatch` to SlateDB. On restart the new consumer reads the last
  persisted sequence number and resumes from that position, then bumps
  the manifest epoch to fence any zombie consumer. No batch is committed
  twice.
- **ULID-named batch files.** Each flushed batch is named with a ULID
  (Universally Unique Lexicographically Sortable Identifier) that encodes
  a millisecond-precision timestamp. Batches sort naturally by creation
  time without coordination, and the timestamp in the ULID lets operators
  instantly identify which batches correspond to a given time range.
- **O(1) manifest appends.** The queue manifest is designed so that
  existing entries are never deserialized during an append — only the tail
  is written. Concurrent producers use compare-and-swap on the manifest;
  on conflict a producer re-reads and retries. Append latency is
  independent of queue depth.
- **Self-describing batch format.** Each batch file contains an
  optionally-compressed record block followed by a compact footer (7
  bytes) encoding the compression type, record count, and format version.
  The consumer reads the footer first, decompresses if needed, and then
  parses the length-prefixed record entries. This makes it safe to add new
  compression codecs or record types without breaking existing consumers.
- **Write availability decoupled from writer availability.** Apps continue
  writing to object storage even while Rocklake is down or deploying.

Trade-off: a fact is not queryable until the producer flushes its batch
(default 100 ms), the consumer polls the manifest (default 1 s), and the
writer commits to SlateDB. For audit, compliance, and metadata workloads
— the primary targets for v2.x — this is acceptable. For sub-second
freshness, use the direct write path (§8.1–§8.6).

This pattern is directly related to the multi-writer exploration in §10 but
implemented as a **single-consumer** queue, which keeps the single-writer
guarantee intact. It is evaluated in Phase 2.3.

### 8.8 Graceful shutdown and deployment strategy

**Graceful shutdown.** On `SIGTERM` / `SIGINT`, the writer:

1. Stops accepting new transactions.
2. Drains in-flight transactions (those already past validation).
3. Flushes the current Delta and pending WAL entries to durable storage.
4. Acknowledges the buffer consumer’s last processed batch.
5. Exits cleanly.

Kubernetes deployments should set `terminationGracePeriodSeconds: 60` to
give the flush time to complete before the kubelet force-kills the pod.

**Recreate strategy.** Because the writer holds an exclusive epoch lock
(writer fencing, §3.4), a `RollingUpdate` creates a window where the new
pod attempts to acquire the epoch while the old pod still holds it —
causing the new pod to be fenced and never become ready. The correct
Kubernetes deployment strategy is `Recreate`: the old pod terminates fully
before the new one starts.

```yaml
spec:
  replicas: 1
  strategy:
    type: Recreate
```

Read replicas have no such constraint and can use `RollingUpdate` freely.

**Health check endpoints.** The writer and reader expose two distinct
HTTP health endpoints:

| Endpoint | Probe type | Behaviour |
|----------|------------|-----------|
| `GET /-/healthy` | Liveness | Returns 200 while the process is running. Kubernetes restarts the pod only if this fails. |
| `GET /-/ready` | Readiness | Returns 200 if the SlateDB storage backend is accessible; returns 503 otherwise. Kubernetes removes the pod from the load balancer until the backend recovers. |

The critical distinction: a writer that temporarily cannot reach the object
store should fail **readiness**, not liveness. Failing liveness would
trigger a restart loop that does not fix the underlying problem; failing
readiness removes the pod from the load balancer while leaving the process
alive to recover when the object store becomes reachable again.

---

## 9. Observability

### 9.1 Per-transaction metrics

The writer exposes per-transaction metrics on Prometheus:

- Commit latency (p50, p99, p99.9) per namespace.
- Facts asserted, retracted per transaction.
- Bytes written per transaction.
- Validation rejections by reason.

### 9.2 Per-query metrics

The reader exposes per-query metrics:

- Query latency by interface (typed / SQL / rules).
- Scan width per index per query class.
- Cache hit rate at the SST level (memory tier, disk tier, S3 fallback).
- Materialised-view freshness lag.

### 9.3 Storage-layer metrics

Both the writer and readers expose the underlying SlateDB metrics under
the `slatedb_*` prefix. These are essential for debugging storage-level
performance and compaction behaviour:

- `slatedb_compaction_*` — compaction throughput, SST sizes, duration.
- `slatedb_cache_*` — hit/miss rates for memory and disk tiers.
- `slatedb_wal_*` — WAL flush latency, WAL file count.
- `slatedb_manifest_*` — manifest update frequency, generation count.

### 9.4 Audit query interface

A SQL view `_audit.transactions` exposes the transaction log:

```sql
SELECT version, committed_at, operator, fact_count
FROM   _audit.transactions
WHERE  committed_at > NOW() - INTERVAL '1 hour'
ORDER  BY version DESC;
```

The view is backed directly by the tag `0x44` index — no separate audit
table needs to be maintained.

### 9.5 Buffer queue observability

When the Buffer front-end (§8.7) is active, the writer exposes additional
metrics for the ingest queue under the `buffer_` prefix:

| Metric | Type | Description |
|--------|------|-------------|
| `buffer_consumer_lag_seconds` | gauge | Wall clock minus last successfully processed batch’s ingestion time. A growing lag indicates the consumer is falling behind ingest rate. |
| `buffer_queue_length` | gauge | Number of pending (unacknowledged) batches in the manifest. |
| `buffer_batches_collected` | counter | Total batches fetched from object storage. |
| `buffer_bytes_collected` | counter | Total compressed bytes read from object storage. |
| `buffer_manifest_conflicts` | counter | CAS conflicts during manifest append. A high rate means many concurrent producers are contending; consider increasing flush interval. |
| `buffer_gc_files_deleted` | counter | Batch files deleted by the garbage collector. |

Key PromQL queries for buffer health:

```promql
# Is the consumer keeping up with producers?
buffer_consumer_lag_seconds

# Ingest rate (batches per second)
rate(buffer_batches_collected[5m])

# Producer contention
rate(buffer_manifest_conflicts[5m])
```

`buffer_consumer_lag_seconds` is the primary SLO signal for the buffered
write path: if it grows beyond the acceptable freshness window (§6.5) alert
on it directly.

---

## 10. Multi-Writer Exploration

### 10.1 Why it might be possible

Because writers only ever **append disjoint keys** (each transaction
allocates a fresh version that prefixes its keys), the substrate can in
principle accept multiple concurrent writers per fact store with conflict
detection at version-allocation time rather than per-key fencing.

### 10.2 The proposed design (provisional)

- A coordinator service (or distributed lock) allocates non-overlapping
  *version ranges* to writers.
- Each writer commits within its assigned range; no two writers can
  produce the same version.
- Schema changes still require single-writer mode (a global lock).
- Uniqueness validation requires cross-writer coordination via an AVET
  probe with a "as of latest committed across all writers" semantics.

### 10.3 The case against

- The operational complexity is large.
- The current "one store per dataset" partitioning pattern (v0.7) is
  cheap and well-understood.
- Multi-writer adds a coordination dependency that the rest of the
  substrate carefully avoids.

### 10.4 Decision

v2.x **evaluates** but does not commit to multi-writer. The deliverable
is a written design and a prototype. Adoption is gated on a real customer
workload that the partitioning pattern cannot serve.

---

## 11. Incremental View Maintenance (IVM)

### 11.1 The delta computation model

The core insight behind efficient IVM is that every relational operator —
filter, map, join, group-by, aggregate — has a *linear* counterpart that
operates on *weighted change sets* rather than full relations. A change
set (Z-set) is a multiset of records with integer weights: `+1` means the
record was added, `−1` means it was removed.

Formally: for any query Q over a relation R, there exists an incremental
query ΔQ such that `Q(R ⊕ ΔR) = Q(R) ⊕ ΔQ(ΔR)`. The cost of running ΔQ
is proportional to `|ΔR|`, not `|R|`. This property — proved rigorously in
DBSP (Automatic Incremental View Maintenance for Rich Query Languages,
VLDB 2023) — holds for the full SQL feature set including joins, recursive
queries, and window functions.

For the fact store, every committed transaction **is already a Z-set**:
it contains a set of `(entity, attribute, value, version)` tuples with
weight `+1` (asserted facts) or `−1` (retractions). There is no impedance
mismatch between the fact store's native output and the IVM engine's
native input.

### 11.2 Use cases

The following views are strong candidates for IVM in v2.x and v3.0:

| View | Description | Why IVM |
|------|-------------|--------|
| **Entity snapshot** | Latest `{attr → value}` map per entity | The most common read pattern; avoids full EAVT scan per entity |
| **Namespace rollup** | Fact count by namespace/attribute/time window | Small delta updates a single counter row per bucket |
| **Access control graph** | Recursive group → member → permission inheritance | Recursive differential dataflow (`iterate`) maintains this in sub-millisecond after any role change |
| **Referential integrity** | Set of dangling entity references | Anti-join maintained incrementally; violations surface immediately |
| **Feature store** | ML feature tables (entity → feature vector) | Feature engineering pipelines traditionally batch-only; IVM makes them real-time |
| **External sync changelog** | Per-entity change log for CDC to Postgres, Kafka, or S3 | View delta IS the CDC event; no separate change-data-capture pipeline needed |
| **Freshness SLO view** | Per-namespace view staleness in seconds | Simple aggregation over transaction timestamps |

### 11.3 Integration with the writer path

After committing a transaction to SlateDB, the writer publishes the fact
delta as an Apache Arrow IPC `RecordBatch` to the IVM scheduler. The
RecordBatch schema is fixed:

```
┌──────────────────────────────────────────────────────────────┐
│ fact_delta RecordBatch schema                                │
│  entity_id   : UInt64                                        │
│  attribute   : Utf8 (dictionary-encoded)                     │
│  value_bytes : Binary                                        │
│  tx_version  : UInt64                                        │
│  weight      : Int8   (+1 assert / -1 retract)               │
└──────────────────────────────────────────────────────────────┘
```

The IVM scheduler routes each batch to the registered view circuits. Each
circuit is a graph of linear operators that process the delta and emit a
view delta RecordBatch. The view delta is applied to the view's current
state.

**In-process path (Phase 2.2):** IVM runs synchronously inside the writer
process, driven by the same `after_commit` hook that triggers audit fact
writing. View state lives in memory (with optional NVMe spill).

**Out-of-process path (Phase 3.0):** IVM workers consume from the Buffer
queue (§8.7) asynchronously. The writer publishes fact deltas as Buffer
batches; IVM workers drain the queue and maintain view state. This
decouples IVM latency from write latency.

### 11.4 Storage format: Arrow IPC vs Parquet

Materialised view state needs a storage format that balances interactive
query latency against analytical compatibility and storage cost.

**Option A — Arrow IPC on NVMe (hot tier):**

- View state stored as memory-mapped Arrow IPC files on the local NVMe
  PVC (the same disk used by the SlateDB block cache).
- Zero-copy in-process reads: DuckDB, DataFusion, and the Rocklake SQL
  layer can query the view without deserialisation.
- SIMD-friendly columnar layout — sub-millisecond point lookups for most
  entity snapshot queries.
- No compression overhead: Arrow IPC is identical to the in-memory
  representation, so reads require no decompression pass.
- Write amplification is low: a view delta is appended as a small
  RecordBatch, and the hot file is rewritten only when a compaction
  threshold is reached.
- **Limitation:** Arrow IPC files are not directly queryable by external
  tools (Spark, Trino, Snowflake) without conversion.

**Option B — Parquet on object storage (cold tier / lakehouse):**

- View snapshots flushed to S3/GCS/Azure as Parquet files, described by
  an Iceberg or DuckLake metadata layer.
- Compressed columnar storage — typically 5–10× smaller than Arrow IPC
  for the same data.
- Predicate pushdown and column pruning: analytical engines only read
  relevant columns and row groups.
- Queryable by any Parquet-aware tool: DuckDB, Spark, Trino, BigQuery,
  Snowflake — no ETL pipeline required to connect BI tools.
- **Limitation:** Parquet writes have higher latency (typically 50–500 ms
  per file flush) and are not suitable for sub-second view updates.

**Recommendation — two-tier approach:**

```
 ┌──────────────────────────────────────────────────────────────┐
 │  IVM worker                                                  │
 │                                                              │
 │  fact delta (Arrow IPC RecordBatch)                          │
 │       │                                                      │
 │       ▼                                                      │
 │  DBSP / Z-set operators                                      │
 │       │                                                      │
 │       ▼                                                      │
 │  view delta RecordBatch                                      │
 │       │                           every N min / M GiB        │
 │       ├──────── apply ──▶  Arrow IPC on NVMe  ──snapshot──▶  │
 │       │                   (hot tier, mmap)                   │
 │       │                                                      │
 │       └──────────────────────────────────────────────────▶   │
 │                               Parquet on object storage      │
 │                               (cold tier, analytics)         │
 └──────────────────────────────────────────────────────────────┘
```

- **Hot tier:** Arrow IPC files on NVMe, updated on every view delta.
  DuckDB's `read_parquet` / `scan_arrow` can query both tiers in one SQL
  expression. Interactive latency: < 5 ms for entity snapshot queries.
- **Cold tier:** Parquet snapshots on object storage, flushed every 15
  minutes or every 1 GiB of view state (whichever comes first). Serves
  BI tools, historical analytics, and cross-system data sharing.
- A DuckLake (DuckDB lakehouse) catalog entry advertises both tiers as a
  single logical table, so consumers use standard SQL with no tier
  awareness.

### 11.5 Scale-out architecture

IVM workers are stateless with respect to the fact store: they consume the
fact delta stream and maintain their own view state. This matches the
disaggregated compaction model (§3.8) and the Buffer queue pattern (§8.7).

**Scale-out properties:**

- **View-level parallelism.** Each registered view can be processed by a
  separate IVM worker pod. Workers do not coordinate with each other.
- **Namespace-level parallelism.** A single view over a large namespace
  can be sharded by entity hash across multiple workers. Each worker
  maintains its partition of the view state independently.
- **Bootstrap / catch-up mode.** A new or restarted IVM worker replays
  the full transaction log (from the SlateDB manifest) to build its
  initial view state, then switches to incremental mode. Replay cost is
  linear in transaction count, not entity count.
- **Exactly-once semantics.** The IVM worker uses the same sequence
  persistence technique as the Buffer consumer (§8.7): it atomically
  writes `(view_state_delta, last_processed_tx_version)` to its own
  SlateDB namespace. On restart it resumes from the last persisted
  version, ensuring no transaction is applied twice.

```
 Writer ──fact delta──▶  Object Storage  ──poll──▶  IVM worker A (Entity snapshot)
                                         ──poll──▶  IVM worker B (Access control graph)
                                         ──poll──▶  IVM worker C (Feature store)
```

### 11.6 DBSP crate evaluation

The `dbsp` crate (from the Feldera project) provides a complete Rust
implementation of the DBSP incremental computation model:

| Capability | Relevance to Rocklake |
|------------|------------------------|
| Full SQL incrementally (joins, aggregates, window functions, recursion) | Enables arbitrary SQL-declared views without hand-writing operators |
| Datasets larger than RAM via NVMe spill (`FallbackZSet`, `FallbackKeyBatch`) | Handles large entity snapshots without memory pressure |
| Multi-worker scale-out via timely dataflow | Long-term horizontal partitioning of expensive views |
| LATENESS annotations for time-series GC | Bounds storage for event/metric views over sliding windows |
| Ad-hoc queries against materialised state via DataFusion | Consistent with Rocklake's existing DataFusion integration |
| MIT licensed, written in Rust | Fits the crate dependency model |

For Phase 2.2, a hand-written Z-set operator set (filter, project, group-by)
is simpler to integrate and audit. The DBSP crate becomes compelling in
Phase 3.0 when SQL-declared views need joins and recursive rules.

### 11.7 Evaluation timeline

| Phase | Scope | Deliverable |
|-------|-------|-------------|
| 2.2 | In-process entity snapshot view via hand-written Z-set filter+aggregate | Working materialised view over EAVT delta |
| 2.3 | Out-of-process IVM worker, Arrow IPC hot tier, Parquet cold-tier snapshots | Disaggregated view worker with DuckLake catalog entry |
| 3.0 | SQL-declared views via DBSP crate, namespace sharding, multi-worker bootstrap | Full IVM with recursive views, feature store pattern |

---

## 12. Federation

### 12.1 Cross-store queries

A query in v2.5+ can reference entities in multiple fact stores:

```
?- alice ∈ store_a.user, alice.orders ⊆ store_b.order.
```

Implementation: the query planner identifies cross-store joins,
parallelises scans across stores, and joins results in memory. There is
no global transaction — each store contributes its own version to the
query, and the planner records the cross-store version vector for
reproducibility.

### 12.2 Time alignment

Cross-store queries can specify time alignment:

- `as_of(wall_clock_time)` — each store resolves its own version at the
  given wall-clock instant.
- `as_of(version_vector)` — explicit (store_a@V₁, store_b@V₂) coordinates.

### 12.3 No global coordinator

There is no central federation service. Stores discover each other via
configuration (URLs in a federation manifest), and queries are planned
client-side. This keeps the operational story consistent with everything
else in v2.x: a bucket and a binary, nothing more.

---

## 13. Extraction Boundary and API Surface

### 13.1 The `rocklake-factstore` crate

```rust
pub struct FactStore { /* ... */ }

impl FactStore {
    pub async fn open(uri: &str, opts: OpenOptions) -> Result<Self>;
    pub fn begin(&self) -> Transaction;
    pub fn as_of(&self, version: Version) -> Reader;
    pub fn current(&self) -> Reader;
    pub async fn excise(&self, request: ExcisionRequest) -> Result<ExcisionReceipt>;
}

pub struct Transaction { /* ... */ }

impl Transaction {
    pub fn assert<V: Into<Value>>(
        &mut self, entity: EntityId, attribute: &str, value: V,
    ) -> &mut Self;
    pub fn retract(&mut self, entity: EntityId, attribute: &str) -> &mut Self;
    pub fn assert_at<V: Into<Value>>(
        &mut self, entity: EntityId, attribute: &str, value: V,
        valid: Range<Instant>,
    ) -> &mut Self;
    pub async fn commit(self) -> Result<Version>;
}

pub struct Reader { /* ... */ }

impl Reader {
    pub async fn get(&self, entity: EntityId, attribute: &str) -> Result<Option<Value>>;
    pub async fn entity(&self, entity: EntityId) -> Result<EntityView>;
    pub fn query(&self) -> QueryBuilder;
    pub fn pull(&self, entity: EntityId, spec: PullSpec) -> Pull;
    pub fn rules(&self) -> RulesEngine;
}
```

### 13.2 The lakehouse adapter

The existing `rocklake-catalog` crate becomes a thin **adapter** on top
of `rocklake-factstore`. Each of the 28 lakehouse tables maps to a
schema with attributes named for the spec columns. The adapter exposes
the v1.x API unchanged for backward compatibility.

This is a powerful proof point: if the lakehouse catalog itself runs
cleanly on the generic substrate, every other schema can too.

### 13.3 Compatibility commitment

- v1.x lakehouse catalogs **upgrade in place** to v2.0 — the on-disk
  format is identical, the adapter speaks the same wire protocol.
- v1.x APIs are preserved through the entire v2.x line.
- The generic API is **independently versioned**: `rocklake-factstore`
  may reach 1.0 before or after Rocklake v2.0 ships.

### 13.4 Possible standalone-project promotion

Once `rocklake-factstore` stabilises (no breaking changes for two minor
releases, ≥ 2 production users beyond the lakehouse adapter), the crate
can be promoted to a **standalone project** with its own repository,
governance, and release cadence. Rocklake would then depend on it as an
external crate. This is explicitly *not* required for v2.0 to ship —
in-workspace extraction is sufficient.

---

## 14. Implementation Phases

### Phase 2.0 — Extraction (foundational)

- [ ] Carve `rocklake-factstore` out of `rocklake-core`.
- [ ] Re-implement the lakehouse catalog as a `rocklake-factstore`
      adapter.
- [ ] All v1.x tests pass against the adapter.
- [ ] Zero on-disk format changes; in-place upgrade verified.

### Phase 2.1 — Generic fact model

- [ ] EAVT primary index implemented with big-endian version in key.
- [ ] Temporal key pruning: `as_of(V)` is a key-range query, no post-filter.
- [ ] AVET and VAET posting lists stored as RoaringBitmaps (§3.3).
- [ ] Merge operators for AVET/VAET writes: blind `AddEntity` operands, no read on the write path (§3.4).
- [ ] AVET built for attributes declaring `:indexed` or `:unique`.
- [ ] AEVT built for attributes declaring `:attribute-scan`.
- [ ] VAET built for attributes declaring `:reverse-ref`.
- [ ] Schema-as-facts: strict mode (§7.1) with declared attributes.
- [ ] Dynamic mode (§7.1): infer and record types from first assertion.
- [ ] ListingEntry records for attribute discovery in dynamic namespaces: one entry per (namespace, attribute) on first assert, enabling O(1) attribute enumeration without full EAVT scans.
- [ ] Block-based sequence allocation (§3.6): pre-allocate ID ranges, O(1) crash recovery.
- [ ] Transaction log (tag `0x44`) and audit view.
- [ ] Typed Rust API: `assert`, `retract`, `as_of`, `entity`, `query`.
- [ ] Property tests: every fact appears in EAVT and every declared secondary
      index; no undeclared secondary index entries.

### Phase 2.2 — Query layer

- [ ] Pipeline query model: source operators (`from`, `rel`) + tail
      operators (`where`, `with`, `return`, `order_by`, `limit`,
      `aggregate`, `join`, `left_join`).
- [ ] Index selection planner (§4.2) using per-attribute statistics.
- [ ] Pull API: nested pull compiles to batched sub-pipelines (§4.4).
- [ ] Rule-based query engine with semi-naïve recursive evaluation (§4.3).
- [ ] SQL surface with `FOR SYSTEM_TIME AS OF` and `FOR ALL SYSTEM_TIME`.
- [ ] `NEST_ONE` / `NEST_MANY` SQL extensions for hierarchical results (§4.5).
- [ ] Cross-interface parity tests: same logical query → same physical plan
      regardless of surface syntax.

### Phase 2.3 — Read scale-out

- [ ] `rocklake reader` binary with three deployment modes.
- [ ] CDN cache contract documentation and validated proxy configurations.
- [ ] Linear-scaling benchmark to ≥ 100 reader pods.
- [ ] Lambda / edge cold-start guide.
- [ ] Buffer front-end for stateless multi-producer ingestion (§8.5):
      CAS manifest, epoch fencing, at-least-once delivery.
- [ ] Disaggregated compaction deployment guide: compactor on a separate
      (spot) instance, writer unaffected.

### Phase 2.4 — Schema evolution

- [ ] All "without rewrite" changes from §7.3.
- [ ] Dynamic-to-strict mode promotion path.
- [ ] Migration log (tag `0x46`) and replay tool.
- [ ] Schema diff CLI.

### Phase 2.5 — Bi-temporal and retention

- [ ] `valid_from` / `valid_to` storage and query.
- [ ] Per-attribute retention policies.
- [ ] Per-entity excision (right-to-be-forgotten).

### Phase 2.6 — Materialised views

- [ ] Declarative view definition.
- [ ] Incremental maintenance.
- [ ] View staleness metric.

### Phase 2.7 — Federation (stretch)

- [ ] Cross-store query planner.
- [ ] Version-vector coordinates.
- [ ] Multi-store time alignment.

### Phase 2.8 — Multi-writer evaluation (stretch)

- [ ] Written design document.
- [ ] Prototype.
- [ ] Decision: adopt / defer / reject.

---

## 15. Testing Strategy

Inherits the v1.x testing pyramid (property + unit + golden + crash +
performance) and adds:

| Layer | What it tests |
|-------|---------------|
| **Index consistency** | Every fact appears in EAVT and every declared secondary index with byte-identical content. Property test, exhaustive. |
| **Index opt-in isolation** | A fact with no secondary indexes declared does not appear in any secondary index. |
| **Temporal key pruning** | A scan `as_of(V)` does not read any key with version > V (verified at the key-range level, not post-filter). |
| **Query-plan parity** | Logically equivalent queries across SQL, rules, and the typed API produce identical physical plans. |
| **Time-travel determinism** | A query at version V returns identical results regardless of how many versions exist past V. |
| **Schema-evolution safety** | Every permitted change in §7.2 leaves historical queries unchanged. |
| **Scale-out fairness** | Adding readers does not degrade writer latency by more than 1 % p99. |
| **Bi-temporal correctness** | `as_of(V)`, `valid_at(t)`, and `bi_temporal(V, t)` all give correct results under random fact / valid-time interleavings. |
| **Excision auditability** | Every byte-level deletion produces a tamper-evident audit fact. |
| **Federation** | Cross-store query results match the union of per-store query results under aligned versions. |

---

## 16. Performance Targets

### 15.1 Write path

| Workload | Target |
|----------|--------|
| Single-fact transaction (warm)         | p50 < 20 ms, p99 < 80 ms |
| 1000-fact batched transaction (warm)   | p50 < 50 ms, p99 < 200 ms |
| Sustained throughput, single writer    | ≥ 5 000 facts / sec       |
| Sustained throughput, batched commits  | ≥ 50 000 facts / sec      |

### 15.2 Read path

| Workload | Target |
|----------|--------|
| EAVT point lookup (warm cache)         | p50 < 5 ms, p99 < 30 ms   |
| EAVT point lookup (cold cache)         | p50 < 80 ms, p99 < 250 ms |
| AVET unique lookup (warm)              | p50 < 5 ms, p99 < 30 ms   |
| Pull spec, 10 entities × 5 attrs each  | p50 < 50 ms, p99 < 150 ms |
| Recursive rule, 1 000 facts visited    | p50 < 200 ms              |

### 15.3 Scale-out

| Workload | Target |
|----------|--------|
| 100 reader pods, indexed lookups       | aggregate ≥ 50 000 QPS    |
| Writer concurrent with 100 readers     | < 1 % p99 degradation     |
| Cold-start reader (Lambda)             | first-byte < 300 ms       |

These targets are aspirational and serve as the gating criteria for
declaring v2.x "world-class". They are revised as Phase 2.0 benchmarks
land.

---

## 17. Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Tiered index model adds schema-declaration friction for new adopters | Medium | Medium | Dynamic mode (§7.1) removes the requirement; strict mode can be adopted incrementally. |
| EAVT-only stores have poor attribute-scan performance | High | Low | By design — §1.2 explicitly excludes bulk analytical aggregations. Materialised views (§4.6) and Parquet export are the recommended escape hatches. |
| `NEST_ONE` / `NEST_MANY` encoding choice (JSON vs. Arrow) blocks adoption | Medium | Medium | Default to `serde_json::Value` for PG-wire; expose Arrow-typed API for embedded use. See Open Questions §19. |
| Query planner cannot beat hand-written scans in early benchmarks | High | Medium | Ship the typed API first; defer SQL/rules optimisation until the substrate is stable. |
| Bi-temporal semantics confuse early adopters | High | Low | Keep bi-temporal opt-in per attribute; default is single-time. |
| Materialised-view freshness lag breaks user expectations | Medium | High | Bound the lag explicitly (10 ms group-commit window) and expose it as a metric. |
| Schema evolution rule subset is too restrictive | Medium | Medium | Ship the migration log + migration tool as escape hatches. |
| Federation introduces a new failure mode (partial-store unavailability) | Low | Medium | Make federation opt-in; default deployment is single-store. |
| The fact-store API competes for developer mindshare with the lakehouse | Low | High | Position the fact store as the substrate; the lakehouse remains the most polished use case for v2.x. |
| Multi-writer evaluation rabbit-hole consumes Phase 2.8 with no shippable outcome | Medium | Low | Time-box to one quarter; ship the design doc even if no prototype. |

---

## 18. Success Criteria

v2.x succeeds when:

1. The `rocklake-factstore` crate publishes a 1.0 API and the lakehouse
   adapter uses it in production with zero regressions.
2. At least one non-lakehouse schema is built on the substrate (internal
   or external) and reaches a usable state.
3. The reader binary demonstrates linear scaling to 100 pods in a public
   benchmark.
4. The rule-based query engine answers recursive queries correctly under
   property-testing pressure.
5. Bi-temporal queries are correct under randomised workloads.
6. Documentation covers every API, every deployment mode, every
   compliance scenario.
7. A written, evidence-backed decision on multi-writer is recorded.

---

## 19. Open Questions

- What is the right "tag block" allocation policy for user-defined
  schemas? (Tag ranges, dynamic registration, or both?)
- Should the rule-based query language be standardised (e.g. to an
  existing dialect) or stay Rocklake-specific to leave room for novel
  features?
- How much of the value-type system should be extensible (custom types
  via plugin?) versus closed (the §3.5 list and no more)?
- Is materialised-view incremental maintenance (§11) best implemented via
  the `dbsp` crate in Phase 2.2, via hand-written Z-set operators, or
  deferred entirely to Phase 3.0? The Phase 2.2 scope (entity snapshot
  only) likely does not need the full DBSP algebra.
- Can the federation design hold across object-storage providers (S3 ↔
  GCS ↔ Azure) without performance cliffs?
- Should counters be per-entity-type or global per-store? (Performance vs.
  conceptual simplicity trade-off.)
- Should `NEST_ONE` / `NEST_MANY` return native JSON, Arrow structs, or
  Rust `serde_json::Value`? The answer affects the SQL surface, the FFI,
  and the PG-wire encoding.
- What is the promotion path from dynamic mode to strict mode? Should
  inferred types be auto-promoted after N asserts, or always require
  explicit operator action?
- How do secondary indexes interact with the excision path? Excision must
  remove facts from EAVT **and** every secondary index; the implementation
  must handle the case where secondary index declarations change between
  the assertion version and the excision version.
- Should entity-level bloom filters and block-level record counts (§3.7) be
  proposed as upstream SlateDB enhancements, or implemented via a
  Rocklake-specific SST extension? The former is the right long-term path
  but requires coordination with SlateDB maintainers.
- Should the Buffer front-end (§8.7) use an existing open-source
  object-storage queue crate (lower maintenance, proven) or be
  re-implemented in-house (tighter integration, no transitive SlateDB
  version constraint)?

---

## 20. References

- [`plans/blueprint.md`](blueprint.md) — v1.x blueprint and the original
  architectural principle (§1.4) and extraction boundary (§5.29).
- [`docs/concepts/fact-store-vision.md`](../docs/concepts/fact-store-vision.md) —
  Conceptual motivation and use-case catalogue.
- [`docs/architecture/key-layout.md`](../docs/architecture/key-layout.md) —
  Current tag namespace allocation and reservations.
- [`docs/architecture/mvcc-implementation.md`](../docs/architecture/mvcc-implementation.md) —
  v1.x MVCC mechanics that v2.x generalises.
- [`ROADMAP.md`](../ROADMAP.md) — v2.x roadmap entry (Exploration).
