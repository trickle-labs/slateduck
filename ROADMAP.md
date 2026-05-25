# SlateDuck Roadmap

A lakehouse catalog backed by SlateDB — catalog and data in the same S3 bucket, zero infrastructure.

---

## Vision

SlateDuck makes a DuckLake lakehouse fully serverless: both the Parquet data
files and the DuckLake catalog live in the same object-storage bucket, with no
external database server required. The catalog is stored in SlateDB — an
embedded, LSM-based key-value store built entirely on top of object storage —
and is queryable from the standard DuckDB `ducklake` extension as well as other
DuckLake-compatible clients.

A second, equally load-bearing commitment shapes every storage decision:
**committed catalog facts are never physically deleted by normal operation,
and are always readable at the `dl_snapshot_id` at which they were written.**
Physical deletion exists only via the explicit, audited `slateduck excise`
command. This buys three properties that matter for the long term:

1. **Horizontal read scale-out.** Because catalog-data keys are stable once
   written (the only permitted change is the terminal `end_snapshot` mark,
   which cannot alter a reader's view at the key's own snapshot), an unbounded
   number of stateless reader replicas can serve queries at any historical
   `dl_snapshot_id` with no coordination between writer and readers and no
   coordination between readers.
2. **Time as a first-class dimension.** Time travel is the natural read mode,
   not a feature layered on top of MVCC.
3. **A path to a general fact store.** The storage substrate — keys scoped by
   `dl_snapshot_id`, rebuildable from object storage, queryable at any
   historical point — is not DuckLake-specific. Future releases can host
   additional schemas without changing the storage engine. See v2.x.

Physical deletion exists only via the explicit, audited `slateduck excise`
command invoked outside the normal write path (compliance erasure, opt-in
bounded retention). The default `slateduck gc` only advances query-visibility
metadata (`retain-from`); it does not delete bytes. The full principle,
including the distinction between catalog-data immutability and infrastructure-
key management, is in [plans/blueprint.md §1.4](plans/blueprint.md) and is
binding on every roadmap release below.

---

## Release Overview

| Release | Milestone | Status |
|---------|-----------|--------|
| **v0.1 — Foundation** | Validated infrastructure, data model, Rust workspace | **Done** |
| **v0.2 — Catalog Core** | All 28 DuckLake tables in SlateDB, full MVCC, catalog-data immutability, Rust API | **Done** |
| **v0.3 — PG-Wire Sidecar (Alpha)** | Strategy B sidecar serving DuckDB end-to-end | **Done** |
| **v0.4 — Production Hardening** | Visibility GC, excision, backups, observability, encryption, repair tooling | **Done** |
| **v0.5 — Native Extension (Beta)** | Strategy C embedded DuckDB extension via FFI | **Done** |
| **v0.6 — Multi-Client & Security** | pg-tide-relay onboarding, TLS/auth, audit log, GCS/Azure validation, compatibility matrix CI | **Done** |
| **v0.7 — Performance & Ecosystem** | Hot-key reads, secondary indexes, SlateDB tuning, multi-writer partitioning, DataFusion integration | **Done** |
| **v0.8 — Documentation** | MkDocs Material site, GitHub Pages, full conceptual, operational, and reference coverage | **Done** |
| **v0.9 — Production Readiness** | K8s deployment, writer routing and failover, performance tuning, cost analysis, migration and corpus tooling | **Done** |
| **v0.9.1 — Write Protocol Correctness** | Atomic snapshot commits, stale-counter fix, `UPDATE end_snapshot` key resolution, writer protocol spec | **Done** |
| **v0.9.2 — Security Enforcement** | Real PG-Wire auth, CLI/env-var alignment, encryption wired into storage, FFI null safety | **Done** |
| **v0.9.3 — Operational Safety** | GC retention enforcement, excision guards, checkpoint restore, typed import validation, rebuild fix | **Done** |
| **v0.9.4 — GA Ready** | Concurrent reads, zone-map (conditional), Spark/Trino clients, DataFusion scan/pg-wire, virtual catalog SQL, test coverage, CI gates, docs complete, versioning policy, release automation | **Done** |
| **v0.10 — Streaming Ingest** | pg-tide-relay integration, Kafka/NATS support, exactly-once delivery, CDC output (snapshot diffs, S3/Kafka/webhook) | **Done** |
| **v0.11 — IVM Foundations** | Catalog schema additions (tags 0x1D–0x20), `slateduck-ivm` crate, single-shard GROUP BY views, end-to-end demo | Done |
| **v0.12 — IVM Scale-Out** | Shard lease management, per-shard SlateDB state stores, multi-shard scale-out, re-sharding | Done |
| **v0.13 — IVM Joins** | Broadcast, co-partitioned, and re-shuffle join strategies; TPC-H Q3/Q4/Q5 | Done |
| **v0.13.1 — IVM Join Correctness** | EC-01 phantom-row fix, aggregate tier classification with auxiliary columns, volatility validation, property-based "differential ≡ full" oracle | Planning |
| **v0.14 — IVM Operational Hardening** | Native `SlateDbTrace`, cost optimization, cost guardrails (per-view budgets), observability, fault injection, 24 h soak, multi-view DAG | Planning |
| **v0.15 — IVM Feature Completeness** | Window functions, ORDER BY, LIMIT/top-N, correlated subqueries, recursive CTEs, non-det capture, WASM UDFs | Planning |
| **v0.16 — pg-trickle Compatibility** | `table_changes()` CDC function, stable `rowid`, snapshot lease, `NOTIFY` event-driven, mixed frontiers, extension schema support | Planning |
| **v1.0 — General Availability** | TPC-H @ SF10/SF100 benchmarks, S3 Express acceptance gate, IVM feature-complete GA sign-off, real-world validation gate | Planning |
| **v1.x — Ecosystem Expansion** | Async FFI v2, Lambda/edge integration, checkpoint-pinned readers, additional performance optimizations | Future |
| **v2.x — General Fact Store** | Non-DuckLake schemas on the same immutable substrate; alternative query interfaces; multi-writer exploration | Exploration |

---

## v0.1 — Foundation

> Validate all infrastructure assumptions before writing a single line of catalog code.

This release is intentionally front-loaded with research and validation work.
No production catalog code should be written until every gate here is green or
the design has been explicitly updated to account for a failed assumption.

### Rust Workspace and CI

- [x] Set up the full Rust workspace structure:
  ```
  slateduck/
  ├── Cargo.toml
  ├── crates/
  │   ├── slateduck-core/
  │   ├── slateduck-catalog/
  │   ├── slateduck-sql/
  │   ├── slateduck-sqlite-vfs/
  │   ├── slateduck-pgwire/
  │   └── slateduck-ffi/
  ├── extension/
  ├── docs/
  └── tests/
  ```
- [x] Configure GitHub Actions: `cargo fmt`, `clippy`, `test` on Linux and macOS.
- [x] Pin initial dependencies: `slatedb`, `object_store`, `bytes`, `tokio`, `serde`, `prost`, `pgwire`, `sqlparser-rs`, `proptest`, `fail-parallel`.
- [x] Add `CONTRIBUTING.md`, `LICENSE`, and `docs/architecture.md` stubs.

### SlateDB API Validation

Produce `docs/phase-0/slatedb-api-validation.md` with working Rust code for each of the following. Every item is a go/no-go gate:

| Gate | Validation | Fallback if it fails |
|------|------------|----------------------|
| Atomic multi-key writes | `WriteBatch` is all-or-none across crash/reopen | Use `DbTransaction`-only path; stop if neither is atomic |
| Conditional initialization | `DbTransaction` can implement insert-if-absent for `ducklake_metadata` | Require explicit `slateduck init` under an external deployment lock |
| Serializable counter allocation | Two concurrent transactions on the same counter: one wins, loser gets a retryable conflict, no ID is reused after crash/reopen | Single-writer in-memory allocator persisting counter and consumed rows in one batch |
| Concurrent initialization convergence | Two processes calling `open_or_create` on a fresh catalog produce exactly one coherent initial key/value set | Require explicit `slateduck init` |
| Durable commit options | `commit_with_options` / `await_durable` survives a crash | Document as required; abort if SlateDB does not expose it |
| `flush()` reader visibility | Write → `flush()` → fresh `DbReader` sees the key on LocalFS and MinIO | Replace with verified memtable flush or serve read-your-writes from the writer process |
| Visibility-barrier latency | Measure p50/p95/p99 on LocalFS and MinIO; record for later Phase 4 latency budgets | — |
| Writer fencing | Force two writers; capture the exact error kind returned; confirm it is distinguishable | Maintain SlateDuck-own epoch check; map stale epochs to `SQLSTATE 57P04` |
| `WriteBatch` logical size | Determine whether SlateDB imposes its own limit | Enforce SlateDuck's own 64 MiB limit unconditionally |
| Prefix-scan latest-value semantics | Verify `scan_prefix` returns fully-merged latest values, not stale LSM entries | Add a decode/dedup layer before applying MVCC filters |

### DuckDB Wire Corpus Capture

Capture the complete PostgreSQL-wire traffic between DuckDB and a real
PostgreSQL-backed DuckLake. Store as
`tests/fixtures/wire-corpus/duckdb-{version}.jsonl`.

The corpus must include:
- [x] Startup handshake: every probe query and its required response (server version, `current_schema()`, `pg_type`, `pg_namespace`, `pg_catalog` queries)
- [x] All `SET`/`SHOW` statement exchanges
- [x] Simple query protocol examples
- [x] Extended query protocol: `Parse`/`Bind`/`Describe`/`Execute`/`Sync` sequences
- [x] `BEGIN`/`COMMIT`/`ROLLBACK` — whether DuckLake uses explicit transactions
- [x] Parameter value encodings and result format codes (text and binary)
- [x] Generated inlined-table DDL/DML for small inserts and deletes
- [x] All SQL emitted by the full DuckLake tutorial
- [x] The complete DuckLake-tutorial output against SQLite-backed DuckLake as the golden reference

Separately capture:
- [x] `tests/fixtures/handshake/duckdb-{version}.jsonl` — handshake-only replay fixtures
- [x] `tests/fixtures/wire-corpus/pgtide-0.34-expected.jsonl` — pg-tide-relay corpus (placeholder for Phase 1.x)

### Access-Pattern and Key-Layout Analysis

Produce `docs/phase-0/access-patterns.md` from the wire corpus. For every DuckLake table with a non-obvious dominant query, confirm or revise the proposed key shape from the design document before any encoder is written.

Decisions to record:
- [x] Whether DuckDB supplies explicit IDs in `INSERT` statements or reads `next_catalog_id`/`next_file_id` and allocates them locally
- [x] Whether `ducklake_metadata.data_path` is absolute or relative in each capture scenario
- [x] Whether `BEGIN`/`COMMIT` wraps DuckLake catalog operations
- [x] Whether extended query protocol is used for all statements or only prepared ones
- [x] Whether DuckDB probes `pg_catalog` tables beyond the known list

### GlueSQL Spike

Spike GlueSQL as the SQL execution layer for Strategy B:
- [x] Connect DuckDB to a minimal GlueSQL-backed `pgwire` server
- [x] Verify the full handshake completes and a `CREATE TABLE` / `INSERT` / `SELECT` round-trip succeeds
- [x] Count PostgreSQL-specific shims required (one handler per mismatch)
- [x] **Decision gate:** fewer than ten shims → adopt GlueSQL; more → build the custom AST-matching dispatcher

### Object-Store and Credential Isolation Spike

- [x] Run MinIO locally with two separate IAM policies:
  - `catalog-only`: read/write to `catalogs/` prefix, no access to `data/`
  - `data-only`: read/write to `data/` prefix, no access to `catalogs/`
- [x] Verify the sidecar works under `catalog-only` policy and DuckDB works under `data-only` policy
- [x] Record expected `SQLSTATE` mappings for permission failures (`42501`)
- [x] Document that the GC / maintenance job requires both

### Latency Baseline

Measure and record p50/p95/p99/p99.9 for each of:
- [x] SlateDB durable commit on LocalFS and MinIO
- [x] `flush()` visibility barrier on LocalFS and MinIO
- [x] Single `get` on LocalFS and MinIO
- [x] Prefix scan of 10 K entries on LocalFS and MinIO

Store in `docs/phase-0/latency-baseline.json`. Phase 4 latency budgets derive from these numbers.

### DuckLake Reference Baseline

Stand up the full DuckLake tutorial against SQLite-backed DuckLake and capture all output as golden fixtures under `tests/golden/duckdb-{version}/`. These fixtures are the spec-conformance oracle for every subsequent phase.

### Deliverables

- [x] Passing `hello world` smoke test: open SlateDB on LocalFS, put/get, scan a prefix, transaction, checkpoint
- [x] All Phase 0 validation artifacts checked in and green
- [x] Go/no-go decision recorded for: GlueSQL vs. custom dispatcher, transaction API, conditional init, `flush()` barrier, `pgwire` crate extended-protocol support
- [x] No Phase 1 data-model code until all gates pass or the plan is updated for failures

---

## v0.2 — Catalog Core

> Store and retrieve every DuckLake v1.0 catalog row, with full MVCC, via a clean Rust API.

### Catalog Key Layout (`slateduck-core`)

Implement the full binary key layout for all 28 DuckLake v1.0 tables plus SlateDuck system namespaces. Every tag byte must be allocated up front, even for tables deferred to later phases, so that unknown tables return an explicit error rather than silent data loss.

```
01  ducklake_metadata          scope | scope_id | metadata_key
02  ducklake_snapshot          snapshot_id
03  ducklake_snapshot_changes  snapshot_id
04  ducklake_schema            schema_id
05  ducklake_table             schema_id | table_id | begin_snapshot
06  ducklake_column            table_id | column_id | begin_snapshot
07  ducklake_view              schema_id | view_id | begin_snapshot
08  ducklake_macro             schema_id | macro_id | begin_snapshot
09  ducklake_macro_impl        macro_id | impl_id
0A  ducklake_macro_parameters  macro_id | impl_id | column_id
0B  ducklake_data_file         table_id | data_file_id
0C  ducklake_delete_file       data_file_id | delete_file_id
0D  ducklake_files_scheduled_for_deletion  schedule_start | data_file_id
0E  ducklake_inlined_data_tables  table_id | schema_version
0F  ducklake_column_mapping    table_id | mapping_id
10  ducklake_name_mapping      mapping_id | column_id | source_name_hash
11  ducklake_table_stats       table_id
12  ducklake_table_column_stats  table_id | column_id
13  ducklake_file_column_stats  table_id | column_id | data_file_id
14  ducklake_file_variant_stats  table_id | column_id | variant_path_hash | data_file_id
15  ducklake_partition_info    table_id | partition_id | begin_snapshot
16  ducklake_partition_column  partition_id | partition_key_index
17  ducklake_file_partition_value  table_id | partition_key_index | data_file_id
18  ducklake_sort_info         table_id | sort_id | begin_snapshot
19  ducklake_sort_expression   sort_id | sort_key_index
1A  ducklake_tag               object_id | tag_key | begin_snapshot
1B  ducklake_column_tag        table_id | column_id | tag_key | begin_snapshot
1C  ducklake_schema_versions   table_id | begin_snapshot
FD  dynamic inlined rows       subtype | table_id | (schema_version | data_file_id) | row_id
FE  SlateDuck counters         counter_id
FF  SlateDuck system keys      writer epoch / endpoint / retain-from / catalog-format-version
```

The `0xFE` counter keys and `0xFF` system keys are managed with simple
transactional writes (see [plans/blueprint.md §1.4](plans/blueprint.md)).
Excision audit records are appended under a dedicated `0xFF | "excised"` prefix
and accumulate without overwriting previous entries.

Produce `crates/slateduck-core/src/tags.rs` as the single source of truth listing every table's tag byte, key shape, versioning rule, MVCC behavior, unique-guard key requirement, and implementation status (`Live`, `Deferred(phase)`, `Unimplemented`).

Key-layout rules:
- Big-endian integers throughout; `u8` table tag as first byte
- **Catalog-data facts (the 28 DuckLake tables and `0xFD` inlined rows) are never physically deleted outside explicit excision** (see Vision and [plans/blueprint.md §1.4](plans/blueprint.md))
- Tables with `begin_snapshot`/`end_snapshot` and no SQL primary key include `begin_snapshot` in the SlateDB key so historical versions are distinct keys — each version is written once at creation and updated at most once when `end_snapshot` is set (that single terminal update is the only permitted in-place change for a version row)
- `ducklake_file_column_stats` keyed by `(table_id, column_id, data_file_id)` for efficient per-column pruning scans
- `ducklake_metadata` scoped key: `scope_enum | scope_id | length-prefixed UTF-8 key`
- `0xFD` dynamic inlined rows: subtype `0x01` for inlined insert rows, subtype `0x02` for inlined delete markers
- `0xFF catalog-format-version`: a single `u32` key written once at init; mismatch on open → refuse (`SQLSTATE 0A000`)

### Value Encoding

- Protobuf for all catalog values (forward/backward compatibility; schema evolution)
- Every value prefixed with: `encoding_version: u8` | `magic: b"SDKV"` | Protobuf payload
- Decode must verify `b"SDKV"` magic before dispatching to version-specific decoder
- Old readers encountering an unknown `encoding_version` return an explicit error, not a silent misparse

### Counter and ID Allocation

Implement all DuckLake ID domains as SlateDB-backed counters under `0xFE`:

```
0xFE | 0x01  →  u64 next_snapshot_id
0xFE | 0x02  →  u64 next_catalog_id
0xFE | 0x03  →  u64 next_file_id
0xFE | 0x10 | table_id  →  u64 next_column_id_for_table
```

Counter allocation, counter increment, and the row that consumes the ID must
commit in a single SlateDB `DbTransaction` — never as separate writes. The
in-memory counter value is cached by the writer process; every allocating
transaction reads from the cache, writes the updated counter and consuming row
atomically, then updates the cache on commit. Include a proptest that runs N
random catalog operations and asserts every allocated ID is strictly greater
than its predecessor, across simulated crash/reopens.

### MVCC Implementation

- Snapshot reads: `SlateDB db.snapshot()` for a consistent, non-torn KV scan; DuckLake `snapshot_id` as the MVCC filter applied to deserialized rows
- Enforce strict naming: `dl_snapshot_id` / `catalog_version` for DuckLake snapshots; `kv_read_view` / `kv_snapshot` for SlateDB-level read views
- `end_snapshot` is stored in the version's value and set by a single in-place update when the version is retired; this is the only permitted update in a version row's lifetime — bounded because `end_snapshot` can be set at most once, and it preserves all original row data while only marking the row invisible to future snapshots
- MVCC filter: `begin_snapshot ≤ dl_snapshot_id AND (end_snapshot IS NULL OR dl_snapshot_id < end_snapshot)`
- Unique-guard keys under `0xFE` for any table where the hot-scan key does not enforce spec primary-key uniqueness

### Inlined Data Storage (`0xFD`)

- Subtype `0x01`: inlined insert rows keyed by `table_id | schema_version | row_id`; values carry row payload + `begin_snapshot` + `end_snapshot`
- Subtype `0x02`: inlined delete markers keyed by `table_id | data_file_id | row_id`; values carry deletion `begin_snapshot` only
- Maximum encoded inlined-row value: 64 MiB; oversized rows return `SQLSTATE 54001`
- Physical GC eligibility for subtype `0x01`: `end_snapshot IS NOT NULL AND end_snapshot <= oldest_retained_snapshot`
- Physical GC eligibility for subtype `0x02`: derived from target data file and table lifecycle

### `schema_version` Tracking

`CatalogWriter` must expose `fn mark_schema_changed(&mut self)`, called explicitly by every schema-mutating operation (`create_table`, `drop_table`, `create_schema`, `drop_schema`, column add/drop/rename/retype, partition/sort/mapping changes). The `create_snapshot` commit path increments `schema_version` iff this flag is set; data-only operations carry it forward unchanged.

Add a schema-version matrix test: every schema-mutating operation increments `schema_version`; every data-only operation preserves it.

### Type-Aware Column Statistics

`prune_files()` accepts a `DuckLakeType` argument and performs type-aware comparison:
- Integers: parse as signed/unsigned integers per width; no lexicographic compare
- Decimals: parse to decimal/rational, not float
- Timestamps: parse to typed temporal values; normalize time zones before compare
- IEEE floats: handle `inf` / `-inf`; ignore NaN bounds separately via `contains_nan`
- Unknown types: fail closed (`SQLSTATE 0A000`) rather than guessing

### Catalog Initialization

Safe `open_or_create` using `DbTransaction` with `SerializableSnapshot` isolation. Two concurrent first-connections must converge on exactly one coherent initial metadata set. Test explicitly: launch two processes simultaneously against a fresh path; verify exactly one metadata set and one counter set.

### Path Canonicalization

- `CatalogPath` struct in `slateduck-core` encapsulates `object_store_root`, `catalog_prefix`, `data_prefix`, `data_path_mode` (`Absolute` | `RelativeToDataPrefix`)
- Prefer absolute object-store URIs (`s3://bucket/data/warehouse-a/`) wherever DuckDB allows
- Relative paths stored only with unambiguous `path_is_relative` flag and enclosing scope path
- Never use raw string concatenation for object-store paths anywhere in the codebase

### `CatalogStore` Public Surface

```rust
pub struct CatalogStore { db: slatedb::Db, /* … */ }

impl CatalogStore {
    pub async fn open(opts: OpenOptions) -> Result<Self>;
    pub async fn read_at(&self, dl_snapshot_id: SnapshotId) -> CatalogReader;
    pub async fn begin_write(&self) -> CatalogWriter;
}
```

### Property Test Suite

Using `proptest`:
- Round-trip: `decode(encode(row)) == row` for all row types across all 28 tables
- Key ordering: `encode(id=5) < encode(id=6)` for all numeric ID fields
- Prefix isolation: `scan_prefix(tag | id)` returns only rows for that entity and no other table
- No key collisions between different table tags with any valid input
- ID monotonicity: N operations in sequence; all allocated IDs strictly increasing across crash/reopen

### Time Travel and Retention Design

Catalog-data immutability means every committed fact is readable at its
original `dl_snapshot_id` by construction. Two distinct operations control
query visibility and physical footprint:

- **Retention advancement (default, safe).** `0xFF | "retain-from"` is a single key updated transactionally by the TTL task. It records the query-visibility floor; `slateduck gc` only advances it, never deletes bytes. Default: infinite / never advance (configurable via `--retention-days`; `0` or omitted means never advance). `catalog.pin_snapshot(id)` blocks advancement.
- **Excision (rare, audited).** Physical deletion of bytes. Invoked only via `slateduck excise`, never as part of the normal write path or default gc sweep. The excision event is persisted under `0xFF | "excised"` so the audit trail accumulates across runs.

Default physical retention is **infinite**. Operators may opt into bounded
storage via `--excise-days` (off by default) plus an explicit
`slateduck excise --before <snapshot> --apply` invocation.

Orphaned Parquet files (not committed to any snapshot) remain eligible for
cleanup by the orphaned-file sweep with the configurable grace period (default
7 days); they are not part of the catalog-data fact set and do not require
excision.

### Early Validation and Benchmark Baseline

- `slateduck verify catalog` command: primary-key uniqueness, foreign-key references, MVCC interval consistency, counter monotonicity
- `benchmarks/phase-2-baseline.json`: p50/p95/p99/p99.9 for `get_current_snapshot`, `list_data_files` at 10 K files, `describe_table` with 100 columns, `prune_files` on one typed column, `create_snapshot` with 100 file additions — on LocalFS and MinIO

### Deliverables

- [x] Documented Rust library storing and retrieving every row type defined by DuckLake v1.0 including `0xFD` dynamic inlined rows
- [x] Property test suite green
- [x] `tags.rs` complete and reviewed
- [x] `slateduck verify catalog` command working
- [x] Benchmark baseline recorded

---

## v0.3 — PG-Wire Sidecar (Alpha)

> Connect the standard DuckDB `ducklake` extension to SlateDuck through a PostgreSQL-wire sidecar.

This is the Strategy B production implementation. The sidecar speaks the PostgreSQL wire protocol and translates DuckLake catalog SQL into `CatalogStore` operations, storing all state in SlateDB.

### DuckLake-Spec Operations (`slateduck-catalog`)

Implement all spec operations from [specification/queries.html](https://ducklake.select/docs/stable/specification/queries.html) as typed Rust methods:

**Read operations:**
- `get_current_snapshot()`
- `list_schemas(dl_snapshot_id)`
- `list_tables(schema_id, dl_snapshot_id)`
- `describe_table(table_id, dl_snapshot_id)` — columns, partitions, sort info
- `list_data_files(table_id, dl_snapshot_id)` — with delete-file merge in application code
- `prune_files(table_id, column_id, predicate, col_type)` — type-aware

**Write operations:**
- `create_snapshot(changes, author?, message?)`
- `create_schema` / `drop_schema`
- `create_table` / `drop_table` / `rename_table`
- `alter_table_add_column` / `alter_table_drop_column` / `alter_table_rename_column`
- `register_data_file` / `register_delete_file`
- `register_inlined_insert` / `mark_inlined_insert_deleted`
- `register_inlined_delete` / `plan_flush_inlined_data`
- `update_table_stats` / `upsert_file_column_stats`
- Writer-epoch fencing: store epoch token in `0xFF`; every commit path checks epoch before writing

Each write operation runs inside a single SlateDB `DbTransaction` (or `WriteBatch` for counter-free bulk inserts) so the new snapshot row and all referenced metadata changes commit atomically.

### `slateduck-pgwire` Sidecar Binary

#### Wire Protocol

- `pgwire` crate for startup, simple query protocol, extended query protocol (`Parse`/`Bind`/`Describe`/`Execute`/`Sync`)
- Prepared-statement caching: cache the parsed + classified AST for named statements
- `SET` handler: accept all settings; store and apply `timezone`, `client_encoding` (`UTF8` only), `DateStyle`
- `SHOW` handler: return plausible hardcoded values for `server_version`, `DateStyle`, `transaction_isolation`
- Pass replay test against `tests/fixtures/handshake/duckdb-{version}.jsonl` before any DuckLake-specific logic is wired

#### Bounded SQL Dispatcher (`slateduck-sql`)

Implement exactly the statement shapes present in the Phase 0 wire corpus. Pattern match on `sqlparser-rs` AST nodes — never on raw SQL strings — and substitute `$N` parameter values at dispatch time.

**Supported read shapes:**
- `SELECT max(snapshot_id) FROM ducklake_snapshot` and `SELECT ... ORDER BY snapshot_id DESC LIMIT 1`
- `SELECT ... FROM ducklake_{schema|table|column} WHERE ... AND begin_snapshot <= $1 AND (end_snapshot IS NULL OR $1 < end_snapshot)`
- `SELECT ... FROM ducklake_data_file LEFT JOIN ducklake_delete_file USING (data_file_id) WHERE table_id = $1`
- `SELECT data_file_id FROM ducklake_file_column_stats WHERE table_id = $1 AND column_id = $2 AND ...`
- `SELECT current_schema()`, `SELECT version()`, `SELECT current_database()`
- `SELECT oid, typname FROM pg_catalog.pg_type WHERE typname IN (...)`

**Supported write shapes:**
- `INSERT INTO ducklake_{snapshot|snapshot_changes|schema|table|column|data_file|delete_file|...} VALUES (...)`
- `UPDATE ducklake_table_stats SET record_count = record_count + $1 WHERE table_id = $2`
- `UPDATE ducklake_{table|column|data_file|...} SET end_snapshot = $1 WHERE id = $2 AND end_snapshot IS NULL`
- Generated inlined-table DDL: `CREATE TABLE ducklake_inlined_*`
- Generated inlined-table DML: `INSERT INTO ducklake_inlined_*`, `UPDATE ducklake_inlined_* SET end_snapshot = ...`, `SELECT FROM ducklake_inlined_*`

Anything outside this bounded set returns `SQLSTATE 0A000` (feature not supported).

#### Transaction Buffering

Session state accumulates `INSERT`/`UPDATE` statements into a `PendingCatalogTxn` between `BEGIN` and `COMMIT`. `ROLLBACK` or disconnect drops the pending batch. Commit path calls `SlateDB DbTransaction` and then the visibility barrier (`flush()`). Cap pending batch at 64 MiB; return `SQLSTATE 54001` if exceeded.

#### PostgreSQL Type OIDs

Implement text encoders/decoders for all types observed in the Phase 0 corpus:

| OID | Type | Used for |
|-----|------|----------|
| 16 | `bool` | `path_is_relative`, nullability flags |
| 20 / 23 / 21 | `int8` / `int4` / `int2` | IDs, counts, sizes |
| 700 / 701 | `float4` / `float8` | statistics values |
| 25 / 1043 | `text` / `varchar` | names, paths, JSON fields |
| 1114 / 1184 | `timestamp` / `timestamptz` | snapshot timestamps |
| 2950 | `uuid` | schema/table UUIDs |
| 114 / 3802 | `json` / `jsonb` | snapshot change metadata |

For binary format codes not observed in the corpus, return `SQLSTATE 0A000` before execution.

#### SQLSTATE Mapping

All errors flow through a single `to_pg_error(err: SlateDuckError) -> PgErrorResponse` function:

| Condition | SQLSTATE | Severity |
|-----------|----------|----------|
| Writer fenced | `57P04` | FATAL |
| Snapshot out of retention window | `22023` | ERROR |
| Object-store timeout / throttle | `08006` | ERROR |
| Row not found | `02000` | — |
| Value decode error (version mismatch) | `22P02` | ERROR |
| Magic mismatch / corruption | `XX001` | ERROR |
| ID counter write failure | `40001` | ERROR |
| Duplicate / PK collision | `23505` | ERROR |
| Write to read-only replica | `25006` | ERROR |
| Unsupported feature | `0A000` | ERROR |
| Object-store permission denied | `42501` | ERROR |
| Catalog not initialized | `3D000` | FATAL |
| Internal error | `XX000` | ERROR |

#### Concurrency Model

- One SlateDB writer per catalog — the sidecar serializes writes through a single-writer actor
- Many readers — each PG session opens a `DbReader` / `DbSnapshot` against a current SlateDB checkpoint
- Rate limits: `--max-sessions` (default 50), `--max-active-scans` (default 25), `--object-store-max-inflight` (default 100)

#### Safe Writer Takeover Protocol

```rust
let db = Db::builder(path, object_store).build().await?;
db.flush().await?;  // durable reader-visible state after takeover
catalog_store.publish_writer_endpoint(my_address).await?;
// safe to accept client connections
```

Integration test: kill writer mid-commit; start new writer; open `DbReader` immediately after `flush()`; verify all pre-crash commits are visible.

### End-to-End Test Suite

- Golden test: replay Phase 0 DuckLake tutorial corpus against SlateDuck sidecar; diff output byte-for-byte against the SQLite-backed reference
- Wire-corpus replay tests for every captured DuckDB version
- Schema-version matrix tests
- Time-travel tests: `SELECT * FROM t AT (SNAPSHOT N)` returns correct rows at every historical snapshot
- Crash injection tests at all required crash points:
  - After S3 PUT, before catalog commit → orphaned file, catalog unchanged
  - During `create_snapshot` batch assembly → no partial snapshot visible
  - During `drop_table` commit → all-or-none tombstones
  - During writer fencing → new writer takes over cleanly
  - Two processes initializing fresh catalog → exactly one coherent result

### DuckDB Compatibility Matrix

Maintain `docs/compatibility.md`:
- DuckDB 1.5.2: baseline (Phase 0 capture)
- Minor version bumps: new corpus capture + explicit sign-off required
- Major version bumps: full new client treatment

### `slateduck serve` Binary

```
slateduck serve \
  --catalog s3://bucket/catalogs/warehouse-a \
  --bind 0.0.0.0:5432
```

Operators who want bounded time-travel visibility pass `--retention-days N` (e.g. `--retention-days 30`).

### Deliverables

- [x] `slateduck serve` binary exposing a SlateDB catalog at a PostgreSQL TCP endpoint
- [x] DuckDB connecting via standard `postgres` extension with all tutorial operations passing
- [x] Golden tests green for DuckDB 1.5.2
- [x] All crash injection tests passing
- [x] SQLSTATE test for every error code path

---

## v0.4 — Production Hardening

> Make SlateDuck safe and operable in production.

### Visibility GC and Excision

Catalog-data immutability splits the old "GC" concept into two distinct
operations:

**Visibility GC (default, safe).** Advances the `retain-from` key by a
transactional write. Never deletes bytes. Run via `slateduck gc plan` /
`slateduck gc apply` or as an optional background task behind `--enable-gc`
(off by default until acceptance tests prove it does not compete with foreground
catalog commits). Pinning via `catalog.pin_snapshot(id)` blocks advancement.

**Excision (rare, audited, foreground only).** Physically deletes catalog
facts and Parquet files older than the floor. Invoked only via
`slateduck excise plan` / `slateduck excise apply --before <snapshot>`. Always
requires explicit operator invocation; never runs in the background. Records
an audit entry under `0xFF | "excised"`. On per-key deletion failure: log and
skip; do not retry aggressively inside any request path.

- Catalog excision scope: version rows whose `end_snapshot IS NOT NULL AND end_snapshot <= oldest_retained_snapshot`; inlined-insert rows (`0xFD | 0x01`) by same rule; inlined-delete markers (`0xFD | 0x02`) when their target data file is excised
- Data-file excision: only when no retained snapshot references the file

### Parquet Data-File Cleanup

- **Orphaned-file sweep** (default-safe, not excision): scan object-store paths not referenced by any `ducklake_data_file` row with a valid `begin_snapshot`; delete after configurable grace period (default 7 days). These files were never committed.
- **Scheduled deletion** (`ducklake_files_scheduled_for_deletion`): files marked for cleanup by merge-on-read `DELETE` / `UPDATE` are deleted only after no retained snapshot references them.
- `verify_data_files(table_id)` method: `HEAD` every referenced file; flag missing files for operator review.

### Checkpoints and Backups

- `slateduck checkpoint create` — thin wrapper around `SlateDB Checkpoint` API; produces a point-in-time catalog backup
- `slateduck checkpoint restore` — restore catalog to a named checkpoint
- `slateduck checkpoint list` — show all available checkpoints with timestamps

### Catalog Export and Migration

- `slateduck export --output catalog.ndjson [--snapshot-id N]` — NDJSON export of all live catalog rows at the specified or latest snapshot; includes `0xFD` inlined rows labeled by generated table name; excludes `0xFE`/`0xFF` system keys
- `slateduck import --input catalog.ndjson` — initialize a fresh catalog from an export file
- `slateduck pg-migrate --input catalog.ndjson | psql ...` — convert NDJSON to PostgreSQL `INSERT` statements for migrating to PostgreSQL-backed DuckLake
- `slateduck rebuild --data-path s3://bucket/data/warehouse` — synthesize a fresh catalog by reading Parquet footers when no export or checkpoint exists

Round-trip test: export from v1, import into v2; verify all snapshot IDs, file registrations, and MVCC visibility are equivalent.

### Observability

Re-export `db_stats` from SlateDB and add catalog-level metrics:
- Snapshots/sec
- Files/snapshot
- Mean rows scanned per `list_data_files`
- Object-store request count, bytes read/written, throttles, retry count
- Compaction backlog, WAL backlog
- Per-query scanned key count
- `--max-sessions` and active session count
- Writer epoch age

Expose metrics via OpenTelemetry and a Prometheus-compatible `/metrics` HTTP endpoint.

### Encryption

- Use SlateDB block transformers for at-rest encryption of catalog values
- `--encryption-key` CLI option and documentation
- Note that Parquet encryption is a separate, Parquet-native concern

### Validation and Repair Tooling

| Command | Purpose |
|---------|---------|
| `slateduck inspect snapshot --latest` | Current snapshot, schema version, counters, file counts |
| `slateduck verify catalog` | PK uniqueness, FK references, MVCC intervals, counter monotonicity |
| `slateduck verify data-files` | HEAD every referenced Parquet/delete file, optionally sample footers |
| `slateduck gc plan` / `slateduck gc apply` | Advance `retain-from`; never delete bytes |
| `slateduck excise plan` / `slateduck excise apply --before <snapshot>` | Physically delete facts and Parquet files older than the floor; records audit fact; requires explicit `--apply` |
| `slateduck repair --dry-run` | Propose repairs; require explicit `--apply` for mutation |

Repair conservatism rules:
- **Repairable:** orphaned dynamic inlined keys, stale counters, dangling rows outside retention window, missing optional stats rows
- **Unrecoverable:** magic mismatch, Protobuf decode failure for retained row, missing `ducklake_snapshot` or `ducklake_metadata`, missing Parquet files for retained snapshots — refuse mutation, direct operator to restore

### Documentation Site

- Quickstart guide: local → MinIO → S3 end-to-end
- Architecture diagram: catalog plane vs. data plane, credential separation
- DuckDB compatibility matrix
- Comparison with PG-backed and SQLite-backed DuckLake
- Time-travel guide
- Troubleshooting with `verify`, `inspect`, `gc plan`

### Object-Store Graduation

| Backend | Target | Status |
|---------|--------|--------|
| `LocalFileSystem` | Development | v0.1 |
| `InMemory` | Unit tests | v0.1 |
| MinIO | CI integration tests | v0.2 |
| AWS S3 Standard | Acceptance and correctness | v0.4 |
| AWS S3 Express One Zone | Performance benchmarking | v0.5 |
| Google Cloud Storage | Validated on demand | v0.6 |
| Azure Blob Storage | Validated on demand | v0.6 |

### Deliverables

- [x] Visibility GC advances `retain-from` without data loss; tested with time-travel queries before and after
- [x] `slateduck excise` deletes only operator-specified history; audit fact written; default behavior (no `--apply`) is plan-only
- [x] `slateduck export` / `import` round-trip test passes
- [x] `slateduck rebuild` recovers a catalog from Parquet-only state
- [x] Checkpoint create / restore tested with crash injection
- [x] Metrics endpoint live
- [x] S3 Standard acceptance tests green
- [x] Documentation site published

---

## v0.5 — Native Extension (Beta)

> Embed SlateDuck directly into DuckDB with no SQL emulation layer — Strategy C.

This is the cleanest and fastest integration path: a DuckDB extension that implements DuckLake's catalog interface in C++ by calling a Rust FFI layer backed by SlateDB. No PostgreSQL sidecar, no SQL parsing, no network hop.

### DuckDB Catalog Interface Analysis

- Read the current `ducklake` extension source
- Document the internal C++ catalog interface surface that must be implemented (analogous to `Catalog` / `FileIO` in Iceberg)
- Draft an upstream RFC / GitHub Discussion proposing a new `slatedb:` backend alongside `duckdb`/`sqlite`/`postgres`/`mysql`
- **Decision gate:** can we contribute upstream, or must we fork/publish as a community extension?

### C ABI (`slateduck-ffi`)

Expose `slateduck-catalog` through a stable C ABI:
- Opaque handles for `CatalogStore`, `CatalogReader`, `CatalogWriter`
- C functions for each spec operation
- Well-defined error codes mapped to DuckDB's expected return values
- All Rust `async fn` bridged via a blocking Tokio runtime (Strategy C v1)
- **ABI versioning:** export `uint32_t slateduck_abi_version()` returning a compile-time constant (`major * 1000 + minor`); the DuckDB extension checks this at load time and refuses to proceed on version mismatch — a mismatch otherwise produces a silent crash (see §5.29 for full requirements)

```c
slateduck_catalog_t* slateduck_open(const char* uri, slateduck_error_t* err);
slateduck_snapshot_t* slateduck_get_current_snapshot(slateduck_catalog_t*, slateduck_error_t*);
void slateduck_list_data_files(slateduck_catalog_t*, uint64_t table_id, uint64_t snapshot_id,
                                slateduck_file_list_t** out, slateduck_error_t* err);
// …
```

### Async–Sync Bridge

Strategy C v1 uses a blocking Tokio runtime (Option 1):
- FFI layer owns a `tokio::runtime::Runtime` initialized once at extension load
- Each catalog call uses `runtime.block_on(async { ... })` — correct and safe
- Profile under realistic workloads before investing in callback-based async FFI (Option 2)

Record Phase 0 finding on whether DuckDB ≥1.5 has an async catalog extension API.

### C++ Extension Backend

- Implement the DuckDB extension in C++ against the SlateDuck C ABI
- `ATTACH 'ducklake:slatedb:s3://bucket/catalogs/warehouse-a' AS lake;`
- Reuse all Phase 0.3 test suites plus Phase 0.3 golden fixtures to validate equivalence with Strategy B

### Distribution

- Community extension repository submission if upstream adoption is not immediate
- `INSTALL slateduck; LOAD slateduck;` in a vanilla DuckDB

### Phase 6 Optional Features Completion (bundled with v0.5)

Implement the deferred `Phase 6` catalog tables and features alongside Strategy C:

**Views, macros, tags:**
- `ducklake_view` (tag `0x07`)
- `ducklake_macro` / `ducklake_macro_impl` / `ducklake_macro_parameters` (tags `0x08`–`0x0A`)
- `ducklake_tag` / `ducklake_column_tag` (tags `0x1A`–`0x1B`)
- `ducklake_file_variant_stats` (tag `0x14`)

**File cleanup:**
- `ducklake_files_scheduled_for_deletion` (tag `0x0D`) — full lifecycle for merge-on-read `DELETE` / `UPDATE`

All deferred tables return `SQLSTATE 0A000` in Phase 0.3; this release removes those stubs.

### Deliverables

- [x] `INSTALL slateduck; ATTACH 'ducklake:slatedb://…' AS lake;` works in a vanilla DuckDB
- [x] All Phase 0.3 golden tests pass through the native extension path
- [x] Strategy B and Strategy C produce identical query results on the same catalog
- [x] All 28 DuckLake v1.0 tables implemented and tested

---

## v0.6 — Multi-Client & Security

> Onboard the first non-DuckDB client, harden the sidecar for production deployments, and validate all planned object-store backends.

### Multi-Client Support (Strategy B)

Formalize the client onboarding process for non-DuckDB DuckLake clients:

1. [x] Capture client's full SQL corpus as `tests/fixtures/wire-corpus/{client}-{version}.jsonl`
2. [x] Classify each statement: already supported / trivial extension / new pattern requiring dispatcher work
3. [x] Add category-b statements behind a feature flag gated on the client corpus
4. [x] Category-c statements evaluated case-by-case; dispatcher will not grow into a general SQL engine
5. [x] Replay tests run in CI alongside DuckDB's

**First planned non-DuckDB client: pg-tide-relay**

Known extensions required (all within or trivially near the bounded set):
- `ORDER BY ... ASC LIMIT 1` on `ducklake_snapshot`
- `SELECT max(snapshot_id) FROM ducklake_snapshot WHERE snapshot_id > $1`
- Parameterized `LIMIT $1` on data-file SELECT
- `gen_random_uuid()` in INSERT VALUES (or client-generated UUIDs as literal parameters — preferred)
- `INSERT INTO ducklake_metadata` / `SELECT value FROM ducklake_metadata WHERE metadata_key = $1` (offset tracking — already in DuckDB corpus)

**Application metadata key namespace.** Document and enforce the dotted-prefix convention for non-DuckDB client application state:
```
{application}.{instance}.{key}  →  stored in ducklake_metadata, scope = global
e.g. pg_tide.orders-to-lake.offset  →  "4782"
```
Multiple applications can coexist by using distinct prefixes. Application keys participate in snapshot transactions, enabling exactly-once semantics for streaming pipelines.

### GCS and Azure Validation

- [x] Run full acceptance test suite against Google Cloud Storage and Azure Blob Storage
- [x] Verify writer fencing, manifest updates, and conditional initialization on each backend
- [x] Add to `docs/compatibility.md`

### DuckDB Compatibility Matrix Maintenance

- [x] CI runs wire-corpus replay on every DuckDB patch release
- [x] Minor version bumps: new corpus capture + sign-off before version added to matrix
- [x] Major version bumps: full re-capture, new corpus fixture, explicit compatibility review

### Security Hardening

- [x] IAM separation tests: catalog-only role vs. data-only role; verify expected `SQLSTATE 42501` failures
- [x] TLS support for `slateduck serve` (`--tls-cert`, `--tls-key`)
- [x] Authentication: PostgreSQL `md5` / `scram-sha-256` password auth for sidecar connections
- [x] Audit log: write a structured log entry for every snapshot commit (who, when, what changed)

### Deliverables

- [x] pg-tide-relay corpus captured and replay tests green in CI
- [x] All category-b dispatcher extensions behind feature flags with replay coverage
- [x] GCS and Azure acceptance tests green; `docs/compatibility.md` updated
- [x] TLS and password auth working for `slateduck serve`
- [x] Audit log entries verified for every snapshot commit
- [x] DuckDB compatibility matrix CI running on patch releases

---

## v0.7 — Performance & Ecosystem

> Optimize catalog hot paths, introduce multi-writer partitioning, and expose the catalog to DataFusion.

### Performance Optimization

Profile and optimize the catalog hot paths. All optimizations compare against the `benchmarks/phase-2-baseline.json` established in v0.2.

**Target: within 2–3× of PostgreSQL on common DuckLake planning queries.**

**Secondary indexes.** Add skip-index keys for MVCC-heavy scans:
```
e.g. (snapshot_id, table_id) → data_file_id for snapshot-scoped file lookups
```
Add a secondary index only when profiling shows MVCC filter overhead exceeds 10× amplification on the reference workload.

**Packing.** Store all small per-table metadata — columns, partitions, sort info — as one composite value per table. A single point read pulls everything needed to plan a query.

**Hot key.** Persist the current snapshot ID and per-table file count under a dedicated hot key so a cold DuckDB process can resume in a single `GET`.

**SlateDB tuning.** Evaluate `Settings` for:
- Block size
- Bloom filters
- On-disk block cache (SSD)
- `l0_sst_count_threshold` for update-heavy workloads
- Levelled compaction aggressiveness

**LSM tombstone management.** The `UPDATE SET end_snapshot` pattern generates a new SST entry per retired version that masks the old value until compaction merges them. This is normal LSM behavior and does not violate catalog-data immutability — the catalog row still exists with `end_snapshot` set. Tune compaction to merge dead LSM entries earlier for high-ingest workloads. Physical deletion of catalog *keys* happens only through `slateduck excise`, not through compaction tuning.

**Initial benchmark suite.** Compare SlateDuck against the phase-2 baseline and SQLite-backed DuckLake:
- `list_data_files` at 10⁴, 10⁵ files
- `create_snapshot` at 1, 10, 100 file additions
- Cold-start read latency from a fresh process
- p50/p95/p99 for all operations on LocalFS and S3 Standard

### Multi-Writer via Catalog Partitioning

SlateDB is single-writer per database, and DuckLake is single-writer per catalog. However, SlateDuck can offer a pattern of "one SlateDB catalog per dataset" with a thin global registry, exploiting SlateDB's cheap database creation:

- Global registry catalog: maps logical dataset names to their catalog paths
- Each dataset gets its own isolated SlateDB-backed catalog
- Writers shard across datasets with no cross-dataset contention
- The global registry itself is a SlateDuck catalog, providing a queryable inventory

### DataFusion Integration

Expose `slateduck-catalog` to DataFusion's [`datafusion-ducklake`](https://github.com/datafusion-contrib/datafusion-ducklake) via Rust trait implementation:
- Both are Rust crates, so integration avoids FFI entirely
- Implement DataFusion's `CatalogProvider` trait backed by `CatalogStore`
- Enables DataFusion users to run SQL against a SlateDuck-backed lakehouse without DuckDB

### Deliverables

- [x] Hot-key cold-start optimization implemented and measured
- [x] Secondary indexes added where profiling shows ≥ 10× MVCC amplification
- [x] Initial benchmark report: p50/p95/p99 vs. phase-2 baseline and SQLite-backed DuckLake
- [x] Multi-writer partitioning pattern documented with example architecture and tested with multiple concurrent dataset writers
- [x] DataFusion integration passing DuckLake tutorial equivalence tests

---

## v0.8 — Documentation

> Publish a complete documentation site that explains every aspect of SlateDuck — architecture, design decisions, trade-offs, deployment, operations, and integration — to the same standard as the engineering.

The full specification for this release is in [plans/documentation-1.md](plans/documentation-1.md). That document contains the complete `mkdocs.yml` configuration, the GitHub Actions workflow, rich per-page content plans for all 80 pages, the writing style guide, and the quality gates. This section is the binding roadmap summary: scope, rationale, and deliverables.

A project that handles production data — data that operators have stored in S3, annotated with schemas, and exposed to DuckDB for business queries — owes its users documentation that is accurate, complete, and honest. Operators who encounter a limitation undocumented will stop trusting the documentation. Engineers evaluating SlateDuck for adoption will look first at the Design Decisions section to understand what trade-offs were made and why; if that section is thin or evasive, the evaluation ends. Contributors who want to improve the codebase need an accurate map of the architecture before they can make safe changes. v0.8 provides all of this. It is not a stretch goal or a nice-to-have: without documentation, the software is incomplete.

### Technology Stack

The documentation site is built with [MkDocs](https://www.mkdocs.org/) and the [Material for MkDocs](https://squidfundinglab.github.io/mkdocs-material/) theme. Material was chosen over alternatives (Docusaurus, Hugo, Sphinx) because it is maintained by a team that treats technical documentation as a first-class product, offers the best support for multi-cloud tabbed configurations and Mermaid diagrams, and ships a polished search experience that works entirely in the browser without a backend. The theme's dark-mode support and mobile responsiveness are production-grade and require no custom CSS to work correctly.

The plugin set is chosen for specific needs: `git-revision-date-localized` surfaces a "last updated" timestamp on every page so readers can see at a glance whether content is current; `social` generates Open Graph preview cards for GitHub and social sharing with no manual effort; `glightbox` makes architecture diagrams lightbox-expandable so readers can study the detail without leaving the page; `redirects` enables stable external links even when internal page paths change between minor documentation reorganizations; `minify` reduces page weight for readers on mobile connections. All plugins are pinned in `requirements-docs.txt` so CI builds are reproducible and upgrades are intentional. The complete `mkdocs.yml` including the full navigation tree, all extension settings, and the palette configuration is in `plans/documentation-1.md` and is used verbatim.

### GitHub Actions Workflow

A dedicated `.github/workflows/docs.yml` workflow handles both build verification and deployment. The build job runs on every push to `main` (path-filtered to `docs/**` and `mkdocs.yml`) and on every pull request, using `mkdocs build --strict` which turns broken internal links, missing navigation entries, and malformed extension directives into hard failures rather than warnings. This means a PR that introduces a broken cross-reference cannot be merged without fixing it — documentation quality is a CI gate, not a post-merge cleanup task.

The deploy job runs only on push to `main` after the build job succeeds. It uploads the built `site/` directory as a GitHub Actions artifact and deploys it to GitHub Pages via `actions/deploy-pages`. Concurrency is set to cancel in-progress runs on the same branch, so a rapid sequence of commits does not queue up redundant deploys. The full workflow YAML is in `plans/documentation-1.md`.

### Documentation Structure

Eighty content pages organized into 13 top-level sections, each serving a distinct audience with a distinct purpose. The structure reflects the reality that a documentation site has several types of readers who arrive with different questions: new users who want to get something running; evaluators who want to understand the design well enough to make an adoption decision; operators who need practical deployment and operational guidance; engineers who want to contribute or extend the system.

| Section | Pages | Audience |
|---------|-------|----------|
| Getting Started | 4 | New users: zero to working lakehouse in 5 minutes |
| Concepts | 9 | Evaluators: deep understanding of what and why |
| Architecture | 9 | Engineers: how it works at the code level |
| Deployment | 11 | Operators: every supported backend with copy-paste configs |
| Operations | 12 | Day-2 operators: CLI reference, GC, excision, repair, monitoring |
| Integration | 6 | Ecosystem: DuckDB, pg-tide, DataFusion, custom clients |
| Design Decisions | 8 | Architects: honest trade-off analysis for every major choice |
| Performance | 5 | Evaluators: real benchmarks, tuning knobs, workload fit guide |
| Internals | 8 | Contributors: tag allocation, MVCC filter, crash safety |
| Contributing | 5 | Contributors: dev setup, test pyramid, release process |
| Reference | 6 | Quick lookup: tables, SQL shapes, error codes, metrics |
| Roadmap | 2 | Everyone: release timeline, changelog |
| Landing page | 1 | First impression: pitch, architecture, comparison |

### Content Requirements

Every section must meet a defined content bar before the release is considered complete. The bar varies by section type: conceptual pages require flowing prose and honest argument; reference pages require complete coverage and scannability; operational pages require working examples that have been run against the actual binary.

**Getting Started and Concepts** are written as flowing technical essays — longer paragraphs that develop ideas fully and build intuition, not bullet-list summaries that defer the reader to another source. Every claim links to the deeper material that substantiates it. Trade-offs are stated honestly from the first page: a reader who reaches the Concepts section to understand MVCC or the single-writer constraint should come away with a complete picture of both the benefits and the costs, not a sales pitch followed by fine print.

**Architecture** pages include Mermaid sequence diagrams for both the read path and the write path, a dependency graph of the six crates, and annotated source references pointing into the codebase where relevant. The goal is to let a contributor who has just cloned the repository understand how a DuckDB query flows from `ATTACH` through the pg-wire sidecar, into the SQL dispatcher, through `slateduck-catalog`, into `slateduck-core`, and down to SlateDB — without having to trace the code cold.

**Deployment** pages are self-contained: a reader following any single cloud-provider page should need nothing outside that page to stand up a working deployment. Each page includes IAM permission templates, a working `slateduck serve` invocation, a DuckDB `ATTACH` snippet, and a verification query. Tabbed sections present the AWS, GCS, and Azure variants side-by-side so operators can compare object-storage provider requirements without jumping between pages. MinIO is documented as a first-class local/on-prem deployment path alongside the major cloud providers.

**Design Decisions** is the most important section in the site. Each of the eight pages addresses a major architectural choice — why SlateDB over PostgreSQL or SQLite; why Strategy B (pg-wire sidecar) precedes Strategy C (native extension); why bounded SQL over a general query engine; why Protobuf for value encoding; the full cost-benefit analysis of catalog immutability; the single-writer model and its practical workarounds; the rationale behind the key layout for all 28 catalog tables; and an explicit "What SlateDuck Is Not" page that articulates the workloads and use cases for which SlateDuck is the wrong choice. These pages require the most care because they must present both sides honestly: what was chosen, what was rejected, and the real reasons — not a post-hoc rationalization of decisions that were made for simpler reasons. Readers who disagree with a design decision should be able to look at this section, understand the full reasoning, and form an informed opinion. That requires honesty about costs, not just advocacy for the choice made.

**Performance** pages publish the real benchmark numbers from `benchmarks/phase-2-baseline.json` and subsequent runs, with methodology clearly documented. The "vs. Alternatives" page provides a direct, honest comparison table against PostgreSQL-backed and SQLite-backed DuckLake, including the conditions under which SlateDuck is slower (cold-start read latency from S3 is higher than PostgreSQL in the same region) and the conditions under which it is faster or equivalent (write throughput under high fan-out ingest; zero-config deployment cost).

**Reference** pages are scannable lookup tables: all 28 catalog tables documented in tabular form with column types and semantics; every supported SQL shape with parameter types and return types; every SQLSTATE code with its triggering condition and recommended resolution; every exported Prometheus metric with its labels and type; all environment variables and configuration file keys.

### Writing Style

The documentation leads with the "why" on every page. A reader who does not understand in the first paragraph why this page matters and what question it answers will not read further. This applies equally to a reference page listing CLI flags (why is this command useful? When would an operator reach for it?) and a design decision page (what was the question this decision answered? Why did it matter?). The "why" is not an optional introduction — it is the first sentence of the page.

Narrative sections in Getting Started, Concepts, and Design Decisions use longer paragraphs that develop ideas fully. The reader is presumed to be intelligent and to have come to the page with a real question; they deserve a complete answer, not a summary that tells them to look elsewhere. Bullet lists are reserved for genuinely enumerable items — a list of supported object-storage backends, a list of CLI flags, a list of SQLSTATE codes. An idea that requires explanation gets a paragraph, not a bullet.

Every limitation and trade-off is stated plainly. A reader who discovers an undocumented limitation in production will lose trust in the documentation for all future interactions. A reader who finds the limitation documented, understood, and accompanied by a workaround will keep trusting the documentation. Honesty is not just an ethical commitment here — it is a strategic one. Code examples in every section have been run against the actual `slateduck` binary and produce exactly the output shown; an example that has not been verified should not be in the documentation.

### Implementation Phases

The work is divided into seven sequential phases over approximately 35 days. Each phase has a clear definition of done, and each phase's output is the foundation for the next phase, so phases must be completed in order. The full per-phase content plan — including which specific pages are written in which order and what "done" looks like for each — is in `plans/documentation-1.md`.

| Phase | Work | Days |
|-------|------|------|
| D1 — Scaffolding | `mkdocs.yml`, directory structure, GitHub Actions workflow, `requirements-docs.txt`, section stubs | 1–2 |
| D2 — Getting Started & Landing | Landing page, what-is, quickstart (local + cloud), first-lakehouse tutorial | 3–5 |
| D3 — Concepts & Architecture | All 9 concepts pages, all 9 architecture pages, Mermaid diagrams | 6–12 |
| D4 — Deployment & Operations | All 11 deployment guides, all 12 operations pages | 13–19 |
| D5 — Integration & Design Decisions | All 6 integration pages, all 8 design-decision pages | 20–24 |
| D6 — Performance, Internals, Reference | All 5 performance pages, all 8 internals pages, all 6 reference pages | 25–30 |
| D7 — Contributing, Roadmap, Polish | Contributing, roadmap, changelog, cross-link audit, `mkdocs build --strict` clean | 31–35 |

Phase D2 is intentionally the highest-stakes phase despite being one of the shortest: the Getting Started pages are the highest-traffic pages on the site and set the tone for the reader's entire relationship with the project. Writing these pages first forces the author to articulate the core value proposition clearly, and that articulation should inform the language used throughout the rest of the documentation.

Phase D5 is the most intellectually demanding: the Design Decisions pages require sustained honest argument, not just description, and the Integration pages require end-to-end testing against real DuckDB versions to verify the compatibility claims.

Phase D7's polish step is not optional. Running `mkdocs build --strict` to zero warnings, auditing every cross-link, verifying the top 20 user-searchable terms, and reviewing every page on a mobile viewport are the difference between a documentation site that looks finished and one that actually is.

### Quality Gates

The release is complete when all of the following are true:

- [x] `mkdocs build --strict` produces zero warnings on CI — the `--strict` flag treats broken links and misconfigured extensions as build failures, not warnings; zero warnings means zero known defects
- [x] No broken internal or external links — internal links verified by `--strict`; external links spot-checked before publish and on each subsequent update
- [x] No stub pages remain in the published site — a page with only a title and no real content is misinformation, worse than a missing page
- [x] Every `bash` and `sql` code block has been run against the actual binary and produces the output shown
- [x] The top 20 terms a new user would search for return relevant results — assembled by asking multiple people "what would you search for?" before the polish phase
- [x] All pages render correctly on mobile viewports — particularly important for wide tables in the Reference section and Mermaid diagrams that can overflow their container
- [x] Heading hierarchy is correct on every page; images have meaningful alt text; body text contrast meets WCAG AA in both light and dark mode
- [x] At least one reviewer other than the primary author has read every Getting Started and Concepts page — a second reader finds the gaps the author is blind to

### Maintenance Contract

Once published, the documentation becomes a first-class project artifact, not a snapshot. It requires the same attention as the code: changes to observable behavior require corresponding documentation updates; undocumented changes are documentation bugs introduced deliberately, and the review process treats them as such.

The PR template includes a checkbox — "Documentation updated (if behavior changed)" — that gates every merge affecting observable behavior. New CLI commands get a CLI Reference entry before the release is tagged. New Prometheus metrics get a Metrics reference entry before the release is tagged; monitoring setups break silently when metrics disappear or are renamed, and an operator whose dashboard stops working because a metric was renamed without documentation is a preventable failure. New error codes get an Error Codes entry before the release is tagged, so an operator who sees an unfamiliar SQLSTATE can look it up immediately. Benchmark results are refreshed with each performance-relevant release, and the `git-revision-date-localized` plugin surfaces the last-updated date on every page so readers can see at a glance whether performance numbers are current. The DuckDB compatibility matrix is updated within two weeks of a new DuckDB release so the version matrix is never stale when operators are evaluating an upgrade.

### Deliverables

- [x] `mkdocs.yml` at workspace root with full configuration from `plans/documentation-1.md`
- [x] `.github/workflows/docs.yml` building and deploying to GitHub Pages on every push to `main`
- [x] `requirements-docs.txt` pinning all documentation dependencies
- [x] All 80 content pages published with complete, reviewed content — no stubs
- [x] Mermaid architecture diagrams for system overview, crate dependency graph, read path, and write path
- [x] All 28 catalog tables documented in the Reference section
- [x] Full CLI reference covering every `slateduck` subcommand
- [x] Performance comparison page with real benchmark data from `benchmarks/phase-2-baseline.json`
- [x] Design Decisions section covering all 8 major architectural choices with honest trade-off analysis
- [x] DuckDB compatibility matrix with verified version coverage
- [x] Documentation site live at GitHub Pages URL
- [x] `mkdocs build --strict` green in CI

---

## v0.9 — Production Readiness

> Kubernetes deployment architecture, writer routing and failover, credential separation, pre-benchmark performance tuning, cost analysis tooling, catalog migration subcommand, and wire-corpus validation — everything needed to run SlateDuck confidently in production before the v1.0 GA benchmark sign-off.

### Pre-Benchmark Performance Tuning

Before the formal TPC-H benchmarks in v1.0, apply targeted optimizations based on profiling. All changes compare against `benchmarks/phase-2-baseline.json` and the v0.7 benchmark results.

**FlatBuffers evaluation.** The v0.2 decision to use Protobuf for value encoding was correct for correctness and schema evolution; FlatBuffers was deferred as a Phase 7 performance candidate. In v0.9, run a decode-overhead microbenchmark for the five highest-frequency row types (`ducklake_data_file`, `ducklake_file_column_stats`, `ducklake_column`, `ducklake_table`, `ducklake_snapshot`) across a cold-cache read of 10⁵ rows. If FlatBuffers reduces total decode overhead by more than 15% end-to-end and migration risk is contained, schedule the encoding migration gated behind a new `encoding_version` byte. If the savings are smaller, close this item and document the result in `docs/design-decisions/value-encoding.md`. The `encoding_version` byte means migration is forward-safe without a `catalog-format-version` bump.

**Zone-map readiness decision.** Profile `list_data_files` at 10⁵ and 10⁶ files using the exact-stats key layout from v0.2. If MVCC filter amplification exceeds 10× live-rows-returned on the reference workload, schedule the coarse zone-map index (full algorithm is in v1.x) for implementation in this release; otherwise defer to v1.x. Document the measurement and the decision in `docs/performance/pruning.md` so the v1.x team has a quantitative basis.

**Block cache sizing guidance.** Add `slateduck inspect cache-utilization` that reports hit/miss ratio, eviction rate, and a recommended `--cache-size-mb` value based on the catalog's observed working-set size. Document the rule of thumb: a block cache sized to hold the last 30 days of active file stats reduces `list_data_files` latency to near-PostgreSQL levels even on S3 Standard.

**On-disk cache persistence across pod restarts.** Test and document the `--cache-path` option for mounting a persistent volume for the SlateDB block cache in Kubernetes so a pod restarted on the same node retains its warm cache. Add a startup-time metric `slateduck_cache_warmup_hit_ratio` for cache-hit ratio on the first 100 reads so operators can verify whether the persisted cache is being loaded correctly.

**SlateDB compaction tuning for the `end_snapshot` update pattern.** Every `DROP TABLE` or `ALTER TABLE` emits one `put(key, updated_value)` call that masks the previous SST entry until compaction merges them. For high-ingest workloads this accumulates dead entries in L0. Tune `l0_sst_count_threshold` to trigger compaction earlier and measure whether it reduces `list_data_files` scan amplification. Document the recommended value and the trade-off against write amplification in `docs/performance/slatedb-tuning.md`.

### Deployment Architecture and Kubernetes Operations

The SlateDuck process is almost entirely stateless: all correctness-critical state lives in object storage, and the process can be killed and recreated at any time without data loss or manual recovery.

| State | Location | Lost on crash? |
|-------|----------|----------------|
| Catalog rows (all 28 tables) | S3 — SlateDB SSTs | No |
| Write-ahead log | S3 — SlateDB WAL | No (recovered on restart) |
| Manifest | S3 | No |
| Checkpoints | S3 | No |
| In-memory MemTable (recent writes) | RAM | Yes — but WAL recovers these |
| Block cache (read acceleration) | RAM / local SSD | Yes — automatically rebuilt |

#### Kubernetes Deployment Patterns

Three patterns cover the range from simple horizontal scale-out to automatic failover:

**Pattern 1 — Read replicas (horizontal scale for reads)**

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: slateduck-reader
spec:
  replicas: 3           # freely scalable; each pod is independent and stateless
  template:
    spec:
      containers:
      - name: slateduck
        args: ["serve", "--mode=reader", "--catalog=s3://bucket/cat"]
```

Every pod reads from the same object-store catalog with no coordination. Suitable for read-only or append-only workloads where catalog writes are infrequent.

**Pattern 2 — Single writer + read replicas (recommended for most deployments)**

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

The writer and readers are separate Deployments. Scaling the reader Deployment does not affect the writer. SlateDB's fencing ensures that if the writer pod is replaced, the old pod cannot commit after the new pod takes over.

**Pattern 3 — Writer election for automatic failover**

Deploy N replicas as a `StatefulSet` with Kubernetes `Lease`-based leader election. The pod holding the lease runs in `--mode=writer`; all others run in `--mode=reader`. If the writer crashes or becomes unreachable, another pod acquires the lease and calls the safe takeover protocol (`flush()` before accepting client connections). SlateDB's fencing enforces that the old writer cannot commit after the new writer's first successful `flush()`. Document the expected failover window (see Writer Failover SLOs below) and the exact K8s RBAC permissions required for `Lease` acquisition.

#### Writer Routing Patterns

Because only the writer replica accepts catalog mutations, clients and load balancers must route writes correctly. Four options are available, in ascending order of infrastructure complexity:

**Option A — PostgreSQL `target_session_attrs` (zero infrastructure)**

```
host=pod-a,pod-b,pod-c port=5432 target_session_attrs=read-write
```

The libpq client (used by DuckDB's `postgres` extension) tries each host until one accepts writes. A reader pod responds to write attempts with `SQLSTATE 25006` (`read_only_sql_transaction`); libpq automatically tries the next host. No proxy, no label updates, no service discovery. The only requirement is that all pod addresses are listed in the connection string.

**Option B — Writer self-publishes in the catalog (recommended for production)**

When a pod acquires the writer role it writes its own network address into two keys in the same atomic SlateDB transaction as the fencing epoch update:

```
0xFF | "writer-epoch"    → u64 epoch
0xFF | "writer-endpoint" → "pod-a.slateduck.svc.cluster.local:5432"
```

These two keys are always consistent because they are written atomically. Any replica that receives a write request performs a single `get("writer-endpoint")` lookup and forwards the TCP connection. The address is cached until a write attempt fails with `SQLSTATE 57P04` (writer fenced), at which point the replica re-reads the key to discover the new writer's address. No external dependencies; the catalog is its own service directory. This is already planned in the `0xFF` key layout and must be implemented as part of the writer startup sequence.

**Option C — Kubernetes label selector**

The writer pod labels itself `slateduck-role=writer`. A dedicated K8s `Service` uses a label selector targeting only that label. When a pod takes over the writer role it patches its own labels via the Kubernetes API; the Service endpoint list updates in under one second via the standard endpoint controller. Requires that the pod's ServiceAccount has `patch` permission on `pods`. The label selector pattern is simple and well-understood but requires K8s API access from the pod and has a one-second propagation window where the old label may still be present.

**Option D — Protocol-aware proxy**

A stateless SlateDuck proxy `Deployment` (multiple replicas, behind a standard K8s Service) sits in front of all writer and reader pods. For each incoming SQL statement it uses `sqlparser-rs` to classify the statement as read or write in under 1 ms, then routes reads round-robin to reader pods and writes to the current writer (located via Option A, B, or C). Because the proxy is stateless it scales freely, adds no single point of failure, and adds no more than 2 ms overhead per request. Use this when clients cannot handle `SQLSTATE 25006` retry logic — for example, when integrating a third-party DuckLake client that does not use libpq.

**Recommended layering.** Start with Option A (free, works immediately). Add Option B as part of the core catalog implementation — it is already specified in the `0xFF` key layout and adds no new dependencies. Introduce Option D only when a specific client cannot tolerate `25006` retries.

#### Cold-Start and Cache Warming

When a fresh pod starts it has an empty block cache; the first few catalog reads pay full S3 round-trip latency. Three mitigations must be documented and tested:

- **Persistent volume cache.** Mount a `PersistentVolumeClaim` for `--cache-path=/mnt/cache --cache-size-mb=2048`. The cache survives pod restarts on the same node. Document the `storageClassName` requirements (local SSD preferred; network volumes acceptable but slower).
- **Init container warm-up.** Add a `slateduck warmup --tables 20` init container that reads the current snapshot and the N most recently active table metadata entries before the serving container starts. Implement `slateduck warmup` as a CLI subcommand that exits 0 when warm-up is complete.
- **DuckDB client-side caching.** DuckDB caches the current snapshot ID between queries; for long-lived DuckDB processes the cold-start overhead is paid at most once per session. Document this behavior and its implications in `docs/concepts/mvcc.md`.

Add a startup metric `slateduck_cache_warmup_hit_ratio` (0.0–1.0) that measures the cache hit rate for the first 100 reads after process start. An operator alert on this metric below 0.5 can catch accidental cache eviction or misrouted pods.

#### Multi-Tenancy and Path Layout

Multiple independent DuckLake instances sharing the same S3 bucket require a standardized path layout locked in before any path strings appear in the codebase. The `CatalogPath` struct (in `slateduck-core`) encapsulates all path segments as typed fields; raw string concatenation for object-store paths is forbidden.

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

The v1 connection URL conventions:

```
# Strategy B (PG-wire sidecar)
ducklake:postgres:host=slateduck-writer catalog=warehouse-a

# Strategy C (native extension)
ducklake:slatedb:s3://my-bucket/catalogs/warehouse-a
```

Each catalog is an isolated SlateDB `Db` at a distinct path prefix. Two catalogs must never share a `Db` path even if they would use disjoint tag ranges — the WAL, manifest, and compaction pipeline are shared at the path level and tag-range isolation is not enforced at the storage layer.

#### Credential Separation in Kubernetes

Three distinct credential planes must be documented with IAM policy templates for AWS (IRSA), GCP (Workload Identity), and Azure (AKS Workload Identity):

| Workload | Identity | Required access |
|---------|----------|----------------|
| `slateduck-writer` / `slateduck-reader` | Catalog ServiceAccount | SlateDB catalog prefix only: `s3://bucket/catalogs/**` |
| DuckDB ingestion / query jobs | Data ServiceAccount | Parquet data/delete-file prefix only: `s3://bucket/data/**` |
| `slateduck gc` / `slateduck excise` | Maintenance ServiceAccount | Read catalog + conditionally delete data files |
| `slateduck checkpoint` | Backup ServiceAccount | Read catalog + write checkpoint prefix |

The sidecar must not be given data-plane credentials by default. A sidecar that accidentally receives the data role should fail catalog startup with `SQLSTATE 42501` rather than silently operating with incorrect permissions. Add a startup credential validation check: attempt a catalog-prefix write and a data-prefix write at process start; the former must succeed and the latter must fail for the catalog credentials to be considered correct.

The MinIO credential-isolation tests established in v0.1 are the local proof of this contract. The v0.9 acceptance suite must run equivalent tests against real AWS IAM policies.

#### Writer Failover SLOs

Acceptance tests must verify the following across all target backends:

| Backend | Failover SLO | Measurement |
|---------|--------------|-------------|
| LocalFS | < 5 seconds | From `kill -9` to new writer accepting writes |
| MinIO | < 10 seconds | Same |
| S3 Standard | < 30 seconds | Same |
| S3 Express One Zone | < 10 seconds | Same |

Each SLO is tested by: (1) starting a writer and writing 10 snapshots; (2) sending `SIGKILL`; (3) starting a second writer immediately; (4) measuring time until the second writer returns success for a new `create_snapshot` call; (5) verifying all 10 pre-kill snapshots are visible to a `DbReader` opened after the second writer's `flush()`.

### S3 API Cost Analysis and Cost-Mode Configuration

One of the open questions in [plans/blueprint.md §12](plans/blueprint.md) is: at what scale does SlateDB's WAL write cost become significant compared to PostgreSQL hosting cost? v0.9 provides concrete tooling and documented answers.

- `slateduck inspect api-costs [--estimate-monthly]` — emit a report of observed S3 API call counts per catalog operation category (PUT, GET, LIST), their estimated monthly cost at standard S3 pricing, and the equivalent RDS `db.t4g.medium` hourly cost. The report enables an operator to determine the crossover point for their specific ingest rate.
- `slateduck inspect api-costs --compare-postgres --rds-instance db.t3.medium --region us-east-1` — fetch the current AWS pricing API for the specified RDS instance and emit a side-by-side cost comparison at the catalog's current ingest rate. Requires IAM permission to call `pricing:GetProducts`.
- `slateduck inspect api-costs --stream` — run continuously (one report per minute) and output a time-series of API call rates. This enables operators to see cost spikes during burst ingest and tune buffer/compaction settings without waiting for a monthly invoice.
- Document the cost crossover point (estimated and measured) in `docs/performance/cost-analysis.md`. Include a worked example: at 100 Parquet files/minute registered, what is the monthly S3 API cost vs. a `db.t3.medium` RDS instance in the same region?
- `slateduck tune --target-cost-usd-per-month N` — output recommended settings (`--cache-size-mb`, `l0_sst_count_threshold`, compaction mode) that reduce API call volume toward the target cost envelope without degrading p99 latency by more than 50%.

**Cost-mode configuration flag.** Add a `--cost-mode` flag with three named presets to make the cost/latency trade-off accessible without requiring operators to understand SlateDB `Settings` internals:

| Mode | Profile | Use case |
|------|---------|----------|
| `conservative` | Larger memtable, lower L0 flush frequency, fewer S3 PUTs | Cost-sensitive workloads; accepts higher p99 write latency |
| `balanced` (default) | Tuned for the TPC-H SF10 benchmark workload | General-purpose production |
| `latency` | Frequent flushes, aggressive compaction, more S3 API calls | Interactive analyst workloads on S3 Express |

Document the measured cost and latency profile for each mode in `docs/performance/cost-analysis.md`.

### Catalog Migration and Corpus Tooling

**`slateduck migrate` subcommand.** Automates the `export → reinitialize-at-new-format-version → import` sequence for forward-incompatible `catalog-format-version` bumps. Includes a `--dry-run` mode that reports the number of rows to migrate and estimated duration without making changes.

**`slateduck corpus diff`.** Compare two wire-corpus fixture files and emit a structured diff of all statement families, handshake probes, and type OID requests that changed between versions:

```
slateduck corpus diff \
  --old tests/fixtures/wire-corpus/duckdb-1.x.jsonl \
  --new tests/fixtures/wire-corpus/duckdb-2.x.jsonl
```

Groups changes into: removed, added, modified parameter types, modified result columns.

**`slateduck corpus validate`.** Replay a corpus fixture file against the current dispatcher and report which statement families are already handled, which need dispatcher updates (category-b), and which require new SQL operator types (category-c):

```
slateduck corpus validate --corpus tests/fixtures/wire-corpus/duckdb-2.x.jsonl
```

**CI workflow for corpus PRs.** On any PR that updates a `wire-corpus/*.jsonl` file, automatically run `corpus diff` and `corpus validate` and post the results as a PR comment. A major-version DuckDB upgrade requires two reviewers and an explicit sign-off on any category-c items.

### Deliverables

- [x] All three K8s deployment patterns (Patterns 1–3) with tested manifests in `docs/deployment/kubernetes.md`
- [x] All four writer routing options (A–D) documented; Options A and B tested with integration tests
- [x] Writer failover SLOs verified for LocalFS, MinIO, S3 Standard, and S3 Express
- [x] IAM policy templates for AWS, GCP, and Azure in `docs/deployment/credential-isolation.md`; acceptance tests against real AWS IAM policies
- [x] `slateduck warmup` CLI subcommand shipping in the binary; init-container example in `docs/deployment/kubernetes.md`
- [x] `slateduck inspect api-costs` (with `--estimate-monthly`, `--compare-postgres`, `--stream`), `slateduck tune`, and `--cost-mode` flag shipped
- [x] Cost analysis and cost mode documentation in `docs/performance/cost-analysis.md`
- [x] `slateduck inspect cache-utilization` shipped; block cache sizing guide in `docs/performance/slatedb-tuning.md`
- [x] FlatBuffers evaluation complete; result documented in `docs/design-decisions/value-encoding.md`
- [x] Zone-map readiness decision documented with profiling data in `docs/performance/pruning.md`
- [x] Compaction tuning documented in `docs/performance/slatedb-tuning.md`
- [x] `slateduck migrate` subcommand tested with dry-run and apply modes on a v0.x catalog
- [x] `slateduck corpus diff` and `slateduck corpus validate` subcommands shipping in the binary
- [x] CI workflow for corpus PRs deployed and verified on a test corpus update

---

## v0.9.1 — Write Protocol Correctness

> Close the critical MVCC correctness gaps identified in `plans/overall-assessment-1.md`: stale in-memory counters enabling ID reuse, non-atomic catalog mutations, and faulty `UPDATE end_snapshot` key resolution. These issues undermine every correctness property the project claims.

### Counter State and Read-Latest Correctness (F-01)

`CatalogStore::begin_write()` clones counters into a `CatalogWriter` and never synchronises them back on commit. `read_latest()` returns stale snapshot IDs from the same uncorrected cache. PG-Wire sessions that each create a writer via `execute_commit()` can reuse `snapshot_id`, `catalog_id`, and `file_id`.

- [x] Introduce `CatalogStore::commit_writer(writer)` that updates in-memory counters from the committed writer after every successful SlateDB transaction
- [x] Make `read_latest()` derive its snapshot ID from the authoritative counter, not a stale in-memory copy
- [x] Add regression tests for sequential `begin_write()` calls on one store: IDs must be monotonically increasing across sessions
- [x] Add regression tests for `read_latest()` after every commit: must return the just-committed snapshot ID
- [x] Add PG-Wire-level regression tests for `SELECT max(snapshot)` after multiple write sessions on one connection

### Atomic Snapshot Publication (F-02)

Each writer operation commits its own SlateDB transaction using the current `peek_snapshot_id()`. The snapshot row is committed separately by `create_snapshot()`. A failure between any mutation and the matching `create_snapshot()` leaves unpublished rows that a later snapshot can inadvertently publish.

- [x] Stage all catalog row writes in memory within a single logical writer transaction
- [x] Commit all row writes, counter updates, and the snapshot row in one atomic SlateDB transaction inside `create_snapshot()`
- [x] Remove or clearly mark as internal-only any public writer methods that commit individual rows without a snapshot
- [x] Add tests for simulated mid-write failures: verify no phantom rows appear in subsequent snapshots
- [x] Update the writer API documentation to describe the staging model explicitly

### Fix `UPDATE end_snapshot` Key Resolution (F-04)

`execute_commit()` calls `drop_table(0, entity_id, begin_snapshot)` with a hard-coded `schema_id = 0` and `drop_column(entity_id, entity_id, begin_snapshot)` using the same value for both table ID and column ID.

- [x] Resolve the owning `(schema_id, table_id, begin_snapshot)` tuple by reading the existing row before mutating it for table drops
- [x] Resolve the owning `(table_id, column_id, begin_snapshot)` tuple by reading the existing row before mutating it for column drops
- [x] Add end-to-end PG-Wire tests for `DROP TABLE` and `ALTER TABLE DROP COLUMN` that verify the correct row is marked with `end_snapshot`

### Writer Protocol State-Machine Specification (F-30)

Writer fencing prevents concurrent writers but does not guard against stale in-memory state or non-atomic staging.

- [x] Document the single writer protocol: acquire fencing epoch → load counters from SlateDB → stage mutations in memory → commit rows + snapshot + counters atomically → update in-memory state → emit observability event
- [x] Add a conformance test that verifies no variant of this protocol produces duplicate IDs or unpublished facts under simulated failures
- [x] Document the protocol in `docs/architecture/transaction-model.md`

### Deliverables

- [x] `CatalogStore::begin_write()` and `read_latest()` always reflect post-commit state
- [x] `create_snapshot()` is the sole commit boundary; all mutations are committed atomically with the snapshot row
- [x] `UPDATE end_snapshot` for tables and columns uses correct key resolution
- [x] Sequential write sessions on one `CatalogStore` produce monotonically increasing IDs with no reuse
- [x] PG-Wire `SELECT max(snapshot)` is consistent with committed state after every transaction
- [x] Writer protocol state-machine documented in `docs/architecture/transaction-model.md`

---

## v0.9.2 — Security Enforcement

> Turn every security feature that is configured but not enforced into a real enforcement boundary: authentication bypass, FFI memory safety, CLI/env-var misalignment, and encryption wiring.

### Real PG-Wire Authentication (F-16 / F-03)

`SlateDuckHandler` stores `AuthConfig` but unconditionally uses `NoopStartupHandler`. Any client can connect regardless of configured credentials.

- [x] Implement a `SlateDuckStartupHandler` that enforces cleartext password authentication when `AuthConfig.is_enabled()` is true
- [x] Use constant-time comparison for password verification to prevent timing-based credential inference
- [x] Deny connections that do not supply the configured username; return `SQLSTATE 28P01`
- [x] Add end-to-end tests: correct credentials → `AuthenticationOk`; wrong password → `ErrorResponse 28P01`; missing credentials when auth required → `ErrorResponse 28P01`
- [x] Verify `NoopStartupHandler` behaviour is only present when auth is explicitly disabled

### Fix CLI/Docs/Env-Var Alignment (F-18 / F-12)

Docs advertise `--auth-user` / `SLATEDUCK_AUTH_USER`, `--auth-password` / `SLATEDUCK_AUTH_PASSWORD`, `--tls-required`, and GCS/Azure catalog URLs. Code parses `--username` / `--password`, reads no env vars, has no `--tls-required`, and only resolves `s3://` and local paths.

- [x] Rename CLI flags to `--auth-user` and `--auth-password` to match all documentation
- [x] Read `SLATEDUCK_AUTH_USER` and `SLATEDUCK_AUTH_PASSWORD` environment variables as documented
- [x] Implement `--tls-required` that rejects plaintext connections when TLS is configured
- [x] Implement `gs://` and Azure catalog URL resolution, or mark GCS/Azure docs as planned and update binary help text to reflect actual support
- [x] Implement `--read-only`, `--s3-path-style`, `--s3-endpoint`, and `--metrics-bind` if documented, or remove from docs
- [x] Add a CI smoke test that validates every documented flag is accepted by the binary

### Wire Encryption Into Storage (F-19)

`EncryptionConfig` validates a hex key and `--encryption-key` is parsed by the CLI, but the key is discarded and `CatalogStore::open()` has no encryption option.

- [x] Wire `EncryptionConfig` into `CatalogStore::open()` using SlateDB's block-transformer encryption option
- [x] Add an integration test that writes encrypted and reads back the same data using the same key
- [x] Add a test that opening an encrypted catalog with the wrong key returns a clear error
- [x] Document the encryption model (catalog values encrypted; Parquet data encryption is a separate Parquet-native concern) in `docs/deployment/tls.md` and the CLI reference

### FFI Null and Handle Safety (F-17 / F-08)

Every FFI entrypoint dereferences caller-supplied pointers without null or ownership validation. Invalid C input can cause undefined behaviour.

- [x] Add null checks to every `#[no_mangle] pub extern "C"` function before any dereference
- [x] Add an opaque magic/version field to `SlateduckCatalog` and validate it on every read/write operation
- [x] Return structured error codes rather than undefined behaviour for double-close and invalid handles
- [x] Document the ownership contract for every returned pointer in `include/slateduck.h`
- [x] Add CI sanitizer job (`-Zsanitizer=address,leak`) for the FFI crate
- [x] Add tests for: null URI, null error pointer, null catalog handle, double-close, handle-after-close

### Deliverables

- [x] PG-Wire authentication enforced when `AuthConfig.is_enabled()` is true; verified by end-to-end credential tests
- [x] CLI flags and env-var names match all documentation exactly
- [x] `--tls-required` implemented and tested
- [x] `--encryption-key` wired into `CatalogStore::open()` and covered by round-trip and wrong-key tests
- [x] Every FFI entrypoint null-checks inputs before dereference; sanitizer CI green
- [x] Undocumented or unimplemented features removed from binary help text or clearly labelled as planned with a target version

---

## v0.9.3 — Operational Safety

> Make every operational command safe to invoke in production: enforce the GC visibility floor, require a valid retention floor before excision, redesign checkpoint restore to prevent snapshot ID reuse, validate import input strictly, and ensure `rebuild_catalog()` produces a coherent catalog.

### Enforce Retain-From in Readers (F-05)

`gc_apply()` advances `retain-from`, but `CatalogReader::read_at()` and PG-Wire snapshot reads never consult it. Snapshots below the floor remain readable despite being operationally hidden.

- [x] Read the current `retain-from` value at reader open time (or validate on every `read_at()` call)
- [x] Return `SQLSTATE 22023` (snapshot out of retention window) when a client requests a snapshot below `retain-from`
- [x] Add tests that verify `read_at(hidden_snapshot)` returns the retention error after `gc_apply()`
- [x] Update `docs/concepts/snapshots.md` to document the visibility floor semantics

### Fix Excision Safety at `retain_from == 0` (F-06)

`excise_plan()` sets `is_safe = retain_from >= before_snapshot || retain_from == 0`. The `retain_from == 0` branch permits physical deletion before retention has ever been set, inverting the safety logic.

- [x] Change the safety check to require `retain_from > 0 && retain_from >= before_snapshot`
- [x] Apply the same corrected condition to `excise_plan()` `is_safe` field
- [x] Add a test that `excise_apply()` fails when `retain_from == 0` regardless of `before_snapshot`
- [x] Document the required sequence in `docs/operations/garbage-collection.md`: advance `retain-from` first, then excise

### Fix Checkpoint Restore Snapshot ID Reuse (F-07)

`restore_checkpoint()` only resets `next_snapshot_id` to `checkpoint.snapshot_id + 1`. Facts written after the checkpoint remain in the catalog; new writes reuse post-checkpoint snapshot IDs, creating a split timeline.

- [x] Implement logical restore: write a new snapshot that marks all facts created after `checkpoint.snapshot_id` as ended, hiding post-checkpoint facts from new writes while preserving historical reads
- [x] Guarantee post-restore snapshot IDs are strictly greater than all pre-restore IDs (no reuse)
- [x] Add tests: write facts, checkpoint, write more facts, restore, write new facts — verify pre-checkpoint facts visible, between-checkpoint facts hidden, and post-restore facts visible without ID collisions
- [x] Document the logical restore model in `docs/operations/backup-restore.md`

### Typed Import Validation (F-09)

`import_catalog()` uses `unwrap_or(0)` / `unwrap_or("")` / `unwrap_or(true)` throughout; the hand-rolled base64 decoder silently maps every invalid byte to `0`.

- [x] Replace per-field `serde_json::Value` extraction with typed per-table structs and `serde` deserialization
- [x] Return a structured import error including line number and table name on any field parse failure
- [x] Replace the hand-rolled base64 decoder with the `base64` crate and fail explicitly on invalid input
- [x] Add import tests with deliberately malformed NDJSON: missing required field, wrong type, invalid base64 payload

### Fix `rebuild_catalog()` Missing Table Row (F-10)

`rebuild_catalog()` registers data files with `table_id = 1` but never writes a `TableRow` for table `1`, producing a catalog where data files cannot be reached through table queries.

- [x] Write schema and table rows for each inferred table before registering its data files
- [x] Set `next_catalog_id` and `next_file_id` from actual max IDs, not hard-coded `1` / `file_id`
- [x] Run `verify_catalog()` at the end of `rebuild_catalog()` and return an error if verification fails
- [x] Add a test that rebuild output is queryable through `CatalogReader`: list schemas → list tables → list data files → non-empty results

### Fix Float NaN Comparison in Pruning (F-07 medium)

`compare_floats()` uses `partial_cmp().unwrap_or(Ordering::Equal)`. NaN comparisons return `Equal`, making file pruning non-deterministic.

- [x] Replace `unwrap_or(Ordering::Equal)` with fail-closed behaviour: return `Ordering::Greater` (keep the file) or propagate a `TypeCompareError::NanComparison` variant
- [x] Add tests for NaN in predicate, min-value, and max-value positions

### Fix `pg_migrate()` Unescaped SQL Output (F-08 medium)

SQL strings in `row_to_pg_insert()` are built with `format!("... '{}' ...", value)` without SQL literal escaping.

- [x] Add a `sql_literal_escape(s: &str) -> String` helper that doubles single quotes
- [x] Apply it to every string field in `row_to_pg_insert()`
- [x] Add tests for names containing single quotes and backslashes

### Snapshot Lifecycle State-Machine Specification (F-31)

GC, excision, and checkpoint docs and code do not share a consistent model of when a snapshot is committed, retained, hidden, excised, or restored.

- [x] Write a formal snapshot lifecycle spec in `docs/architecture/transaction-model.md` defining each state and the valid transitions
- [x] Verify every operational command (`gc plan/apply`, `excise plan/apply`, `checkpoint create/restore`, `repair`) respects the spec
- [x] Update operational docs to reference the spec

### Deliverables

- [x] `read_at(snapshot)` returns `SQLSTATE 22023` for snapshots below `retain-from`
- [x] `excise_apply()` rejects invocation when `retain_from == 0` or `retain_from < before_snapshot`
- [x] Checkpoint restore does not reuse snapshot IDs; post-checkpoint facts are hidden, not deleted
- [x] `import_catalog()` returns a typed error with line number and table name for any malformed row; base64 errors return a decode error
- [x] `rebuild_catalog()` produces a catalog that passes `verify_catalog()` and returns non-empty tables and files via `CatalogReader`
- [x] NaN pruning comparisons fail closed (keep file) instead of returning `Equal`
- [x] `pg_migrate()` output correctly escapes single quotes in all string fields
- [x] Snapshot lifecycle state-machine documented in `docs/architecture/transaction-model.md`

---

## v0.9.4 — GA Ready

> Bring the project to GA readiness: unlock concurrent read throughput, add zone-map index if profiling warrants it, onboard production DuckLake clients (Spark, Trino), deliver real DataFusion integration (Parquet scan + pg-wire mode), expose catalog as read-only SQL tables, establish versioning and deprecation policies, document the complete compatibility matrix, expand test coverage to cover the highest-risk scenarios, add CI quality gates, and deliver release automation.

### Unlock PG-Wire Concurrent Reads (F-11 scalability)

PG-Wire holds `Arc<Mutex<CatalogStore>>` across async SlateDB reads, serialising every concurrent session.

- [x] Restructure read paths to clone the `Db` handle or a `CatalogReader` snapshot while holding the mutex, then drop the lock before any async I/O
- [x] Verify that write paths still hold the lock for the minimum required window (counter allocation + commit only)
- [x] Add a concurrency test: N concurrent read-only sessions must not block each other
- [x] Benchmark median read latency before and after; confirm improvement for ≥ 4 concurrent sessions

### Fix `describe_table()` O(n) Table Scan (F-13 performance)

`describe_table(table_id)` scans all `TAG_TABLE` rows because the key layout encodes `(schema_id, table_id, begin_snapshot)` but the caller only has `table_id`.

- [x] Add `schema_id` as a required parameter to `describe_table`, or add a secondary `TAG_TABLE_BY_ID` index keyed by `table_id` alone
- [x] Verify PG-Wire and DataFusion callers can supply schema ID or resolve it from a single point-lookup
- [x] Add a microbenchmark for `describe_table` at 100, 1 000, and 10 000 historical table versions

### DataFusion Sync/Async Bridge (F-14)

`schema_names()` and `table_names()` spawn threads and `block_on()` async operations; if no Tokio runtime is present they silently return empty lists.

- [x] Replace `try_current()` + `thread::spawn` + `block_on` with a stored `tokio::runtime::Handle` or `Arc<Runtime>` inside the provider
- [x] Return an explicit error or log a warning rather than an empty list when the runtime is unavailable
- [x] Add a test verifying both methods return correct results when called from outside an async context

### Coarse Zone-Map Index for Large-Scale Pruning (conditional)

If profiling during v0.9 shows MVCC filter amplification exceeds 10× at 10⁵ files on S3 Standard, implement the zone-map index here to meet the **3× PostgreSQL p99 latency** S3 Express acceptance criterion for v1.0.

**Algorithm (full design in planned v1.x section below):**

1. Divide the value range of each typed column into approximately 100 bins per column per table
2. Write zone-map keys during data file registration: `0x13-zone | table_id_be | column_id_be | stats_bucket_be | data_file_id_be`
3. For `WHERE col >= X AND col <= Y` predicates, compute bin range and scan only zone-map keys in that range
4. Correctness: zone-map result must be a superset of exact-stats result (false positives OK; false negatives are bugs)

**Conditional gate.** Only implement if v0.9 profiling shows amplification >10× and latency projection exceeds 3× PostgreSQL p99. Otherwise defer to v1.x.

- [x] Run v0.9 profiling benchmark: `list_data_files` at 10⁵ files measuring MVCC filter amplification
- [x] If amplification >10×, implement zone-map as above
- [x] Add correctness fuzz test: 10 000 random files, random predicates, verify zone-map superset property
- [x] Add performance test: zone-map scan latency <5% of full exact-stats scan at 10⁶ files
- [x] Verify S3 Express `list_data_files` p99 is within 3× of PostgreSQL after optimization

### Additional DuckLake Clients: Spark and Trino

Onboard the first non-DuckDB production clients using the wire-corpus onboarding process formalized in v0.6.

**Spark-DuckLake:**
- [x] Capture the full SQL corpus from the Spark DuckLake connector against a PostgreSQL-backed DuckLake
- [x] Classify each statement family into category-a (already supported), category-b (trivial extension), or category-c (new operators)
- [x] Add category-b dispatcher extensions behind a feature flag gated on the Spark corpus
- [x] Add replay tests in CI covering all Spark corpus versions
- [x] Update `docs/compatibility.md` with Spark connector version support matrix

**Trino-DuckLake:**
- [x] Same capture-and-classify process for the Trino connector
- [x] Add category-b dispatcher extensions behind a feature flag
- [x] Add replay tests in CI
- [x] Update `docs/compatibility.md` with Trino connector version support matrix

**Acceptance criteria:** All startup probes, SQL shapes, transaction behavior, parameter/result format codes, and generated inlined-table operations captured in `tests/fixtures/wire-corpus/{spark,trino}-{version}.jsonl`; replay tests pass in CI for every captured version; no category-c statements required (if any arise, evaluate case-by-case and document decision).

### DataFusion Parquet Scan Real Implementation (F-15 / F-32)

`TableProvider::scan()` returns `EmptyExec`, silently producing zero rows for all queries. Implement real Parquet reading via DataFusion's native Parquet scanner.

- [x] Integrate DataFusion's built-in Parquet reader to execute scans against actual data files
- [x] Add a test verifying scan results match data files referenced by the catalog
- [x] Document in `docs/integration/datafusion.md` the full scan capability and performance characteristics
- [x] Add performance benchmark: DataFusion scan vs. DuckDB native scan on a TPC-H table

### DataFusion pg-wire Mode

The DataFusion `CatalogProvider` trait implementation from v0.7 exposes the catalog over Rust traits. Add a pg-wire-compatible mode so DataFusion-based query engines can also connect through the sidecar without a direct Rust dependency.

- [x] Add a new `--datafusion-pg-wire` mode flag to `slateduck serve` that listens on a separate port
- [x] When a DataFusion engine connects via pg-wire, treat it as a DuckLake client and dispatch SQL using the same bounded dispatcher
- [x] Add end-to-end test: DataFusion engine connects, runs full DuckLake tutorial queries, produces correct results
- [x] Document in `docs/integration/datafusion.md` the pg-wire mode and connection string format
- [x] Add performance benchmark: DataFusion pg-wire queries vs. native Rust trait queries

### Writer Session and MVCC Regression Tests (F-20)

The existing test suite uses single-writer patterns and only checks non-empty result shapes after failover.

- [x] Add test: two sequential `begin_write()` sessions on one `CatalogStore` produce monotonically increasing, non-overlapping snapshot IDs
- [x] Add test: `read_latest()` after commit returns the committed snapshot ID, not a prior one
- [x] Add test: aborted write session (dropped without `create_snapshot()`) must not expose its mutations in subsequent snapshots
- [x] Add property-based test: any sequence of begin/mutate/commit produces strictly increasing snapshot IDs

### Security Protocol Tests (F-21)

Auth and TLS tests only verify `is_enabled()` on config structs; no real protocol round-trip test exists.

- [x] Add end-to-end test: connect with valid credentials → `AuthenticationOk`
- [x] Add end-to-end test: connect with wrong password → `ErrorResponse 28P01`
- [x] Add end-to-end test: connect with no credentials when auth required → rejection
- [x] Add test: TLS handshake success with a self-signed certificate
- [x] Add test: `--tls-required` rejects a plaintext connection

### FFI and DataFusion Coverage (F-22)

FFI has four basic happy-path tests; DataFusion has five.

- [x] FFI: add tests for null URI, null error pointer, null catalog handle, double-close, and handle-after-close
- [x] FFI: add a test verifying all free functions do not crash on null input
- [x] DataFusion: add test for `schema_names()`/`table_names()` called without a Tokio runtime
- [x] DataFusion: add test for concurrent calls to `schema_names()` from multiple threads
- [x] DataFusion: add test verifying the scan path returns the expected error or data, not silently empty

### Read-Only Virtual Catalog SQL Tables

Expose all 28 DuckLake catalog tables plus the `0xFD` inlined tables as read-only SQL views through the PG-wire sidecar:

- `SELECT * FROM slateduck_catalog.ducklake_snapshot` — all snapshot rows (no MVCC filter; all versions)
- `SELECT * FROM slateduck_catalog.ducklake_table WHERE begin_snapshot <= $1 AND (end_snapshot IS NULL OR $1 < end_snapshot)` — MVCC-filtered view at a specific snapshot
- `SELECT * FROM slateduck_catalog.ducklake_file_column_stats WHERE table_id = $1` — raw stats rows for a table
- `SELECT * FROM slateduck_catalog.slateduck_counters` — current counter values (next_snapshot_id, next_catalog_id, next_file_id)
- `SELECT * FROM slateduck_catalog.slateduck_system` — writer epoch, endpoint, retain-from, catalog-format-version

These are exposed under a `slateduck_catalog` schema prefix to avoid name collisions with DuckLake's own table names in the `public` schema. They are read-only: `INSERT`, `UPDATE`, and `DELETE` against `slateduck_catalog.*` return `SQLSTATE 25006`.

**Implementation.** The PG-wire dispatcher already executes bounded SELECT shapes against the catalog tables. Virtual catalog SQL tables are an extension of the same dispatcher: add a new statement family that recognizes `SELECT * FROM slateduck_catalog.{table_name}` shapes and dispatches to full-table scans with optional MVCC filtering. No new storage layer changes are needed; this is entirely a dispatcher and result-encoding change.

**Operator use cases.** An operator debugging a missing file can run:
```sql
SELECT data_file_id, path, begin_snapshot FROM slateduck_catalog.ducklake_data_file
  WHERE table_id = 42 ORDER BY begin_snapshot DESC LIMIT 20;
```
An operator verifying time-travel coverage can run:
```sql
SELECT snapshot_id, snapshot_time, schema_version
  FROM slateduck_catalog.ducklake_snapshot ORDER BY snapshot_id;
```

This feature makes `slateduck inspect` and `slateduck verify` less necessary for interactive debugging, and enables operators already familiar with DuckDB SQL to explore the catalog without learning a new CLI tool.

- [x] Implement `SELECT * FROM slateduck_catalog.*` statement family in the dispatcher
- [x] Add MVCC filtering support for time-travel queries: `WHERE begin_snapshot <= $1 AND (end_snapshot IS NULL OR $1 < end_snapshot)`
- [x] Add end-to-end tests verifying all 28 tables return correct results
- [x] Document in `docs/operations/operational-sql.md` with worked examples

### Release and Versioning Policy

Establish the policies that enable confident production upgrades and long-term compatibility.

**Deprecation policy.** Six-month notice period before removing any CLI flag, metric name, SQLSTATE code, or public Rust API. Deprecation warnings are emitted in the binary and documented in `CHANGELOG.md` with the target removal version.

**Semantic versioning policy.** `catalog-format-version` bumps require a major version bump of the SlateDuck binary. `encoding_version` bumps within the same `catalog-format-version` require a minor version bump. Patch versions are backward-compatible on both dimensions.

**Release verification checklist.** Documented in `CONTRIBUTING.md`: run full benchmark suite (v1.0 requirement, placeholder here); run TPC-H SF10 golden test; check `mkdocs build --strict`; verify `slateduck migrate --dry-run` succeeds on a v0.x catalog; tag and push. No release may be tagged without all checklist items signed off.

**Complete `docs/compatibility.md`.** DuckDB version matrix with verified patch versions; DuckLake spec version matrix; object-store backend status (LocalFS, MinIO, S3 Standard, S3 Express, GCS, Azure Blob); Spark connector matrix; Trino connector matrix; pg-tide-relay version matrix; DataFusion version matrix.

- [x] Define and document all four policies in `CONTRIBUTING.md`
- [x] Create `docs/compatibility.md` with all version matrices populated based on v0.9.4 client support
- [x] Add CI check that validates `CHANGELOG.md` has an entry for every tagged release

### v0.9.4 Acceptance Criteria Definition

Convert the notion of "GA Ready" from self-reported to criteria-driven:

- [x] Define measurable acceptance criteria for v0.9.4: specific test suites that must pass, CLI compatibility matrix, docs completeness, operational drill results
- [x] Add acceptance criteria to `docs/contributing/release-process.md`
- [x] No v0.9.4 release tag until every acceptance criterion is documented, automated, and green

### Remove or Gate `slateduck-sqlite-vfs` Placeholder (F-23 / F-10)

The crate has no implementation, no tests, and is a workspace member implying parity it does not have.

- [x] Remove `slateduck-sqlite-vfs` from the workspace `members` list, or add a `[features]` gate `experimental = []` and document it as a future direction
- [x] Update README and docs to note that Strategy C (native SQLite VFS) is a future milestone, not a current feature

### Structured Parameter Validation in PG-Wire (F-24)

Missing or unparsable parameters default to `0`, `u64::MAX`, or empty strings across executor read and write paths.

- [x] Define `require_param_u64`, `require_param_i64`, and `require_param_string` helpers that return a structured SQLSTATE error rather than a default
- [x] Apply them to every `params.get_u64(idx).unwrap_or(0)` and equivalent call in the executor
- [x] Add tests that deliberately omit required parameters and verify the returned SQLSTATE code

### Tracing and Metrics on Critical Paths (F-25)

Core write, read, GC, excision, repair, and FFI paths lack tracing spans and metrics counters.

- [x] Add `#[tracing::instrument]` to `CatalogWriter::create_snapshot()`, `execute_commit()`, `gc_apply()`, `excise_apply()`, and `repair_apply()`
- [x] Emit counter metrics for snapshot commits, transaction conflicts, auth failures, FFI errors, and excision events
- [x] Emit histogram metrics for commit latency, read latency, and scan row counts
- [x] Integrate with the existing `metrics.rs` module and document metric names in `docs/operations/logging.md`

### Docs/CLI Conformance Gates (F-26 / F-12)

Documentation is ahead of implementation in TLS, auth, CLI flags, env vars, and cloud backends.

- [x] Add a CI smoke test that runs `slateduck --help` and validates every documented flag is present in the output
- [x] Audit `docs/deployment/tls.md`, `docs/operations/cli-reference.md`, and `docs/reference/environment-vars.md` against the actual binary; mark any planned-but-unimplemented features with an "Available from: v0.9.x" callout
- [x] Verify `--tls-required`, GCS URL support, Azure URL support, and all documented env vars exist in the binary before any GA claim

### Roadmap Status Accuracy (F-27)

Roadmap phases v0.4 through v0.9 are marked Done but contain features that are scaffolded rather than fully implemented and tested.

- [x] Add per-phase acceptance criteria specifying the tests, docs pages, and CI gates that must be green for a phase to be marked Done
- [x] Audit phases v0.4 through v0.9 against those criteria; downgrade phases where criteria are not met
- [x] Record findings from `plans/overall-assessment-1.md` as closed items in the relevant roadmap phases when resolved

### Supply Chain and MSRV Gates (F-28 / F-29)

No `cargo audit`, `cargo deny`, or MSRV check exists in CI; workspace-level feature flags pull broad dependencies into all crates.

- [x] Add `deny.toml` with advisories, bans, licenses, and sources policies
- [x] Add `cargo deny check` and `cargo audit` to the CI `check` job
- [x] Declare `rust-version` in workspace `[package]` metadata and add an MSRV CI job pinned to that version
- [x] Audit `tokio = { features = ["full"] }` and `object_store = { features = ["aws", "gcp", "azure"] }` and scope features by crate where not all are needed

### Error Type Preservation (F-09 / F-11)

SlateDB errors and lower-level errors are collapsed into strings via `.map_err(|e| CatalogError::SlateDb(e.to_string()))`, making programmatic error classification impossible.

- [x] Preserve source errors using `#[source]` or structured variants for at least: transaction conflict, object-store permission denied, decode failure, and writer fenced
- [x] Add error context (operation, table name, key) when mapping errors at catalog module boundaries
- [x] Update SQLSTATE mappings in `error.rs` to use the new structured variants

### CI Quality Gates (F-33)

CI currently runs fmt, clippy, tests, compatibility replay, and strict docs. No coverage, security audit, sanitizer, MSRV, or benchmark regression gate exists.

- [x] Add a `coverage` CI job using `cargo llvm-cov --all-features` targeting ≥ 80% line coverage for `slateduck-catalog` and `slateduck-core`
- [x] Add a `security` CI job running `cargo deny check` and `cargo audit`
- [x] Add an `msrv` CI job using the declared `rust-version`
- [x] Add a `sanitizer` CI job for the FFI crate using `-Zsanitizer=address,leak` on nightly
- [x] Add a `bench-regression` CI job that runs criterion benchmarks on PRs touching catalog read/write paths and fails if p99 degrades more than 20% vs. `benchmarks/phase-2-baseline.json`

### Release Automation (F-34)

No release workflow for signed artifacts, checksums, crates publishing, or binary publishing exists.

- [x] Add a `release.yml` GitHub Actions workflow triggered on `v*` tags
- [x] Workflow must: run full quality gates, build binaries for Linux x86-64/arm64 and macOS arm64, generate checksums, create a GitHub Release with attached binaries and checksums, and update `CHANGELOG.md`
- [x] Add a release sign-off checklist to `CONTRIBUTING.md` referencing v1.0 GA acceptance criteria

### v1.0 Acceptance Criteria Definition

Convert roadmap Done status from self-reported to criteria-driven:

- [x] Define measurable acceptance criteria for v1.0: specific test names that must pass, benchmark thresholds, supported deployment matrix, security checks, and operational drill results
- [x] Add acceptance criteria to `docs/contributing/release-process.md`
- [x] No v1.0 release tag until every acceptance criterion is documented, automated, and green

### Deliverables

- [x] Concurrent PG-Wire read sessions do not block each other; confirmed by concurrency test and benchmark
- [x] `describe_table()` is O(1) or O(log n) for any catalog size
- [x] DataFusion `schema_names()`/`table_names()` do not spawn threads; return an explicit error or correct results outside a runtime
- [x] Zone-map index: v0.9 profiling report completed; if amplification >10×, zone-map implemented and tested
- [x] Spark-DuckLake corpus captured and replay tests green in CI; `docs/compatibility.md` updated
- [x] Trino-DuckLake corpus captured and replay tests green in CI; `docs/compatibility.md` updated
- [x] DataFusion Parquet scan implemented with real data reads and performance benchmarks
- [x] DataFusion pg-wire mode available; end-to-end integration tests pass
- [x] Virtual catalog SQL tables implemented and tested; all 28 tables queryable via `SELECT * FROM slateduck_catalog.*`
- [x] Writer session regression tests pass for ID monotonicity, `read_latest()` consistency, and aborted session isolation
- [x] Security protocol tests pass: valid/invalid auth, TLS handshake, tls-required plaintext rejection
- [x] FFI null/invalid-handle tests pass under address and leak sanitizers
- [x] `slateduck-sqlite-vfs` removed from workspace or clearly gated as experimental with docs updated
- [x] All documented PG-Wire parameters return structured errors rather than silent defaults
- [x] Tracing spans and counters emitted on all critical paths; metric names documented
- [x] Every documented CLI flag present in the binary help text; CI smoke test enforces this
- [x] `cargo deny` and `cargo audit` green in CI
- [x] MSRV declared and tested in CI
- [x] Coverage ≥ 80% for `slateduck-catalog` and `slateduck-core`
- [x] Release automation workflow present and documented
- [x] Deprecation, semantic versioning, and release verification policies documented in `CONTRIBUTING.md`
- [x] `docs/compatibility.md` complete with version matrices for DuckDB, Spark, Trino, DataFusion, pg-tide, object-store backends
- [x] v0.9.4 acceptance criteria documented and automated

---

## v0.11 — Incremental View Maintenance (Foundations)

> **The defining feature.** Bring first-class incremental materialized views to a lakehouse-on-object-storage. No external streaming system, no Kubernetes operator, no separate database — just stateless workers writing more DuckLake into the same bucket. Companion design documents: [plans/slateduck-differential-dataflow.md](plans/slateduck-differential-dataflow.md) (architecture), [plans/slatedb-differential-dataflow.md](plans/slatedb-differential-dataflow.md) (substrate analysis), and [plans/incremental-view-maintenance-implementation.md](plans/incremental-view-maintenance-implementation.md) (engineering plan).

### Thesis

DuckLake snapshots, SlateDB SSTs, SlateDuck catalog facts, and differential-dataflow (DD/DBSP) batches are all immutable. Stacking them yields an IVM system whose compute workers are stateless, whose state is content-addressable in object storage, whose sharding is trivial because nothing ever moves between shards, and whose read fan-out is the same as for base tables. This is a capability Iceberg and Delta achieve only with external streaming systems; SlateDuck delivers it as a single binary against an S3 bucket.

v0.11 lands the *foundations* end-to-end at single-shard scope. v0.12 generalizes to sharded scale-out. v0.13 covers joins. v0.14 is operational hardening. After v0.14 the system is ready to be included in the v1.0 GA story.

### Why pre-1.0

Three reasons IVM ships before GA rather than as a v1.x add-on:

1. **It is the defining feature.** SlateDuck without IVM is one of several lakehouse-on-object-storage implementations. SlateDuck with IVM is the only one that materializes derived views without leaving the bucket.
2. **The architectural seams must be right at GA.** The catalog change-log shape, per-warehouse layout, and snapshot metadata fields that IVM depends on are cheap to design now and expensive to retrofit. Shipping GA without those seams locks in a more painful upgrade path.
3. **No rush on GA.** Correctness, security, and operational tracks (v0.9.x) take priority for ship readiness; once those land, adding the defining feature before declaring "general availability" produces a v1.0 that means something stronger.

### Catalog Schema Additions

Four new MVCC-versioned tables under freshly allocated tag bytes in [crates/slateduck-core/src/tags.rs](crates/slateduck-core/src/tags.rs):

| Table | Tag (proposed) | MVCC behaviour | Purpose |
|---|---|---|---|
| `matviews` | `0x1D` | `Versioned` | View definitions (name, SQL, output table, shard count, freshness target, state URI, lifecycle snapshots) |
| `matview_deps` | `0x1E` | `AppendOnly` | `(matview_id, base_table_id, used_columns)` — input subscription graph |
| `matview_checkpoints` | `0x1F` | `AppendOnly` | `(matview_id, shard_id, last_input_snapshot, last_output_snapshot, frontier_time, durable_at)` — per-shard watermark log |
| `matview_shards` | `0x20` | `MutableSingleton` per `(matview_id, shard_id)` | Lease state (`owner_worker`, `lease_expires_at`, `key_range_lo`, `key_range_hi`) updated via SlateDB CAS |

- [x] Tag descriptors added to `tags.rs` with documentation, MVCC behaviour, and `status: Implemented`
- [x] Protobuf row schemas added to `slateduck-core/src/rows.rs` with `encoding_version = 1`
- [x] Wire-level fixtures captured under `tests/fixtures/matview/` for each table
- [x] Key-encoding test corpus extended for the new tag bytes (round-trip, ordering, prefix isolation)

### Catalog Format Compatibility

Adding tags `0x1D`–`0x21` must NOT require a `catalog-format-version` bump. The design already handles unknown tags: older binaries encountering an unknown tag byte return an explicit error rather than silent data loss (§ v0.2 Key Layout). This means:

- [x] Tags `0x1D`–`0x21` are additive: a v0.9.4 binary opening a v0.11 catalog ignores matview rows (they are not in any scan prefix it uses) and operates normally on the 28 base tables
- [x] A v0.11 binary opening a v0.9.4 catalog (no matview rows) operates normally; `list_matviews()` returns empty
- [x] No `catalog-format-version` increment required for v0.11; the existing format accommodates new tags by construction
- [x] Document this in `docs/architecture/key-layout.md`: "tag bytes are an extensibility mechanism; unknown tags are skipped during prefix scans and error on direct access"
- [x] Add a cross-version integration test: v0.9 binary reads a catalog that contains `0x1D`–`0x21` rows without error

### Matview Output Table Semantics

A materialized view's output is a **normal DuckLake table** — the same Parquet files, same catalog rows, same query path. The magic is that IVM workers write to it instead of a user INSERT.

**User-facing contract:**

- `CREATE INCREMENTAL MATERIALIZED VIEW v AS <select>` creates both a `MatviewRow` (under `0x1D`) and a regular `TableRow` (under `0x05`) for the output. The output table is named `_matview_{name}` by convention (e.g. `_matview_events_by_day`)
- Users query the view by its logical name: `SELECT * FROM events_by_day`. The pg-wire dispatcher resolves `events_by_day` to the output table `_matview_events_by_day` via a name-mapping lookup in the matview registry
- Unmodified DuckDB (via Strategy B or Strategy C) queries the output table like any other DuckLake table — no client-side changes required
- The output table is read-only for users; `INSERT`/`UPDATE`/`DELETE` against it returns `SQLSTATE 25006`
- `DROP INCREMENTAL MATERIALIZED VIEW v` drops both the `MatviewRow` and the output `TableRow` (cascading logical delete via `end_snapshot`)

**Output table lifecycle:**

- [x] Output table created atomically with the matview definition in one catalog snapshot
- [x] Output table schema derived from the view SELECT's output columns
- [x] Output table marked with a `managed_by = 'ivm'` metadata key preventing user writes
- [x] `DROP … CASCADE` drops the matview, its output table, and all output data files (scheduled for GC)
- [x] Stale matviews (schema change on base table) surface a `status = 'stale'` marker in `SHOW MATERIALIZED VIEWS` but output table remains queryable at its last-valid state

**Query semantics during backfill, stale, and time-travel:**

- During initial backfill (matview status `backfilling`), `SELECT * FROM v` returns whatever rows the output table currently contains — **partial results from completed batches, never an error**. The matview's `status` and `last_output_snapshot` are exposed via `SHOW MATERIALIZED VIEWS` and the helper function `matview_status('v') -> ('backfilling'|'fresh'|'stale'|'dropped_dependency', last_output_snapshot, lag_ms)` so applications can gate on readiness without polling
- `SELECT * FROM v AT SNAPSHOT <id>` resolves to a snapshotted read of the output table at the *catalog* snapshot `<id>`. If `<id>` predates the matview's first output snapshot, the query returns an empty result (the matview did not exist yet) — never an error. Operators are warned in docs that time-travel against a matview reflects when the *materialization* observed the data, not when the underlying input was first written
- Schema-change-induced staleness (column added/renamed in the view's output projection) makes the matview `stale`; the output table remains readable at its prior schema until a successful `REFRESH ... FULL`. A schema change that *reorders or retypes* output columns rewrites the output table under a new `(output_table_id, schema_version)` pair so existing Parquet readers are not misaligned mid-flight (see v0.14: Schema Evolution)
- [x] `matview_status()` helper available from v0.11; documented in `docs/reference/sql-ivm.md`
- [x] Time-travel against a matview before its first output snapshot returns empty (regression test)
- [x] During-backfill query returns monotonically growing partial result (regression test)

### View Dependency Cascades

The `matview_deps` table tracks which base tables each view reads. This enables:

**DROP behavior:**
- `DROP TABLE t` where `t` is a dependency of matview `v`: reject with `SQLSTATE 2BP01` (dependent objects exist) unless `CASCADE` specified
- `DROP TABLE t CASCADE`: mark all dependent matviews as `status = 'dropped_dependency'`; IVM workers stop processing; output tables remain queryable at their last state (stale but not deleted)
- `DROP INCREMENTAL MATERIALIZED VIEW v CASCADE`: also drops matviews that depend on `v`'s output table

**Cascading views (view-on-view):**
- A matview can reference another matview's output table as a base input: `CREATE INCREMENTAL MATERIALIZED VIEW summary AS SELECT … FROM events_by_day`
- `matview_deps` records the dependency on the output table; the dependency graph is a DAG (cycles rejected at creation time with `SQLSTATE 42P19`)
- IVM workers process views in topological order: upstream views must publish before downstream views read
- `SHOW MATERIALIZED VIEWS` includes a `depth` column (0 = directly on base tables, 1 = depends on one matview, etc.)
- Documented maximum depth: 10 levels (configurable via `WITH (max_cascade_depth = N)`)

- [x] `DROP TABLE` with dependent matviews returns `SQLSTATE 2BP01` unless CASCADE
- [x] View-on-view creation validates DAG property (no cycles)
- [x] IVM scheduler respects topological ordering for cascading views
- [x] `matview_deps` populated for both base-table and matview-output-table dependencies
- [x] Test: three-level cascade (base → view_a → view_b → view_c) maintains correctness end-to-end

### SQL Surface

Bounded SQL extension in `slateduck-sql` — the inner `<select>` is parsed but stored verbatim; only the new statement shells are validated by pgwire.

```sql
CREATE INCREMENTAL MATERIALIZED VIEW v
  WITH (shard_count = 1, freshness = '5s')
  AS <select>;

DROP    INCREMENTAL MATERIALIZED VIEW v;
ALTER   INCREMENTAL MATERIALIZED VIEW v SET (freshness = '10s');
REFRESH INCREMENTAL MATERIALIZED VIEW v FULL;

SHOW    MATERIALIZED VIEWS;
SHOW    MATVIEW SHARDS FOR v;
EXPLAIN MATERIALIZED VIEW v;             -- shows compiled DBSP plan
SELECT  matview_lag('v');                -- freshness lag in ms
```

- [x] Grammar additions to `slateduck-sql` with happy-path and error-path tests
- [x] PG-wire dispatcher routes each new statement to a `CatalogWriter` method
- [x] `EXPLAIN MATERIALIZED VIEW` returns the DBSP operator tree as a textual plan
- [x] All new statement shapes covered by the v0.6.x wire-corpus replay harness

### `slateduck-ivm` Crate (New)

A new workspace crate hosting the IVM runtime. Single-shard scope in v0.11.

```
crates/slateduck-ivm/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── source.rs        # MatviewInputSource over DuckLake data files
│   ├── circuit.rs       # DBSP circuit compilation from stored view SQL
│   ├── trace.rs         # Persistent trace adapter (DBSP-bundled object-store backend in v0.11)
│   ├── worker.rs        # Lease acquisition, circuit driver, checkpoint advance
│   ├── output.rs        # Per-shard Parquet writer + catalog commit
│   └── bin/
│       └── slateduck-ivm.rs
└── tests/
    └── integration_tests.rs
```

- [x] `dbsp` (Feldera) crate added as workspace dependency, version-pinned with a vendored compatibility shim in `circuit.rs`
- [x] `MatviewInputSource` reads append-only base tables filtered to a key range, emitting `(row, snapshot_id, +1)` deltas
- [x] `SlateDbTrace` (Phase A: DBSP-bundled persistence; native impl deferred to v0.14)
- [x] Worker event loop: poll catalog → acquire lease → drive circuit → durable batch → append checkpoint
- [x] Output writer emits one Parquet file per cycle, commits via existing `CatalogWriter` snapshot path
- [x] `slateduck-ivm serve --catalog-path … --state-prefix … --worker-id … --shard-limit 1` CLI matches `slateduck-pgwire` ergonomics

**Worker startup (discovery → claim → drive) protocol:**

At boot, the worker has no in-memory state. It must transparently rejoin a running fleet without colliding with peers.

1. **Discover.** Call `list_matviews()` and for each non-dropped matview, `list_shards_for_worker(matview_id)` filtered by status `unowned` or `lease_expired`
2. **Sort by oldest lag.** Prefer shards with the largest `now - last_output_snapshot.committed_at` so under-served work is picked up first
3. **Claim (bounded).** For each candidate shard, call `claim_matview_shard(matview_id, shard_id, worker_id, lease_ttl)` until `--shard-limit` is reached. CAS via `DbTransaction` + `SerializableSnapshot` guarantees that two workers booting simultaneously cannot both win the same `(matview_id, shard_id)`; the loser receives `LeaseTaken` and skips to the next candidate
4. **Drive.** Once at quorum (or candidate list exhausted), enter the standard event loop. A background ticker every `lease_ttl/3` re-scans for newly-orphaned shards and tries to grow the held set up to `--shard-limit`
5. **Renew.** Each shard's lease is renewed at `lease_ttl/3` cadence by the worker that owns it; failure to renew (e.g. catalog write outage) releases the shard locally before the lease expires server-side, preventing split-brain

- [x] Two-worker race test: both boot within 100 ms, claim the same matview; CAS resolves deterministically and no shard is double-owned
- [x] Discovery is idempotent — restarting a worker with the same `--worker-id` re-acquires its prior shards without recompute
- [x] `slateduck-ivm doctor` (v0.14) reports any shard that has been `unowned` for more than `2 × lease_ttl`

### Catalog API Additions

In `slateduck-catalog`:

- [x] `CatalogWriter::create_matview(name, view_sql, output_table_id, shard_count, freshness_ms) -> MatviewId`
- [x] `CatalogWriter::drop_matview(matview_id)` (logical drop via `dropped_at_snapshot`)
- [x] `CatalogWriter::update_matview_checkpoint(matview_id, shard_id, input_snapshot, output_snapshot, frontier)`
- [x] `CatalogWriter::claim_matview_shard(matview_id, shard_id, worker_id, lease_ttl) -> ClaimResult` (CAS via `DbTransaction` + `SerializableSnapshot`)
- [x] `CatalogReader::list_matviews()`, `get_matview(id)`, `list_shards_for_worker(worker_id)`, `read_checkpoint_history(matview_id)`
- [x] All new methods covered by property tests (creation idempotence, lease exclusivity, checkpoint monotonicity)

### End-to-End Demo

Acceptance demo encoded as a single test:

1. Open a fresh SlateDuck warehouse on LocalFS.
2. Create base table `events(id BIGINT, occurred_at TIMESTAMP, event_type VARCHAR)`.
3. Bulk-insert 1M rows across 30 catalog snapshots.
4. Execute `CREATE INCREMENTAL MATERIALIZED VIEW events_by_day AS SELECT date_trunc('day', occurred_at) AS day, event_type, count(*) AS n FROM events GROUP BY 1, 2 WITH (shard_count = 1, freshness = '2s');`.
5. Start one `slateduck-ivm` worker.
6. Verify backfill completes; `SELECT * FROM events_by_day` returns correct counts.
7. Insert another 10k events across 5 snapshots.
8. Within 5 s, `events_by_day` reflects the new counts.
9. Kill the worker; restart; verify it resumes from checkpoint without recomputing the backfill.

- [x] Test passes deterministically on LocalFS, MinIO, and S3 Standard
- [x] Documented in `docs/operations/incremental-materialized-views.md`

### Documentation

- [x] `docs/concepts/incremental-views.md` — what an IVM is, freshness semantics, snapshot alignment
- [x] `docs/architecture/ivm-plane.md` — three-plane architecture diagram and explanation
- [x] `docs/operations/incremental-materialized-views.md` — operator playbook: create, drop, refresh, monitor
- [x] `docs/reference/sql-ivm.md` — full SQL grammar reference for the new statements
- [x] `docs/design-decisions/ivm-on-immutable-substrate.md` — the immutability argument and why this is the right place to build IVM

### Testing Infrastructure (Tier 1–3 + Testkit Bootstrap)

v0.11 lays the testing foundation for the entire IVM track. Every subsequent phase adds tiers on top of this base. Reference: [plans/e2e-integration-tests.md](plans/e2e-integration-tests.md).

- [x] Create `crates/slateduck-testkit/` crate with shared test harnesses: `CatalogHarness`, `PgWireHarness`, `DeterministicClock`, `IvmWorkerHarness` stubs
- [x] `MinioHarness` (Testcontainers MinIO) with `OnceLock` container lifecycle — one container per test binary, pinned to a digest for reproducibility
- [x] Add workspace feature flags: `minio-tests`, `fault-injection`, `scale-tests`; `cargo test` without flags runs only `LocalFileSystem`-based tests
- [x] **Tier 6a — Single-shard IVM integration tests** (`crates/slateduck-ivm/tests/integration_tests.rs`): 11 tests covering GROUP BY, deletion, DISTINCT, UNION ALL, HAVING, filter+project, restart recovery, stale detection on schema change, time-travel-before-first-output, lag bound, and two-worker CAS race
- [x] **Tier 3 extension** — `v011_pgwire_tests.rs`: full SQL surface for `CREATE/DROP/ALTER/REFRESH/SHOW/EXPLAIN INCREMENTAL MATERIALIZED VIEW`; `matview_lag()`, `matview_status()`, `matview_shard_count()` functions; `DROP TABLE` on dep table returns `SQLSTATE 2BP01`
- [x] **Tier 2 extension** — `v011_catalog_tests.rs`: all 19 IVM catalog method tests (happy path, conflict, idempotence, wrong-state) for `create_matview`, `drop_matview`, `claim_matview_shard`, `extend_matview_lease`, `release_matview_lease`, `update_matview_checkpoint`
- [x] Add fixture files: `tests/fixtures/matview/{create_view,multi_shard,lease_acquired,checkpoint_history,dropped}.dat`
- [x] Update CI `ci.yml`: add `minio-tests` large-runner job (`ubuntu-latest-8-core`) triggered on every merge to `main`; standard-runner job (Tiers 1–3) on every PR

### Acceptance Criteria

- [x] Single-shard `GROUP BY` IVM holds correct contents across 100 catalog snapshots of input
- [x] Worker restart without checkpoint loss: kill -9 followed by restart resumes from `last_output_snapshot` without recomputing prior work
- [x] Freshness target honoured: median publish-to-visible lag ≤ target on LocalFS and S3 Express
- [x] TPC-H Q1 maintained against a streaming `lineitem` source with sub-second freshness for batches of ≤ 100 rows
- [x] All v0.11 tests pass under MinIO and on S3 Standard
- [x] Wire corpus regenerated to include the four new statement shapes
- [x] **Tier 6a test suite green**: all 11 single-shard IVM integration tests pass on `LocalFileSystem`; subset passes on MinIO in large-runner CI job
- [x] **`slateduck-testkit` crate compiles** with `--features local-only` on every PR and with `--features minio-tests` on the large-runner job
- [x] Implementation doc [plans/incremental-view-maintenance-implementation.md](plans/incremental-view-maintenance-implementation.md) reflects the shipped design

### Deliverables

- [x] `slateduck-ivm` crate published in the workspace
- [x] `slateduck-testkit` crate published in the workspace (dev-only, `publish = false`)
- [x] Catalog schema additions and tag-allocation update
- [x] SQL grammar extension and pgwire routing
- [x] End-to-end single-shard demo test green
- [x] Tier 6a test suite (`integration_tests.rs`) with 11 passing tests
- [x] Tier 2 + 3 IVM extensions (`v011_catalog_tests.rs`, `v011_pgwire_tests.rs`)
- [x] CI large-runner `minio-and-ivm` job green
- [x] Documentation set published under `docs/`

---

## v0.12 — IVM Scale-Out (Sharding & Lease Management)

> Generalize v0.11's single-shard runtime to N-shard horizontal scale-out within a single view. This phase pays off the central claim of the IVM design: that immutability makes sharded streaming computation cheap.

### Sharding Model

Each matview owns `shard_count` shards. A shard is identified by `(matview_id, shard_id)` where `shard_id ∈ [0, shard_count)`. Shards are assigned a *key range* over the matview's shard key — typically the first GROUP BY column or a hash thereof. The shard owner reads only base-table rows whose shard-key value falls in `[key_range_lo, key_range_hi)`.

- [x] Key range computed deterministically from `shard_count` and shard-key column statistics at view creation
- [x] `matview_shards` populated atomically with the view's first catalog snapshot
- [x] Shard-key column auto-detected from view SQL; explicit override via `WITH (shard_key = '<column>')`
- [x] Per-shard SlateDB state store at `{state_prefix}/matviews/{matview_id}/shards/{shard_id}/`

### Lease & Heartbeat Protocol

- [x] `claim_matview_shard` CAS protocol with TTL (default 30 s)
- [x] Worker heartbeat extends lease every TTL/3; missed heartbeat lets another worker claim
- [x] Worker IDs are durable across restarts via `--worker-id` (no random UUIDs in production)
- [x] Lease history retained for 24 h for forensic debugging
- [x] Test: kill -9 a worker holding 4 shards; verify a second worker acquires all 4 within 2× TTL

### Per-Shard Checkpoint Independence

- [x] Each shard advances `last_input_snapshot` independently
- [x] Output mode `consistent` (default): output snapshot waits for all shards
- [x] Output mode `per_shard`: shards publish independently; reader merges
- [x] `MATVIEW_LAG('v')` returns max lag across shards
- [x] `SHOW MATVIEW SHARDS FOR v` returns per-shard owner, lease expiry, last input snapshot, lag

### Per-Shard Parquet Output

- [x] Each shard writes one Parquet file per output cycle; data files registered to the output table
- [x] Output table partitioning aligned with shard key when possible (pruning benefit at read time)
- [x] Compaction policy for output data files: configurable via `WITH (output_compaction = '1h' | 'never')`
- [x] Cleanup of superseded per-shard data files via existing DuckLake GC

### Sharded Scale-Out Demo

- [x] TPC-H Q1 with `shard_count = 8` maintained against streaming `lineitem`, achieving ≥ 6× ingest throughput vs v0.11 single-shard baseline
- [x] Linear scaling demonstrated for 1 → 2 → 4 → 8 → 16 shards on a 1 TB synthetic input
- [x] Cost report: S3 PUT volume per million input rows at each shard count
- [x] Documented sweet-spot recommendations in `docs/operations/incremental-materialized-views.md`

### Re-Sharding

- [x] `ALTER INCREMENTAL MATERIALIZED VIEW v SET (shard_count = 16)` triggers a parallel rebuild of the view at the new shard count; cutover when caught up
- [x] Old and new view versions coexist until cutover; old version GC'd after retention window
- [x] Tested across snapshot boundaries: re-sharding during active ingest does not lose updates

### Graceful Shutdown & Rolling Updates

- [x] SIGTERM triggers graceful drain: finish current batch (bounded by `--max-drain-time`), checkpoint all shards, release leases, exit 0
- [x] Rolling update with `maxSurge: 1, maxUnavailable: 0` achieves zero-downtime shard handoff
- [x] `terminationGracePeriodSeconds` guidance documented (must exceed drain + checkpoint flush)
- [x] Test: rolling restart of a 4-worker pool holding 16 shards results in zero dropped batches and ≤ lease_ttl handoff window

### Testing: Tier 4 (MinIO Catalog) & Tier 6b (Multi-Shard IVM)

- [x] **Tier 4 — MinIO catalog integration tests** (`crates/slateduck-catalog/tests/minio_catalog_tests.rs`): 9 tests covering `open`, `reopen`, `flush` visibility barrier, concurrent init convergence, sequential snapshot IDs, reader snapshot isolation, 10k file registration, zone-map pruning — all against a live MinIO container
- [x] **Tier 4 — Writer failover on MinIO** (3 tests): `writer_failover_on_minio_within_slo`, stale epoch returns `SQLSTATE 57P04`, new writer sees all committed state
- [x] **Tier 4 — Flush visibility barrier latency assertion**: p99 of 100 measured `flush()` → `read_latest()` round-trips < 1 s; measured and recorded in CI output
- [x] **Tier 6b — Multi-shard IVM tests** (`crates/slateduck-ivm/tests/sharded_tests.rs`): 7 tests — 8-shard GROUP BY throughput + union correctness, re-sharding content preservation, lease heartbeat generation increment, lease expiry handoff, 1M-row backfill rate, `--shard-limit` enforcement, consistent output min-frontier check
- [x] `DeterministicClock` implemented in `slateduck-testkit`: all timing tests use `tokio::time::pause()` — no wall-clock sleeps
- [x] `IvmWorkerHarness` fully implemented: spawn/kill `slateduck-ivm` processes, poll lag via catalog reader, assert output Parquet row counts

### Acceptance Criteria

- [x] 8-shard `GROUP BY` view maintains correctness across 1000 input snapshots
- [x] Kill-and-restart of a worker holding multiple shards results in zero data loss and ≤ 2× TTL recovery latency
- [x] Linear ingest scaling 1 → 16 shards within ±15%
- [x] Re-sharding from 1 → 8 shards completes for a 100 GB base table without service interruption
- [x] Graceful shutdown releases all leases within `max_drain_time + 5s`
- [x] No regression in v0.11 single-shard tests
- [x] Per-shard observability surfaces visible in `SHOW MATVIEW SHARDS` and exported metrics
- [x] **Tier 4 catalog tests green on MinIO**: all 12 tests pass in large-runner CI job on every merge to `main`
- [x] **Tier 6b test suite green**: all 7 sharded IVM tests pass; lease heartbeat and expiry tests are clock-driven (no sleeps)
- [x] **Flush visibility barrier p99 < 1 s** on MinIO (same-host container) measured and recorded

### Deliverables

- [x] Lease + heartbeat protocol shipped and stress-tested
- [x] Per-shard SlateDB state stores under `{state_prefix}/matviews/.../shards/.../`
- [x] Re-sharding via `ALTER` shipped
- [x] Tier 4 MinIO catalog test suite (`minio_catalog_tests.rs`) with 12 passing tests
- [x] Tier 6b multi-shard IVM test suite (`sharded_tests.rs`) with 7 passing tests
- [x] `IvmWorkerHarness` and `DeterministicClock` fully implemented in `slateduck-testkit`
- [x] Sharded scale-out benchmark report in `benchmarks/v0.12-ivm-scaleout.json`
- [x] Operator playbook expanded with sharding guidance

---

## v0.13 — IVM Joins

> Extend the IVM runtime from single-input aggregations to multi-input joins. Covers the three join strategies described in the design document: broadcast small-side, co-partitioned, and re-shuffled.

### Broadcast Join Support

The common case: join a large fact table with one or more dimension tables. Dimension tables are broadcast to every shard.

- [x] Detect broadcast candidates at view creation: input estimated row count below `WITH (broadcast_threshold = N)` (default 1M rows)
- [x] Replicate broadcast inputs into each shard's state store at backfill; incremental updates propagated to all shards
- [x] Bounded memory: broadcast inputs above threshold reject view creation with a clear error message
- [x] Tested with TPC-H Q3 (`orders ⋈ lineitem ⋈ customer`) where `customer` and `nation` are broadcast

### Co-Partitioned Join Support

When both inputs share the same shard key, joins are local. No exchange required.

- [x] Detect co-partitioned joins via shard-key analysis in the SQL plan
- [x] Both inputs read filtered to the same key range
- [x] Local hash join via DBSP's join operator
- [x] Tested with TPC-H Q4 (`orders ⋈ lineitem` on `o_orderkey = l_orderkey` with `shard_key = orderkey`)

### Re-Shuffle Exchange

When neither broadcast nor co-partitioning applies, one side is re-partitioned at the join boundary.

- [x] Insert exchange operator that writes intermediate state to a temporary SlateDB region keyed by the join key
- [x] Reader on the other side reads the matching key range from the intermediate region
- [x] Cost: one extra round-trip through SlateDB per join input; documented as the most expensive option
- [x] Tested with TPC-H Q5 (`customer ⋈ orders ⋈ lineitem ⋈ supplier ⋈ nation ⋈ region`)

### Join Plan Selection

- [x] `EXPLAIN MATERIALIZED VIEW v` shows chosen join strategy per operator
- [x] `WITH (join_strategy = 'broadcast' | 'co_partition' | 'reshuffle')` overrides per-view default
- [x] Cost-based planner selects strategy automatically when not overridden

### Delete Propagation in Joins

- [x] `(-1)` updates from delete files propagate correctly through join operators
- [x] Documented limitation: high-volume delete campaigns over joined views may require `REFRESH ... FULL`

### Testing: Tier 6c (IVM Joins) & Tier 5 Extension (Live Client Compat)

- [x] **Tier 6c — IVM join tests** (`crates/slateduck-ivm/tests/join_tests.rs`): 7 tests — broadcast join (events × categories), co-partition join (shared shard key), reshuffle join (non-collocated), TPC-H Q1 streaming correctness, TPC-H Q3 broadcast correctness, TPC-H Q5 co-partition correctness, `EXPLAIN MATERIALIZED VIEW` returns correct `join_strategy`
- [x] All Tier 6c tests use `IvmWorkerHarness` against MinIO; correctness verified by comparing output Parquet to DuckDB single-shot reference via `DuckDbHarness`
- [x] **`DuckDbHarness`** implemented in `slateduck-testkit`: spawns a DuckDB process (Testcontainers `duckdb/duckdb` image), runs SQL, returns rows — used for join correctness assertions
- [x] **Tier 5 extension** — `crates/slateduck-pgwire/tests/compat_tests.rs`: live DuckDB E2E test `duckdb_full_ducklake_tutorial_against_minio` (create schema + table + INSERT + SELECT + time-travel + ALTER + DROP against a live PgWire server backed by MinIO)
- [x] `EXPLAIN MATERIALIZED VIEW` output format documented and golden-tested

### Acceptance Criteria

- [x] TPC-H Q3 maintained incrementally with broadcast `nation`/`region`
- [x] TPC-H Q5 maintained incrementally with explicit `WITH (shard_key = …)` on co-partitionable side
- [x] Re-shuffle exchange operator correctness verified by golden-output comparison against DuckDB single-shot execution
- [x] No correctness regression for v0.11/v0.12 single-input views
- [x] **Tier 6c test suite green**: all 7 join strategy tests pass in large-runner CI on every merge
- [x] **Live DuckDB E2E test** (`duckdb_full_ducklake_tutorial_against_minio`) green in large-runner CI
- [x] Per-join-strategy cost numbers published in `benchmarks/v0.13-ivm-joins.json`

### Deliverables

- [x] Three join strategies shipped behind a common DBSP join operator interface
- [x] Plan-selection logic in `slateduck-ivm/src/circuit.rs`
- [x] Tier 6c join test suite (`join_tests.rs`) with 7 passing tests
- [x] `DuckDbHarness` implemented in `slateduck-testkit`
- [x] Tier 5 live DuckDB E2E test (`compat_tests.rs`)
- [x] TPC-H Q3 / Q4 / Q5 maintained as continuous integration tests
- [x] Documentation updated with join sizing guidance

---

## v0.13.1 — IVM Join Correctness

> Follow-up patch release: fixes the EC-01 phantom-row bug in join deltas, formalises aggregate tier classification with auxiliary columns, adds function-volatility validation at view-creation time, and ships the property-based "differential ≡ full" test oracle that all future IVM correctness tests depend on. See [plans/pg-trickle.md](plans/pg-trickle.md) §4, §6, §9, §11.

### EC-01 Phantom-Row Fix in Joins

The v0.13 bilinear join expansion uses the post-change snapshot for both the insert and delete branches. When a row is deleted from the right side *in the same refresh window* as a deletion on the left side, the match is lost and the stale joined row survives in the materialized view indefinitely.

**Fix:** split Part 1 of the join delta into insert- and delete-asymmetric branches:
- **Part 1a:** `ΔR_insert ⋈ S_post` — new positive contributions
- **Part 1b:** `ΔR_delete ⋈ S_pre` — negatives must use the *pre-change* snapshot of S
- **Part 2:** `R_post ⋈ ΔS` — symmetric handling of ΔS
- **Part 3:** `−(ΔR ⋈ ΔS)` — correction term; subtracts double-counted intersections

`S_pre` reconstructed as `S_post EXCEPT ALL ΔS_insert UNION ALL ΔS_delete`; cached as an L₀ CTE to avoid repeated EXCEPT ALL per join per refresh.

- [ ] Enumerate `(ΔL_ins, ΔL_del, ΔR_ins, ΔR_del)` cases explicitly in `crates/slateduck-ivm/src/join.rs`
- [ ] Reconstruct and cache `S_pre` (L₀ CTE) for the delete branch in each join operator
- [ ] Add Part-3 correction term
- [ ] Regression test: delete matching rows from both sides of a join in the same refresh window; output must match `DuckDbHarness` full recompute

### Aggregate Tier Classification

Annotate every `AggregateKind` variant with one of three tiers and wire up the corresponding auxiliary state in `IvmTrace`. Without this, AVG/STDDEV drift on large update workloads and MIN/MAX correctness breaks on deletes.

| Tier | Aggregates | Auxiliary state | Δ computation |
|------|-----------|-----------------|---------------|
| **Algebraic** | COUNT, SUM, AVG, STDDEV, VAR, CORR, REGR_*, BOOL_AND/OR, BIT_AND/OR/XOR | `sum_arg`, `count_arg`, `M2`, `nonnull_count` | Fully invertible; no source rescan needed |
| **Semi-algebraic** | MIN, MAX | Current extremum | LEAST/GREATEST on insert; rescan group on delete of current extremum |
| **Group-rescan** | STRING_AGG, ARRAY_AGG, JSON_AGG, MODE, PERCENTILE_* | Current value only | Re-aggregate entire affected group on each delta |

- [ ] `AggregateKind` variants in `plan.rs` carry a `tier: AggregateTier` annotation
- [ ] Algebraic aggregates persist auxiliary columns in `IvmTrace`: `sum_arg` + `count_arg` for AVG; `M2` / `sum` / `count` for STDDEV
- [ ] AVG delta: `new_result = (old_sum_arg ± Δsum) / (old_count_arg ± Δcount)`; no floating-point drift, fully invertible
- [ ] Semi-algebraic MIN/MAX: on delete of current extremum, issue a group-rescan from source; otherwise merge with LEAST/GREATEST
- [ ] Group-rescan path implemented in `trace.rs`: re-reads all rows for affected group keys from the input source
- [ ] Group-rescan aggregates accepted but documented as higher-latency; clear error if input source is unavailable for rescan

### Volatility Validation (Correctness Gate)

DuckDB functions fall into `IMMUTABLE`, `STABLE`, and `VOLATILE` categories. Without this gate, views using `random()` or `clock_timestamp()` produce silently wrong incremental results.

- [ ] Walk the view SQL expression tree at `IvmPlan::compile`; look up each function's volatility from DuckDB's catalog
- [ ] VOLATILE functions: return `SQLSTATE 0A000` at view creation with a message naming the offending function
- [ ] STABLE functions (`now()`, `current_timestamp`): emit `WARN`-level log; accept but recommend capture-semantics path (v0.15)
- [ ] IMMUTABLE: always accepted silently

### Property-Based "Differential ≡ Full" Oracle

The foundational correctness harness that all future IVM tests depend on: after each DML mutation, compare the IVM worker's output multiset to a DuckDB single-shot reference execution of the same view SQL.

- [ ] `slateduck-testkit` gains an `IvmOracle` helper: given view SQL + DML sequence → run IVM worker → compare output to `DuckDbHarness` reference via multiset equality
- [ ] `proptest` strategies for random `INSERT` / `UPDATE` / `DELETE` sequences with realistic key distributions; includes phantom-delete edge cases (both-sides delete in same refresh window)
- [ ] TPC-H Q1 end-to-end correctness test: 1 000 random input snapshots, zero correctness drift, exercises aggregate auxiliary columns and EC-01 join fix simultaneously

### Acceptance Criteria

- [ ] EC-01 regression test passes: concurrent same-window delete from both join sides produces correct output matching DuckDB full recompute
- [ ] AVG over 1M rows with 100k updates shows zero floating-point drift vs. DuckDB reference
- [ ] `VOLATILE` function at view creation returns `SQLSTATE 0A000`
- [ ] Property-based oracle passes 1 000 random DML sequences against TPC-H Q1

### Deliverables

- [ ] `join.rs` EC-01 Part 1a/1b/2/3 split with L₀ CTE caching
- [ ] `plan.rs` `AggregateTier` enum and per-`AggregateKind` tier annotation
- [ ] `trace.rs` auxiliary column storage for algebraic aggregates (AVG/STDDEV)
- [ ] `trace.rs` group-rescan path for semi-algebraic and group-rescan tier aggregates
- [ ] `plan.rs` `IvmPlan::compile` volatility gate (VOLATILE reject / STABLE warn)
- [ ] `slateduck-testkit` `IvmOracle` helper + `proptest` DML strategies
- [ ] TPC-H Q1 property-based correctness test green in CI

---

## v0.14 — IVM Operational Hardening

> Production-ready IVM. Cost optimization, fault injection, native persistence backend, observability, and operator tooling. After v0.14 the IVM track is folded into the v1.0 GA story.

### Native `SlateDbTrace` Implementation

Replace DBSP's bundled persistence with a native trace implementation directly over SlateDB. This is the deferred work from v0.11; it unlocks the cost optimizations below.

- [ ] `SlateDbTrace` implements DBSP's persistent `Trace`, `Batch`, and `Cursor` traits
- [ ] Frontier advancement mapped to SlateDB compaction
- [ ] Direct mapping of DBSP batch boundaries to SlateDB SST flushes
- [ ] Benchmark: native trace ≥ 1.5× faster than v0.11 baseline at equal correctness
- [ ] Property-tested against DBSP's reference in-memory trace

### Cost Optimization

The naive implementation flushes a SlateDB batch on every input snapshot, generating thousands of small SSTs per day per shard. Mitigations:

- [ ] Coalesce flushes: only flush when `time-since-last-flush > freshness/2` *and* buffered work exists
- [ ] `await_durable = false` for non-checkpoint writes; `await_durable = true` only at checkpoint boundaries
- [ ] Aggressive compaction policy for matview state stores (configurable per matview)
- [ ] Documented cost model: API calls per million input rows × shard count × freshness, with empirical numbers on S3 Standard, S3 Express, GCS, R2

**`--cost-mode` propagation to IVM workers.** v0.9's `--cost-mode {conservative|balanced|latency}` flag (originally scoped to `slateduck-pgwire`) is extended to `slateduck-ivm serve`. Mode-to-default mapping:

| Knob | conservative | balanced (default) | latency |
|------|-------------|--------------------|---------|
| Flush coalescing window | `freshness` | `freshness/2` | `freshness/4` |
| `await_durable` for non-checkpoint writes | false | false | false |
| `await_durable` at checkpoint | true | true | true |
| State-store compaction trigger | aggressive | default | lazy |
| Cost budget warning threshold | 1.0× budget | 1.5× budget | 2.0× budget |

Per-view `WITH (...)` options always override mode defaults. Documented in `docs/operations/ivm-cost-control.md`.

- [ ] `slateduck-ivm serve --cost-mode=...` accepted and honoured
- [ ] Mode defaults documented per knob; per-view overrides take precedence
- [ ] Cost-mode interaction matrix tested in cost-model regression suite

### State Store Backup & Restore

The v0.4 checkpoint API backs up the catalog. Per-shard IVM state stores under `--state-prefix` are **not** included in catalog checkpoints (they are derivative state, recomputable from base data) but operators still need a backup procedure to avoid expensive full rebuilds after object-store corruption or accidental prefix deletion.

- [ ] `slateduck-ivm backup --matview v --shard N --destination s3://...` issues SlateDB's native `Checkpoint` against the shard's state store and writes a manifest to the destination
- [ ] `slateduck-ivm restore --matview v --shard N --source s3://...` rehydrates a state store from a backup; the next worker to claim the shard's lease resumes from the restored frontier
- [ ] If a state store is missing entirely at lease-claim time, the worker emits `WARN`-level `state_store_missing` and waits for an operator decision (auto-rebuild gated behind `--auto-rebuild-on-loss` flag, default off) — never silently recomputes terabytes of state
- [ ] Documented backup cadence guidance: daily for large matviews; on-demand before any infra migration
- [ ] `docs/operations/ivm-backup-restore.md` published

### Cost Guardrails (User-Facing)

IVM can generate real S3 API costs at scale. Users need visibility and protection *before* they get an unexpected bill.

- [ ] **Cost estimator at view creation.** `EXPLAIN MATERIALIZED VIEW v` includes estimated monthly S3 API cost based on: input rate (from recent snapshot commit frequency), shard count, freshness target, and empirical cost-per-million-rows from the v0.14 cost model
- [ ] **Per-view cost budget.** `WITH (monthly_cost_limit = '$50')` option; workers throttle freshness (relax from 5 s toward 60 s) when projected cost exceeds budget. Clear warning surfaced in `SHOW MATERIALIZED VIEWS`
- [ ] **Automatic freshness degradation.** When cost exceeds budget, freshness widens gracefully rather than stopping the view. Workers reduce flush frequency proportionally. View remains correct, just staler
- [ ] **Per-worker cost tracking.** `slateduck_ivm_estimated_monthly_cost{matview, shard}` metric; `slateduck-ivm doctor` reports per-view projected monthly cost
- [ ] **Cost ceiling alert.** If any view's projected monthly cost exceeds the budget by 2× (burst scenario), emit `WARN`-level log and Prometheus alert. No automatic stop (correctness over cost), but operator visibility is immediate
- [ ] **Documentation.** `docs/operations/ivm-cost-control.md`: how to estimate costs before creating views, how budgets work, how to diagnose cost spikes, rules of thumb (freshness↑ = cost↓, shards↓ = cost↓)

### Backpressure & Per-Shard Publication Modes

- [ ] Backpressure protocol: workers stall ingest when output plane is N snapshots behind (default N = 100)
- [ ] Per-shard `output_mode = 'per_shard'` publishes individual shard frontiers; query layer merges
- [ ] Skewed-shard detection: emit warning when one shard's lag exceeds 5× the median
- [ ] Hot-key mitigation guidance in operator playbook

### Delete-File Support

- [ ] Input source emits `(row, -1)` updates for rows newly covered by delete files
- [ ] Aggregations over deletable base tables correctly subtract deleted rows
- [ ] Documented constraint: large delete campaigns may require `REFRESH ... FULL` for non-monoidal aggregates
- [ ] Tested with DuckLake delete files at various scales

### Schema Evolution

- [ ] Adding a column to a base table the view does not reference: no-op
- [ ] Adding a column the view does reference: view marked stale, requires `REFRESH ... FULL`
- [ ] Column type change: view marked stale
- [ ] Renaming a column referenced by a view: view marked stale (re-creation required)
- [ ] All stale states surfaced in `SHOW MATERIALIZED VIEWS` with a clear `status` column

### Exactly-Once Output Snapshots

- [ ] Each output snapshot tagged with `(matview_id, target_frontier)` in its catalog metadata
- [ ] `CatalogWriter` CAS prevents a duplicate snapshot for the same `(matview_id, target_frontier)`
- [ ] Worker restart mid-output-commit cannot produce duplicate data files
- [ ] Tested under fault injection: kill -9 during every Parquet write and catalog commit step

### Observability

- [ ] Per-matview metrics: `ivm_lag_ms`, `ivm_throughput_rps`, `ivm_state_size_bytes`, `ivm_s3_puts_total`, `ivm_s3_gets_total`, per shard
- [ ] OpenTelemetry traces from input read → DBSP circuit → state write → output commit
- [ ] `slateduck-ivm doctor` CLI: reports stuck shards, expired leases, lagging frontiers, cost outliers
- [ ] Prometheus exporter compatible with existing `slateduck-pgwire` observability story

### `REFRESH ... FULL` & Repair

- [ ] `REFRESH INCREMENTAL MATERIALIZED VIEW v FULL` drops state stores, rebuilds from scratch in parallel
- [ ] `slateduck-ivm repair --matview v --shard N` recomputes a single shard from base data
- [ ] Repair operations leave a durable audit trail in `matview_checkpoints`

### Fault Injection Test Suite

- [ ] `fail-parallel` harness covers: worker death mid-batch, mid-commit, mid-output; lease expiry races; S3 partial failures; SlateDB compaction during checkpoint
- [ ] All scenarios survive a 1-hour soak test without correctness loss
- [ ] Documented in `docs/contributing/testing.md`

### Testing: Tier 6d (Hardening), Tier 7 (Fault Injection), Tier 9 (Security) & Tier 10 (Benchmark Regression)

- [ ] **Tier 6d — IVM hardening tests** (`crates/slateduck-ivm/tests/hardening_tests.rs`): 4 tests — repair shard rebuilds from base, `REFRESH ... FULL` rebuilds all shards, `doctor` identifies stuck/expired shards, exactly-once output under output-plane restart
- [ ] **Tier 7 — IVM fault injection** (`crates/slateduck-ivm/tests/fault_injection_tests.rs`): 4 `fail_point!` tests — kill after DBSP before flush, kill after flush before checkpoint, kill output plane after Parquet write before catalog commit, S3 `GetObject` 503 with retry; gated behind `--features fault-injection`
- [ ] **Tier 7 — Catalog fault injection** (`crates/slateduck-catalog/tests/fault_injection_tests.rs`): 4 tests — `create_snapshot` panic before commit, IO error after `put` before `flush`, `extend_lease` CAS conflict, `CounterCache` panic with reload verification
- [ ] **Tier 7 — Network fault injection** via `toxiproxy` Testcontainers proxy in front of MinIO: S3 PUT 503, GET truncated, heartbeat partition, 10 s latency degradation — all confirming no data loss and graceful degradation
- [ ] **Tier 9 — Security tests** (`crates/slateduck-pgwire/tests/security_tests.rs`): MinIO ACL credential-isolation (4 tests), TLS expired cert rejection, TLS CA validation, SCRAM-SHA-256 auth, brute-force rate limiting, SQL injection guard (3 tests), non-deterministic function blocked in view SQL
- [ ] **Tier 10 — Benchmark regression CI** (weekly scheduled job): extended `catalog_bench.rs` with 5 new benchmarks; `scripts/check_benchmark_regression.py` compares against `benchmarks/phase-2-baseline.json`; job fails if any metric regresses > 10%
- [ ] All Tier 7 tests are pre-release gate (run on tag push, not every PR)
- [ ] All Tier 9 security tests run on the standard large runner (MinIO covers credential isolation; no real AWS required)

### Multi-View DAG and Frontier Coordination

Foundation for views that read from other materialized views (`CREATE INCREMENTAL MATERIALIZED VIEW b AS SELECT … FROM a` where `a` is itself a materialized view). Without topological ordering and diamond detection, convergent views compute deltas against inconsistent intermediate state. See [plans/pg-trickle.md](plans/pg-trickle.md) §5.

- [ ] New `crates/slateduck-ivm/src/dag.rs`: Kahn's topological sort (O(V+E)) over the view dependency graph; guarantees upstream views are fully refreshed before any downstream consumer reads their delta
- [ ] Diamond detection: during topo-sort, track the set of ancestor root nodes per node; a node reachable from the same root via two or more paths is a diamond apex; O(V+E)
- [ ] Persist view dependency edges in `slateduck-catalog` (tag in existing matview key range; see `plans/blueprint.md` §9.1)
- [ ] Frontier vector clocks in `state_store.rs`: `BTreeMap<SourceId, Sequence>` per view per shard, persisted durably; worker reads on (re)start and skips CDC events with `seq ≤ frontier[source]`
- [ ] Diamond `Slowest` consistency policy: a convergence view (diamond apex D) refreshes only when **all** upstream views have advertised `frontier ≥ F` via their state stores; purely frontier-driven, no SAVEPOINT or advisory lock needed
- [ ] `EXPLAIN MATERIALIZED VIEW v` extended to show dependency graph, detected diamonds, and current frontier per source
- [ ] Unit test: diamond topology (A→B, A→C, B→D, C→D); assert D never refreshes with mismatched B/C frontiers

### Acceptance Criteria

- [ ] Native `SlateDbTrace` 1.5× faster than v0.11 on TPC-H Q1 streaming benchmark
- [ ] Steady-state S3 PUT cost ≤ 2× SlateDB's bare-substrate cost for the same write volume
- [ ] All fault-injection scenarios pass deterministically
- [ ] `slateduck-ivm doctor` correctly identifies every fault class in the test suite
- [ ] Continuous-soak test: TPC-H Q1 maintained for 24 h with zero correctness drift (runs on scale-test infrastructure, see Cross-Cutting: Scale Testing Infrastructure)
- [ ] All v0.11–v0.13 acceptance tests still pass
- [ ] IVM worker K8s deployment pattern tested with 4-worker pool and rolling updates
- [ ] **Tier 6d hardening tests green** (4 tests including repair and exactly-once output)
- [ ] **Tier 7 fault injection suite green** on every pre-release tag: catalog faults (4), IVM worker faults (4), network faults via toxiproxy (4)
- [ ] **Tier 9 security suite green**: credential isolation, TLS, auth, SQL injection guards — 14 tests total
- [ ] **Tier 10 benchmark regression < 10%** on weekly CI run vs `benchmarks/phase-2-baseline.json`

### Deliverables

- [ ] Native `SlateDbTrace` shipped
- [ ] Cost-optimization knobs documented and defaulted sensibly
- [ ] Observability surface complete (metrics, traces, `doctor` CLI)
- [ ] `REFRESH ... FULL` and per-shard repair shipped
- [ ] Tier 6d hardening tests (`hardening_tests.rs`) with 4 passing tests
- [ ] Tier 7 fault injection suites (`fault_injection_tests.rs` in catalog + ivm) with 12 passing tests
- [ ] Tier 9 security test suite (`security_tests.rs`) with 14 passing tests
- [ ] Tier 10 benchmark regression job in `.github/workflows/ci.yml` (weekly cron)
- [ ] `scripts/check_benchmark_regression.py` with 10% threshold gate
- [ ] `benchmarks/v0.14-ivm-hardening.json` published
- [ ] Final IVM operator playbook in `docs/operations/incremental-materialized-views.md`
- [ ] IVM design retrospective in `docs/design-decisions/ivm-retrospective.md` capturing what survived from the design and what changed

---

## v0.15 — IVM Feature Completeness

> Expand the IVM SQL surface to full feature-parity with a general-purpose streaming SQL engine: window functions, ordered results, top-N materialization, correlated subqueries, recursive computation, non-deterministic functions with deterministic capture semantics, and WASM user-defined functions. After v0.15 the answer to "what SQL can a materialized view use?" is: anything you can write against a static DuckDB table.

### Why feature completeness before v1.0

SlateDuck's goal is to be the only lakehouse that materializes *any* SQL view without leaving S3. A restricted SQL surface invites the question "what can't it do?" v0.15 closes that gap. Each feature adds real implementation complexity, but the architectural seams — ordered traces, UDF registry, deterministic timestamp capture — are far cheaper to design before GA than to retrofit afterward. v1.0 should mean something complete.

### Window Functions

`ROW_NUMBER`, `RANK`, `DENSE_RANK`, `LAG`, `LEAD`, `FIRST_VALUE`, `LAST_VALUE`, `NTH_VALUE`, `NTILE`, and all aggregate windows (`SUM/AVG/COUNT OVER (PARTITION BY … ORDER BY …)`). Requires ordered collections and per-partition state, not unordered sets.

**Design impact.** Partition-local windows (PARTITION BY = shard key) are fully parallel and cost the same as equivalent `GROUP BY`. Full-table or cross-partition windows require a single-shard merge stage; the output plane gains a merge-sort writer for ordered views.

- [ ] `PARTITION BY` windows where partition key = shard key: fully parallel, same throughput as aggregation
- [ ] `PARTITION BY` windows where partition key ≠ shard key: route to single-shard merge stage
- [ ] Full-table windows (no PARTITION BY): `shard_count = 1` enforced at create time with a clear error message if user attempts sharded
- [ ] `SlateDbOrderedTrace` extending `SlateDbTrace` with per-partition sort order
- [ ] Output plane `merge_sorted_parquet_writer` for total-ordered output tables
- [ ] Supported window frames: `ROWS BETWEEN`, `RANGE BETWEEN`, `GROUPS BETWEEN`
- [ ] Navigation functions: `LAG`, `LEAD`, `FIRST_VALUE`, `LAST_VALUE`, `NTH_VALUE`
- [ ] Ranking functions: `ROW_NUMBER`, `RANK`, `DENSE_RANK`, `PERCENT_RANK`, `CUME_DIST`, `NTILE`
- [ ] `WITH (window_mode = 'partitioned' | 'total_order')` option; auto-selected from SQL plan if unambiguous
- [ ] TPC-DS Q4 and Q11 maintained incrementally
- [ ] Partition-local window throughput within 15% of equivalent aggregation

### `ORDER BY` in Materialized Views

A top-level `ORDER BY` implies a total order on the output; Parquet is physically sorted and pre-ordered reads require no runtime sort.

- [ ] `ORDER BY` accepted in view SQL; stored as `output_sort_key` in `MatviewRow`
- [ ] Output Parquet written with `sorting_columns` metadata
- [ ] Multiple `ORDER BY` columns with `ASC`/`DESC`/`NULLS FIRST`/`NULLS LAST`
- [ ] `shard_count = 1` auto-enforced for total-order views
- [ ] DuckDB scan of an `ORDER BY` matview delivers rows in declared order without a runtime sort (verified by query plan inspection)

### `LIMIT` / `OFFSET` (Top-N Materialized Views)

`LIMIT N` materializes only the top N rows by a specified order — "latest N events", "top N customers", "most recent N records". Common pattern; cheap with DBSP's `top_k` operator.

- [ ] `LIMIT N [OFFSET M]` requires `ORDER BY`; error if absent
- [ ] Incremental top-N via DBSP `top_k`: bounded sorted heap of N candidates maintained across updates
- [ ] Output Parquet contains exactly N rows; previous output superseded atomically on each publish
- [ ] Sharded top-N: each shard maintains local top-N; merge shard selects global top-N from `shard_count × N` candidates
- [ ] `OFFSET` only with `ORDER BY + LIMIT`; document stable-row-numbering caveat
- [ ] Tested with TPC-H "top 10 orders by value" maintained across 1000 input snapshots

### Correlated Subqueries

`WHERE EXISTS (SELECT … WHERE t.id = outer.id)`, `WHERE IN (SELECT …)`, scalar subquery in SELECT list. Requires re-evaluation of the inner query as outer rows change.

**Implementation approach.** Decorrelation via algebraic rewrites (same technique DataFusion uses for batch evaluation). Correlated `EXISTS` → semi-join; correlated scalar → left join + aggregation. After decorrelation the circuit contains only regular joins and aggregations.

- [ ] Decorrelation pass in `plan.rs` via DataFusion's `PullUpCorrelatedPredicates` / `DecorrelatePredicateSubquery` rewrites
- [ ] `EXISTS`, `NOT EXISTS`, `IN (SELECT …)`, `NOT IN (SELECT …)` → semi/anti-join
- [ ] Scalar correlated subquery in SELECT list → left join + aggregation
- [ ] Clear "cannot decorrelate" error for subqueries that escape the rewrite (deep mutual correlation)
- [ ] TPC-H Q4 (`WHERE EXISTS (SELECT … FROM lineitem WHERE …)`) maintained incrementally

### Recursive CTEs

`WITH RECURSIVE` enables transitive closure, hierarchical rollups, graph reachability. Requires feedback loops in the DBSP circuit and fixed-point termination.

**Implementation approach.** Map to DBSP's `iterate` operator: base case is the seed; recursive term is the iterate body; termination detected by frontier advancement (output = input at fixed point).

- [ ] Recursive CTEs identified in the SQL plan (cycles in CTE dependency graph)
- [ ] Lowered to DBSP `iterate` operators
- [ ] Bounded iteration: configurable `max_iterations` (default 100); exceeding it sets view to `Stale` and alerts
- [ ] `CONNECT BY`-style depth-bounded expansion (org-chart / BOM queries)
- [ ] Non-recursive `WITH` (already handled in v0.11 as inline subquery expansion) unchanged
- [ ] Transitive closure over a 1M-edge graph maintained incrementally as edges are added and removed
- [ ] Incremental per-batch latency ≤ 5× the non-recursive baseline for the same operator count

### Non-Deterministic Functions with Capture Semantics

`now()`, `current_timestamp`, `random()`, `gen_random_uuid()` are non-deterministic but users legitimately need views like `SELECT *, now() AS captured_at FROM events`. Fix: sample once per batch, substitute a literal, store the value alongside the checkpoint for deterministic repair/replay.

- [ ] Allow-listed functions: `now()`, `current_timestamp`, `current_date`, `current_time`, `localtime`, `localtimestamp`, `random()`, `gen_random_uuid()`
- [ ] Per-batch sampling: each listed function sampled once at the start of a DBSP batch; substituted as a literal throughout
- [ ] Sampled value stored in the checkpoint row for deterministic repair (repair re-uses captured value, not re-sampled)
- [ ] `current_snapshot_id()` — new IVM-specific function returning the batch's `last_input_snapshot` as a stable integer
- [ ] `random()` / `gen_random_uuid()` subject to a per-batch seed stored in checkpoint (enables deterministic replay)
- [ ] Error on functions that cannot be safely allow-listed (volatile functions with side effects)
- [ ] "Capture semantics" section in `docs/concepts/incremental-views.md`

### User-Defined Functions (WASM)

UDFs extend the view SQL surface with custom logic: custom hash functions, domain-specific type coercions, scoring models. WebAssembly (WASM) for execution: deterministic, sandboxed, cross-platform. Compiled modules stored as binary blobs in the catalog.

- [ ] New catalog table `matview_udfs` (tag `0x21`): `(udf_id, name, schema_name, wasm_blob, signature, deterministic, created_at_snapshot)`
- [ ] `CREATE FUNCTION name(arg_type, …) RETURNS type LANGUAGE WASM AS '…'` DDL surface
- [ ] `DROP FUNCTION`, `ALTER FUNCTION … REPLACE` (bumps `udf_id`; views pin to specific `udf_id` at creation)
- [ ] WASM execution via `wasmtime` embedded in `slateduck-ivm`; sandboxed (no I/O, no network, bounded fuel + memory)
- [ ] `deterministic = true` annotation required; non-deterministic UDFs rejected at view creation with a clear error
- [ ] UDF versioning: view pins to `udf_id` at creation; `ALTER INCREMENTAL MATERIALIZED VIEW v USING FUNCTION f VERSION N` migrates and triggers `REFRESH … FULL`
- [ ] Argument and return types limited to Arrow-compatible scalars: BOOLEAN, INT8–INT64, FLOAT32/FLOAT64, UTF8, BINARY, DATE32, TIMESTAMP
- [ ] Fuel limit: 10M instructions per row; memory limit: 64 MiB per invocation; violation → clean error, not panic
- [ ] WASM module validates against a whitelist of allowed WASI imports (none for pure functions)
- [ ] Tested with a custom tokenizer UDF over event strings maintained incrementally
- [ ] `docs/reference/udfs.md`: authoring guide, WASM compilation instructions (Rust → wasm32-unknown-unknown), determinism contract, version migration

### Incremental Delta Optimizations

A set of targeted optimizations derived from pg-trickle's production experience (see [plans/pg-trickle.md](plans/pg-trickle.md) §7–§8). Each is a self-contained PR and can be landed independently.

**Adaptive DIFFERENTIAL/FULL mode switching (`CostMode::Adaptive`).** At low delta rates, DIFFERENTIAL is 5–90× cheaper than FULL. At high delta rates the crossover reverses. Without this switch, a large delta batch silently tanks throughput.

- [ ] `CostMode::Adaptive` variant in `config.rs`
- [ ] Per-view rolling statistics tracked in the state store and surfaced via `observability.rs`: `rows_in`, `rows_out`, `ms_spent`, `last_full_cost`
- [ ] Query complexity multiplier table: `Scan 1.0×`, `Filter 1.1×`, `Aggregate 1.5×`, `Join 2.5×`, `JoinAggregate 4.0×`; switch DIFFERENTIAL→FULL when `Δ_rows / N_rows × multiplier > threshold` (default 0.5)
- [ ] `WITH (cost_mode = 'adaptive', adaptive_threshold = 0.3)` per-view override; documented in `docs/operations/ivm-cost-control.md`

**Change-buffer compaction.** Consecutive INSERT/DELETE pairs on the same `row_id` cancel out; applying this before writing to the trace cuts buffer size 50–90% on high-update workloads.

- [ ] In `source.rs`: coalesce delta batches before landing in `IvmTrace`; cancel `(INSERT row_id=X) + (DELETE row_id=X)` pairs within the same batch
- [ ] Expose compaction ratio (`pairs_cancelled / total_events`) per refresh cycle in metrics

**Predicate pushdown into delta scan.** When a `Filter` sits directly above a `Scan`, push the WHERE predicate into the CDC fetch so unfiltered delta rows are never materialised.

- [ ] In `plan.rs`: detect `Filter(Scan(R))` pattern; pass predicate as parameter to the CDC source read
- [ ] For UPDATE rows: apply predicate to both `old_` and `new_` column values
- [ ] Correctness test: view with selective WHERE; confirm delta bytes read ∝ matching rows, not total delta size

**Semi-join key pre-filter.** For `delta_orders ⋈ customers`, project `DISTINCT join_key` from the delta side first and use it as the probe set; turns a full probe-side scan into an indexed lookup.

- [ ] In `join.rs`: when probe side is a full-table scan and build side is a delta, inject a `DISTINCT join_key` pre-filter on the probe side before `hash_join_batch`
- [ ] Benchmark: join throughput with and without pre-filter on TPC-H Q3 at varying delta sizes

**Append-only fast path.** For INSERT-only views, skip the negative-multiplicity path entirely (~30% throughput gain).

- [ ] Detect INSERT-only workload automatically (no DELETE or UPDATE events in last N batches; N configurable)
- [ ] Skip negative-multiplicity accumulation in `IvmTrace`; use plain INSERT accumulation
- [ ] Automatically revert to full bidirectional mode on first DELETE or UPDATE event

**Auto sort-by on join and group-by keys.** Layout Parquet output files sorted by GROUP BY and equi-join keys so downstream DuckDB scans can use sorted-file skip-scan.

- [ ] `parquet.rs::CompactionPolicy`: add `sort_keys: Vec<ColumnName>` field
- [ ] At view creation, auto-populate `sort_keys` from the SQL plan's GROUP BY and equi-join key columns
- [ ] Write output Parquet with `sorting_columns` metadata

**Reference-counted DISTINCT and set operators.** The current DISTINCT implementation does not track duplicate counts, producing incorrect output when the same row is inserted multiple times and then partially deleted.

- [ ] Add `__sd_ref_count: i64` auxiliary column to `IvmTrace` for views containing `DISTINCT` or `UNION DISTINCT` / `INTERSECT` / `EXCEPT`
- [ ] INSERT increments `__sd_ref_count`; DELETE decrements; row visible in output only when `__sd_ref_count > 0`
- [ ] `UNION DISTINCT`: union of ref counts; `INTERSECT`: minimum of counts; `EXCEPT`: subtraction of counts
- [ ] Correctness test: insert same row 3×, delete 2×; confirm exactly one output row

### Extended Operator Support Matrix

| Operator | v0.11 | v0.12 | v0.13 | v0.14 | v0.15 |
|---|---|---|---|---|---|
| `SELECT` / `WHERE` / `GROUP BY` / `HAVING` | ✓ | ✓ | ✓ | ✓ | ✓ |
| Aggregates (count, sum, min, max, avg) | ✓ | ✓ | ✓ | ✓ | ✓ |
| `DISTINCT`, `UNION ALL/DISTINCT` | ✓ | ✓ | ✓ | ✓ | ✓ |
| `JOIN` (broadcast, co-partition, reshuffle) | — | — | ✓ | ✓ | ✓ |
| Uncorrelated subqueries | — | — | ✓ | ✓ | ✓ |
| `ORDER BY` (total-order output) | — | — | — | — | ✓ |
| `LIMIT` / `OFFSET` (top-N) | — | — | — | — | ✓ |
| Window functions (partitioned) | — | — | — | — | ✓ |
| Window functions (total-order) | — | — | — | — | ✓ |
| Correlated subqueries (`EXISTS`, `IN`, scalar) | — | — | — | — | ✓ |
| `CONNECT BY` / depth-bounded recursion | — | — | — | — | ✓ |
| Recursive CTEs | — | — | — | — | ✓ |
| `now()` / `random()` (capture semantics) | — | — | — | — | ✓ |
| User-defined functions (WASM) | — | — | — | — | ✓ |

### Testing: Tier 8 (Scale & Soak)

- [ ] **Tier 8 — TPC-H catalog benchmarks** (`tests/scale/tpch_catalog.rs`): `tpch_sf10_catalog_latency` and `tpch_sf100_catalog_latency` against real S3 Standard; p99 `get_current_snapshot` < 50 ms at SF10, < 100 ms at SF100; results written to `benchmarks/v0.15-tpch-{date}.json`
- [ ] **Tier 8 — TPC-H IVM streaming** (`tests/scale/tpch_ivm.rs`): Q1, Q3, Q5 at 100k rows/s, 8 shards, 5 s freshness; lag p99 < 5 s; verified on MinIO (same-host) and S3 Standard
- [ ] **Tier 8 — 24-hour soak test** (`tests/scale/soak.rs`): TPC-H Q1 continuous ingest; correctness drift check every 15 min (output row count matches DuckDB reference); fault injection every 15 min; `ivm_circuit_panic_total` = 0 after T+1h; **soak failure blocks GA tag**
- [ ] **Tier 8 — 16-shard scale-out benchmark**: 16 workers on 16 separate instances, 1M rows/s ingest, aggregate throughput ≥ 500k rows/s, lag p99 ≤ 3 s
- [ ] Scale and soak tests run on dedicated EC2 `c6i.4xlarge` via self-hosted GitHub Actions runner; triggered manually and on `v*` release tags
- [ ] CI comparison job alerts if any Tier 8 metric regresses > 10% vs previous run
- [ ] Scale test setup documented in `docs/contributing/testing.md` under "Scale Testing Infrastructure"

### Acceptance Criteria

- [ ] Every operator in the matrix passes a correctness test against a DuckDB single-shot reference query over the same input data
- [ ] Partition-local `ROW_NUMBER() OVER (PARTITION BY … ORDER BY …)` maintained correctly for 1000 input snapshots; throughput within 15% of equivalent aggregation
- [ ] Transitive closure over 1M edges processes 10k-edge incremental batches in ≤ 10 s
- [ ] `LIMIT 100 ORDER BY value DESC` view correctly maintains the global top-100 across 1000 input snapshots
- [ ] `now()` capture: repaired shard re-uses stored captured value, not re-sampled; output is bit-identical to original
- [ ] WASM UDF exceeding fuel/memory limit returns a clean error; no worker panic, no view corruption
- [ ] All v0.11–v0.14 acceptance tests still pass
- [ ] Extended benchmark: TPC-DS Q4, Q11, Q14, Q47, Q49 maintained incrementally with correctness verified
- [ ] **Tier 8 soak test passes**: 24 h with zero correctness drift and fault injection recovery within SLO on every pre-release run
- [ ] **Tier 8 TPC-H p99 within targets**: SF10 < 50 ms catalog, SF100 < 100 ms catalog; IVM lag p99 < 5 s at 8 shards
- [ ] **16-shard scale benchmark**: aggregate throughput ≥ 500k rows/s, lag p99 ≤ 3 s
- [ ] `CostMode::Adaptive` correctly switches DIFFERENTIAL→FULL when `Δ_rows/N_rows × complexity > 0.5`; verified on TPC-H Q1 with synthetic 60%-delta batches
- [ ] Change-buffer compaction reduces CDC event count by ≥ 50% on a 100%-update synthetic workload
- [ ] Predicate-pushdown confirmed: CDC bytes read proportional to WHERE-matching rows, not total delta size
- [ ] Append-only fast path shows ≥ 25% throughput improvement on a pure-INSERT TPC-H Q1 variant
- [ ] DISTINCT reference counting correct: insert-3×-delete-2× produces exactly one output row
- [ ] All 10 test tiers green (Tiers 1–7 and 9–10 from prior phases; Tier 8 from this phase)

### Deliverables

- [ ] `SlateDbOrderedTrace` implementation
- [ ] Merge-sort output writer in the output plane
- [ ] Decorrelation pass in `plan.rs`
- [ ] DBSP `iterate` integration for recursive CTEs
- [ ] Non-deterministic function capture with per-batch seed storage
- [ ] `matview_udfs` catalog table (tag `0x21`) and `CREATE/DROP/ALTER FUNCTION` SQL surface
- [ ] `wasmtime` integration in `slateduck-ivm` with fuel + memory sandboxing
- [ ] Tier 8 scale + soak test suite (`tests/scale/`) with TPC-H catalog, IVM streaming, and 24 h soak tests
- [ ] Self-hosted EC2 runner configuration documented in `docs/contributing/testing.md`
- [ ] TPC-DS Q4/Q11/Q14/Q47/Q49 streaming benchmark suite in `benches/`
- [ ] `benchmarks/v0.15-ivm-feature-complete.json` published
- [ ] `docs/reference/udfs.md` authoring guide
- [ ] All SQL reference docs in `docs/reference/sql-ivm.md` updated to reflect full operator coverage
- [ ] `CostMode::Adaptive` with per-view rolling cost statistics in `config.rs` and `worker.rs`
- [ ] Change-buffer compaction in `source.rs`
- [ ] Predicate pushdown and semi-join key pre-filter in `plan.rs` and `join.rs`
- [ ] Append-only fast path detection in `IvmTrace`
- [ ] `parquet.rs::CompactionPolicy` `sort_keys` auto-population from SQL plan GROUP BY / join keys
- [ ] `__sd_ref_count` auxiliary column for DISTINCT and set operators in `trace.rs`
- [ ] Implementation plan [plans/incremental-view-maintenance-implementation.md](plans/incremental-view-maintenance-implementation.md) updated to reflect v0.15 additions

---

## v0.16 — pg-trickle Compatibility

> Make SlateDuck a 100% drop-in replacement for PostgreSQL as the DuckLake catalog backend that pg-trickle targets. pg-trickle is a production-grade PostgreSQL IVM extension that can both *read from* DuckLake tables (O(Δ) via `table_changes()`) and *write IVM results back to* DuckLake (Parquet sink + snapshot commit). All of that traffic goes through the DuckLake catalog SQL API — exactly the PG-wire surface SlateDuck exposes. See [plans/pg-trickle-ducklake-support.md](plans/pg-trickle-ducklake-support.md) for the full gap analysis.

### Gap 1 — `table_changes()` SQL Function

Expose `reader.rs::SnapshotDiff` as a callable SQL table function over PG-wire:

```sql
SELECT rowid, change_type, <user_columns>
FROM table_changes('schema.table', start_snapshot := 42, end_snapshot := 45);
-- change_type ∈ { insert, delete, update_preimage, update_postimage }
```

Without this, pg-trickle falls back to O(N) polling (`EXCEPT ALL` full diff) instead of O(Δ) incremental CDC. For a 10M-row table with a 100-row delta, this is ~10⁷× more work per refresh cycle.

**Implementation:**
- Add `table_changes` to the bounded SQL dispatcher in `crates/slateduck-sql/src/`.
- Wire through `reader.rs::SnapshotDiff` with the DuckLake change-record vocabulary.
- Return `SQLSTATE 55000` (snapshot too old) when `start_snapshot` has been GC'd so pg-trickle can fall back gracefully to full refresh.

**Acceptance criteria:**
- [ ] `table_changes()` callable from DuckDB `ATTACH 'ducklake:postgresql://slateduck-sidecar/…'`
- [ ] pg-trickle `cdc_mode` reports `DUCKLAKE_CHANGE_FEED` when source is SlateDuck-backed DuckLake
- [ ] Property test: apply change records from `table_changes(start, end)` to `start` state → produces `end` state (multiset equality)

### Gap 2 — Stable `rowid` on DuckLake Tables

Every SlateDuck-managed DuckLake table must expose a stable `rowid` column that survives UPDATE, file compaction, and Parquet file re-registration. pg-trickle's EC-01 phantom-row fix (see `plans/pg-trickle.md` §4) matches insert/delete pairs by `rowid`; without it, delete deltas are silently dropped and stale rows accumulate in pg-trickle's stream tables.

**Implementation:**
- Derive `rowid` from the per-table monotone counter already available at key `0xFE | 0x10 | table_id`.
- Assign `rowid` at row-creation time; persist alongside the row in Parquet as a hidden column.
- Expose `rowid` in `table_changes()` output.
- Document the stability guarantee in `docs/concepts/ducklake.md`.

**Acceptance criteria:**
- [ ] `rowid` appears in `table_changes()` output
- [ ] `rowid` is stable across compaction, GC, and file splits (test with `slateduck compact` between two change windows)
- [ ] EC-01 test case: delete row from both source and joined table in same refresh window; pg-trickle stream table matches full recompute

### Gap 3 — Snapshot Lease / Hold Mechanism

GC must not advance past a snapshot ID that an external consumer (pg-trickle) has registered as its frontier. Otherwise, the next `table_changes(start_snapshot=42, …)` call returns `55000` and pg-trickle must do a full refresh unnecessarily.

**Implementation:**
- New catalog tag `0x22`: `snapshot_lease` with columns `(consumer_id TEXT, min_snapshot_id BIGINT, expires_at TIMESTAMPTZ)`.
- SQL function: `SELECT slateduck.hold_snapshot(min_snapshot_id := 42, consumer_id := 'pgtrickle:stream_1', ttl_seconds := 300)`.
- SQL function: `SELECT slateduck.release_snapshot(consumer_id := 'pgtrickle:stream_1')`.
- `gc.rs` reads minimum leased snapshot before advancing the visibility frontier.
- TTL prevents leaked leases from indefinitely blocking GC after ungraceful pg-trickle shutdown.

**Acceptance criteria:**
- [ ] GC blocked at leased snapshot; advances once lease released
- [ ] TTL expiry allows GC to advance after consumer disappears
- [ ] `slateduck.hold_snapshot()` / `slateduck.release_snapshot()` callable via PG-wire from pg-trickle

### Gap 4 — `NOTIFY` on Snapshot Advance

pg-trickle's event-driven scheduler wakes up immediately when a `NOTIFY pgt_source_changed_<relid>` is emitted. Without this, pg-trickle falls back to polling (default 1 s), adding latency.

**Implementation:**
- After each `INSERT INTO ducklake_snapshot` (any source), emit `NOTIFY pgt_source_changed_<table_id>` to all connected PG-wire clients that have issued a matching `LISTEN`.
- Implement `LISTEN channel` and `UNLISTEN channel` in `slateduck-pgwire`.
- Clean up subscriptions on connection close.

**Acceptance criteria:**
- [ ] `LISTEN`/`NOTIFY`/`UNLISTEN` round-trip via PG-wire
- [ ] pg-trickle `scheduler` uses event-driven mode (not polling) when connected to SlateDuck
- [ ] Latency test: snapshot advance → pg-trickle refresh start ≤ 50 ms end-to-end

### Gap 5 — Extension Schema Tables (`pgtrickle.*`)

pg-trickle issues `CREATE TABLE IF NOT EXISTS pgtrickle.pgt_ducklake_provenance (…)` and `INSERT INTO pgtrickle.pgt_ducklake_provenance (…)` against the catalog database at install time. SlateDuck's bounded SQL dispatcher currently returns `SQLSTATE 0A000` for user-schema DDL/DML.

**Implementation (minimal-viable):** Add a reserved extension-metadata key range (tag `0x23`) and handle `CREATE TABLE IF NOT EXISTS <extension_schema>.<table>` DDL for known extension schemas. Support `INSERT`, `SELECT`, `DELETE` against these tables.

**Alternative (lower-effort):** Provide a configuration shim in SlateDuck's `slateduck-pgwire` that silently ACKs `pgtrickle.*` writes and stores them in a sidecar SQLite file. Document this as the supported compatibility mode.

**Acceptance criteria:**
- [ ] pg-trickle installs without errors against SlateDuck
- [ ] `INSERT INTO pgtrickle.pgt_ducklake_provenance` succeeds
- [ ] `SELECT * FROM pgtrickle.pgt_ducklake_provenance` returns inserted rows

### Gap 6 — Encryption Key Pass-Through

When DuckLake per-file Parquet encryption is enabled, `INSERT INTO ducklake_data_file` includes an `encryption_key` column. Audit and validate that SlateDuck stores and returns this column without mangling it.

**Acceptance criteria:**
- [ ] `encryption_key` column present in `ducklake_data_file` schema
- [ ] Round-trip test: insert file with `encryption_key = '\xDEADBEEF…'`, select it back, bytes identical
- [ ] pg-trickle fixture corpus includes an encryption-key-bearing INSERT

### Gap 7 — Mixed Frontier (DuckLake Snapshot + WAL LSN)

For stream tables that read from both SlateDuck-backed DuckLake tables and PostgreSQL heap tables, the frontier must be a vector clock over heterogeneous source types.

**Implementation:**
- Extend frontier type in `state_store.rs`: `BTreeMap<SourceId, SourceFrontier>` where `SourceFrontier` is `{SequenceNumber(u64) | DuckLakeSnapshot(i64) | WalLsn(u64)}`.
- `plan.rs` must resolve each source's frontier type from `MatviewInputSource` variant.
- Serialize frontier as JSON (matching pg-trickle's `{"ducklake:lake.events": {"snapshot_id": 42}, "wal:postgres": {"lsn": "…"}}` format) for cross-system observability.

**Acceptance criteria:**
- [ ] View definition mixing DuckLake source + PG heap source plans and refreshes correctly
- [ ] Frontier serialized as JSON, visible in `pgt_stream_tables.frontier`

### pg-trickle Compatibility Test Suite

A dedicated test crate (or test module in `slateduck-testkit`) that validates the full pg-trickle × SlateDuck integration:

**Tier A — Catalog Write Compatibility:** replay pg-trickle's internal DuckLake catalog SQL corpus against SlateDuck PG-wire; assert no `0A000` errors and correct final state.

**Tier B — `table_changes()` Property Tests:** property-based test applying change records to reconstruct any target snapshot; multiset equality assertion.

**Tier C — End-to-End Pipeline (Docker):** actual pg-trickle container → PostgreSQL sources → SlateDuck sink → DuckDB query verification.

**Tier D — Snapshot Hold Under GC:** GC blocked by lease; advances after release; TTL expiry.

### Acceptance Criteria

All of the following must be green before v0.16 is tagged:

- [ ] pg-trickle connects to SlateDuck PG-wire sidecar with zero configuration changes vs. a standard PostgreSQL catalog
- [ ] `CdcMode::DUCKLAKE_CHANGE_FEED` activates automatically when source table is SlateDuck-backed DuckLake
- [ ] `table_changes()` passes the Tier-B property test suite
- [ ] pg-trickle sink (`sink => 'ducklake'`) writes Parquet and commits DuckLake snapshots through SlateDuck
- [ ] Provenance table (`pgtrickle.pgt_ducklake_provenance`) readable from pg-trickle
- [ ] Snapshot lease prevents GC from breaking pg-trickle's frontier
- [ ] `LISTEN`/`NOTIFY` round-trip enables event-driven scheduling
- [ ] Encryption key pass-through validated
- [ ] Tier A + B + D tests green in CI; Tier C green in pre-release gate
- [ ] `docs/operations/pgtrickle-compatibility.md` published

### Deliverables

- [ ] `table_changes()` SQL function in `crates/slateduck-sql/src/`
- [ ] Stable `rowid` implementation in `crates/slateduck-catalog/src/writer.rs` and `crates/slateduck-ivm/src/parquet.rs`
- [ ] Snapshot lease catalog tag `0x22` + `slateduck.hold_snapshot()` / `release_snapshot()` SQL API
- [ ] `LISTEN`/`NOTIFY`/`UNLISTEN` in `crates/slateduck-pgwire/src/`
- [ ] Extension schema compatibility shim for `pgtrickle.*` tables
- [ ] Encryption key column audit + fixture
- [ ] Mixed frontier support in `crates/slateduck-ivm/src/state_store.rs` and `plan.rs`
- [ ] Compatibility test suite: `tests/compat/pgtrickle_*.rs`
- [ ] `docs/operations/pgtrickle-compatibility.md`
- [ ] DuckLake Spec Upgrade Policy updated to include pg-trickle `CHANGELOG.md` in review process

---

## v1.0 — General Availability

> Formal TPC-H @ SF10/SF100 benchmark publication, S3 Express acceptance gate, IVM correctness gate, and GA sign-off.

### Full Benchmark Suite

TPC-H @ SF10 comparison across all three catalog backends — SlateDuck, DuckLake-on-PostgreSQL (RDS same AZ), and DuckLake-on-SQLite — for each operation family:

- `get_current_snapshot()` — 1 point read; cold-process and warm-cache
- `list_data_files(table)` — at 10⁴, 10⁵, and 10⁶ files; MVCC filter ratio measured separately
- `describe_table` — with 50, 100, 500 columns; measures MVCC amplification from historical versions
- `create_snapshot` — at 1, 10, 100, 1 000 file additions; measures write batching efficiency
- `prune_files` — single typed column predicate at 10⁵ files; measures zone-map vs. exact-stats path
- Cold-start read latency — time from fresh process open to first `get_current_snapshot()` response
- Concurrent reader throughput — 1, 4, 16 concurrent `DbReader` processes; linear scale expected

Run all benchmarks on: LocalFS, MinIO (same host), S3 Standard (same region), S3 Express One Zone. Publish p50/p95/p99/p99.9 for every combination. Store results in `benchmarks/v1.0-tpch-sf10.json`.

**S3 Express acceptance gate.** If `get_current_snapshot()` on S3 Express is within 2× of PostgreSQL p99, declare S3 Express the recommended production tier and document it prominently. If common S3 Express planning operations exceed 3× PostgreSQL p99 after v0.9 optimizations, document the gap and defer the production-readiness claim; correctness milestones may still ship as beta.

**Benchmark methodology.** All benchmarks run three warm-up iterations followed by thirty measured iterations. Cold-start benchmarks restart the process for every measured iteration. The benchmark binary is checked in under `benches/` and is runnable by any contributor with `cargo bench`. Results must be reproducible within ±10% across three independent runs on the same hardware; if variance exceeds that, identify and document the source before publishing.

### GA Sign-Off Success Criteria

Measurable acceptance criteria that must all be green before v1.0 is tagged:

1. Full DuckLake tutorial runs end-to-end from the standard DuckDB `ducklake` extension through the SlateDuck PG-wire sidecar, with catalog in S3 and no PostgreSQL or SQLite database required.
2. Concurrent reads from a second DuckDB process see consistent, snapshot-isolated catalog views.
3. `kill -9` on the writer mid-commit leaves the catalog readable and consistent; the next writer fences and takes over within the SLOs verified in v0.9.
4. Benchmarks published: p50/p95/p99 catalog latency vs. PostgreSQL-backed DuckLake on RDS and SQLite-backed DuckLake; cost crossover point documented.
5. Common S3 Express planning operations are within 3× of PostgreSQL p99 latency; if not, the gap is clearly documented with a v1.x optimization plan.
6. All 28 DuckLake v1.0 catalog tables implemented, tag-allocated, fixture-covered, and explicitly status-tracked in `tags.rs`.
7. Phase 0 validation gates pass on LocalFS, MinIO, S3 Standard, and S3 Express; results documented.
8. `mkdocs build --strict` green; documentation site live with no stub pages.
9. **IVM GA gate.** v0.11–v0.15 acceptance tests all green: single-shard demo, 8-shard scale-out, TPC-H Q1/Q3/Q5 maintained incrementally, window functions correct (partition-local and total-order), recursive CTEs stable under fixed-point iteration, correlated subqueries decorrelated, non-deterministic function capture reproducible on repair, WASM UDFs sandboxed under fuel + memory limits, 24 h soak with zero correctness drift, fault-injection suite passing, native `SlateDbTrace` benchmarked, operator playbook complete. IVM is feature-complete at v1.0: any SQL view that can be written against a static DuckDB table can be maintained incrementally by SlateDuck.
10. **Real-world validation gate.** At least 30 days of dogfood deployment on a realistic workload (see Cross-Cutting Concerns: Real-World Validation Policy). Friction log reviewed and all blocking findings resolved. One external-to-the-team developer has successfully deployed IVM using only published docs.
11. **IVM documentation gate.** Every IVM-track docs deliverable from v0.11–v0.15 is published and non-stub: `docs/concepts/incremental-views.md`, `docs/architecture/ivm-plane.md`, `docs/operations/incremental-materialized-views.md`, `docs/operations/ivm-cost-control.md`, `docs/operations/ivm-backup-restore.md`, `docs/operations/ivm-upgrades.md`, `docs/reference/sql-ivm.md`, `docs/design-decisions/ivm-on-immutable-substrate.md`, `docs/design-decisions/dbsp-dependency.md`, `docs/design-decisions/ivm-retrospective.md`, and a first-time-user tutorial under `docs/getting-started/first-materialized-view.md` that takes a user from `slateduck serve` to a working incremental view in < 15 minutes. `mkdocs build --strict` green.
12. **Migration path from existing DuckLake deployments.** A documented and tested migration tool (`slateduck migrate-from-ducklake --source postgres://... --catalog s3://...`) reads an existing PostgreSQL- or SQLite-backed DuckLake catalog, replays its current snapshot into a fresh SlateDuck catalog (data files are not copied — they remain at their original object-store paths and are referenced by the new catalog), and emits a verification report. `docs/operations/migration-from-ducklake.md` covers cutover, rollback, and known-incompatibility surfaces. End-to-end tested against both PostgreSQL- and SQLite-backed source catalogs at SF1 scale.
13. **World-class testing foundation.** All 10 test tiers from [plans/e2e-integration-tests.md](plans/e2e-integration-tests.md) are fully implemented and green:
    - **Tiers 1–3** (unit/property, catalog, PG-Wire): green on every PR — standard GitHub Actions runner
    - **Tiers 4–5** (MinIO object store, client compat): green on every merge to `main` — large runner (8-vCPU), Testcontainers MinIO
    - **Tiers 6a–6d** (IVM single-shard through hardening): green on every merge to `main` for IVM paths
    - **Tier 7** (fault injection — catalog, IVM worker, toxiproxy): green on every pre-release tag
    - **Tier 8** (24 h soak, 16-shard scale-out, TPC-H SF10/SF100): green on pre-release — dedicated EC2 `c6i.4xlarge`
    - **Tier 9** (security — credential isolation, TLS, auth, SQL injection guards): green on pre-release
    - **Tier 10** (benchmark regression < 10% vs baseline): green on weekly scheduled CI
    - `slateduck-testkit` ships all 6 harnesses: `MinioHarness`, `CatalogHarness`, `PgWireHarness`, `DuckDbHarness`, `IvmWorkerHarness`, `DeterministicClock`
    - At least 150 named test functions across all tiers at GA; test inventory published in `docs/contributing/testing.md`

### Deliverables

- v1.0 release tag and `CHANGELOG.md` entry
- Benchmark report `benchmarks/v1.0-tpch-sf10.json` published in the repository and linked from `docs/performance/`
- Final S3 Express acceptance decision documented in `docs/performance/s3-express-validation.md`
- `slateduck-testkit` crate complete with all 6 harness types
- Complete test inventory in `docs/contributing/testing.md`: tier-by-tier test count, CI job mapping, feature flags, and scale-test runner setup

---

## v0.10.0 — Streaming Ingest

> **Note:** v0.10 is documented after v1.0 because it is an independent, parallel workstream. It can be implemented concurrently with the IVM track (v0.11–v0.15) and does not block or depend on IVM. Its CDC output primitives *feed into* IVM when both are deployed — see "Streaming Ingest + IVM Integration" below.

> Kafka/NATS streaming pipelines, exactly-once delivery semantics, and pg-tide-relay integration for zero-infrastructure ingest paths from transactional sources to S3-backed data lakes.

### Streaming Ingest via pg-tide-relay

[pg-tide](https://github.com/trickle-labs/pg-tide) v0.34.0 registers DuckLake (and `SlateDuckSink`) as a valid reverse pipeline sink. This enables:

- **Kafka → SlateDuck** and **NATS → SlateDuck** patterns with no persistent database other than the SlateDB-backed catalog
- Any external source (Kafka, NATS, Redis, SQS, webhook) writes directly to a DuckLake catalog without routing through a PostgreSQL inbox
- `SlateDuckSink` connects directly to the PG-wire sidecar, giving a zero-infrastructure path from a transactional source to a queryable data lake in S3

The pg-tide-relay SQL corpus is bounded by the patterns validated in v0.6 and v1.0. The key additional patterns beyond the base DuckDB corpus:

- `SELECT max(snapshot_id) FROM ducklake_snapshot WHERE snapshot_id > $1` — pg-tide offset tracking
- `INSERT INTO ducklake_metadata` with `scope = 'global'` — application metadata key for consumer offsets
- `SELECT value FROM ducklake_metadata WHERE metadata_key = $1 AND scope = 'global'` — offset retrieval

**Application metadata key namespace.** The dotted-prefix convention for non-DuckDB client application state is enforced and documented:

```
{application}.{instance}.{key}  →  stored in ducklake_metadata, scope = global
e.g. pg_tide.orders-to-lake.offset  →  "4782"
```

Multiple applications coexist by using distinct prefixes. Application metadata rows participate in snapshot transactions, enabling exactly-once semantics for streaming pipelines: a consumer commits its offset in the same SlateDB transaction as the snapshot that consumed those records.

**Exactly-once delivery guarantee.** Document and test the two-phase commit pattern: (1) write Parquet files to S3; (2) in one catalog transaction, register data files AND update the consumer offset key under `ducklake_metadata`. If the process dies between steps 1 and 2, the orphaned Parquet files are cleaned up by the orphan-file sweep after the grace period; the consumer re-reads from its last committed offset and re-registers the same data files. Because data file registration is idempotent for a given Parquet file path, the retry is safe.

### Deliverables

- [x] `SlateDuckSink` implementation in pg-tide registers without errors
- [x] End-to-end Kafka → SlateDuck → DuckDB query test passes with ≥100k records
- [x] NATS → SlateDuck → DuckDB query test passes with ≥100k records
- [x] Application metadata key namespace enforced: `{app}.{instance}.{key}` pattern validated in tests
- [x] Exactly-once delivery: process death between Parquet write and metadata commit is survivable; offset is not advanced on retry
- [x] Consumer offset tracking test: offset advances monotonically across 10 consecutive ingest batches
- [x] Performance test: Kafka ingest throughput ≥ 10k records/sec to S3 with catalog commit latency ≤ 50ms p95
- [x] Documentation: `docs/integration/streaming-ingest.md` with Kafka and NATS examples, offset recovery procedure, and failure mode handling

### CDC Output (Change Data Capture Export)

The complement to ingest: when a DuckLake snapshot is committed, the *diff* between the previous and current snapshot is a natural change stream. This turns SlateDuck from a streaming sink into a streaming source — enabling pipelines like `Source → SlateDuck → IVM → CDC → downstream`.

**Snapshot diff as a first-class primitive.** The diff between snapshots `S_n` and `S_{n+1}` is already computed implicitly: it's the set of catalog facts with `begin_snapshot = S_{n+1}` (new) or `end_snapshot = S_{n+1}` (retired). Expose this as a typed API and as a streaming output.

**CDC output targets:**

- **S3 CDC files.** Write per-snapshot diff as a Parquet or JSON-lines file under `{warehouse}/cdc/{table_id}/snapshot-{id}.parquet`. Readers poll or use S3 event notifications. Zero-infrastructure; natural for batch-oriented downstream.
- **Kafka/NATS CDC producer.** A sidecar (`slateduck-cdc`) tails the catalog and publishes per-table diffs to Kafka topics or NATS subjects. Exactly-once via consumer-offset tracking (same pattern as ingest, reversed).
- **Webhook CDC.** HTTP POST to a configurable URL on each snapshot commit. Includes snapshot ID, affected tables, and a pre-signed URL to the diff file. Useful for serverless triggers (Lambda, Cloud Functions).

**IVM-aware CDC.** When a materialized view updates, its output snapshot diff is itself a CDC event. This enables `Base table → IVM → Materialized view → CDC → external system`. The CDC producer treats materialized views identically to base tables.

- [x] `CatalogReader::snapshot_diff(from_snapshot, to_snapshot)` → structured diff (added/retired facts per table)
- [x] S3 CDC file writer: per-snapshot Parquet diff files under `{warehouse}/cdc/`
- [x] `slateduck-cdc` sidecar: tail catalog, produce to Kafka/NATS/webhook
- [x] CDC for materialized views: view output snapshot diffs exported like any table
- [x] End-to-end test: write → IVM update → CDC event → verify downstream receives correct diff
- [x] Documentation: `docs/integration/cdc-output.md` with Kafka, webhook, and S3-polling examples

### Streaming Ingest + IVM Integration

When v0.10 (streaming ingest) and v0.11+ (IVM) are both deployed, the end-to-end story is:

```
Kafka/NATS → pg-tide-relay → SlateDuck snapshot → IVM worker → materialized view update → CDC export
```

All within a single S3 bucket, no external state, no coordination servers.

- [x] Integration test: Kafka → ingest → base table → IVM view auto-updates within freshness target → CDC output matches expected diff
- [x] Documented architecture diagram in `docs/architecture/streaming-pipeline.md`
- [x] Latency budget documented: ingest commit + IVM processing + output publish ≤ freshness target + ingest batch interval

### Deliverables (updated)

- [x] `SlateDuckSink` implementation in pg-tide registers without errors
- [x] CDC output: `snapshot_diff()` API, S3 CDC writer, and `slateduck-cdc` sidecar (Kafka + webhook)
- [x] End-to-end streaming pipeline test: ingest → IVM → CDC → downstream
- [x] Documentation: streaming ingest + CDC output + IVM integration

---

## v1.x — Ecosystem Expansion

> Async FFI v2 for concurrent catalog operations, Lambda/edge-function integration, and post-GA performance optimizations for extreme-scale deployments.

### Async Catalog FFI (Strategy C v2)

Strategy C v1 (v0.5) uses a blocking Tokio runtime where each catalog call does `runtime.block_on(async { ... })`. This is correct and safe but blocks a DuckDB execution thread for the full duration of each S3 round-trip (10–50 ms on S3 Standard). For multi-table join planning, DuckDB may issue multiple concurrent catalog lookups; the blocking model serializes them at the thread boundary.

**Gate: DuckDB async catalog API.** Before scheduling this work, check whether DuckDB ≥1.5 exposes an async catalog interface in its extension API. If DuckDB provides a callback-based catalog operation model, proceed with Option 2. If not, the async bridge requires an upstream DuckDB contribution and must be deferred pending acceptance.

**Option 2 — Callback-based async FFI (if DuckDB provides the API).**

The C++ extension provides a completion callback. The Rust FFI layer spawns a Tokio task and calls the callback when the S3 operation completes:

```c
typedef void (*slateduck_completion_fn)(void* ctx, slateduck_result_t* result, slateduck_error_t* err);

void slateduck_list_data_files_async(
    slateduck_catalog_t* catalog,
    uint64_t table_id,
    uint64_t snapshot_id,
    void* ctx,
    slateduck_completion_fn on_complete
);
```

The Tokio runtime spawns the async task and returns immediately; `on_complete` is called from a Tokio worker thread when the operation finishes. DuckDB's thread pool is never blocked during S3 round-trips. Expected improvement: multi-table join planning with N catalog lookups completes in O(max_latency) rather than O(N × max_latency).

**Option 3 — Shared runtime via channel (if DuckDB API is blocking but the extension can run init code).**

The extension starts a background thread running a Tokio runtime at load time. Each catalog call sends a request onto an `mpsc` channel and blocks the calling thread on a `std::sync::mpsc::Receiver`. The Tokio worker processes the request asynchronously. This decouples the Tokio runtime from DuckDB's thread pool and adds approximately 1–5 µs channel-crossing overhead per call — negligible compared to S3 latency.

**ABI versioning for v2 FFI.** Any change to function signatures, added callback parameters, or changed opaque handle layouts increments `slateduck_abi_version()`. The DuckDB extension checks the ABI version at load time and refuses to proceed on mismatch. Document in `extension/CMakeLists.txt`.

### Lambda and Edge-Function Integration

Blueprint §1.4 identifies Lambda functions, container tasks, and CDN edge workers as first-class reader targets: because catalog-data keys are never overwritten, a `DbReader` opened at a known checkpoint can serve any historical `dl_snapshot_id` with no coordination with the writer.

Formalize this pattern for v1.x:

**Lambda catalog reader.** Publish a documented pattern (with example code) for an AWS Lambda function that:
1. Opens a `DbReader` against a named SlateDB checkpoint in S3 (checkpoint selected at function initialization, or passed as an event parameter for time-travel queries).
2. Executes a `list_data_files` or `describe_table` call and returns the result as JSON.
3. Never opens a `Db` writer handle; cannot corrupt the catalog.

The Lambda function uses the read-only `DbReader` API and requires only the catalog-prefix read IAM permission. It can run with sub-second cold-start latency on S3 Express One Zone if the checkpoint's manifest SST is cached in the Lambda function's `/tmp` storage.

**Checkpoint-pinned readers.** Add `slateduck checkpoint pin --name for-lambda-reader --snapshot-id N` which creates a named SlateDB checkpoint pinned at a specific `dl_snapshot_id`. The named checkpoint can be referenced in Lambda event payloads or CDN cache keys. Add `slateduck checkpoint unpin --name ...` when the checkpoint is no longer needed.

**CDN cache contract.** Because catalog-data keys are immutable (written once, retired via a bounded `end_snapshot` update, never physically deleted outside excision), the value at any given key is stable for any read at or before the key's `end_snapshot`. Document this as a cache contract: HTTP GET responses for catalog prefix reads can be cached by a CDN using the SlateDB checkpoint generation as a cache-control key. Provide example CloudFront distribution configuration and Lambda@Edge origin logic.

**Test requirement.** Add an integration test that: (1) writes 100 snapshots; (2) creates a checkpoint; (3) starts a Lambda-style read-only process using only the checkpoint; (4) verifies the process returns correct `list_data_files` results at any `dl_snapshot_id` up to the checkpoint; (5) verifies the process cannot write to the catalog (write attempts return an error from the `DbReader` API).

### Deliverables

- Async catalog FFI: scope decision recorded (Option 2 if DuckDB API available, Option 3 otherwise); implementation shipped and benchmarked
- Lambda/edge reader pattern documented with example code and integration test
- Checkpoint-pinned reader API shipped (`pin`, `unpin`, `list` subcommands)
- DuckDB major version upgrade process documented step-by-step in `docs/contributing/release-process.md`

---

## v2.x — General Fact Store

> Expose the immutable append-only substrate beyond DuckLake. SlateDuck's storage engine is schema-agnostic by design; this release line opens it up to non-DuckLake workloads.

The architectural principle in [plans/blueprint.md §1.4](plans/blueprint.md)
treats the storage engine as a generic fact log over object storage. DuckLake
is the first schema. v2.x explores what else the same substrate can carry,
without changing the storage engine.

### Generalized Fact Model

Carve out `slateduck-factstore` as a standalone crate by following the extraction boundary defined in [plans/blueprint.md §5.29](plans/blueprint.md):

| What moves into `slateduck-factstore` | What stays in `slateduck-catalog` |
| --- | --- |
| Key encoding utilities | 28-table tag allocation (`tags.rs`) |
| SDKV value header + `encoding-version` + Protobuf dispatch | DuckLake MVCC filter logic |
| Counter allocation (`0xFE` + transactional read-modify-write) | `schema_version` increment and `mark_schema_changed()` |
| `retain-from` key and TTL advancement | Inlined-data (`0xFD`) encoding |
| Excision primitives and audit log | DuckLake spec operations |
| Leadership/epoch keys | `dl_snapshot_id` semantics |
| `CatalogStore` skeleton with neutral `SnapshotId(u64)` | — |

Each schema gets its own isolated SlateDB `Db` at a dedicated path; schemas
never share a `Db`, WAL, or compaction process. `slateduck-factstore` exposes
a generic fact API: `assert(entity, attribute, value, snapshot)`,
`retract(entity, attribute, snapshot)`, `as_of(snapshot)`,
`history(entity, attribute)`.

### Alternative Schemas on the Same Substrate

Demonstrate the substrate hosting workloads other than DuckLake:
- **User-defined relational schemas.** A small DDL surface (`CREATE TABLE … WITH (catalog = 'slateduck')`) that allocates a tag prefix and lets users define their own tables stored as facts, queryable through the existing PG-wire dispatcher or a typed Rust API.
- **Event-sourced application store.** Append-only entity/attribute/value/transaction quads; current-state derivation via materialized views built from the fact log; native time travel.
- **Datalog query interface.** A read-only Datalog engine over the fact log for exploratory and graph-style queries.

Each schema opens its own `Db` at a distinct path prefix and reuses the same
counter, leadership, retain-from, and excision *code* from `slateduck-factstore`.

### Horizontal Read Scale-Out as a Product Feature

The immutable substrate already makes unbounded reader replicas correct; v2.x
formalizes them as a deployment pattern:
- A `slateduck reader` binary that serves either the DuckLake schema or any registered alternative schema
- A CDN-friendly cache contract: cache keys are immutable, so HTTP caching is sound by construction; document recommended cache headers and proxy patterns
- A Lambda/edge-worker integration example: open a `DbReader` against a known checkpoint and serve queries with no writer involvement
- Benchmark: linear read-throughput scaling to N readers on a single immutable catalog

### Optional Multi-Writer (Append-Disjoint)

Because writers only append disjoint keys (each scoped by their own
`dl_snapshot_id` allocation), the substrate can in principle accept multiple
concurrent writers per catalog with conflict detection at allocation time
rather than per-key fencing. v2.x evaluates whether this is worth the
operational complexity given the existing "one catalog per dataset"
partitioning pattern from v1.x.

### Deliverables

- `slateduck-factstore` crate published independently of `slateduck-catalog`
- At least one demonstrated non-DuckLake schema (user-defined relational or event-sourced application store)
- Read-replica deployment guide with measured linear scaling to ≥ 10 reader pods
- Documented decision on multi-writer support (adopt / defer / reject)

---

## Cross-Cutting Concerns

### Testing Pyramid

| Layer | What it tests | Tools |
|-------|---------------|-------|
| Property tests | Key encoding, ordering, prefix isolation, round-trip, ID monotonicity | `proptest`, `fail-parallel` |
| Unit tests | Each spec operation's Rust API behavior | `tokio::test` |
| Spec conformance (golden) | Bit-for-bit output match against SQLite-backed DuckLake reference | DuckDB CLI, diff |
| Wire-corpus replay | Strategy B handshake and statement dispatch against captured traffic | Custom replay harness |
| Crash injection | Atomicity and durability at every required crash point | `fail-parallel` |
| Performance benchmarks | p50/p95/p99 vs. Phase 0.2 baseline and vs. PostgreSQL | Custom harness, criterion |
| IAM / credential isolation | Sidecar rejects data-plane writes; client rejects catalog-plane writes | MinIO, LocalStack |

### Naming Conventions (enforced in code review)

| Concept | Variable name |
|---------|---------------|
| DuckLake logical snapshot | `dl_snapshot_id`, `catalog_version` |
| SlateDB physical read view | `kv_read_view`, `kv_snapshot` |
| SlateDB database object | `db`, `kv_db` |
| In-progress pending catalog write | `pending_txn`, `pending_batch` |
| Object-store path (absolute URI) | `CatalogPath`, `object_store_uri` |

### DuckLake Spec Upgrade Policy

When a new DuckLake spec version is published:
1. Check whether any of the 28 fixed table schemas changed
2. Allocate new tag bytes for added tables (in `tags.rs`)
3. Add new `encoding_version` decoders for changed row shapes
4. Update the DuckDB compatibility matrix and recapture the wire corpus
5. Do not change existing tag bytes or field positions in existing versions

### Versioning and Compatibility Policy

- `catalog-format-version` under `0xFF` gates binary compatibility at the catalog level
- `encoding_version` byte in every value gates row-level compatibility
- Older binaries encountering a higher `catalog-format-version` refuse to open (`SQLSTATE 0A000`)
- Migration path for incompatible upgrades: `slateduck export` → reinitialize → `slateduck import`
- DuckDB patch bumps: corpus replay CI; expected to remain compatible
- DuckDB minor bumps: new corpus capture required; explicit sign-off
- DuckDB major bumps: treated as a new client; full re-capture

### DBSP/Feldera Dependency Strategy

The IVM track (v0.11–v0.15) depends on the `dbsp` crate from [Feldera](https://www.feldera.com/). Feldera is a venture-funded startup; the crate is open-source (MIT) but its maintenance trajectory is not guaranteed. This is the single most important external dependency in the entire roadmap.

**Risk mitigation layers:**

1. **Thin adapter boundary.** All DBSP interaction is confined to `slateduck-ivm/src/circuit.rs`. The rest of the crate (source, trace, worker, output) depends only on the adapter's trait surface. Swapping DBSP for another engine requires rewriting one module, not the entire crate.

2. **Version pinning with `=` constraint.** Never float on `^` or `~`. Every DBSP upgrade is an explicit decision with a full regression pass.

3. **Contingency: vendored fork.** If Feldera stops maintaining `dbsp` or makes breaking changes incompatible with SlateDuck's needs:
   - Fork the crate at the last known-good version
   - Maintain a `slateduck-dbsp` fork under `trickle-labs/` with only the operators SlateDuck uses (filter, map, aggregate, join, iterate, top_k, window)
   - Strip unused operators and external storage backends; reduce surface area

4. **Contingency: raw differential-dataflow.** If a fork becomes untenable:
   - Replace DBSP with direct `differential-dataflow` + `timely-dataflow` (same underlying engine, more stable, maintained by Frank McSherry)
   - Loses the SQL-to-circuit compiler but retains the core DD semantics
   - The `plan.rs` → circuit lowering step becomes more manual but the architecture remains unchanged

5. **Evaluation gate.** At the start of v0.11, spend one week evaluating:
   - DBSP's current release cadence
   - Feldera's financial health (recent funding, hiring/layoffs)
   - Alternative: [arroyo](https://github.com/ArroyoSystems/arroyo) (Rust, streaming SQL)
   - Decision: proceed with DBSP, proceed with alternative, or vendor from day one

6. **Circuit compilation versioning.** Compiled DBSP circuits are derived from `view_sql` and the DBSP version active at compilation time. A DBSP minor/major upgrade may change operator semantics, serialized state layouts, or trace formats. To survive upgrades safely:
   - Every `MatviewRow` carries a `circuit_compilation_version` field: `(dbsp_semver, slateduck_ivm_semver, compiled_at_snapshot)`
   - On worker boot, if a held shard's `circuit_compilation_version` is older than the running worker's, the worker enters `recompile_pending` status, recompiles the circuit, validates by replaying the last N checkpoints from input snapshots (without overwriting output), and only then takes over
   - If recompilation produces incompatible state layouts (detected by a self-check on the first recovered batch), the matview is marked `stale_dbsp_upgrade` and requires `REFRESH ... FULL`. Operators see this in `SHOW MATERIALIZED VIEWS` and the release notes for every DBSP-touching upgrade enumerate which view shapes are forward-compatible
   - Upgrade docs: every release that bumps `dbsp` includes a compatibility matrix ("views using only filter/map/aggregate are forward-compatible; views using window functions require REFRESH FULL") in `CHANGELOG.md` and `docs/operations/ivm-upgrades.md`

**Document the decision in `docs/design-decisions/dbsp-dependency.md` before v0.11 alpha. The circuit-versioning contract lives in the same document and is updated on every DBSP bump.**

### Real-World Validation Policy

Synthetic benchmarks (TPC-H, TPC-DS) catch performance regressions and correctness bugs, but they do not catch usability gaps, cost surprises, or workflow friction. Before v1.0 GA:

1. **Internal dogfood.** Run a real SlateDuck+IVM deployment against pg-tide's own analytics pipeline (if available) or a synthetic-but-realistic workload (e.g. GitHub event stream, NYC taxi stream) for ≥ 30 days.
2. **Document surprises.** Any unexpected behaviour, cost spike, or operational friction discovered during dogfooding becomes a documented finding and must be resolved or explicitly accepted before GA.
3. **User-experience review.** At least one developer unfamiliar with SlateDuck internals must successfully create, monitor, and debug a materialized view using only the published documentation. Their friction log becomes a documentation and UX backlog item.

### SlateDB Dependency Strategy

SlateDB is the storage foundation. Unlike DBSP (an IVM-track dependency), SlateDB underpins *every* roadmap phase. It is pre-1.0, actively evolving, and maintained by a small team. The risk profile is different from DBSP but equally consequential.

**Risk mitigation layers:**

1. **API surface confinement.** All SlateDB interaction is confined to `slateduck-core/src/store.rs` (reads) and `slateduck-catalog/src/writer.rs` (writes). The rest of the codebase depends on `CatalogStore`/`CatalogReader`/`CatalogWriter` traits, not raw SlateDB types. This is already true today.

2. **Version pinning with `=` constraint.** Same as DBSP: every SlateDB upgrade is an explicit decision. Pin to a specific release; never float.

3. **SlateDB API contract surface.** The SlateDuck-relevant API is small:
   - `Db::open()`, `Db::close()`
   - `Db::get()`, `Db::put()`, `Db::delete()`, `Db::scan()`
   - `WriteBatch` (atomic multi-key writes)
   - `DbReader` / snapshots (concurrent readers)
   - `Db::flush()` (visibility barrier)
   - `Checkpoint` API (backup/restore)
   - Fencing / writer epoch

   If any of these changes semantics (not just signature), it is a correctness-critical event requiring a full regression pass.

4. **Contingency: vendored fork.** If SlateDB introduces incompatible changes or is abandoned:
   - Fork at last known-good version
   - Maintain `trickle-labs/slatedb` fork with only the features SlateDuck uses
   - Object-store agnosticism (via `object_store` crate) means the fork remains portable

5. **Contingency: alternative embedded KV.** If forking becomes untenable:
   - Evaluate `sled` (mature but different persistence model)
   - Evaluate writing a minimal WAL + SST layer directly on `object_store` (high effort, last resort)
   - The `CatalogStore` abstraction layer means migration is confined to one module

6. **Relationship maintenance.** SlateDB is maintained by a team with whom we can collaborate:
   - File issues for any behavior that affects SlateDuck
   - Contribute fixes upstream when possible
   - Monitor the SlateDB changelog and test against each release before adopting

**Monitor:** SlateDB release cadence, open issue count, and maintainer activity quarterly. Document findings in `docs/design-decisions/slatedb-dependency.md`.

### IVM Worker Deployment Model (Kubernetes)

v0.9 defines K8s patterns for catalog writer and reader pods. IVM workers (v0.11+) are a third workload class with distinct requirements:

**Pattern — IVM Worker Pool**

```yaml
apiVersion: apps/v1
kind: Deployment
metadata: { name: slateduck-ivm-worker }
spec:
  replicas: 4    # scale based on total shard count / shards-per-worker
  template:
    spec:
      containers:
      - name: slateduck-ivm
        args:
          - "serve"
          - "--catalog=s3://bucket/catalogs/warehouse-a"
          - "--state-prefix=s3://bucket/ivm-state/"
          - "--shard-limit=8"         # max shards this worker claims
          - "--lease-ttl=30s"
        resources:
          requests: { cpu: "2", memory: "4Gi" }
          limits:   { cpu: "4", memory: "8Gi" }
```

**Key differences from writer/reader pods:**

| Dimension | Writer/Reader | IVM Worker |
|-----------|--------------|------------|
| Catalog access | Writer: read+write, Reader: read | Read (base tables) + Write (checkpoints + output) |
| State stores | None (stateless) | Per-shard SlateDB instances under `--state-prefix` |
| Scaling unit | By session count | By total shard count across all matviews |
| Statefulness | Stateless (S3 is the state) | Lease-based: holds shard leases, releases on shutdown |
| Failure mode | Writer fencing handles crash | Lease expiry handles crash; peer workers pick up shards |

**Autoscaling guidance:**

- Scale on `max(ivm_lag_ms)` across all shards: if any shard exceeds 2× freshness target, add a worker
- Scale on `ivm_shards_unassigned`: if > 0 for longer than 2× lease TTL, add a worker
- Scale down conservatively: only remove a worker when it holds 0 shards for > 5 min (all its shards were redistributed)
- Document HPA configuration in `docs/deployment/kubernetes.md`

**IAM credentials for IVM workers:**

| Permission | Scope |
|-----------|-------|
| Read base table data files | `s3://bucket/data/**` (read-only) |
| Read/write catalog | `s3://bucket/catalogs/**` |
| Read/write state stores | `s3://bucket/ivm-state/**` |
| Write output data files | `s3://bucket/data/**` (write to output table paths) |

IVM workers need broader access than reader pods because they read Parquet data *and* write output Parquet. Document this IAM template in `docs/deployment/credential-isolation.md`.

### Graceful Shutdown & Rolling Updates (IVM Workers)

IVM workers are long-lived processes that hold shard leases and buffer in-flight DBSP batches. Ungraceful shutdown (kill -9) is always safe (lease expiry + checkpoint recovery), but graceful shutdown minimizes wasted work and recovery time.

**Graceful shutdown protocol (on SIGTERM):**

1. Stop acquiring new shard leases
2. Finish the current DBSP batch for all held shards (bounded by `--max-drain-time`, default 30 s)
3. Flush and checkpoint all shard state stores
4. Release all shard leases (set `lease_expires_at = now` via CAS)
5. Exit 0

If `--max-drain-time` elapses before step 2 completes, abandon in-progress batches (they will be replayed by the next worker from the checkpoint) and proceed to step 4.

**Rolling update strategy:**

- Kubernetes `maxSurge: 1, maxUnavailable: 0` ensures new workers start before old ones drain
- `terminationGracePeriodSeconds: 60` (must exceed `--max-drain-time` + checkpoint flush time)
- New workers wait for lease expiry on shards held by draining pods (lease TTL bounds the handoff window)
- Zero-downtime guarantee: at no point are shards unprocessed for longer than `lease_ttl + max_drain_time`

**Version upgrade (v0.11 → v0.12 etc.):**

- State store format changes require a `state_format_version` key in each shard's state store
- If a new worker binary encounters an incompatible state format, it runs `REFRESH ... FULL` for that shard rather than attempting migration (correctness over speed)
- Document the upgrade path in `docs/operations/ivm-upgrades.md`

### Scale Testing Infrastructure

The IVM acceptance criteria include tests that cannot run in normal CI: 24 h soaks, 1 TB inputs, 1M-edge graphs, and 16-shard scale-out benchmarks. These require dedicated infrastructure.

**Testing tiers:**

| Tier | What runs | Where | Trigger |
|------|-----------|-------|---------|
| CI (every PR) | Unit tests, property tests, single-shard correctness on LocalFS | GitHub Actions (standard runner) | Push/PR |
| Integration (every merge to main) | Multi-shard correctness, MinIO end-to-end, fault injection (<1h) | GitHub Actions (large runner, 8 vCPU) | Merge to main |
| Scale (weekly / pre-release) | 16-shard scale-out, 1 TB input, TPC-DS full suite, cost measurement | Dedicated EC2 (c6i.4xlarge) + S3 Standard | Scheduled / manual |
| Soak (pre-release) | 24 h continuous ingest, fault injection every 15 min, correctness drift check | Dedicated EC2 + S3 Express | Manual gate before GA |

**Infrastructure requirements:**

- Scale and soak tests run via a GitHub Actions self-hosted runner on a dedicated EC2 instance
- S3 bucket dedicated to scale tests: `slateduck-scale-tests-{region}`
- Results published to `benchmarks/` directory as JSON; compared against previous run
- Soak test failure blocks the release: any correctness drift in 24 h means the release is not ready
- Document the setup in `docs/contributing/testing.md` under "Scale Testing"

---

## What SlateDuck Is Not

- A general-purpose SQL engine *in v1* (the substrate is designed to make this possible later — see v2.x)
- A multi-writer database in v1 (one writer per catalog; SlateDB fencing handles takeover; the v0.7 partitioning pattern is the recommended workaround; v2.x evaluates append-disjoint multi-writer)
- A data-plane proxy (DuckDB writes Parquet directly; SlateDuck writes only the catalog)
- A system where user-visible catalog history can be silently deleted (physical deletion only via the explicit, audited `slateduck excise` command)
- A replacement for PostgreSQL-backed DuckLake in low-latency, high-concurrency analyst workloads
- A drop-in for any workload without first reading the performance analysis in `docs/performance.md`

**Choose SlateDuck when:** you are serverless or spot-based and cannot afford a persistent database server; you want a lakehouse with zero external infrastructure; you need cheap point-in-time catalog snapshots; your workload is write-heavy rather than read-heavy; or you are already in the SlateDB ecosystem.
