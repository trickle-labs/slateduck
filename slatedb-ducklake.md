# SlateDuck: A DuckLake Catalog on SlateDB

## Introduction (for a non-technical audience)

Modern data analysis often relies on a **"data lake"** — a big pile of files
(usually in cloud storage like Amazon S3) that hold all of an organization's
data. To make a data lake useful, you also need a **"catalog"**: a separate
piece of software that keeps track of which files belong to which tables, what
columns those tables have, and what the data looked like at any point in
history (so you can do "time travel" queries). The combination of a data lake
plus a catalog is called a **lakehouse**.

**DuckLake** is a new, simple, and elegant open-source lakehouse format
created by the team behind DuckDB. The clever idea behind DuckLake is that
the catalog is just a small, ordinary SQL database (today: PostgreSQL,
SQLite, or DuckDB itself), and the data is just Parquet files in cloud
storage. This is simpler and faster than older formats like Iceberg or Delta
Lake, which encode their catalog as a maze of small JSON and Avro files
scattered throughout the data lake.

**SlateDB** is a brand-new, very fast storage engine that is designed from
the ground up to live entirely inside cloud object storage (S3, GCS, Azure
Blob Storage). It has no servers to run, no disks to manage, and yet it
behaves like a real transactional database. It is "embedded" — your
application links to it as a library, the same way it might link to SQLite.

**SlateDuck** is the project of marrying the two: using SlateDB as the
catalog backend for DuckLake. The result would be:

- A lakehouse where **both the catalog and the data live in the same S3
  bucket** — no separate database server to provision, monitor, patch, or
  back up.
- **Serverless-friendly**: the catalog state itself needs no database server.
  In the B-first plan, clients contact a stateless SlateDuck sidecar plus S3;
  in the long-term native path, a Lambda function, container, or stream
  processor can read or write without contacting any external service besides
  S3.
- **Safer concurrent access**: SlateDB's writer-fencing and
  compare-and-swap semantics prevent the kinds of catalog corruption that
  can happen when you naively put a SQLite file on S3.
- **Cheap**: object storage is the cheapest durable storage available, and
  SlateDB is engineered to minimize the number of API calls it makes.

If it works, SlateDuck would be the first DuckLake catalog whose durable state
is *truly* zero-infrastructure for cloud deployments — a real "lakehouse in a
bucket". Strategy B still has a small stateless service; Strategy C removes
that service.

**A second, deeper goal shapes every storage decision in this plan: committed
catalog facts are *never physically deleted* by normal operation and are
always readable at the `dl_snapshot_id` at which they were written.** Physical
deletion exists only via an explicit, audited excision command. This buys three
things that matter for the long term:

1. **Horizontal read scale-out.** Because catalog-data keys are stable once
   written, any number of readers anywhere can serve a query at any historical
   `dl_snapshot_id` without coordinating with a writer or even with each other.
   The catalog becomes a content-addressable log plus derived indexes; replicas
   are pure caches.
2. **Time as a first-class dimension.** Time travel stops being a feature
   layered on top of MVCC and becomes the natural read mode. The "current"
   view is just "as of the largest committed `dl_snapshot_id`".
3. **A path to a general fact store.** DuckLake is the first schema SlateDuck
   ships, but the same storage substrate — append-only keys scoped by
   `dl_snapshot_id`, rebuildable from object storage, queryable at any
   historical point — can host other relational schemas in future releases.
   See section 1.4 and the v2.x ecosystem entry in the roadmap.

This principle is binding at the *catalog-data layer*: the rows that record
tables, columns, snapshots, and data files are never overwritten outside their
bounded lifecycle, and are never physically deleted outside explicit excision.
Internal infrastructure state (counters, leadership, `retain-from`) is managed
with simple transactional writes, which are correct and safe because SlateDB
enforces a single-writer constraint.

---

## 1. Background and Reference Material

### 1.1 SlateDB at a glance

- Embedded LSM-tree key-value store written in Rust.
- All durable state (WAL, SSTs, manifest) lives in an
  [`object_store`](https://docs.rs/object_store) backend (S3, GCS, Azure,
  local FS, in-memory).
- Public Rust API surface includes
  [`Db`](https://docs.rs/slatedb/latest/slatedb/struct.Db.html),
  [`DbReader`](https://docs.rs/slatedb/latest/slatedb/struct.DbReader.html),
  [`DbSnapshot`](https://docs.rs/slatedb/latest/slatedb/struct.DbSnapshot.html),
  [`DbTransaction`](https://docs.rs/slatedb/latest/slatedb/struct.DbTransaction.html),
  [`WriteBatch`](https://docs.rs/slatedb/latest/slatedb/struct.WriteBatch.html),
  [`Checkpoint`](https://docs.rs/slatedb/latest/slatedb/struct.Checkpoint.html),
  with configurable
  [`IsolationLevel`](https://docs.rs/slatedb/latest/slatedb/enum.IsolationLevel.html).
- Operations: `put`, `get`, `delete`, `scan(range)`, `scan_prefix`,
  atomic `WriteBatch`, transactions with snapshot isolation, point-in-time
  snapshots/checkpoints.
- Constraints: **single writer**, multiple readers; writer fencing
  enforced; max key 65 KiB, max value 4 GiB; values are opaque bytes.
- Existing bindings: Rust (primary), Go, Python, Node.js.

### 1.2 DuckLake at a glance

- A lakehouse format (spec at <https://ducklake.select/docs/stable/>).
- Two building blocks:
  1. **Catalog database** — a SQL database with transactions and SQL-92
     primary-key constraints. The schema is fixed: 28 tables defined by
     the spec (see
     <https://ducklake.select/docs/stable/specification/tables/overview.html>).
  2. **Data storage** — Parquet files (and small Parquet "delete files"
     for merge-on-read deletes), stored in object storage or a local
     filesystem.
- MVCC is done in SQL: versioned metadata rows carry `begin_snapshot` and
  `end_snapshot` columns; a snapshot ID is a monotonically increasing integer.
  Some auxiliary/statistics tables are not versioned and must follow the full
  schema script rather than a blanket MVCC rule.
- The reference client is the
  [`ducklake` DuckDB extension](https://duckdb.org/docs/current/core_extensions/ducklake)
  (DuckDB ≥ 1.5.2 as of 2026). The extension reaches the catalog database
  through one of DuckDB's database-connector extensions: `duckdb`
  (native), `sqlite`, `postgres`, or `mysql`. Other clients exist for
  DataFusion, Spark, Trino, and pg_ducklake.
- The set of SQL operations issued against the catalog is **small and
  well-specified** (see
  <https://ducklake.select/docs/stable/specification/queries.html>) —
  basically `INSERT`, point and range `SELECT` with simple `WHERE`
  clauses, and `UPDATE` of `end_snapshot`, plus generated DDL/DML for
  inlined-data tables. No general-purpose SQL is needed.

### 1.3 Core design question

DuckLake's catalog is **relational**; SlateDB is a **key-value** store.
We must therefore choose one of three layering strategies for the
catalog integration:

| Strategy | What it is | Pros | Cons |
| --- | --- | --- | --- |
| **A. SQLite VFS over SlateDB** | Implement a SQLite Virtual File System whose pages are stored as keys in SlateDB. Reuse the `sqlite` + `ducklake` DuckDB extensions unchanged. | Smallest amount of new code; reuses all existing DuckLake SQL execution. | SQLite expects a coherent random-access file; we'd be re-creating a page server on top of an LSM. Performance and concurrency may be poor. Single-writer is fine; SQLite's locking is a poor fit for a distributed setting. This should be an optional spike, not the main implementation path. |
| **B. PostgreSQL-wire sidecar over SlateDB** | Write a server process that speaks the PostgreSQL wire protocol, executes the captured DuckDB/DuckLake catalog query set, and stores everything in SlateDB. Reuse the `postgres` + `ducklake` DuckDB extensions unchanged. | Reuses DuckDB's existing DuckLake client path; supports multiple readers via TCP. Works with DuckDB's existing connector model and does not require a DuckDB extension fork. | Requires implementing a meaningful subset of PG protocol + SQL parser. Adds a network hop. Not "embedded". Other clients need their own compatibility corpus. |
| **C. Native DuckLake catalog backend** | Implement DuckLake's `Catalog` C++ interface directly, dispatching to a Rust SlateDB library via FFI. No SQL involved on the SlateDB side. | Cleanest design; full embedded; best performance; lets us choose KV key layouts optimised for DuckLake access patterns. | Requires patching/forking the `ducklake` DuckDB extension or contributing upstream. Most C++ work. |

The recommended plan after assessment is **B first**: build the
PostgreSQL-wire sidecar as the production implementation, keep **C** as
the long-term embedded path, and treat **A** only as an optional,
time-boxed spike if we need a throwaway demo or a SQLite conformance
oracle. Strategy A should not block Phase 4.

**Assessment updates folded into this plan:**

- Validate SlateDB transaction, flush, and fencing behavior before data-model
  work; several code snippets are intentionally pseudocode until Phase 0 proves
  the exact APIs.
- Use the actual DuckLake v1.0 list of 28 catalog tables; the previous matrix
  included several non-spec table names and missed macro/mapping/sort tables.
- Follow the full schema script for key shapes: versioned tables without SQL
  primary keys must include `begin_snapshot` in the SlateDB key so historical
  rows are never overwritten.
- Treat PostgreSQL wire compatibility as more than SQL parsing: type OIDs,
  text/binary formats, session `SET`/`SHOW`, and extended query protocol are
  Phase 4 requirements.
- Keep the data plane and catalog plane separate: DuckDB writes Parquet files
  directly, while SlateDuck writes only the catalog.

### 1.4 Architectural Principle: Catalog-Data Immutability

SlateDuck's durability commitment is: **every catalog fact committed at
`dl_snapshot_id = N` is always readable at N via time travel, and can only be
physically removed via the explicit, audited `slateduck excise` command.** This
is the hard constraint. Everything else in this section derives from it.

#### Two categories of storage state

| Category | Examples | Immutability rule |
| --- | --- | --- |
| **Catalog-data facts** | Rows in any of the 28 DuckLake tables; inlined-data rows under `0xFD` | Never physically deleted outside excision. Each logical version occupies a distinct key (via `begin_snapshot` in the key); the value is written once on creation, then updated at most once when `end_snapshot` is set at drop/alter time. That single terminal update is permitted because it preserves all original row data and only marks the row invisible to future snapshots — time travel to the row's `begin_snapshot` still works correctly. |
| **Infrastructure state** | Counters (`0xFE`), writer epoch, endpoint, `retain-from`, catalog-format-version (`0xFF`) | Managed with simple transactional writes. Safe because SlateDB enforces a single writer; no concurrent update races exist. |

The distinction is deliberate. Applying the strictest "append new key for every
change" discipline to infrastructure state — counters, leadership records,
`retain-from` — would require O(N-snapshots) keys per counter and a
scan-to-max on every writer startup, with no user-visible correctness benefit.
It would also contradict how LSMs work: SlateDB already implements every value
update as an SST append that masks the old version; there is no meaningful
difference between "new key" and "new value at existing key" at the storage
layer. The right boundary is the application-semantic one: user-visible history
is preserved forever; implementation-private housekeeping keys are
transactionally updated.

#### What this means concretely

| DuckLake-level concept | Naive (rejected) | SlateDuck |
| --- | --- | --- |
| `UPDATE … SET end_snapshot = N` | Delete the old row or treat as general mutation | Write `end_snapshot` into the version's value in one SlateDB transaction; this is the only permitted update in the version's lifetime, bounded because `end_snapshot` can be set at most once |
| Counter increment (`next_catalog_id += 1`) | Separate counter write and row write | Transactional read-modify-write of the counter key, committed atomically with the row that consumes the ID; safe because there is exactly one writer |
| Writer epoch / leader handoff | Allow two concurrent writers | Transactional write of new epoch and endpoint under `0xFF`; SlateDB's writer fencing makes this race-free |
| Catalog GC of dropped rows | Physical `delete(key)` during background GC | Refuse by default; gated behind the explicit, audited `slateduck excise` command |
| `retain-from` advancement | Silently delete rows below the floor | Advance the `retain-from` key transactionally (visibility only, no byte deletion); separate `slateduck excise` command handles physical deletion |

#### Why SlateDB's LSM reinforces this

SlateDB is an LSM tree. All writes — whether to a fresh key or an update to an
existing key — are SST appends. Historical versions of a key survive in
lower-level SSTs until compaction discards them. A `DbSnapshot` or `DbReader`
opened before a `put(key, new_value)` still sees the old value; one opened
after sees the new value. The catalog-data rule ("each version row has a
distinct key via `begin_snapshot`") means the history of user-visible data is
preserved in the key space itself, not relying on LSM multi-versioning. The
infrastructure-key rule ("transactional updates are fine") relies on LSM
multi-versioning being correct, which SlateDB guarantees.

#### Reader scale-out

Because catalog-data keys are never overwritten, a reader at `dl_snapshot_id = N`
sees a stable set of facts. New writes at N+1, N+2, … insert new keys; they
cannot alter the reader's view. This means:

- Stateless reader replicas can serve any historical `dl_snapshot_id` with
  read-only access to the catalog prefix, no coordination with the writer, and
  no coordination with each other.
- A reader can be a Lambda function, a worker pod, a CDN edge cache, or an
  embedded process.
- Read replicas need no consistency protocol beyond "open a SlateDB reader at
  a known checkpoint or manifest generation".

#### Excision: the only path to physical deletion

Two legitimate reasons exist to physically remove bytes from object storage:
legal/compliance erasure (GDPR right-to-be-forgotten, court orders) and
bounded-retention deployments where operators opt into a finite history window.
Both go through `slateduck excise`:

- Default physical retention is **infinite**. `slateduck gc` only advances
  `retain-from` (query-visibility floor); it never calls `object_store.delete`.
- Physical deletion requires `slateduck excise --before <snapshot> --apply`,
  with a recorded operator identity and reason. The excision event is itself
  persisted as an immutable audit fact under `0xFF` so the audit trail survives
  after the bytes are gone.
- Orphaned Parquet files (section 5.3) are an exception: they were never
  committed to any snapshot and are not part of the catalog fact set; the
  orphan-file sweep may delete them after the configurable grace period without
  invoking excision.

#### The long-term vision: a general fact store

The key tag space, the scope-by-snapshot indexing, and the Protobuf-with-
version-header value format together describe a generic **fact log over object
storage**. The DuckLake schema is one application of this substrate.

Future versions can expose the same substrate under additional schemas — a
user-defined relational schema, a Datalog query interface, an event-sourced
application store — without changing the storage engine. This is tracked as
the v2.x "general fact store" roadmap item.

---

## 2. Goals and Non-Goals

### Goals
- A working DuckLake catalog backed by SlateDB, queryable from the
  standard DuckDB `ducklake` extension.
- Catalog data and Parquet data live in the **same** object-storage
  bucket.
- **Catalog-data immutability** (section 1.4). Catalog facts are never
  physically deleted by normal operation; each version occupies a distinct key
  and is updated at most once (the terminal `end_snapshot` write). Infrastructure
  keys (counters, writer epoch, retain-from) use simple transactional updates
  because they carry no user-visible history.
- **Unbounded horizontal read scale-out**: any number of stateless reader
  replicas can serve queries at any historical `dl_snapshot_id` with no
  coordination.
- Correct MVCC semantics: snapshot reads, time travel, atomic snapshot
  creation.
- Writer fencing: a zombie writer cannot corrupt the catalog.
- Demonstrated end-to-end on local FS, MinIO, and S3.
- A storage substrate that is not DuckLake-specific and that can host
  additional schemas (general fact store, alternative relational catalogs)
  in future releases without changes to the storage engine.

### Non-Goals (initial)
- Multi-writer support beyond what SlateDB natively provides (one writer
  per database, plus fencing on takeover). The immutability model makes
  multi-writer *eventually* feasible (writers append disjoint facts; conflicts
  resolve at commit by `dl_snapshot_id` ordering), but v1 does not pursue it.
- SQL features beyond what DuckLake's spec queries and generated inlined-data
  table DDL/DML require.
- A general-purpose KV-backed SQL engine *in v1*. The substrate is designed to
  make this possible later (section 1.4); v1 ships only the DuckLake schema.
- Bindings for languages other than Rust and (eventually) C++ FFI for
  DuckDB.

---

## 3. Repository Layout

```
slateduck/
├── Cargo.toml                # Rust workspace root
├── crates/
│   ├── slateduck-core/       # Catalog model: types, key encoding,
│   │                         #   SlateDB read/write primitives
│   ├── slateduck-catalog/    # DuckLake-spec-aware operations
│   │                         #   (snapshot, schema, table, file, stats)
│   ├── slateduck-sql/        # Minimal SQL parser/executor for the
│   │                         #   spec query set (used by strategies B/C)
│   ├── slateduck-sqlite-vfs/ # Optional Strategy A spike only
│   ├── slateduck-pgwire/     # Strategy B: PG-wire sidecar binary
│   └── slateduck-ffi/        # C ABI for strategy C
├── extension/                # (later, only if upstream Strategy C is blocked)
├── docs/
└── tests/                    # Integration tests with DuckDB CLI
```

---

## 4. Implementation Phases

### Phase 0 — Project bootstrap (1–2 weeks)

- Set up Rust workspace, CI (GitHub Actions: fmt, clippy, test on
  Linux/macOS).
- Add `slatedb`, `object_store`, `bytes`, `tokio`, `serde`, and `prost`
  as the initial encoding dependency set.
- Stand up a tiny smoke test: open a SlateDB on the local FS, put/get,
  scan a prefix, run a transaction, take a checkpoint.
- Stand up the DuckDB CLI with the `ducklake` extension against a plain
  SQLite catalog to lock in a known-good end-to-end baseline we can
  diff against.
- Complete the SlateDB API validation gates from section 5.26 before
  Phase 1 starts: atomic `WriteBatch`, transaction-based conditional
  initialization, reader visibility after `flush()` or equivalent,
  distinguishable writer-fencing errors, SlateDuck-side batch limits,
  and prefix-scan latest-value semantics.
- Check in `docs/phase-0/slatedb-api-validation.md` with working Rust snippets
  against the pinned SlateDB crate version for `DbTransaction::get`/`put`,
  `DbTransaction::commit_with_options`, `db.write(WriteBatch)`, durable write
  options, reader visibility, writer fencing, and transaction conflict errors.
- Capture a complete DuckDB PostgreSQL-wire corpus in
  `tests/fixtures/wire-corpus/duckdb-{version}.jsonl` before Phase 1/4 work:
  startup probes, `SET`/`SHOW`, simple and extended query protocol,
  `BEGIN`/`COMMIT`/`ROLLBACK`, parameter values, parameter/result format codes,
  generated inlined-table DDL/DML, and all SQL emitted by the DuckLake tutorial.
- Record **Protobuf** as the v1 value encoding format because schema
  evolution, generated types, and debugging tooling matter more than marginal
  decode speed at this stage; keep FlatBuffers as a Phase 7 optimization
  candidate if benchmarks show encoding overhead is material.
- Run the required Strategy B frontend validation early: verify the `pgwire`
  crate handles DuckDB's Parse/Bind/Describe/Execute/Sync traffic, measure
  startup compatibility against `psql` and DuckDB, and decide whether GlueSQL
  can execute the captured query corpus with fewer than ten well-defined
  PostgreSQL-specific shims.
- Run a MinIO credential-isolation spike with separate catalog-only and
  data-only policies, and record the expected SQLSTATE mappings for permission
  failures.
- Measure durable commit and visibility-barrier latency on LocalFS, MinIO,
  S3 Standard, and S3 Express; store the p50/p95/p99 results with the Phase 0
  report so Phase 4 latency budgets are based on observed costs.

**Deliverable:** Reproducible local dev environment + a passing
"hello world" test + checked-in Phase 0 validation artifacts: SlateDB API
validation report, DuckDB wire corpus, GlueSQL/custom-dispatcher decision,
credential-isolation report, and latency baseline. No Phase 1 data-model code
should be written until these artifacts are green or the plan has been updated
for any failed assumption.

**If a Phase 0 gate fails, use these fallbacks before proceeding:**

| Failed validation | Fallback decision |
| --- | --- |
| Atomic `WriteBatch` is not all-or-none | Stop and pin or upgrade SlateDB; SlateDuck should not emulate catalog atomicity above a non-atomic KV write path. |
| Transactional insert-if-absent cannot be implemented | Require explicit one-time `slateduck init` under an external deployment lock, then reopen read-only until a transactional API is available. |
| `flush()` does not make fresh `DbReader`s observe commits | Replace `catalog_visibility_barrier` with the verified memtable/manifest flush or serve read-your-writes from the writer process until readers catch up. |
| Writer-fencing errors are not distinguishable | Keep SlateDuck's own epoch check on every commit path and map stale epochs to SQLSTATE `57P04`; treat raw SlateDB errors as internal until classified. |
| Batch size is effectively unbounded but operationally unsafe | Enforce SlateDuck's 64 MiB default limit before writing and make the limit configurable per deployment. |
| Prefix scans expose stale versions or recency order | Add a decode/dedup layer keyed by the table's row identity/version key before applying MVCC filters, and only use recency scans for diagnostics. |
| `pgwire` cannot handle DuckDB's extended protocol | Switch Strategy B to a lower-level PostgreSQL protocol implementation or defer B in favor of Strategy C; do not build SQL semantics on an incompatible wire layer. |
| `DbTransaction` cannot provide serializable read-modify-write for counters | Replace transaction-backed counters with an explicit compare-and-swap loop or a single-writer in-memory allocator that persists the counter and consumed rows in one durable batch. |
| Concurrent `open_or_create` cannot be made race-free | Require explicit `slateduck init` under a deployment lock before serving DuckDB clients. |
| DuckDB's wire corpus exceeds the bounded dispatcher model | Re-estimate Phase 4 before coding; if more than 50 distinct statement families appear, prefer GlueSQL or a fuller SQL engine rather than an ad-hoc dispatcher. |
| Credential isolation fails on MinIO/S3 | Stop before beta; a sidecar that can mutate data files or a client that can mutate catalog files violates the deployment model. |

---

### Phase 1 — Catalog data model in SlateDB (`slateduck-core`)

DuckLake's 28 tables are translated into a key-space layout in SlateDB.
The guiding principle: encode keys so that the most common spec queries
become **single point reads** or **single prefix scans**.

#### 1.1 Key layout

Use a fixed binary key encoding (big-endian integers; `u8` table tag in
the first byte for cheap prefix scans). Allocate every DuckLake v1.0 table
up front, even if some handlers are deferred, so Phase 4 cannot silently
discard writes into an unknown catalog table.

```
tag │ table / namespace                         │ dominant key payload
────┼───────────────────────────────────────────┼──────────────────────────────
01  │ ducklake_metadata                         │ scope | scope_id | metadata_key
02  │ ducklake_snapshot                         │ snapshot_id
03  │ ducklake_snapshot_changes                 │ snapshot_id
04  │ ducklake_schema                           │ schema_id
05  │ ducklake_table                            │ schema_id | table_id | begin_snapshot
06  │ ducklake_column                           │ table_id | column_id | begin_snapshot
07  │ ducklake_view                             │ schema_id | view_id | begin_snapshot
08  │ ducklake_macro                            │ schema_id | macro_id | begin_snapshot
09  │ ducklake_macro_impl                       │ macro_id | impl_id
0A  │ ducklake_macro_parameters                 │ macro_id | impl_id | column_id
0B  │ ducklake_data_file                        │ table_id | data_file_id
0C  │ ducklake_delete_file                      │ data_file_id | delete_file_id
0D  │ ducklake_files_scheduled_for_deletion     │ schedule_start | data_file_id
0E  │ ducklake_inlined_data_tables              │ table_id | schema_version
0F  │ ducklake_column_mapping                   │ table_id | mapping_id
10  │ ducklake_name_mapping                     │ mapping_id | column_id | source_name_hash
11  │ ducklake_table_stats                      │ table_id
12  │ ducklake_table_column_stats               │ table_id | column_id
13  │ ducklake_file_column_stats                │ table_id | column_id | data_file_id
14  │ ducklake_file_variant_stats               │ table_id | column_id | variant_path_hash | data_file_id
15  │ ducklake_partition_info                   │ table_id | partition_id | begin_snapshot
16  │ ducklake_partition_column                 │ partition_id | partition_key_index
17  │ ducklake_file_partition_value             │ table_id | partition_key_index | data_file_id
18  │ ducklake_sort_info                        │ table_id | sort_id | begin_snapshot
19  │ ducklake_sort_expression                  │ sort_id | sort_key_index
1A  │ ducklake_tag                              │ object_id | tag_key | begin_snapshot
1B  │ ducklake_column_tag                       │ table_id | column_id | tag_key | begin_snapshot
1C  │ ducklake_schema_versions                  │ table_id | begin_snapshot
FD  │ dynamic inlined row/delete storage        │ subtype | table_id | (schema_version or data_file_id) | row_id
FE  │ SlateDuck counters                        │ counter_id
FF  │ SlateDuck system keys                     │ writer epoch / endpoint / retain-from / catalog-format-version
```

The `0xFE` counter keys and `0xFF` system keys are managed with simple
transactional writes (section 1.4). Infrastructure state is safe to update
because there is exactly one writer; no concurrent-update races exist. The
`0xFF | catalog-format-version` key is written once at init and never updated;
a version mismatch on open returns `SQLSTATE 0A000`. Excision audit records
are appended under a dedicated `0xFF | "excised"` prefix so the audit trail
accumulates across excision runs.

`0xFD` is reserved for DuckLake-generated inlined data/delete tables rather
than a fixed spec table. Subtype `0x01` stores inlined insert rows keyed by
`table_id | schema_version | row_id`; subtype `0x02` stores inlined delete
markers keyed by `table_id | data_file_id | row_id`. Subtype `0x01` values
carry row payload plus `begin_snapshot` and `end_snapshot`; subtype `0x02`
values carry the deletion `begin_snapshot` only, matching DuckLake's generated
inlined delete-table schema.

The less obvious fixed tables need explicit access-pattern tests: column and
name mapping tables preserve stable column identity across schema evolution;
partition tables describe how file paths map to partition values; sort tables
record table ordering metadata used by flush/compaction; and variant stats hold
JSON statistics for shredded `variant` fields. These should be implemented as
prefix scans by parent object (`table_id`, `partition_id`, `sort_id`, or
`mapping_id`) unless the captured DuckLake SQL corpus proves a different
dominant lookup.

`ducklake_snapshot_changes` is a single value per `snapshot_id` in the v1.0
schema; do not invent a per-change index unless a future DuckLake version adds
one.

Where ordering matters and row counts are large, consider appending the sort
field before the unique ID. For low-cardinality metadata such as columns,
prefer a stable identity/version key and sort the small result set in memory so
updates by logical ID stay simple.

For **MVCC-style filtering** (`begin_snapshot ≤ snapshot_id <
end_snapshot`), do not assume each logical ID has only one physical row. Several
DuckLake tables intentionally omit SQL primary keys because operations such as
`ALTER TABLE` create a new version with the same logical ID and a later
`begin_snapshot`. For those tables, include `begin_snapshot` in the SlateDB key
after the stable parent/logical ID fields so historical versions cannot be
overwritten. `end_snapshot` is stored in the version's value and is set by a
single in-place update when the version is retired (`DROP`/`ALTER`). This is
the *only* permitted update in a version row's lifetime — bounded because
`end_snapshot` can be set at most once — and it preserves all original row data
while only marking the row invisible to future snapshots, so time travel to
the version's `begin_snapshot` still works correctly. Creating a new version
inserts a distinct key with the same logical ID and a higher `begin_snapshot`.
(Phase 7 will revisit whether to add per-snapshot secondary indexes.)

Some dominant scan keys differ from DuckLake's SQL primary keys. For example,
`ducklake_data_file` is globally identified by `data_file_id`, but the hot read
path scans by `table_id`. For any table where the hot key does not enforce a
spec primary key, maintain a small unique-guard key under `0xFE` in the same
transaction so externally supplied IDs cannot collide silently.

#### 1.2 Value encoding

- Use **Protobuf** for forward/backward compatibility across catalog format
  versions in v1. FlatBuffers remains a performance experiment for Phase 7,
  after real catalog traces show whether decode overhead matters.
- Each value carries an internal `encoding_version` byte so we can evolve
  SlateDuck's serialized row format without confusing it with DuckLake's
  `ducklake_snapshot.schema_version` counter.

#### 1.3 Counter / ID allocation

`ducklake_snapshot` exposes `next_catalog_id` and `next_file_id`. In
SlateDB these are stored as dedicated counter keys under `0xFE`, updated
transactionally in the same atomic SlateDB transaction as the snapshot row
itself. With a single writer there are no concurrent-update races; the
transactional read-modify-write is both safe and sufficient. The in-memory
value of each counter is cached after the initial read on writer startup; every
allocating transaction reads from the cache, writes the new counter value and
the consuming row in one `DbTransaction`, then updates the cache on commit.

#### 1.5 Module surface (`slateduck-core`)

```rust
pub struct CatalogStore { db: slatedb::Db, /* … */ }

impl CatalogStore {
    pub async fn open(opts: OpenOptions) -> Result<Self>;
    pub async fn read_at(&self, snapshot_id: SnapshotId) -> CatalogReader;
    pub async fn begin_write(&self) -> CatalogWriter;
}
```

`CatalogReader` exposes typed accessors (`list_schemas`,
`get_table_files`, etc.) that internally do prefix scans + filtering
on the row metadata. `CatalogWriter` uses SlateDB `DbTransaction` for any
operation that allocates counters, checks key absence, or otherwise reads
before writing. Counter-free bulk inserts may be lowered to a final
`slatedb::WriteBatch`, but never for read-modify-write operations.

**Deliverable:** A documented Rust library that can store and retrieve every
row type defined by the DuckLake 1.0 spec plus the `0xFD` dynamic inlined
insert/delete encodings, with property tests.

---

### Phase 2 — DuckLake-spec operations (`slateduck-catalog`)

Implement the operations from
[`specification/queries.html`](https://ducklake.select/docs/stable/specification/queries.html)
as Rust methods on top of `CatalogStore`:

- `get_current_snapshot()`
- `list_schemas(snapshot_id)`
- `list_tables(schema_id, snapshot_id)`
- `describe_table(table_id, snapshot_id)` (columns + nested)
- `list_data_files(table_id, snapshot_id)` (with `LEFT JOIN` to delete
  files, in code)
- `prune_files(table_id, column_id, predicate)`
- `create_snapshot(changes, author?, message?)`
- `create_schema`, `create_table`, `drop_table`, `drop_schema`
- `register_data_file`, `register_delete_file`
- `register_inlined_insert`, `mark_inlined_insert_deleted`,
  `register_inlined_delete`, `plan_flush_inlined_data`
- `update_table_stats`, `upsert_file_column_stats`

Each write op runs inside a single SlateDB transaction so that the new
snapshot row and all referenced metadata changes are committed
atomically. SlateDB's transaction model plus the sidecar's single-writer
actor gives us a single serial write order, once Phase 0 has validated
the exact conflict-detection and durability semantics.

Also implement **multi-process fencing** as defense in depth: store a
SlateDuck writer-epoch token in a dedicated `0xFF | "writer-epoch"` key and
require every sidecar writer to prove it still owns that epoch before accepting
writes. This does not replace SlateDB's own zombie-writer fencing; it gives
SlateDuck a catalog-level role marker that can also publish the writer endpoint
and produce PostgreSQL-compatible failover errors.

**Deliverable:** End-to-end tests that perform the spec's example
sequence (snapshot → schema → table → insert → delete → time travel)
purely through the Rust API, and verify the resulting state.

---

### Phase 3 — Optional Non-Blocking Spike: SQLite VFS over SlateDB

Goal: get DuckDB's `ducklake` extension to talk to SlateDB **without
modifying DuckDB at all**, but only as a time-boxed feasibility spike.
The production implementation is Strategy B, so this phase may be
skipped if it does not produce useful evidence quickly. Treat it as a side
experiment that may run in parallel with Phase 4, not as a prerequisite for
the PG-wire sidecar.

#### 3.1 How

- The DuckDB `sqlite` extension uses libsqlite3, which already supports
  pluggable [VFS modules](https://www.sqlite.org/vfs.html).
- We implement a SQLite VFS in Rust (using
  [`rusqlite`](https://crates.io/crates/rusqlite) /
  [`libsqlite3-sys`](https://crates.io/crates/libsqlite3-sys) bindings,
  or a hand-rolled C shim) whose `xRead` / `xWrite` map 4 KiB SQLite
  pages into 4 KiB SlateDB values keyed by `(db_id, page_no)`.
- File locking maps to a writer-epoch + CAS dance in SlateDB.

#### 3.2 Steps

1. Implement a read-only VFS first; verify with a copy of an existing
   SQLite DuckLake catalog uploaded into SlateDB page-by-page.
2. Add write support; pass SQLite's own test suite for VFS conformance.
3. Wire it into DuckDB by registering the VFS before loading the
   `sqlite` extension (via `PRAGMA` or an init script).
4. Run the standard DuckLake tutorial against it.

#### 3.3 Honest assessment

- This may be the fastest demonstration of feasibility, but it is not
  on the critical path. Limit it to 1-2 weeks.
- It is **not** the long-term answer: an LSM is a bad backing store for
  a B-tree's random-access page reads, and SQLite's locking model
  doesn't translate well to S3 latencies. Expect bad write
  amplification and large p99 latencies.
- Useful as: a conformance oracle for later strategies, and as a way to
  benchmark whether the SlateDB read path is fast enough for the
  DuckLake catalog workload at all.

**Kill criteria:** Skip or stop this phase if a read-only VFS does not work
within one week after Phase 0, if write support requires substantial SQLite
locking emulation, if p99 catalog writes exceed the latency budget by more than
10x, or if it delays the Phase 4 PG-wire sidecar.

**Deliverable:** `duckdb ducklake:slatedb://bucket/path/cat.db` working
end-to-end through the SQLite VFS path.

---

### Phase 4 — Strategy B: PostgreSQL-wire sidecar (`slateduck-pgwire`)

Goal: a focused server process that the standard DuckDB `ducklake` extension
can connect to through DuckDB's `postgres` extension, storing all catalog state
in SlateDB and supporting concurrent readers. Other DuckLake clients
(pg_ducklake, Trino, Spark, DataFusion) are compatibility targets after v1, but
each must bring its own captured SQL/protocol corpus before being claimed as
supported.

#### 4.1 Components

- [`pgwire`](https://crates.io/crates/pgwire) crate for the wire
  protocol.
- A **minimal SQL frontend** (`slateduck-sql`) that recognises *only*
  the bounded set of statements DuckLake actually issues (see the spec
  queries page). Anything else returns an error. This is *not* a
  general PG; it's a narrowly-scoped query executor.
- Statement planner → `slateduck-catalog` operations.

#### 4.2 Steps

1. Turn the Phase 0 PostgreSQL wire corpus into a query catalogue of every distinct SQL statement the `ducklake` extension emits. This catalogue must include simple query protocol, extended query protocol, `SET`/`SHOW`, `BEGIN`/`COMMIT`/`ROLLBACK`, parameter formats, result format codes, and generated inlined-table DDL/DML.
2. Implement a parser/executor that covers exactly those shapes, with AST-based matching and parameter binding.
3. Implement just enough `pg_catalog` introspection for the extension's handshake to succeed (`SELECT current_schema()`, etc.).
4. Implement text and binary encoders for the PostgreSQL type OIDs the extension requests.
5. Run DuckLake's own test suite against the sidecar.

#### 4.3 The bounded SQL subset

A key insight for Strategy B is that the `ducklake` extension does **not**
issue arbitrary SQL. It issues a bounded set of statement shapes. The exact
bounded set is defined by the Phase 0 wire corpus, not by the illustrative
examples below. The `slateduck-sql` frontend only needs to recognise and
execute corpus-backed patterns. Anything outside this set returns an error.

**Read operations (6 shapes):**

```sql
-- Current snapshot
SELECT snapshot_id FROM ducklake_snapshot
  WHERE snapshot_id = (SELECT max(snapshot_id) FROM ducklake_snapshot);

-- List schemas / tables / columns (same MVCC filter pattern)
SELECT ... FROM ducklake_{schema|table|column}
  WHERE [parent_id = ?] AND snapshot_id >= ? AND (end_snapshot IS NULL OR snapshot_id < ?);

-- Data files + delete files (one LEFT JOIN)
SELECT data.path, del.path
  FROM ducklake_data_file AS data
  LEFT JOIN ducklake_delete_file AS del USING (data_file_id)
  WHERE data.table_id = ? AND ...;

-- File pruning with column stats
SELECT data_file_id FROM ducklake_file_column_stats
  WHERE table_id = ? AND column_id = ?
    AND (? >= min_value OR min_value IS NULL)
    AND (? <= max_value OR max_value IS NULL);
```

The `LEFT JOIN` is not a general SQL join operator in SlateDuck. The catalog
layer implements it as two KV reads: scan the live `ducklake_data_file` rows
for the table, then scan or point-read `ducklake_delete_file` rows keyed by
each `data_file_id`, and merge the delete-file lists in memory before building
PG result rows.

**Write operations (~15 shapes — all `INSERT` or targeted `UPDATE`):**

```sql
INSERT INTO ducklake_snapshot (snapshot_id, snapshot_time, schema_version, ...) VALUES (...);
INSERT INTO ducklake_snapshot_changes (...) VALUES (...);
INSERT INTO ducklake_{schema|table|column} (...) VALUES (...);
INSERT INTO ducklake_{data_file|delete_file} (...) VALUES (...);
INSERT INTO ducklake_file_column_stats (...) VALUES (...);
UPDATE ducklake_table_stats SET record_count = record_count + ?, ... WHERE table_id = ?;
UPDATE ducklake_{table|column|data_file|delete_file|...}
  SET end_snapshot = ? WHERE [id_col] = ? AND end_snapshot IS NULL;
```

**Dynamic inlined-data operations:** DuckLake may issue DDL/DML against
per-table inlined data/delete tables. Inlined insert table names include table
ID and schema version; inlined delete table names include table ID. Treat these
as another bounded pattern family, not as general SQL: recognize `CREATE TABLE
ducklake_inlined_*`, `INSERT` into inlined insert/delete tables, `UPDATE
end_snapshot` on inlined insert tables, `SELECT`, and flush-related reads for
those generated table names. Map them to the `0xFD` dynamic row/delete storage
from section 5.2. Inlined delete tables do not have `end_snapshot`; they store
`file_id`, `row_id`, and deletion `begin_snapshot`.

**Handshake / introspection (~5 shapes, PostgreSQL client initialisation):**

```sql
SELECT current_schema();
SELECT version();
SHOW server_version;
-- and a small number of pg_catalog queries for type resolution
```

Notably absent outside generated inlined-table DDL: `GROUP BY`, window
functions, CTEs, subqueries beyond the one `max()` in the snapshot read, `JOIN`
beyond the one `LEFT JOIN` on file IDs, triggers, stored procedures, and
constraint declarations. This is closer to a
**statement dispatcher** than a SQL engine. Using
[`sqlparser-rs`](https://crates.io/crates/sqlparser) for parsing and then
pattern-matching on the AST to dispatch to `slateduck-catalog` methods keeps
the SQL layer bounded, but the full Strategy B server is still a meaningful
protocol project. Including PG startup, `SET`/`SHOW`, extended query protocol,
prepared statement caching, OID/type encoders, SQLSTATE mapping, and tests,
budget for a few thousand lines of Rust rather than a tiny dispatcher.

**Client compatibility rule:** The v1 support matrix is DuckDB `ducklake` via
DuckDB `postgres` only. A new client is supported only after its startup probes,
SQL shapes, transaction behavior, parameter/result format codes, and generated
inlined-table operations are captured in `tests/fixtures/wire-corpus/` and pass
the same replay tests.

#### 4.4 Concurrency

- One SlateDB writer per catalog → the sidecar serialises writes
  internally via a single-writer actor.
- Many readers → cheap, since each PG session opens a `DbReader` /
  `DbSnapshot` against a current SlateDB checkpoint.

**Deliverable:** A `slateduck serve` binary that exposes a SlateDB
catalog at a PostgreSQL TCP endpoint, with DuckDB connecting to it via
the standard `postgres` extension.

---

### Phase 5 — Strategy C: Native DuckLake catalog backend

Goal: full embedded integration with no SQL emulation layer.

#### 5.1 What needs to change

- DuckLake's catalog access in the `ducklake` extension is mediated by
  internal C++ interfaces (analogous to Iceberg's `Catalog` /
  `FileIO`). We need to either:
  - Add a new "backend" alongside `duckdb`/`sqlite`/`postgres`/`mysql`
    in the upstream extension (preferred — community contribution), or
  - Fork the extension and add a `slatedb:` URL scheme.
- Expose `slateduck-catalog` through a stable C ABI
  (`slateduck-ffi`).
- New extension code in C++ calls the FFI to satisfy DuckLake's
  catalog reads and writes directly, with no SQL in the middle.

#### 5.2 Steps

1. Read the current upstream extension source; document the catalog
   interface surface we must implement.
2. Draft an upstream RFC / GitHub discussion proposing the new backend.
3. Build the C ABI: opaque handles for `CatalogStore`, `CatalogReader`,
   `CatalogWriter`; functions for each spec operation; well-defined
   error codes.
4. Implement the C++ backend that calls into our FFI.
5. Reuse Phase 4 test suites, plus any Phase 3 conformance fixtures if
  the optional SQLite spike was built, to validate equivalence.

#### 5.3 Distribution

- Publish the extension via DuckDB's community extension repository if
  upstream adoption isn't immediate.

**Deliverable:** `INSTALL slateduck; ATTACH 'ducklake:slatedb://…' AS
lake;` Just Works in a vanilla DuckDB.

---

### Phase 6 — Operational hardening

- **Catalog visibility GC and excision.** DuckLake's `DROP TABLE` only
  marks rows with `end_snapshot`. Implement two distinct operations:
  (1) `slateduck gc` advances the `retain-from` visibility floor (no
  physical deletion); (2) `slateduck excise` physically removes catalog
  rows and Parquet files older than the floor when explicitly invoked by
  an operator. Default behavior is visibility-only; physical deletion
  requires explicit `--apply` and records an audit fact under `0xFF`.
- **Parquet data-file garbage collection.** Walk
  `ducklake_files_scheduled_for_deletion` and delete from object
  storage only when no retained snapshot references the file.
  Additionally, scan for orphaned Parquet files (never committed to any
  snapshot) and delete after the configurable grace period.
- **Checkpoints / backups.** Expose `slateduck checkpoint create` and
  `slateduck checkpoint restore` thin wrappers around SlateDB's
  `Checkpoint` API for trivial point-in-time backups of the catalog.
- **Observability.** Re-export
  [`db_stats`](https://docs.rs/slatedb/latest/slatedb/db_stats/index.html)
  and add catalog-level metrics: snapshots/sec, files/snapshot, mean
  rows scanned per `list_data_files`.
- **Encryption.** SlateDB supports
  [block transformers](https://slatedb.io/docs/design/block-transformer/);
  use one for at-rest encryption of the catalog. Parquet encryption is
  a separate (Parquet-native) concern.
- **Catalog validation and repair.** Provide `slateduck inspect`,
  `slateduck verify`, and `slateduck repair --dry-run` commands before
  beta. They should check cross-table references, snapshot continuity,
  object-store file existence, and orphaned Parquet files.
- **Documentation site.** Quickstart, architecture diagram, comparison
  vs. PG-backed and SQLite-backed DuckLake.

---

### Phase 7 — Performance work

Only meaningful once Phase 4 is stable. Re-run the same benchmarks after
Phase 5 lands, but do not wait for the native extension before tuning the
production B-first path.

- Benchmark the catalog hot paths:
  - `list_data_files(table)` at 10⁴, 10⁵, 10⁶ files.
  - `create_snapshot` at 1, 10, 100 file additions.
  - Cold-start read latency from a fresh process.
- Add **secondary indexes** in the key layout if MVCC filtering becomes
  the bottleneck (e.g. an index from `(snapshot_id, table_id)` to
  `data_file_id`).
- Consider **packing**: store all small per-table metadata (columns,
  partitions, sort info) as one composite value per table so a single
  point read pulls everything needed to plan a query.
- Tune SlateDB:
  [`Settings`](https://docs.rs/slatedb/latest/slatedb/config/struct.Settings.html)
  for block size, bloom filters, on-disk cache, etc.
- Compare end-to-end query latency on a TPC-H @ SF10 dataset against
  ducklake-on-PG and ducklake-on-SQLite baselines.

---

### Phase 8 — Stretch goals

- **Multi-writer via partitioning.** DuckLake itself supports a single
  writer per catalog; SlateDB does too. But we can offer the user a
  pattern of "one SlateDB catalog per dataset" with a thin global
  registry, taking advantage of SlateDB's cheap database creation.
- **Embed in non-DuckDB engines.** Expose `slateduck-catalog` to
  DataFusion's
  [`datafusion-ducklake`](https://github.com/datafusion-contrib/datafusion-ducklake)
  via Rust trait impl — could be easier than the DuckDB integration
  since both are Rust.
- **Streaming ingest.** Combine SlateDB's stream-processing positioning
  with DuckLake's append-only nature to offer a "Kafka → DuckLake"
  durable pipeline in one process.

---

## 5. Pre-Implementation Considerations

The following issues were identified through design analysis before any code
was written. Each one has the potential to cause significant rework if
discovered late. They are documented here so that each Phase can address
them at the right moment rather than as a surprise.

---

### 5.1 Two Separate Snapshot Systems — Keep Them Distinct

This is the single most important conceptual clarity to establish before
writing any catalog code, because conflating the two systems leads to
correctness bugs that are very hard to reproduce.

**DuckLake snapshots** are logical, user-visible commits. A DuckLake snapshot
is a row in `ducklake_snapshot` with a monotonically increasing `snapshot_id`
(e.g. `42`). Versioned catalog tables carry `begin_snapshot` and
`end_snapshot` columns. A catalog query at snapshot 42 returns versioned rows
where `begin_snapshot ≤ 42 AND (end_snapshot IS NULL OR 42 < end_snapshot)`;
non-versioned auxiliary tables follow their table-specific schema rules. This
MVCC mechanism tracks the history of the *user's data* — which tables existed,
which files belonged to them — and is implemented entirely in application
logic on top of the KV store.

**SlateDB snapshots** are physical KV read-views created by calling
`db.snapshot()`. They pin the current state of SlateDB's internal key space
so that a multi-key scan sees a consistent, non-torn picture of the raw bytes
without interference from concurrent writes. They are in-process only, not
persisted, and have no relationship to `snapshot_id` values.

The two interact like this:

```
Catalog query at DuckLake snapshot 42
  └── db.snapshot()  [SlateDB snapshot — for a consistent KV scan]
      └── scan prefix 0x0B | table_id_be
            └── deserialize each row
                 └── apply MVCC filter: begin_snapshot ≤ 42 < end_snapshot
                      └── return matching rows
```

SlateDB's snapshot ensures the scan is non-torn. DuckLake's snapshot ID is
a filter value applied to the deserialized rows. They operate at different
layers and must never be conflated.

**The trap to avoid:** It may seem elegant to align DuckLake `snapshot_id`
values with SlateDB sequence numbers. This breaks in several ways: SlateDB
sequence numbers are not stable across compaction; they increment per write
batch, not per DuckLake commit (one DuckLake snapshot may involve many
SlateDB batches); and forcing alignment hides bugs in the MVCC filter logic
by making incorrect code appear to work on a fresh database.

**Naming convention to enforce in code:** use `dl_snapshot_id` or
`catalog_version` for DuckLake snapshot IDs, and `kv_read_view` or
`kv_snapshot` for SlateDB-level read views. Never use the unqualified word
`snapshot` in variable names.

---

### 5.2 Inlined Data Tables

The DuckLake spec defines
[`ducklake_inlined_data_tables`](https://ducklake.select/docs/stable/specification/tables/ducklake_inlined_data_tables.html),
and the DuckDB extension enables data inlining by default with a row limit
of 10. Small inserts and deletes can therefore store actual row-level data
inside the catalog rather than creating Parquet data/delete files. The
`ducklake_inlined_data_tables` row is only the registry entry; DuckLake also
creates dynamic per-table inlined insert tables named by table ID and schema
version, plus inlined delete tables named by table ID.

This has three concrete implications for SlateDuck:

**Key layout.** A dedicated dynamic-storage prefix is required in the Phase 1
layout, separate from the spec table tag for `ducklake_inlined_data_tables`:

```
0xFD | 0x01 | table_id | schema_version | row_id
  → inlined insert row with begin_snapshot/end_snapshot
0xFD | 0x02 | table_id | data_file_id | row_id
  → inlined delete marker for a Parquet data-file row
```

Subtype `0x01` values contain the inlined row payload plus `begin_snapshot` and
`end_snapshot`. Deletes that target inlined insert rows are represented by
updating the row's `end_snapshot`; they must not create subtype `0x02` entries.
Subtype `0x02` values contain the deletion `begin_snapshot` only, because
DuckLake's inlined deletion table is append-only for rows in existing Parquet
data files. Do not put lifecycle fields in the dynamic key unless a later
indexed layout is explicitly added; keeping them in values preserves simple
prefix scans while matching the generated table schemas.

The row limit is configurable (`DATA_INLINING_ROW_LIMIT`, persistent
`data_inlining_row_limit`, and DuckDB's global default setting), so SlateDuck
cannot assume only 10 rows forever. Enforce a maximum encoded inlined-row value
size in SlateDuck, defaulting to 64 MiB, even though SlateDB values can be much
larger. Oversized inlined rows should return `SQLSTATE 54001` and force the
client to flush or write Parquet.

**Drop, flush, and garbage collection.** When a table with inlined data is
dropped, the generated inlined insert rows must no longer be visible to later
snapshots. If DuckDB emits `UPDATE ... SET end_snapshot` against the generated
inlined insert table, the Strategy B dispatcher applies that update directly;
the typed drop path must do the same in Strategy C. Inlined delete markers do
not have `end_snapshot`; they become eligible for physical deletion when their
target data file or parent table is outside the retained snapshot window.
Physical GC scans `0xFD | 0x01 | table_id` and `0xFD | 0x02 | table_id` and
deletes only entries whose lifecycle is no longer needed for time travel.
`ducklake_flush_inlined_data` must preserve the same semantics when it
materializes inlined inserts and delete markers into Parquet data/delete files.

**Type caveat.** For non-DuckDB metadata catalogs, DuckLake stores nested
`STRUCT`, `MAP`, and `LIST` values as strings in generated inlined tables, and
does not inline `VARIANT` columns because they do not round-trip safely through
PostgreSQL/SQLite-style string storage. SlateDuck should mirror that behavior:
accept nested values using the captured string representation, and return
`SQLSTATE 0A000` if DuckDB attempts unsupported `VARIANT` inlining through the
PG-wire path.

**Failure mode if skipped.** If inlined data is not implemented,
`CREATE TABLE t AS SELECT 1` or `INSERT INTO t VALUES (1)` can appear to
succeed and the table will appear in `LIST TABLES`, but `SELECT * FROM t`
will return zero rows with no error. This silent correctness failure is
difficult to diagnose without knowing to look for it. Test insertion inlining,
deletion inlining from Parquet files, deletion of rows that are still inlined,
schema changes over inlined tables, and `ducklake_flush_inlined_data`
explicitly in Phase 2/4.

---

### 5.3 Failure Mode: Parquet Write vs. Catalog Registration

DuckLake's write protocol is: write the Parquet file to object storage first,
then register it in the catalog. This ordering is deliberate and creates two
distinct failure modes under process crashes:

**Orphaned Parquet file (safe, handled by GC).** The process dies after the
S3 `PUT` returns successfully but before the catalog transaction or write batch
commits.
The file exists on S3 but is unknown to any catalog snapshot. Queries are
completely unaffected. The GC pass handles this by scanning for object-store
paths under the data prefix that are not referenced by any
`ducklake_data_file` row with a valid `begin_snapshot`, and deleting them
after a configurable age threshold. Default `--orphan-file-grace-period-days`
to **7 days** for v1: conservative enough for slow ingest retries,
object-store retry storms, and operator investigation, but still finite so
genuinely abandoned uploads do not grow forever. Tests may lower this threshold
explicitly; production should not use a hidden hard-coded value.

**Dangling catalog reference (dangerous, but avoidable by protocol).** A
catalog row references a Parquet file that does not exist on S3. Under
normal operation this cannot happen: S3 `PUT` is durable and fully
consistent once the HTTP 200 response is received. The protocol invariant —
register only after a successful `PUT` response — must be documented and
enforced in `CatalogWriter`.

As a safety net, add an `object_store.head(path)` existence check in the GC
verification pass for referenced data/delete file rows that are older than the
orphan-file threshold. If the object does not exist on S3, log a warning and
flag the row for operator review before removal. Missing files for retained
snapshots are not safe to auto-repair; they require restore or rebuild.

**Required implementation:** Add a `verify_data_files(table_id)` method to
`slateduck-catalog` that runs this check on demand and as part of the
periodic GC cycle.

---

### 5.4 `schema_version` Tracking Is Easy to Get Wrong

`ducklake_snapshot.schema_version` is a counter that is incremented *only*
when the schema changes and carried forward unchanged for data-only operations
(`INSERT`, `DELETE`, `UPDATE`). Schema-changing operations include at least
`CREATE SCHEMA`, `DROP SCHEMA`, `CREATE TABLE`, `DROP TABLE`, `RENAME TABLE`,
`ALTER TABLE ADD COLUMN`, `ALTER TABLE DROP COLUMN`, `ALTER TABLE RENAME
COLUMN`, `ALTER TABLE ALTER COLUMN TYPE`, `SET/DROP NOT NULL`, and changes to
partitioning, sorting, name mapping, or schema-version rows. DuckDB uses this field
to aggressively cache schema metadata: if `schema_version` is unchanged
since the last read, DuckDB skips re-fetching table and column information.

**If you always increment:** DuckDB's schema cache is effectively disabled.
Every data insert forces a full schema re-fetch, multiplying catalog read
traffic. For a workload that appends many small files rapidly this exhausts
the per-query latency budget.

**If you never increment:** DuckDB caches stale schema indefinitely. After
`ALTER TABLE ADD COLUMN`, DuckDB's cached column list does not include the
new column, causing wrong query results or crashes on schema mismatch.

**Correct implementation:** `CatalogWriter` must track whether the current
write batch contains any schema-mutating operation. Expose an explicit
`fn mark_schema_changed(&mut self)` method, called by `create_table`,
`drop_table`, `create_schema`, and similar operations. The
`create_snapshot` commit path checks this flag: if set, increment
`schema_version`; if not, copy it from the previous snapshot. This flag
cannot be inferred post-hoc from the key set of the `WriteBatch` — it must
be set explicitly and tested with its own test case. The Phase 0 wire-capture
corpus should record which DuckLake SQL statements mutate schema state so the
dispatcher and typed API stay in lockstep.

**Test requirement:** Add a schema-version matrix test before Phase 4: every
schema-mutating operation listed above must increment `schema_version`, while
data-only `INSERT`, `DELETE`, `UPDATE`, and inlined-data flush operations must
preserve it unless the captured DuckDB SQL proves otherwise.

---

### 5.5 Consider GlueSQL for the Strategy B SQL Layer

Before implementing `slateduck-sql` from scratch, evaluate
[GlueSQL](https://github.com/gluesql/gluesql) — a Rust-native SQL engine
with a **pluggable storage backend** designed for exactly this use case.

GlueSQL defines two traits: `Store` (read-only access: `scan`,
`fetch_schema`, etc.) and `StoreMut` (write access: `insert_data`,
`delete_data`, etc.). Implementing these traits against `CatalogStore`
provides a complete SQL execution layer at no additional cost:

```
DuckDB postgres extension
  └── pgwire crate (PostgreSQL wire protocol)
       └── GlueSQL (SQL parsing + execution)
            └── Store / StoreMut trait implementations
                 └── slateduck-catalog (typed Rust API)
                      └── SlateDB (key-value store)
```

The DuckLake catalog query set — `SELECT` with `WHERE` and `ORDER BY`,
`INSERT`, `UPDATE`, one `LEFT JOIN`, and scalar `max()` — is well within
GlueSQL's scope. Adopting it could save several weeks of SQL frontend work
and concentrate engineering on the SlateDB integration layer.

**Risk:** GlueSQL's SQL dialect differs from PostgreSQL's in some areas.
The `ducklake` extension probes the backend with PostgreSQL-specific queries
during the connection handshake (see section 5.11) that GlueSQL may not
respond to correctly without shims.

Define a "PostgreSQL-specific shim" before the spike starts: one shim is one
named handler or translation function that detects a PostgreSQL-specific query,
message, type/OID behavior, or result-shape mismatch that GlueSQL does not
handle natively. Multiple call sites using the same handler count once; a
catch-all that hides unrelated behavior does not count as one shim.

**Recommended approach:** Run a spike in Phase 0. Connect the DuckDB client
to a minimal GlueSQL-backed pgwire server and verify the full handshake
completes and the first `CREATE TABLE` / `INSERT` / `SELECT` round-trip
succeeds. If fewer than ten probe responses require PostgreSQL-specific
emulation, adopt GlueSQL. If many require deep dialect work, build the
minimal custom dispatcher instead.

---

### 5.6 Testing Strategy

Three test layers are needed. Define tooling and create fixture
infrastructure in Phase 0 before writing production code — tests written
alongside code catch more bugs than tests written after the fact.

#### Layer 1 — Property tests on the key layout

Use [`proptest`](https://crates.io/crates/proptest) to verify:

- **Round-trip:** `decode(encode(row)) == row` for every row type across
  all 28 catalog tables.
- **Key ordering:** `encode(id=5) < encode(id=6)` for all numeric ID fields
  in all table prefixes.
- **Prefix isolation:** a `scan_prefix(0x0B | table_id)` returns exactly
  the data files for that table and no rows from any other table.
- **No key collisions** between different table tags with any valid input.

This layer is cheap to write (a few hundred lines) and provides the highest
confidence in the encoding layer that everything above depends on.

#### Layer 2 — Spec conformance via DuckDB CLI (golden tests)

1. Run the full DuckLake tutorial against a SQLite-backed DuckLake (the
   reference implementation). Capture the output of every DuckDB statement
   as a golden fixture file.
2. Replay the identical sequence against SlateDuck.
3. Diff the outputs byte-for-byte.

Automate this in CI for every supported DuckDB version. Store fixtures in
`tests/golden/duckdb-{version}/`. Also capture the full PostgreSQL wire
traffic between the `ducklake` extension and a real PostgreSQL backend
(using `pgwire` tracing or packet capture) as a corpus for Strategy B
handshake tests.

Add a basic `slateduck verify catalog` command during Phase 2, not Phase 6.
At this layer it only needs to check primary-key uniqueness, foreign-key
references, MVCC interval consistency, and counter monotonicity. The later
Phase 6 tooling can add object-store checks and repair planning.

Also add a Phase 2 benchmark baseline in `benchmarks/phase-2-baseline.json`:
p50/p95/p99/p99.9 latency for `get_current_snapshot`, `list_data_files` at
10K files, `describe_table` with 100 columns, `prune_files` on one typed
column, and `create_snapshot` with 100 file additions on LocalFS and MinIO.
Phase 7 optimizations must compare against this baseline rather than against
anecdotal performance.

#### Layer 3 — Chaos tests (crash safety)

Use SlateDB's [`fail_parallel`](https://docs.rs/fail-parallel) fault
injection framework — the same one used in SlateDB's own test suite — to
inject panics at specific points during catalog writes. After each crash,
verify: (1) the catalog reopens without error; (2) all rows are internally
consistent; (3) a new writer can take over and proceed.

Minimum required crash injection points:

| Crash point | Expected outcome |
| --- | --- |
| After S3 Parquet PUT, before catalog commit (`DbTransaction::commit_with_options` or `db.write(batch)`) | Orphaned file on S3; catalog unchanged; GC cleans file |
| During `create_snapshot` batch assembly, after snapshot row is staged but before data file rows are staged | No catalog mutation is visible because the batch was never committed |
| During `create_snapshot` commit | Either the entire snapshot is visible or none of it is; never a snapshot row without its required metadata |
| During `drop_table` batch assembly, after the first tombstone is staged but before the last tombstone is staged | No tombstone is visible because the batch was never committed |
| During `drop_table` commit | Either all tombstones are visible or none are; partial table drops are a correctness failure |
| During writer fencing (old writer receives fencing error) | New writer takes over; all pre-fence commits remain visible |
| Two processes concurrently initialize a fresh catalog | Exactly one coherent initial `ducklake_metadata` key/value set and counter set is committed; both clients converge on the same metadata |
| PG session disconnects between `BEGIN` and `COMMIT` | Pending in-memory batch is dropped; no partial catalog mutation or session memory leak remains |

Without Layer 3, the durability and fencing guarantees in Section 9 are
aspirational rather than verified.

---

### 5.7 Object Store Backend Compatibility

SlateDB supports multiple backends through the
[`object_store`](https://docs.rs/object_store) crate. SlateDuck inherits
this, but not all backends behave identically in production.

| Backend | Fencing / conditional-write expectation | Typical round-trip | Recommended use |
| --- | --- | --- | --- |
| `LocalFileSystem` | Expected via local atomicity; not valid for NFS/EFS | < 1 ms | **Primary development and testing** |
| `InMemory` | Expected, but tests only | ~0 | Unit tests only |
| MinIO (S3-compatible) | Must verify against configured compatibility mode | < 1 ms (local) | Integration tests, on-prem |
| AWS S3 Standard | Must verify with SlateDB's manifest/fencing path | 10–50 ms | General cloud |
| AWS S3 Express One Zone | Must verify with SlateDB's manifest/fencing path | 1–10 ms | Interactive / low-latency |
| Google Cloud Storage | Must verify through `object_store` backend behavior | 10–30 ms | GCP deployments |
| Azure Blob Storage | Must verify through `object_store` backend behavior | 10–50 ms | Azure deployments |

**Strong recommendation: develop entirely on `LocalFileSystem` first.**
SlateDB's backend abstraction means switching to S3 requires zero code
changes in SlateDuck. Local FS eliminates all S3 latency from the
development loop, avoids API costs, and makes debugging straightforward —
catalog files are visible as ordinary files on disk and can be inspected
with standard tools (`xxd`, a custom `slateduck inspect` CLI, etc.).

Graduation path: `LocalFileSystem` (development) → MinIO (CI integration
tests) → S3 Standard (acceptance and correctness testing) → S3 Express One
Zone (Phase 7 performance benchmarking).

Scope v1 acceptance to LocalFS, MinIO, S3 Standard, and S3 Express. GCS and
Azure are design-supported through `object_store`, but they should remain
"validated on demand" until CI credentials and backend-specific fencing tests
exist.

**CAS note:** The writer epoch fencing mechanism relies on compare-and-swap
semantics as exposed through SlateDB's manifest and fencing implementation. Do
not assume the raw `object_store` backend exposes identical conditional-write
primitives everywhere. Phase 0 must verify writer fencing, manifest updates,
and conditional initialization on every v1 target backend (LocalFS, MinIO, S3
Standard, S3 Express). `LocalFileSystem` uses local filesystem atomicity as a
substitute — sufficient for single-host development but not a guarantee for
network filesystems (NFS, EFS).

---

### 5.8 Data Type Serialization for Column Statistics

DuckLake stores `min_value` and `max_value` in `ducklake_file_column_stats`
and `ducklake_table_column_stats` **as plain strings**, regardless of the
column's actual data type. This simplifies the catalog schema but creates
a correctness requirement in the file pruning logic that is easy to miss.

The pruning query filters files using comparisons such as:
```
(query_value >= min_value OR min_value IS NULL) AND
(query_value <= max_value OR max_value IS NULL)
```

These comparisons must be **type-aware**, not lexicographic. Cases where
naive string comparison produces silently wrong results:

- **Integers:** `'9' > '10'` lexicographically but `9 < 10` numerically.
  A file with rows in range 10–99 would be incorrectly pruned for a query
  `WHERE col = 15`.
- **Negative numbers:** `'-10' > '-9'` lexicographically but
  `-10 < -9` numerically — a prunable file would be incorrectly retained.
- **Scientific notation floats:** `'1e10'` sorts before `'9'`
  lexicographically (because `'1' < '9'`) but `1e10 >> 9` numerically.
- **Timestamps with timezones:** `+02:00` suffix vs. `Z` suffix breaks
  lexicographic ordering even for ISO 8601 strings.

The spec acknowledges this explicitly: *"The minimum and maximum values
for each column are stored as strings and need to be cast for correct
range filters on numeric columns."*

**Required implementation:** The `prune_files()` method in
`slateduck-catalog` must accept a `DuckLakeType` argument alongside the
comparison value and perform type-aware casting before comparison. Write a
`parse_stats_value(raw: &str, col_type: DuckLakeType) -> Comparable`
helper and fuzz it against the full set of DuckLake data types.

Minimum parsing matrix for Phase 2:

| DuckLake type family | Stats encoding | Comparison rule |
| --- | --- | --- |
| `boolean` | `0` / `1` | Parse to bool/integer |
| signed/unsigned ints through `uint64` | Integer string | Parse with checked signed/unsigned width |
| `float32`, `float64` | Numeric string plus `inf` / `-inf` | IEEE numeric compare; ignore NaN min/max and use `contains_nan` separately |
| `decimal(P,S)` | Scale-independent numeric string | Parse to decimal/rational, not float |
| `date`, `time`, `timestamp*`, `timestamptz` | ISO strings plus infinities | Parse to typed temporal values; normalize time zones before compare |
| `varchar`, `json` | As-is string | Compare with DuckDB-compatible collation assumptions, initially binary/UTF-8 |
| `blob` | Hex string | Decode bytes before lexicographic byte compare |
| `uuid` | Canonical UUID string | Parse UUID bytes and compare canonical representation |
| `int128`, `uint128`, `timetz`, `interval` | No min/max stats in DuckLake v1.0 | Never prune by min/max |
| `list`, `struct`, `map` | Stats live on child columns | Recurse through child `ducklake_column` rows |
| `geometry`, `variant` | JSON `extra_stats` | Use specialized handlers; do not treat `min_value`/`max_value` as strings |

If DuckLake adds min/max stats for a type not listed here, SlateDuck must fail
closed: skip pruning for that column or return `SQLSTATE 0A000` for an explicit
unsupported pruning request. It must never compare unknown typed stats as raw
strings just to keep planning moving.

Pruning bugs here cause queries to silently omit correct rows — they do not
produce errors and are not detectable from the result set alone.

---

### 5.9 Writer Takeover Window

When a new writer takes over from a crashed or stale writer, there is a
brief but safety-critical gap between fencing and the new writer's first
successful commit:

1. SlateDB fences the old writer. The old writer's future `WriteBatch`
   commits fail with a fencing error.
2. The new writer opens `Db::builder(...).build().await`. SlateDB replays
   the WAL from the last manifest checkpoint during opening.
3. The replay finishes. The new writer's in-memory state is consistent with
   the last durable state of the old writer.
4. The new writer begins accepting client requests.

The risk in step 3–4: if the old writer had written data to the WAL that
had not yet been flushed to an SST and reflected in the manifest, a
`DbReader` opened immediately after the new writer starts (but before it
calls `flush()`) may or may not replay those WAL entries, depending on its
`skip_wal_replay` setting. This creates a window where read-your-writes
(guarantee G8) is violated.

**Safe takeover protocol — required in the writer startup sequence:**

```rust
let db = Db::builder(path, object_store).build().await?;
db.flush().await?;  // force durable reader-visible state after takeover
catalog_store.publish_writer_endpoint(my_address).await?;
// now safe to accept client connections
```

The exact SlateDB flush mode matters: with WAL enabled, public docs describe
`flush()` as a WAL flush, while `DbReader` examples show `flush()` being
required before a fresh reader observes writes. Phase 0 must verify whether
`flush()` alone is sufficient for fresh `DbReader` visibility on every target
backend or whether takeover should call an explicit memtable/manifest flush.
The extra object-store write is acceptable for a takeover event that occurs at
most once per writer lifetime. Skipping the visibility barrier violates G8 for
the window between takeover and the next organic flush.

**Required test:** An integration test that (1) kills the writer
mid-commit, (2) starts a new writer, (3) opens a `DbReader` immediately
after the new writer's `flush()`, and (4) verifies all pre-crash catalog
commits are visible to the reader.

---

### 5.10 Multi-Tenancy and Path Layout

Multiple independent DuckLake instances sharing the same S3 bucket is a
common deployment pattern. SlateDB handles this natively — each `Db` is
isolated to a path prefix — but SlateDuck must standardize the path layout
before any code writes paths, to prevent ad-hoc string construction that
becomes inconsistent across deployments.

**Recommended bucket layout:**

```
s3://my-bucket/
├── catalogs/
│   ├── warehouse-a/          ← SlateDB database for catalog A
│   │   ├── manifest/
│   │   ├── wal/
│   │   └── compacted/
│   └── warehouse-b/          ← SlateDB database for catalog B
└── data/
    ├── warehouse-a/          ← Parquet files for lakehouse A
    └── warehouse-b/          ← Parquet files for lakehouse B
```

The `ducklake_metadata` table is a scoped key/value configuration table; the
global `data_path` entry defines the root data path, and file/schema/table rows
separately carry `path_is_relative`. Do not hard-code a universal relative-path
rule before the wire corpus proves how DuckDB initializes and reads `data_path`
through the PostgreSQL catalog path. Phase 0 must capture the exact `ATTACH`/metadata
sequence for absolute data paths, relative data paths, and S3 URLs.

Use this v1 resolution model unless the Phase 0 capture disproves it:

- `CatalogPath` stores `object_store_root`, `catalog_prefix`, `data_prefix`,
  and `data_path_mode` (`Absolute` or `RelativeToDataPrefix`).
- Internally, SlateDuck may store a relative `data_path` only when it is
  unambiguously relative to the configured `data_prefix`, not to the catalog
  prefix.
- Preserve DuckLake's path hierarchy: `ducklake_schema.path` is relative to
  global `ducklake_metadata.data_path` when its `path_is_relative` is true;
  `ducklake_table.path` is relative to the schema path; and
  `ducklake_data_file.path` / `ducklake_delete_file.path` are relative to the
  table path. `ducklake_files_scheduled_for_deletion.path` follows the spec's
  file-cleanup semantics and may be relative to the global data path.
- If DuckDB requires an absolute `data_path` through the PG-wire path, store
  absolute URIs for compatibility and provide a later migration/rewrite tool
  for bucket failover instead of relying on ambiguous `../..` paths.

**Connection URL conventions (decide before Phase 4/5):**

```
# Strategy B (PG-wire sidecar)
ducklake:postgres:host=slateduck-writer catalog=warehouse-a

# Strategy C (native extension)
ducklake:slatedb:s3://my-bucket/catalogs/warehouse-a
```

**Implementation note:** Define the `CatalogPath` struct in `slateduck-core`
before any path strings appear in the codebase. It encapsulates the
object-store base URL, catalog prefix, data prefix, and data-path mode as
separate typed fields, and provides methods for constructing SlateDB database
paths, `ducklake_metadata.data_path` values, and resolved Parquet object paths.
Never use raw string concatenation for object-store paths.

---

### 5.11 DuckDB Extension Version Probing

The `ducklake` DuckDB extension is not passive — it actively probes the
catalog backend during the connection handshake before any DuckLake
operations begin. For Strategy B (PG-wire sidecar), the sidecar must
respond correctly to PostgreSQL introspection queries or the extension
refuses to proceed.

Known probe queries captured from DuckDB v1.5.2 against a
PostgreSQL-backed DuckLake:

```sql
SHOW server_version;                          -- must resemble "14.x" or higher
SELECT current_schema();                      -- return "public"
SELECT current_database();                   -- return catalog name
SELECT version();                            -- PostgreSQL version string
SELECT oid, typname FROM pg_catalog.pg_type  -- type OID resolution
  WHERE typname IN ('bool', 'int4', 'text', ...);
SELECT nspname FROM pg_catalog.pg_namespace WHERE nspname = 'public';
```

The exact set changes between DuckDB releases — new probes can be added
when the extension is updated. A Strategy B sidecar that works with DuckDB
1.5.2 may silently break with DuckDB 1.6.x if new probes are added.

**Mitigation plan:**

1. In Phase 0, run DuckDB against a real PostgreSQL DuckLake and capture
   the full connection handshake using `tcpdump`, Wireshark, or the
   `pgwire` crate's built-in tracing. Record every query and response.
2. Store the capture as
   `tests/fixtures/handshake/duckdb-{version}.jsonl` (one JSON object per
   query/response pair).
3. The Strategy B handshake handler must pass a replay test against this
   fixture before any DuckLake-specific operations are exercised.
4. When a new DuckDB release is published, re-capture the handshake and
   add a new fixture file. If the fixture differs from the previous version,
   update the handshake handler before merging the DuckDB version bump.

This makes DuckDB version compatibility a first-class concern tracked in
the test suite rather than something discovered at runtime by users
updating their DuckDB installation.

---

### 5.12 Monotonic ID Generation

DuckLake has several ID domains, and they are not all the same. The snapshot
ID is globally increasing (`max(snapshot_id) + 1`). The snapshot row stores
`next_catalog_id` for catalog objects such as schemas, tables, and views, and
`next_file_id` for data/delete files. Some child objects, such as columns, are
unique only within their parent table over that table's lifetime. SlateDuck
must mirror these spec domains rather than inventing one global counter per
table.

Duplicate IDs or reused IDs do not fail loudly — they cause silently wrong
query results, where rows appear in the wrong snapshot's view or file
registrations shadow each other.

**Counter storage.** In a single-writer system, counters are stored as
dedicated keys under `0xFE`, one key per counter domain:

```
0xFE | 0x01  →  u64 next_snapshot_id
0xFE | 0x02  →  u64 next_catalog_id
0xFE | 0x03  →  u64 next_file_id
0xFE | 0x10 | table_id  →  u64 next_column_id_for_table
```

Only add additional counters after confirming the target DuckLake table's
schema and ID semantics. For example, `ducklake_column.column_id` is table
scoped, so it should not consume from `next_catalog_id` unless the upstream
spec changes.

Strategy B must handle two ID sources. The typed Rust/FFI path can allocate IDs
inside `CatalogWriter`. The DuckDB PG-wire path may instead send explicit IDs
in `INSERT` statements after reading `next_catalog_id` / `next_file_id` from the
previous `ducklake_snapshot`. In that path, SlateDuck must validate the supplied
IDs against the persisted counters and the new snapshot row's advertised next
IDs, then advance the `0xFE` counters in the same SlateDB transaction. A
DuckDB-supplied duplicate or regressing ID is a constraint error, not an
overwrite.

Each ID allocation must commit atomically with the row that consumes the ID.
Because a plain `WriteBatch` cannot read the current counter value, use a
SlateDB transaction for counter-backed writes, or have the single-writer actor
read the counter first and then submit one final atomic batch. The transaction
form is the default because it also supports conflict detection during tests:

```rust
// CORRECT — counter read, counter increment, and row write commit atomically
let txn = db.begin(IsolationLevel::SerializableSnapshot).await?;
let next = decode_u64(txn.get(COUNTER_KEY_CATALOG).await?.expect("initialized"));
txn.put(COUNTER_KEY_CATALOG, encode_u64(next + 1))?;
txn.put(encode_table_key(next), encode_table_row(...))?;
txn.commit().await?;

// WRONG — crash between these two calls creates a permanently
// inconsistent state if the counter write succeeded but the row write did not
let table_id = catalog.allocate_catalog_id().await?; // writes counter
db.put(encode_table_key(table_id), ...).await?;     // separate write
```

**Crash safety.** If a crash occurs after the transaction commit succeeds,
both the counter increment and the row are durable. If the crash occurs before
the commit, neither is written — the same ID will be reallocated on the next
attempt. Gaps in the ID space are permitted by the spec and must be tolerated
by all query code.

**Recovery.** On writer startup, read all counters from SlateDB before
accepting any client requests. If a counter key does not exist (fresh
catalog), initialize it to 1.

**Monotonicity invariant test.** Add a proptest that runs N random
catalog operations in sequence and asserts that every allocated ID is
strictly greater than the previous one, across crashes (simulated by
reopening `Db`).

---

### 5.13 Value Encoding Versioning

Every serialized value stored in SlateDB must carry a **1-byte encoding
version** followed by a 4-byte magic marker before the serialized payload:

```
┌──────────────┬──────────────────┬──────────────────────┐
│ encoding: u8 │ magic: b"SDKV"   │ payload: Protobuf     │
│   0x01       │ 53 44 4B 56      │ <row data>            │
└──────────────┴──────────────────┴──────────────────────┘
```

Without the version byte, there is no safe migration path if the encoding
format needs to change — adding a field, renaming a field, switching encoding
families, or changing a numeric type. Without the magic marker, a missing
version byte, wrong table decoder, or corrupted value can be misinterpreted as
valid Protobuf. The only alternative without a versioned header is a full
catalog migration in a single atomic operation, which may not be feasible at
scale and cannot be done incrementally.

**Cost:** five bytes per value. For a catalog with millions of file
registration rows this is negligible compared with row payloads and S3 object
metadata overhead.

**Migration model with versioning:**
- A new writer that understands both version `0x01` and `0x02` can read
  either, write `0x02`, and migrate lazily on read or in a background
  compaction sweep.
- Old readers that only understand `0x01` return an error on encountering
  a `0x02` value, signalling that an upgrade is needed — rather than
  silently misinterpreting the bytes.

**Implementation rule:** The `encode(row)` function always writes the current
version byte and `b"SDKV"` magic first. The `decode(bytes)` function verifies
the magic before dispatching to the version-specific decoder. Add tests that
(1) encode with version `N`, change the schema to version `N+1`, and verify
that the `N+1` decoder correctly reads both old and new formats; and (2)
corrupt the magic bytes and verify decoding fails loudly with a catalog
corruption error.

**Keep this fixed from Phase 1 onward.** Retrofitting this after encoding
functions are in use across multiple crates requires touching every call site.

---

### 5.14 Time Travel and Retention Policy

DuckLake supports time travel: `SELECT * FROM my_table AT (SNAPSHOT 42)`.
Because catalog-data facts (section 1.4) are never physically deleted by
default, every committed fact remains addressable indefinitely. Time travel
works for *any* historical `dl_snapshot_id` by construction, not by virtue of
a retention policy that happens not to have purged the relevant rows yet.

This changes the role of the "GC" pass compared with a mutable design.
There are two distinct operations:

1. **Retention advancement (default, safe).** A `retain-from` value is the
   *query-visibility floor*: snapshots below it may not be requested through
   the normal client API. Advancing `retain-from` is a transactional update of
   the `0xFF | "retain-from"` key (section 1.4, infrastructure state).
   No bytes are deleted from object storage.
2. **Excision (rare, audited).** Physical deletion of historical facts. This
   exists only for legal/compliance erasure or for operators who explicitly opt
   into a bounded storage footprint, and it is invoked through the dedicated
   `slateduck excise` command described in section 1.4 and section 5.28. It is
   *not* part of the normal write path or the normal GC sweep.

#### What gets physically deleted vs. logically retracted

A DuckLake `UPDATE` appends a new row with a higher `begin_snapshot`. A
`DROP TABLE` sets `end_snapshot` on every affected versioned row (the single
terminal update described in section 1.4). **Neither operation deletes any
SlateDB key.** Physical deletion only happens via `slateduck excise` and only
outside the retain-from window.

The excision tool may delete a key when:
```
fact.end_snapshot IS NOT NULL
  AND fact.end_snapshot <= oldest_retained_snapshot
  AND operator explicitly invokes `slateduck excise --apply`
```

This rule applies to ordinary catalog facts and inlined insert facts. Inlined
delete markers do not have `end_snapshot`; their excision eligibility is
derived from the target data file and table lifecycle as described in section
5.2. The default `slateduck gc` command never runs the excision step.

#### Oldest retained snapshot tracking

`retain-from` is stored as a single key under the `0xFF` system prefix and is
updated by a transactional write (safe because there is exactly one writer):

```
0xFF | "retain-from"  →  u64 oldest_retained_snapshot_id
```

This value is updated by two paths:
1. **Automatic TTL (opt-in).** When `--retention-days N` is configured, a
   background task scans `ducklake_snapshot` for snapshots whose
   `snapshot_time` is older than the configured retention period and advances
   `retain-from` to the oldest still-within-window snapshot. With the default
   infinite retention, this task is a no-op.
2. **Explicit operator pin.** `catalog.pin_snapshot(id)` prevents the TTL task
   from advancing `retain-from` past a given snapshot ID (useful for long-
   running reports, disaster-recovery checkpoints, etc.).

#### Parquet file retention

Parquet files referenced by retained snapshots must not be deleted from object
storage. Only `slateduck excise --apply` may delete Parquet files, and only
when no in-window snapshot references them.

#### Default retention

The v1 default retention is **infinite** in both dimensions: the query-
visibility floor (`retain-from`) never advances unless the operator passes
`--retention-days N`, and `slateduck gc` never deletes bytes. This means time
travel works for any historical `dl_snapshot_id` by default, matching the
commitment in section 1.4. Operators may opt into bounded visibility by
configuring `--retention-days`, and may opt into bounded storage by running
`slateduck excise --before <snapshot> --apply` on a schedule with a recorded
retention policy. A separate `--excise-days` flag, off by default, controls
automatic physical excision scheduling. Tests may set both explicitly.

#### Execution mode

Implement the visibility and excision tools as separate CLI verbs. The CLI
path (`slateduck gc plan` / `slateduck gc apply` for visibility,
`slateduck excise plan` / `slateduck excise apply` for physical deletion) is
required before beta and is the default operational path. The sidecar may
also run the *visibility* advancement task as an optional background process
behind `--enable-gc` and `--gc-interval-minutes`; it must never run excision
in the background. Excision always requires a foreground operator invocation
with `--apply`. If object-store deletion fails inside an excision run, record
the failure and skip the file/key for that pass; do not retry aggressively
inside any request path.

---

### 5.15 Error Taxonomy: SlateDB Errors → SQLSTATE

For Strategy B, DuckDB interprets every response from the catalog backend
as a PostgreSQL-protocol message including a `SQLSTATE` code. If all
errors map to `XX000` (internal error), DuckDB: (a) shows the user an
unhelpful generic message; (b) cannot distinguish retriable errors (e.g.,
transient S3 throttle) from fatal errors (e.g., catalog corruption); (c)
cannot automatically reconnect after a writer takeover.

Define the translation function in `slateduck-pgwire` before writing any
handler code. Every Rust `?` in the handler should flow through a single
`to_pg_error(err: SlateDuckError) -> PgErrorResponse` function, not
scattered inline.

**Required mapping table (minimum viable):**

| SlateDuck / SlateDB error | SQLSTATE | Severity | DuckDB behavior |
| --- | --- | --- | --- |
| Writer fenced (`EpochMismatch`) | `57P04` admin_shutdown | FATAL | Client closes connection and reconnects |
| Snapshot not found (time travel out of retention) | `22023` invalid_parameter_value | ERROR | Surfaces to user with message |
| Object store timeout / throttle | `08006` connection_failure | ERROR | Client may retry |
| Requested row not found | `02000` no_data | — | Empty result set (not an error) |
| Value decode error (version mismatch) | `22P02` invalid_text_representation | ERROR | Surfaces to user; indicates upgrade needed |
| Value header magic mismatch / retained-row corruption | `XX001` data_corrupted | ERROR | Surfaces to user; repair refuses mutation |
| ID counter write failure | `40001` serialization_failure | ERROR | Client may retry (serializable conflict) |
| Duplicate supplied ID / primary-key collision | `23505` unique_violation | ERROR | Surfaces as constraint failure |
| Write sent to read-only replica | `25006` read_only_sql_transaction | ERROR | libpq can retry another host with `target_session_attrs=read-write` |
| Unsupported table/feature/binary format | `0A000` feature_not_supported | ERROR | Surfaces to user with actionable message |
| Object-store permission denied | `42501` insufficient_privilege | ERROR | Surfaces IAM/configuration problem |
| Catalog schema not initialized | `3D000` invalid_catalog_name | FATAL | Client surfaces to user |
| Unexpected internal error | `XX000` internal_error | ERROR | Generic fallback |

The fencing case (`57P04`) is the most important: it tells the client and any
routing layer that the current connection is no longer attached to the active
writer and must be torn down before retrying. Without a distinct fatal code,
DuckDB can remain attached to a stale writer after failover and require a manual
`DETACH` / `ATTACH`.

Add a test for each code path: simulate the relevant SlateDB or
object-store error, verify the PG wire response carries the correct
5-character SQLSTATE, and verify DuckDB reacts as expected.

---

### 5.16 Strategy C Async–Sync Bridge

The `ducklake` DuckDB extension is C++. DuckDB's extension API is
synchronous — catalog methods are called on DuckDB's query execution
thread and must return a result before execution continues. SlateDB is
built entirely on Tokio and all of its operations are `async fn`.

This is a hard interface mismatch that must be resolved before any
Strategy C work begins, because the choice affects both the performance
characteristics and the implementation architecture of the FFI layer.

#### Option 1 — Blocking Tokio runtime (simplest, viable for Strategy C v1)

The FFI layer owns a `tokio::runtime::Runtime` created once at
initialization. Each catalog call does:

```rust
#[no_mangle]
pub extern "C" fn slateduck_get_snapshot(catalog: *mut Catalog, id: u64) -> ... {
    let catalog = unsafe { &*catalog };
    catalog.runtime.block_on(async {
        catalog.store.get_snapshot(id).await
    })
}
```

`block_on` spins the Tokio reactor on the calling thread until the future
completes. This is correct and safe. The downside is that every catalog
operation blocks a DuckDB execution thread for the full duration of the
S3 round-trip — typically 10–50 ms on S3 Standard. If DuckDB executes
multiple catalog lookups in parallel (e.g., during a multi-table join),
this serialises them at the thread boundary.

Mitigation: use a multi-threaded Tokio runtime and spawn the async
work onto it with `runtime.block_on`, which still blocks the calling
thread but allows the Tokio workers to execute other S3 futures
concurrently with tasks spawned by other threads.

#### Option 2 — Callback-based async FFI (higher performance, requires DuckDB API support)

The C++ extension provides a completion callback; the Rust FFI side
spawns a Tokio task and calls the callback when the S3 operation
completes. This avoids blocking any DuckDB thread, but requires DuckDB's
extension API to support async catalog operations.

**Verify before scheduling:** check whether DuckDB 1.5+ has an async
catalog interface in its extension API. As of early 2026 this has not
been confirmed. If the API does not exist, Option 2 requires an upstream
DuckDB change — which may or may not be accepted and would not land
quickly.

#### Option 3 — Shared Tokio runtime via thread-local (experimental)

If DuckDB allows extensions to run initialization code at extension load
time, the SlateDuck extension can start a background thread running a
Tokio runtime and expose a channel-based interface. Catalog calls push a
request onto the channel, block the calling thread on a `std::sync::mpsc`
receiver, and the Tokio worker processes the request asynchronously. This
decouples the Tokio runtime from DuckDB's thread pool but adds a
channel-crossing overhead per catalog call (~1–5 µs, negligible compared
to S3 latency).

#### Recommended path

Implement Strategy C v1 with **Option 1** (blocking runtime). It is
correct, straightforward to implement, and the per-call overhead is
dominated by S3 latency rather than the block_on overhead. Profile under
realistic workloads before investing in a more complex option. Schedule
Strategy C only after Strategy B is production-stable, and revisit the
async model at that point using real benchmark data.

**Action item for Phase 0:** Check the DuckDB extension API for async
catalog interfaces and record the finding in this document before
committing to Strategy C timelines.

---

### 5.17 UPDATE-Heavy Workload and LSM Behavior

DuckLake's most frequent write operation is `UPDATE ... SET end_snapshot = ?`.
In an LSM tree, an update is a new SST append; the old version is masked and
eventually discarded during compaction. This is the workload LSMs are built
for, and the catalog-data immutability principle (section 1.4) aligns well with
it: each version row occupies a unique key (via `begin_snapshot` in the key),
and the single terminal `end_snapshot` update is an in-place value update at
that key. No read-amplification from dead duplicate keys arises.

What does warrant monitoring:

- **Historical-version amplification.** Even with one physical key per
  logical version, MVCC scans traverse all versions in a prefix and filter by
  `begin_snapshot`/`end_snapshot`. For a table with many `ALTER TABLE`
  operations, a `describe_table` scan touches all historical column versions.
  Phase 2 benchmarks must record versions-scanned vs. live-rows-returned for
  `list_data_files`, `describe_table`, and file pruning. If the amplification
  exceeds 10× on the reference workload before beta, add a secondary index or
  pack per-table metadata into a single composite value (Phase 7).

- **Compaction tuning.** The `end_snapshot` update generates one dead SST
  entry per retired version. Tune `l0_sst_count_threshold` to trigger
  compaction earlier for update-heavy workloads so dead entries are merged
  quickly. Aggressive levelled compaction is the right mode for a catalog with
  frequent schema changes.

- **Physical deletes in excision.** When `slateduck excise` runs, it issues
  SlateDB `delete(key)` calls for keys past the retention floor. These
  tombstones propagate downward through compaction normally, reclaiming
  storage. Without excision, retired versions accumulate indefinitely; this is
  by design (default infinite retention) and acceptable for most deployments.

**Key layout note:** `end_snapshot` lives in the value of the version key, not
in a separate retraction key. Putting it in the key would enable efficient
range scans over live-only rows but would require inserting a new key on every
drop/alter and would double the key-space size for versioned tables. The
value-encoded design is the decision for v1; future contributors should add a
secondary index if MVCC filter overhead is proven to dominate, not change the
primary key shape.

---

### 5.18 PostgreSQL Extended Query Protocol

DuckDB's `postgres` extension uses libpq, which by default uses the **extended
query protocol** rather than the simple query protocol. The extended protocol
is a multi-step exchange:

```
Client → Parse      (SQL text with $1/$2 placeholders, optional statement name)
Client → Bind       (parameter values, result format codes)
Client → Describe   (optional — request RowDescription before Execute)
Client → Execute
Client → Sync
Server → ParseComplete
Server → BindComplete
Server → RowDescription
Server → DataRow (× N)
Server → CommandComplete
Server → ReadyForQuery
```

The simple protocol sends SQL as one `Query` message with inline literals. In
the extended protocol, **parameter values arrive separately in the `Bind`
message**, not inline in the SQL text. The sidecar's statement classifier
(`slateduck-sql`) will receive SQL containing `$1`, `$2`, etc. rather than
literal values, and must substitute parameter values at execution time.

**This affects the pattern matcher in `slateduck-sql`.** A classifier that
matches `WHERE table_id = 42` will fail against `WHERE table_id = $1`. The
classifier must either:
1. Normalize parameter placeholders before matching (replace `$N` with a
   wildcard), or
2. Match structural AST patterns (via `sqlparser-rs`) rather than string
   patterns, and substitute `$N` values from the bound parameters at dispatch
   time.

Option 2 is the correct approach and is required anyway for correct execution.

**Prepared statement caching.** libpq sends the same prepared statement
(identified by name or by an anonymous unnamed statement) across many `Bind`
/ `Execute` cycles. The sidecar should cache the parsed + classified AST for
each named statement to avoid re-parsing on every execution.

**Action item for Phase 0:** Capture full extended-protocol wire traffic from
DuckDB connecting to a real PostgreSQL DuckLake. Verify that the `pgwire` crate
correctly handles `Parse`/`Bind`/`Execute`/`Sync` sequences before any catalog
logic is written. This is infrastructure, not catalog logic — find out early if
the crate has gaps.

---

### 5.19 Catalog Initialization Race

The `ducklake_metadata` table is a scoped key/value table, not a single-row
catalog record. Catalog creation writes a coherent initial set of global rows
(`version`, `created_by`, `data_path`, data-inlining defaults, etc.) plus
SlateDuck counters and system keys. If two DuckDB clients simultaneously attach
to the same previously-uninitialised path, both may attempt to write that
initial metadata set.

The safe initialization protocol is conceptually:

```rust
pub async fn open_or_create(db: &Db, config: CatalogConfig) -> Result<CatalogStore> {
    // Step 1: check if already initialized (fast path)
    if let Some(meta) = db.get(METADATA_GLOBAL_VERSION_KEY).await? {
        return Ok(CatalogStore::from_existing(meta));
    }

    // Step 2: use SlateDB's transaction/conditional-write API to make the
    // metadata set fail if another process initialized first.
    let txn = db.begin(IsolationLevel::SerializableSnapshot).await?;
    if txn.get(METADATA_GLOBAL_VERSION_KEY).await?.is_some() {
        let meta = txn.get(METADATA_GLOBAL_VERSION_KEY).await?.expect("checked above");
        return Ok(CatalogStore::from_existing(meta));
    }

    for row in initial_metadata_rows(&config) {
        txn.put(encode_metadata_key(&row), encode_metadata_row(&row))?;
    }
    txn.put(COUNTER_KEY_SNAPSHOT, encode_u64(1))?;
    // ... other initial keys ...
    txn.commit().await?;
    Ok(CatalogStore::new(config))
}
```

This is pseudocode. The exact method names must be replaced with the current
SlateDB transaction API during Phase 0. Do not invent a helper such as
`write_if_absent` unless it is actually implemented and tested.

Without a CAS-gated initialization, two concurrent first-connections can each
write different `ducklake_metadata` rows (different `data_path`, different
partition defaults), leaving the catalog in an inconsistent state with no error
visible to either client.

This is distinct from the writer fencing mechanism (section 5.9), which handles
writer *takeover* after a crash. Initialization is a one-time, catalog-lifetime
event, and must be explicitly tested: launch two processes simultaneously
against a fresh catalog path and verify exactly one coherent initial metadata
set and counter set is committed and both processes proceed without error.

---

### 5.20 DuckLake Catalog Table Coverage Matrix

The DuckLake v1.0 spec defines 28 catalog tables in the stable table overview
and the full schema creation script linked from that page. The first version
of this plan used several non-spec names (`ducklake_partition_spec`,
`ducklake_partition_field`, `ducklake_snapshot_tag`) and omitted several real
tables (`ducklake_macro*`, `ducklake_column_mapping`,
`ducklake_name_mapping`, `ducklake_file_variant_stats`,
`ducklake_schema_versions`, `ducklake_column_tag`, sort metadata). Fixing the
coverage matrix before implementation is mandatory.

DuckLake-generated inlined data/delete tables are not part of the fixed 28-table
schema; SlateDuck handles them under the reserved dynamic `0xFD` prefix from
sections 1.1 and 5.2.

The full schema script is the source of truth for key shapes, not only the
per-table prose pages. Several tables (`ducklake_table`, `ducklake_column`,
`ducklake_view`, `ducklake_macro`, `ducklake_partition_info`,
`ducklake_sort_info`, `ducklake_tag`, and `ducklake_column_tag`) have
`begin_snapshot`/`end_snapshot` but no SQL primary key because they store
multiple historical versions of the same logical object. Their SlateDB keys
must include `begin_snapshot`; otherwise an ALTER/rename/drop sequence can
overwrite the old version and break time travel.

Deciding the coverage matrix before Phase 1 serves two purposes:

1. **Tag prefix allocation.** Allocate a 1-byte tag prefix for every spec
  table in Phase 1, even tables not implemented until Phase 6. Prefixes are
  cheap; retrofitting the layout later requires migrating existing data.

2. **Scope management.** If the sidecar receives an `INSERT` into a table it
  does not recognize, it must return a clear error rather than silently
  discarding the row. Knowing which tables are intentionally deferred vs.
  which are bugs in the extension makes triage faster.

**Spec-accurate coverage by phase:**

| Tag | Table | Tier |
| --- | --- | --- |
| `0x01` | `ducklake_metadata` | Phase 4 core |
| `0x02` | `ducklake_snapshot` | Phase 4 core |
| `0x03` | `ducklake_snapshot_changes` | Phase 4 core |
| `0x04` | `ducklake_schema` | Phase 4 core |
| `0x05` | `ducklake_table` | Phase 4 core |
| `0x06` | `ducklake_column` | Phase 4 core |
| `0x07` | `ducklake_view` | Phase 6 optional features |
| `0x08` | `ducklake_macro` | Phase 6 optional features |
| `0x09` | `ducklake_macro_impl` | Phase 6 optional features |
| `0x0A` | `ducklake_macro_parameters` | Phase 6 optional features |
| `0x0B` | `ducklake_data_file` | Phase 4 core |
| `0x0C` | `ducklake_delete_file` | Phase 4 core |
| `0x0D` | `ducklake_files_scheduled_for_deletion` | Phase 6 maintenance |
| `0x0E` | `ducklake_inlined_data_tables` | Phase 4 must-not-skip |
| `0x0F` | `ducklake_column_mapping` | Phase 4 schema/mapping |
| `0x10` | `ducklake_name_mapping` | Phase 4 schema/mapping |
| `0x11` | `ducklake_table_stats` | Phase 4 core |
| `0x12` | `ducklake_table_column_stats` | Phase 4 core |
| `0x13` | `ducklake_file_column_stats` | Phase 4 core |
| `0x14` | `ducklake_file_variant_stats` | Phase 6 optional features |
| `0x15` | `ducklake_partition_info` | Phase 4 partition/sort |
| `0x16` | `ducklake_partition_column` | Phase 4 partition/sort |
| `0x17` | `ducklake_file_partition_value` | Phase 4 partition/sort |
| `0x18` | `ducklake_sort_info` | Phase 4 partition/sort |
| `0x19` | `ducklake_sort_expression` | Phase 4 partition/sort |
| `0x1A` | `ducklake_tag` | Phase 6 optional features |
| `0x1B` | `ducklake_column_tag` | Phase 6 optional features |
| `0x1C` | `ducklake_schema_versions` | Phase 4 schema/mapping |

| Tier | Tables | First needed |
| --- | --- | --- |
| **Phase 4 core** | `ducklake_metadata`, `ducklake_snapshot`, `ducklake_snapshot_changes`, `ducklake_schema`, `ducklake_table`, `ducklake_column`, `ducklake_data_file`, `ducklake_delete_file`, `ducklake_table_stats`, `ducklake_table_column_stats`, `ducklake_file_column_stats` | Basic `CREATE TABLE`, `INSERT`, `SELECT`, file pruning |
| **Phase 4 must-not-skip** | `ducklake_inlined_data_tables` | Small tables and CTAS smoke tests |
| **Phase 4 schema/mapping** | `ducklake_column_mapping`, `ducklake_name_mapping`, `ducklake_schema_versions` | Schema evolution and stable column identity |
| **Phase 4 partition/sort** | `ducklake_partition_info`, `ducklake_partition_column`, `ducklake_file_partition_value`, `ducklake_sort_info`, `ducklake_sort_expression` | Partitioned/sorted tables and realistic datasets |
| **Phase 6 maintenance** | `ducklake_files_scheduled_for_deletion` | File cleanup after merge-on-read `DELETE` / `UPDATE` |
| **Phase 6 optional features** | `ducklake_view`, `ducklake_macro`, `ducklake_macro_impl`, `ducklake_macro_parameters`, `ducklake_tag`, `ducklake_column_tag`, `ducklake_file_variant_stats` | Views, macros, tags, column tags, variant stats |

There is no `ducklake_encryption_key` table in the v1.0 table overview. Do
not allocate a tag for non-spec tables unless the full schema script confirms
one exists in the target DuckLake version.

Allocate all 28 spec tag bytes in Phase 1 (28 tables fit in one byte with room
to spare); the concrete allocation is in section 1.1 and must be mirrored in
`tags.rs`. Return `SQLSTATE 0A000` (feature not supported) for inserts into
Phase 6+ tables when they are first encountered in Phase 4, so the error is
explicit and diagnosable rather than silent data loss.

**Action item for Phase 1:** Before writing any `encode_key` function, produce
a single file `crates/slateduck-core/src/tags.rs` that lists all 28 tables
with their assigned tag byte, key shape, versioning rule, required unique-guard
keys, and implementation status (`Live`, `Deferred(phase)`, `Unimplemented`).
Every subsequent PR that adds a new table implementation must update this file
as part of the review checklist.

---

### 5.21 BEGIN / COMMIT / ROLLBACK Transaction Semantics

DuckDB's libpq wraps some operations in explicit `BEGIN`/`COMMIT` blocks.
Whether it does so for DuckLake catalog operations — which are themselves
atomic by design — needs to be verified by capturing wire traffic in Phase 0.
The answer determines the sidecar's internal write architecture.

**If DuckDB sends explicit transactions:** The sidecar must buffer all
`INSERT`/`UPDATE` statements as a pending transaction plan and only commit it
to SlateDB when `COMMIT` arrives. `ROLLBACK` discards the plan. If the plan is
write-only it may lower to a `WriteBatch`; if it allocates counters, checks key
absence, or otherwise depends on reads, it must execute as a `DbTransaction`.
The sidecar cannot call `db.write(batch)` on each statement individually — it
must accumulate the logical catalog transaction across multiple messages in a
single session.

**If DuckDB auto-commits each catalog statement:** The sidecar can commit each
`INSERT`/`UPDATE` immediately as a single-statement SlateDB transaction or
`WriteBatch`, depending on whether the statement needs read-before-write
semantics.

The two cases require different internal session-state management. Assume the
worst (explicit transactions) until wire captures confirm otherwise. The
buffered-batch model is strictly safer:

```rust
struct Session {
  pending_txn: Option<PendingCatalogTxn>,  // Some inside BEGIN
  snapshot_id: Option<u64>,               // snapshot being built, if any
}

fn handle_begin(&mut self) {
  self.pending_txn = Some(PendingCatalogTxn::new());
}

async fn handle_commit(&mut self, db: &Db) -> Result<()> {
  if let Some(pending) = self.pending_txn.take() {
    pending.commit(db).await?;
    catalog_visibility_barrier(db).await?;  // read-your-writes guarantee (§5.9)
  }
  Ok(())
}

fn handle_rollback(&mut self) {
  self.pending_txn = None;  // discard; nothing was written to SlateDB
}

fn handle_disconnect(&mut self) {
  if self.pending_txn.take().is_some() {
    warn!("dropping uncommitted DuckLake catalog transaction on disconnect");
  }
}
```

`catalog_visibility_barrier` is a placeholder for the flush/manifest operation
described in section 5.9 and validated in section 5.26.

**Memory bound for a pending batch:** A single DuckLake snapshot commit can
produce many catalog rows (snapshot + schema/table/column rows + potentially
thousands of `ducklake_data_file` rows for bulk ingest). Cap the pending batch
at a configurable size (default: 64 MiB) and return `SQLSTATE 54001`
(program limit exceeded) if exceeded, requiring the client to split the
operation. On connection close or protocol error, drop the `Session` struct and
its pending transaction buffers before returning the connection slot to the
pool; `--max-sessions` is the outer guard against aggregate memory growth.

**Action item for Phase 0:** Capture wire traffic of DuckDB connecting to a
real PostgreSQL DuckLake and record whether `BEGIN`/`COMMIT` appears. Store
the capture in `tests/fixtures/handshake/` alongside the version probing
fixtures from section 5.11.

---

### 5.22 PostgreSQL `SET` Statements and Session Configuration

DuckDB sends a burst of session configuration `SET` statements immediately
after connecting, before any DuckLake-specific SQL:

```sql
SET client_encoding = 'UTF8';
SET DateStyle = 'ISO, MDY';
SET timezone = 'UTC';
SET extra_float_digits = 3;
SET search_path = '"$user", public';
```

If the sidecar returns an error on any of these, DuckDB aborts the connection
before any catalog work begins. The sidecar must accept all `SET` statements
silently — returning `CommandComplete` with tag `SET` — regardless of whether
it uses the setting internally.

This is distinct from the handshake probe `SELECT` queries in section 5.11.
Those are read queries that need meaningful responses. `SET` commands only
need a success acknowledgement.

**Implementation:** A catch-all `SET` handler that logs the setting name and
value at `DEBUG` level and returns success. A small allow-list of
semantically meaningful settings to actually store in session state:

| Setting | Used for |
| --- | --- |
| `timezone` | Normalize timestamps to UTC before storing |
| `client_encoding` | Validate it is `UTF8`; reject others with `SQLSTATE 22021` |
| `DateStyle` | Parse date literals from `Bind` parameter values |

All other settings: accept and ignore.

**Also handle `SHOW` commands.** DuckDB interleaves `SHOW` queries with `SET`
commands during startup (`SHOW DateStyle`, `SHOW server_version`,
`SHOW transaction_isolation`). Return plausible hardcoded values — see the
capture list in section 5.11.

**Test:** Write a test that connects a plain `psql` client to the sidecar and
completes the startup handshake without error before any DuckDB-specific tests
are written. `psql` sends a similar (but smaller) set of configuration
commands and is easy to script in CI.

---

### 5.23 `ducklake_file_column_stats` Key Layout for Per-Column Pruning

`ducklake_file_column_stats` is the largest catalog table in a typical
lakehouse and has a unique access pattern that differs from every other
DuckLake table. It requires an explicitly designed key layout before Phase 1
— not a generic one derived from the other tables.

**Access pattern.** File pruning for a predicate `WHERE col_a = 42` requires
scanning all rows for `(table_id, column_id=col_a)` across every data file to
find files that can be skipped:

```sql
SELECT data_file_id FROM ducklake_file_column_stats
  WHERE table_id = ?
    AND column_id = ?
    AND (? >= min_value OR min_value IS NULL)
    AND (? <= max_value OR max_value IS NULL);
```

**Problem with naive key layout.** If the key is
`0x13 | table_id | file_id | column_id`, a pruning scan requires reading the
entire `0x13 | table_id` prefix and filtering by `column_id` in memory. For
a table with 1 million files and 100 columns, this reads 100 million rows to
find the 1 million rows for the target column — a 100× amplification factor.

**Correct key layout.** Arrange keys so all stats for a given
`(table_id, column_id)` pair are contiguous:

```
key: 0x13 | table_id_be (4B) | column_id_be (4B) | file_id_be (8B)
```

A prefix scan on `0x13 | table_id | column_id` returns exactly the stats for
that column across all files in one sequential read. Filtering by
`min_value`/`max_value` happens in memory on the returned rows.

**Scale note.** For 1 million files × 100 columns:
- Rows: 100 million entries
- Key size: 17 bytes each → ~1.7 GB of keys
- Value size: ~50 bytes each (min, max, null count) → ~5 GB of values
- Total: ~7 GB of column stats for one large table

At this scale, the per-column scan (`scan_prefix(0x13 | table_id | column_id)`)
returns 1 million rows × ~67 bytes = ~67 MB per pruning query. This is
acceptable for a batch query but too slow for interactive filtering. A bloom
filter is not the right index for min/max range pruning; Phase 7 should instead
evaluate a coarse zone-map / interval index, such as
`(table_id, column_id, stats_bucket, data_file_id)`, where `stats_bucket`
groups typed min/max ranges for approximate pruning before reading the full
stats rows.

**Why this table uses a query-first key.** The full DuckLake schema does not
declare a SQL primary key for `ducklake_file_column_stats`, but its row identity
is effectively `(data_file_id, column_id)` and the dominant query filters on
`(table_id, column_id)`. SlateDuck therefore keys by the dominant pruning scan
and must enforce duplicate `(data_file_id, column_id)` stats rows in the typed
writer/dispatcher. Document this explicitly in `tags.rs` (section 5.20) so the
design intent is clear during code review.

---

### 5.24 Data Plane vs. Catalog Plane Responsibilities

Strategy B can be misunderstood as "the sidecar writes the lakehouse." It does
not. The sidecar is a PostgreSQL-compatible **catalog service**. The DuckLake
client still writes and reads Parquet data files through DuckDB's DuckLake
extension and the configured object-store filesystem. The exception is
DuckLake data inlining: small inserts/deletes may be represented as generated
catalog tables, which SlateDuck stores under the `0xFD` catalog prefix rather
than in Parquet.

The practical write path is:

```
DuckDB + ducklake extension
  ├── writes Parquet data/delete files directly to object storage when not inlined
  └── sends catalog SQL over PostgreSQL wire protocol to SlateDuck
          └── SlateDuck writes catalog metadata to SlateDB/object storage
```

This creates two separate credential planes:

| Actor | Needs access to | Minimum permissions |
| --- | --- | --- |
| DuckDB client / ingestion job | Data prefix (`s3://bucket/data/...`) | read/write/delete Parquet and delete files |
| SlateDuck sidecar | Catalog prefix (`s3://bucket/catalogs/...`) | read/write SlateDB WAL, SSTs, manifests, checkpoints |
| GC / maintenance job | Both catalog and data prefixes | read catalog, delete unreferenced data files |

Do not make the sidecar proxy Parquet bytes unless a future product decision
explicitly requires it. Proxying the data plane would destroy the main value of
DuckLake: engines can read and write Parquet directly.

**Implementation requirement:** `CatalogPath` must distinguish
`catalog_prefix` from `data_prefix` and carry the data-path mode chosen in
Phase 0. Documentation must show that clients need data-plane credentials while
the sidecar needs catalog-plane credentials. Test this with two restricted IAM
roles: one role that can mutate only the catalog prefix, and one role that can
mutate only the data prefix.

---

### 5.25 PostgreSQL Wire Type Formats and OIDs

The PG-wire sidecar is not only a SQL string dispatcher. PostgreSQL clients
also negotiate type OIDs, parameter formats, and result formats. DuckDB may ask
for text results in one path and binary parameters/results in another. If the
sidecar ignores format codes, basic queries may work while prepared statements
or newer DuckDB versions fail mysteriously.

Minimum type surface to implement in Phase 4:

| PostgreSQL type | OID | Needed for |
| --- | --- | --- |
| `bool` | 16 | flags such as `path_is_relative`, nullability |
| `bytea` | 17 | opaque serialized fields if exposed through SQL |
| `int2` / `int4` / `int8` | 21 / 23 / 20 | IDs, counts, sizes, schema versions |
| `float4` / `float8` | 700 / 701 | statistics values if client requests floats |
| `text` / `varchar` | 25 / 1043 | names, paths, types, JSON-ish fields |
| `timestamp` / `timestamptz` | 1114 / 1184 | snapshot timestamps |
| `uuid` | 2950 | schema/table UUIDs |
| `json` / `jsonb` | 114 / 3802 | snapshot change metadata if surfaced as JSON |

For every supported OID, implement text encoders/decoders in v1. Implement
binary encoders/decoders for any format codes observed in the DuckDB capture;
for unimplemented binary requests, return `SQLSTATE 0A000` before execution
rather than guessing. Result rows must also include correct NULL handling and
column metadata in `RowDescription`.

**Test requirement:** The handshake fixture from section 5.11 must capture
format codes, not only SQL text. Add round-trip tests for every text-format OID
above using both a DuckDB client and `psql` prepared statements, plus explicit
tests that unsupported binary requests fail with `0A000`.

---

### 5.26 SlateDB API Validation Gates

Several sections intentionally use pseudocode. Before implementation starts,
convert these assumptions into an explicit Phase 0 API validation checklist:

| Assumption in this plan | Validation task |
| --- | --- |
| Atomic multi-key writes use `WriteBatch` | Write a batch, crash/reopen, verify all-or-none visibility |
| Conditional initialization is possible | Verify `DbTransaction` or another API can implement "insert if absent" for `ducklake_metadata` |
| Counter allocation is serializable | Run two `DbTransaction`s that read and update the same counter under `SerializableSnapshot`; verify one wins cleanly, the loser gets a retryable conflict, and no ID is reused after crash/reopen |
| Concurrent initialization converges | Launch two processes calling `open_or_create` on a fresh catalog; verify exactly one coherent initial metadata key/value set and one initialized counter set exist, and both clients read the same metadata |
| Durable commit options are available | Use `DbTransaction::commit_with_options` or the current `Db` write-options API with `await_durable` or the current equivalent, crash after success, and verify committed rows survive |
| `db.flush()` gives `DbReader` visibility | Write with `Db`, call `flush()`, open fresh `DbReader`, verify the key is visible on LocalFS, MinIO, S3 Standard, and S3 Express |
| Visibility-barrier latency is acceptable | Measure p50/p95/p99 for the verified barrier on LocalFS, MinIO, S3 Standard, and S3 Express |
| Writer fencing returns a distinguishable error | Force two writers, capture exact error kind, map it in `to_pg_error()` |
| `WriteBatch` has unlimited logical size | Enforce SlateDuck's own batch byte limit before calling SlateDB |
| Prefix scans use fully merged latest values | Verify `scan_prefix` deduplicates older LSM versions; use `scan_prefix_by_recency` only for specialized freshness probes |

Do not start Phase 1 data-model work until these validations are green. If any
assumption is false, update this plan before writing production code. This is
especially important for conditional initialization: the public `Db` docs do
not show a `write_if_absent` helper, so the implementation must use the real
transaction API rather than a made-up convenience method.

The Phase 0 report should include the exact SlateDB crate version, the code
used for each validation, the observed error kinds, and backend-specific notes.
If any validation requires a workaround, patch this plan before opening the
Phase 1 key-layout PR.

---

### 5.27 Object-Store Rate Limits, Backpressure, and Compaction Scheduling

SlateDuck's operational profile is dominated by object-store requests: WAL
writes, manifest reads, SST reads, cache fills, compaction reads/writes, and
Parquet file verification. A single writer is not enough to protect S3/GCS/Azure
from bursts: one large snapshot can register thousands of files and emit a very
large `WriteBatch`.

Add explicit backpressure before beta:

- Limit concurrent object-store operations per process (`--object-store-max-inflight`, default 100 until benchmarks say otherwise).
- Limit PG sessions and active catalog queries (`--max-sessions`,
  default 50, and `--max-active-scans`, default 25).
- Reject oversized transactions before they enter SlateDB (`SQLSTATE 54001`).
- Return a retriable error for object-store throttling (`SQLSTATE 08006`) and
  include retry-after metadata in the error message where possible.
- Schedule compaction and GC so they do not compete with foreground catalog
  commits during ingest bursts.

Metrics must expose object-store request count, bytes read/written, throttles,
retry count, compaction backlog, WAL backlog, and per-query scanned key count.
Without these metrics, production latency problems will look like random DuckDB
slowness rather than a saturated catalog backend.

---

### 5.28 Catalog Validation and Repair Tooling

Because SlateDuck stores a relational catalog in a KV database, operators need
tools that understand both layers. A generic SlateDB dump is not enough to
diagnose DuckLake-level problems.

Build these CLI commands before the first real deployment. The basic
`slateduck verify catalog` subset is a Phase 2 deliverable; the data-file
verification and repair commands can arrive during Phase 6 hardening.

| Command | Purpose |
| --- | --- |
| `slateduck inspect snapshot --latest` | Print current snapshot, schema version, counters, file counts |
| `slateduck verify catalog` | Check primary-key uniqueness, foreign keys, MVCC intervals, counter monotonicity |
| `slateduck verify data-files` | `HEAD` every referenced Parquet/delete file, optionally sample footers |
| `slateduck gc plan` | Show what catalog rows and data files would be deleted under current retention |
| `slateduck repair --dry-run` | Propose repairs for orphaned files, dangling references, or counter drift without mutating state |

The repair command must be conservative: it should emit a plan first and require
an explicit `--apply` flag for mutation. Repair operations themselves must be
written as normal DuckLake snapshots where possible so that the repair is
auditable through the same history mechanism as ordinary writes.

Classify corruption before attempting repair:

- **Repairable:** orphaned dynamic inlined keys, stale counters, dangling rows
  outside the retained snapshot window, or missing optional stats rows. The
  repair plan may delete orphaned keys, recompute counters from live rows, or
  regenerate stats.
- **Unrecoverable without restore:** value-header magic mismatch, Protobuf
  decode failure for a retained row, missing `ducklake_snapshot` rows, missing
  `ducklake_metadata`, or data-file references for retained snapshots whose
  object-store files are gone. `repair --dry-run` must refuse mutation in these
  cases and direct the operator to restore the latest good SlateDB checkpoint or
  rebuild a new catalog from Parquet metadata where possible.

---

## 6. Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
| --- | --- | --- | --- |
| DuckDB extension API turns out to be too closed to add a new catalog backend cleanly. | Medium | High | Fall back permanently to Strategy B (PG-wire). It is a fine product for DuckDB once the captured DuckDB corpus passes; other clients need separate compatibility work. |
| SlateDB read latency for many small point lookups is too high for interactive query planning. | Medium | Medium | Phase 7 packing + aggressive on-disk caching + DuckDB-side caching of catalog snapshots. |
| DuckLake spec churn between 1.0 and 1.1. | Low | Low | The spec is versioned and stable as of v1.0; carry DuckLake's `schema_version` in snapshot rows and SlateDuck's `encoding_version` in serialized values. |
| Single-writer constraint surprises users. | Medium | Low | Document loudly; SlateDB's fencing means at worst the new writer takes over safely. |
| The SQLite-VFS-over-LSM approach (Phase 3) is too slow even for a demo. | Medium | Low | Phase 3 is optional and time-boxed. Skip to Phase 4 if it does not produce useful evidence quickly. |
| SlateDB API assumptions differ from the current crate. | Medium | High | Phase 0 validation gates in section 5.26; no production code based on pseudocode APIs. |
| PG-wire type/format handling is incomplete. | Medium | High | Implement OID + text/binary format matrix in section 5.25 before DuckLake tests. |
| Generated inlined-table SQL differs from the assumed pattern family. | Medium | High | Phase 0 wire corpus must include inlined insert, inlined delete, schema-change, and flush paths before Phase 4 dispatch code is written. |
| `ducklake_metadata.data_path` resolution differs across DuckDB/catalog modes. | Medium | High | Treat path mode as a Phase 0 captured decision and centralize all path construction in `CatalogPath`. |
| Object-store throttling dominates p99 latency. | Medium | Medium | Backpressure, compaction scheduling, and metrics in section 5.27. |
| Catalog/data IAM boundaries are unclear. | Medium | High | Separate data-plane and catalog-plane credentials as described in section 5.24. |

---

## 7. Success Criteria

1. The full DuckLake tutorial runs end-to-end from the standard DuckDB `ducklake` extension through the SlateDuck PG-wire sidecar, with the catalog stored in SlateDB/S3 and no PostgreSQL or SQLite catalog database.
2. Concurrent reads from a second DuckDB process see consistent, snapshot-isolated views of the catalog.
3. A kill -9 on the writer mid-commit leaves the catalog readable and internally consistent; the next writer takes over via fencing.
4. Benchmarks publish p50/p95/p99 catalog operation latency against PostgreSQL-backed DuckLake on RDS and SQLite-backed DuckLake.
5. If common S3 Express planning operations exceed 3× PostgreSQL p99 latency, Phase 7 indexing/caching work is required before claiming production readiness; correctness milestones may still ship as alpha/beta.
6. All 28 DuckLake v1.0 catalog tables have assigned tag bytes, fixture coverage, and explicit implementation status.
7. Phase 0 validation gates for SlateDB transactions, reader visibility, writer fencing, and PG-wire format codes pass on LocalFS, MinIO, S3 Standard, and S3 Express.
8. Writer failover from fence detection to a ready new writer completes within 30 seconds on S3 Standard and within 10 seconds on S3 Express in acceptance tests.
9. IAM separation is tested with one role that can mutate only the catalog prefix and another that can mutate only the data prefix; expected failures must return clear SQLSTATEs instead of partial writes.
10. The implementation-readiness artifacts in section 11 are checked in before Phase 1 data-model work starts.

---

## 8. Performance: SlateDuck vs. DuckLake-on-PostgreSQL

This section gives an honest assessment of where SlateDuck will be faster,
slower, and on-par with the most common production DuckLake deployment today:
DuckLake backed by PostgreSQL (e.g. AWS RDS in the same region).

### Where PostgreSQL wins

**Catalog read latency.** PostgreSQL's query planner can use composite indexes
(e.g. `(table_id, snapshot_id)` on `ducklake_data_file`) to satisfy a typical
spec query in a single index seek. In the same AZ, round-trip latency for a
warm query is **sub-millisecond**. S3 can never match that.

**Application-level filtering.** DuckLake's MVCC filter
(`begin_snapshot ≤ snapshot_id < end_snapshot`) runs inside PostgreSQL, close
to the data. In SlateDuck it runs in application code after SlateDB returns raw
rows — this adds deserialization and branching overhead proportional to the
number of historical (dead) rows in a prefix scan.

**Concurrent readers.** PostgreSQL's MVCC and connection pooling handle
many concurrent readers elegantly with no application-layer coordination.
SlateDB's `DbReader` / `DbSnapshot` are also multi-reader friendly, but each
reader adds object-store GET load.

**Predictability.** PostgreSQL has decades of optimization for exactly this
workload. Variance between p50 and p99 is low. SlateDB performance at p99
is more sensitive to compaction, S3 request queuing, and cold-cache reads.

### Where SlateDuck wins (or ties)

**Operational simplicity and cost.** No separate database instance to run,
monitor, patch, or back up. The catalog lives in the same S3 bucket as the
Parquet data. At modest scale, this is worth accepting some latency overhead.

**Write-heavy ingest.** LSM trees excel at append-heavy workloads.
SlateDB batches WAL writes and compacts asynchronously; for workloads
that register thousands of new data files per minute, it can outperform
PostgreSQL's synchronous B-tree insert path.

**Embedded single-process scenarios (Strategy C).** A Lambda function or
stream processor embedding SlateDuck avoids the network hop to a PostgreSQL
server entirely. Catalog reads from in-process MemTable and on-disk cache can
reach PostgreSQL-competitive latency.

**Point-in-time catalog snapshots.** SlateDB's `Checkpoint` API makes
capturing and restoring an exact catalog state trivially cheap — useful for
development, testing, and disaster recovery.

**Multi-region / high-availability.** SlateDB inherits S3's durability and
can trivially operate against multi-region buckets. Running a highly-available
PostgreSQL cluster adds significant operational overhead by comparison.

### Estimated latency comparison

The table below uses rough estimates based on typical cloud latencies (May 2026).
All figures assume the writer is in the same region as the object-store bucket.
PostgreSQL rows assume a managed RDS instance (e.g. `db.t4g.medium`) in the
same AZ as the application.

| Catalog operation | PostgreSQL (same AZ) | SlateDuck + S3 Standard | SlateDuck + S3 Express One Zone |
| --- | --- | --- | --- |
| `get_current_snapshot()` (1 point read) | ~0.5 ms | ~20–50 ms | ~1–5 ms |
| `list_data_files` (10 K files, scan + filter) | ~5–10 ms | ~50–200 ms | ~10–30 ms |
| `create_snapshot` (atomic write + counters) | ~2–5 ms | ~50–100 ms | ~5–20 ms |
| Concurrent reader throughput | ~1 K reads/sec | ~50 reads/sec | ~200–500 reads/sec |

Key variables that move these numbers significantly:
- **On-disk cache hit rate**: SlateDB supports a local block cache (SSD). A
  warm cache can bring `list_data_files` latency close to PostgreSQL levels.
- **Catalog size**: DuckDB caches snapshot IDs client-side, so the expensive
  scan happens less often for long-lived DuckDB processes.
- **S3 tier**: S3 Express One Zone is the only realistic path to interactive
  DuckLake query planning from a remote client; Standard is better suited to
  background / batch workloads.

### Where Phase 7 work aims to close the gap

The performance roadmap in Phase 7 targets the three largest sources of
overhead:

1. **Application-level filtering** — Add secondary index keys (e.g.
   `(snapshot_id, table_id) → data_file_id` skip-index) so that MVCC scans
   avoid touching dead rows at all.
2. **Multiple small reads** — Pack all per-table metadata (columns,
   partitions, sort info) into a single composite value so that planning a
   query requires at most two point reads: one for the snapshot, one for the
   table bundle.
3. **Cold-start** — Persist the current snapshot ID and per-table file count
   in a hot key so a cold DuckDB process can resume work in a single GET,
   mirroring PostgreSQL's warm connection pool advantage.

With these optimisations in place the target is to be **within 2–3× of
PostgreSQL** on common DuckLake planning queries (see the benchmark target in
section 7). For the long tail of large-catalog workloads with many data files
this is harder to guarantee without local disk caching, which partially
undermines the "zero-disk" positioning.

### The honest bottom line

**For interactive data-warehouse workloads with a persistent query engine
and a managed PostgreSQL instance already in the stack, PostgreSQL-backed
DuckLake is faster and simpler.** Do not choose SlateDuck for raw speed.

**Choose SlateDuck when:**
- You are serverless or spot-based and cannot afford a persistent database.
- You want a "lakehouse in a bucket" with no external dependencies.
- You need cheap, reliable point-in-time catalog snapshots.
- You are already in the SlateDB ecosystem (stream processing, durable
  execution) and want DuckLake to live alongside your existing state.
- Your workload is write-heavy (many small appends) rather than read-heavy
  (many concurrent analysts running short queries).

---

## 9. Correctness Guarantees

The DuckLake specification requires a catalog database that supports
**transactions and primary-key constraints as defined by SQL-92.** This
section analyses each implied guarantee individually and evaluates whether
SlateDB can satisfy it.

### Guarantee analysis

| # | Guarantee | What it means | SlateDuck verdict |
| --- | --- | --- | --- |
| G1 | **Atomic transactions** | All writes in a snapshot succeed together or not at all | ✅ SlateDB `DbTransaction` / `WriteBatch` commit atomically once Phase 0 validates the exact API path |
| G2 | **Durability** | Once `create_snapshot` returns the data survives a crash | ✅ Requires durable write options (`await_durable=true` or equivalent) verified in Phase 0 |
| G3 | **Reader snapshot isolation** | A reader at snapshot N sees a stable view; writer at N+1 doesn't disturb it | ✅ `DbSnapshot` / `DbReader` with checkpoint |
| G4 | **Monotonically increasing IDs** | `snapshot_id = max + 1` is always safe; no duplicate IDs | ✅ Stronger than PG: single-writer eliminates any race |
| G5 | **Cross-table atomic update** | `DROP TABLE` updates 7+ row groups atomically | ✅ All updates go in one SlateDB transaction or atomic write batch |
| G6 | **Check-and-set on end_snapshot** | `UPDATE … WHERE end_snapshot IS NULL` cannot be lost | ✅ No concurrent writers; the conditional is always safe |
| G7 | **Primary/effective uniqueness** | Duplicate `snapshot_id`, `table_id`, version identity, etc. are rejected | ⚠️ Not enforced at storage layer — correct by discipline plus guard keys |
| G8 | **Read-your-writes** | After commit returns, new readers see the new snapshot | ⚠️ Requires explicit `flush()` after each snapshot commit |

### Notes on the two caveats

**G7 — Primary/effective uniqueness.**
SlateDB is a KV store; it has no concept of a primary key. However,
because there is only one writer and all IDs are allocated from a
monotonically-incrementing counter committed in the same SlateDB transaction as
the new row, duplicate internally allocated IDs are impossible in correct code.
That is still a software invariant, not a storage invariant. Some DuckLake
tables also intentionally lack SQL primary keys because they store historical
versions with the same logical ID and different `begin_snapshot` values. For
tables with declared or effective uniqueness that is not enforced by the hot
scan key, SlateDuck must write a unique-guard key in the same transaction.
Safeguards: validate key/guard absence in release builds for any externally
supplied ID or SQL-side insert; keep `debug_assert!` checks around internally
allocated IDs; and rely on integration tests against the DuckDB extension,
which would surface duplicate IDs as a visible anomaly. A duplicate primary key
or duplicate effective row identity must return a PostgreSQL-compatible
constraint error, not silently overwrite the existing value.

**G8 — Read-your-writes lag.**
SlateDB's `DbReader` polls for manifest updates at a configurable
interval (`manifest_poll_interval`). A freshly-committed snapshot may
not be visible to a new reader immediately. The fix is one line:
call the verified SlateDB visibility barrier after every `create_snapshot`.
This is expected to be `db.flush()`, but Phase 0 must prove whether the default
WAL flush is sufficient for fresh `DbReader` visibility or whether SlateDuck
must use an explicit memtable/manifest flush mode.
Estimated cost of the flush: 50–200 ms on S3 Standard, which is
acceptable for catalog operations that already pay a similar S3 latency
budget.

### Where SlateDuck is strictly stronger than PostgreSQL

PostgreSQL allows concurrent writers and detects write-write conflicts at
commit time — a transaction can fail and must be retried. SlateDB
prevents a second writer from existing at all via fencing. DuckLake
clients therefore never experience a snapshot commit failure due to a
concurrent write: they either hold the writer lock and always succeed, or
they do not hold it and cannot submit writes in the first place.

---

## 10. Deployment Architecture

### 10.1 Is the SlateDuck process stateless?

Almost entirely yes. All correctness-critical state lives in object
storage:

| State | Location | Lost on crash? |
| --- | --- | --- |
| Catalog rows (all 28 tables) | S3 — SlateDB SSTs | No |
| Write-ahead log | S3 — SlateDB WAL | No (recovered on restart) |
| Manifest (database metadata) | S3 | No |
| Checkpoints | S3 | No |
| In-memory MemTable (recent writes) | RAM only | Yes — but WAL recovers these |
| Block cache (read acceleration) | RAM / local SSD | Yes — automatically rebuilt |

A SlateDuck replica can be killed and recreated at any time without data
loss or manual recovery steps.

### 10.2 Kubernetes deployment patterns

#### Pattern 1 — Read replicas (horizontal scale for reads)

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: slateduck-reader
spec:
  replicas: 3           # freely scalable
  template:
    spec:
      containers:
      - name: slateduck
        args: ["serve", "--mode=reader", "--catalog=s3://bucket/cat"]
```

Every pod is independent, stateless, and reads from the same object-store
catalog. No coordination needed. Suitable for read-only or append-only
workloads where catalog writes are rare.

#### Pattern 2 — Single writer + read replicas (recommended)

```yaml
# Writer (exactly one replica)
apiVersion: apps/v1
kind: Deployment
metadata: { name: slateduck-writer }
spec:
  replicas: 1
  template:
    spec:
      containers:
      - args: ["serve", "--mode=writer", "--catalog=s3://bucket/cat"]
---
# Readers (freely scalable)
apiVersion: apps/v1
kind: Deployment
metadata: { name: slateduck-reader }
spec:
  replicas: 4
  template:
    spec:
      containers:
      - args: ["serve", "--mode=reader", "--catalog=s3://bucket/cat"]
```

#### Pattern 3 — Writer election for automatic failover

Deploy N replicas as a `StatefulSet` with K8s `Lease`-based leader
election. The pod holding the lease runs in writer mode; all others run
in reader mode. If the writer crashes, another pod acquires the lease
and takes over. SlateDB's fencing ensures the old writer cannot commit
after the new writer starts.

### 10.3 Routing writes to the correct replica

Because only the writer replica accepts catalog mutations, clients and
load balancers must route writes correctly. Four options, from simplest
to most powerful:

#### Option A — PostgreSQL `target_session_attrs` (zero infrastructure)

The DuckDB `postgres` extension, like all libpq clients, supports
multi-host connection strings with `target_session_attrs=read-write`:

```
host=pod-a,pod-b,pod-c port=5432 target_session_attrs=read-write
```

The client tries each host in turn until it finds one that accepts
writes. A reader pod responds to any write attempt with PostgreSQL error
code `25006` (`read_only_sql_transaction`); the client automatically
tries the next. **No proxy, no labels, no external service.**

#### Option B — Writer self-publishes in the SlateDB catalog (most elegant)

When a pod acquires the writer role it writes its own address into two
well-known keys in the same atomic SlateDB transaction as the fencing epoch:

```
0xFF | "writer-epoch"    → u64 epoch
0xFF | "writer-endpoint" → "pod-a.slateduck.svc:5432"
```

These are always consistent because they are written atomically. Any
replica that receives a write request does a single `get("writer-endpoint")`
and forwards the connection. Caches the address until a write fails (e.g. a
`57P04` fencing error), then re-reads. **Zero external dependencies; the
catalog is its own service directory.**

#### Option C — Kubernetes label selector

The writer pod labels itself `slateduck-role=writer`. A dedicated K8s
Service targets only that label. When a pod takes over the writer role
it patches its own label via the Kubernetes API; the Service endpoint
updates in < 1 second. Requires K8s API access from the pod.

#### Option D — Protocol-aware proxy

A thin stateless proxy (itself a `Deployment` with N replicas) sits in
front of all SlateDuck pods and routes traffic by inspecting whether each
SQL statement is a read or a write:

```
Client → proxy (any replica)
           ├── read  → round-robin to reader pods
           └── write → writer pod (via Option A, B, or C)
```

The proxy uses [`sqlparser-rs`](https://crates.io/crates/sqlparser) to
classify each statement in microseconds. Because the proxy is stateless
it scales freely and adds no single-point-of-failure.

#### Recommended layering

Start with Option A (free, works immediately). Add Option B (writer
self-publishes address) as part of the core catalog implementation —
it's already planned in the key layout under tag `0xFF`. Introduce
Option D only if you need transparent routing from clients that cannot
accept a `25006` retry.

### 10.4 Cold-start and cache warming

When a fresh pod starts it has an empty block cache. The first few
catalog reads will be slow (full S3 round-trips). Mitigations:
- Mount a persistent volume for the on-disk block cache
  (`--cache-path=/mnt/cache --cache-size-mb=2048`). The cache survives
  pod restarts on the same node.
- Pre-warm the cache on startup by reading the current snapshot and the
  most recently active tables.
- DuckDB caches the current snapshot ID client-side; many queries never
  reach SlateDuck after the initial connection.

### 10.5 Credential separation in deployment

In Kubernetes, use separate service accounts or workload identities for each
credential plane:

| Workload | Identity | Required access |
| --- | --- | --- |
| `slateduck-writer` / `slateduck-reader` | Catalog role | SlateDB catalog prefix only |
| DuckDB ingestion/query jobs | Data role | Parquet data/delete-file prefix only |
| `slateduck gc` / maintenance job | Maintenance role | Read catalog plus delete eligible data files |

Do not mount data-plane credentials into the sidecar by default. If a sidecar
process accidentally receives the data role only, catalog startup should fail
with a clear permission SQLSTATE instead of creating a partial catalog. If a
client receives catalog credentials only, Parquet writes should fail before any
catalog mutation is accepted. The MinIO credential-isolation test from Phase 0
is the local proof of this deployment contract.

---

## 11. Implementation Readiness Checklist

The plan is ready to implement **Phase 0 now**. Phase 0 is not optional
research; it is the first implementation phase and produces the artifacts that
make the rest of the build safe.

Before opening the first Phase 1 data-model PR, the repository must contain:

1. `docs/phase-0/slatedb-api-validation.md` with working code and observed
  results for every gate in section 5.26.
2. `tests/fixtures/wire-corpus/duckdb-{version}.jsonl` for each supported
  DuckDB version, including generated inlined-table SQL and format codes.
3. A written GlueSQL vs. custom-dispatcher decision with the shim count and
  updated Phase 4 effort estimate.
4. A path-resolution decision for `ducklake_metadata.data_path` based on real
  DuckDB captures, reflected in `CatalogPath` tests.
5. A checked-in `tags.rs` skeleton with all 28 DuckLake table tags, `0xFD`,
   `0xFE`, and `0xFF` plus key shape, versioning rule, unique-guard keys, and
   implementation status.
6. A Phase 2 benchmark harness stub and the value-header encoder/decoder tests.
7. MinIO credential-isolation tests for catalog-only and data-only policies.

If all seven artifacts exist and the gates are green, Phases 1-4 are ready to
execute from this document without another architecture rewrite. If any artifact
fails, patch this plan first and keep the failed assumption visible in the risk
table.

## 12. Open Questions

These questions do not block Phase 0-4 implementation:

- Should we target the upstream `ducklake` extension or build a
  separate `slateduck` extension? Upstream is cleaner but slower
  socially.
- Do we want to expose the catalog tables to users as **read-only
  virtual SQL tables** (so they can run ad-hoc SQL over catalog
  metadata, the way DuckLake users can today against the PG catalog)?
  This would push Strategy B's SQL layer into the embedded case too.
- Pricing/operational guidance: at what scale does the S3 PUT cost of
  SlateDB's WAL become significant compared to PG hosting cost?

---

## 13. References

- SlateDB introduction: <https://slatedb.io/docs/get-started/introduction/>
- SlateDB design overview: <https://slatedb.io/docs/design/overview/>
- SlateDB Rust API: <https://docs.rs/slatedb>
- DuckLake home: <https://ducklake.select/docs/stable/>
- DuckLake catalog spec: <https://ducklake.select/docs/stable/specification/introduction.html>
- DuckLake query spec: <https://ducklake.select/docs/stable/specification/queries.html>
- DuckLake tables: <https://ducklake.select/docs/stable/specification/tables/overview.html>
- DuckLake catalog database choices: <https://ducklake.select/docs/stable/duckdb/usage/choosing_a_catalog_database.html>
- DuckDB `ducklake` extension: <https://duckdb.org/docs/current/core_extensions/ducklake>
