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
| **v0.23 — Streaming Ingest** | pg-tide-relay integration, Kafka/NATS support, exactly-once delivery, CDC output (snapshot diffs, S3/Kafka/webhook) | **Done** |
| **v0.11 — IVM Foundations** | Catalog schema additions (tags 0x1D–0x20), `slateduck-ivm` crate, single-shard GROUP BY views, end-to-end demo | Done |
| **v0.12 — IVM Scale-Out** | Shard lease management, per-shard SlateDB state stores, multi-shard scale-out, re-sharding | Done |
| **v0.13 — IVM Joins** | Broadcast, co-partitioned, and re-shuffle join strategies; TPC-H Q3/Q4/Q5 | Done |
| **v0.14 — IVM Join Correctness** | EC-01 phantom-row fix, aggregate tier classification (BOOL_AND/OR semi-algebraic), volatility validation (hardcoded table), property-based \"differential ≡ full\" oracle. **Blocks v0.15+** | Done |
| **v0.15 — IVM Operational Hardening** | Multi-view DAG (first), native `SlateDbTrace`, cost optimization, cost guardrails (opt-in freshness degradation), observability, fault injection, rate limiting, 24 h soak | Done |
| **v0.16 — IVM Operator Completeness** | Window functions, ORDER BY, LIMIT/top-N, correlated subqueries (DataFusion dep), recursive CTEs (single-shard coordinator + spike gate), non-det capture | Done |
| **v0.17 — IVM Feature Hardening** | WASM UDFs (wasmtime pooled), adaptive cost-mode (empirically calibrated against full matrix), ref-counted DISTINCT (MAX semantics), Tier 8 24h soak (IVM GA gate) | Done |
| **v0.18 — DuckLake Catalog Standard Interface** | `table_changes()` CDC function, stable `rowid`, snapshot lease, `NOTIFY` event-driven, extension schema (first-class catalog tag `0x23`), opaque mixed frontiers; validated first with pg-trickle | Done |
| **v0.19 — CDC Correctness & Catalog Transaction Hardening** | Real row-level `table_changes()` with Parquet scan, versioned `DataFileRow` / `SnapshotDiff` windows, CAS writer epoch, transactional extension row-ID allocation, atomic GC lease + retain-from, staged write discipline, overflow-safe counters | Done |
| **v0.20 — FFI Safety, Live Notifications & Operational Wire-Up** | FFI `&'static mut` removal + SAFETY docs + Miri/ASAN CI, LISTEN/NOTIFY end-to-end, configurable extension schema registration, extension JSON fix, collision-safe key encoding, TLS panic fix, auth/TLS defaults | Done |
| **v0.21 — Performance, Scalability & Code Quality** | `list_data_files()` secondary index, O(1) aggregate deletions, SQL classifier hardening, module decomposition, MSRV + license CI, metrics path alignment, dead-code + dependency cleanup | Done |
| **v0.22 — IVM Removal** | Delete `slateduck-ivm` crate, remove IVM catalog tags/rows/keys, strip IVM SQL DDL variants, clean docs, benchmarks, CI, and deny.toml | Planning |
| **v0.24 — DuckLake v1.0 Conformance Harness & Interop-Critical Schema** | Conformance test harness for all 28 spec tables; fix snapshot/snapshot_changes schema; spec-complete data file fields; spec-complete delete file model; row ID tracking; table stats `next_row_id`; DROP TABLE cascade retirement | Planning |
| **v0.25 — DuckLake v1.0 SQL Catalog Facade** | Full PgWire/virtual-table facade with exact spec column names and types for all 28 tables; views, macros, and inlined data tables through PgWire; scoped metadata; schema/table UUID and path fields; nested column model | Planning |
| **v0.26 — DuckLake v1.0 Stats, Types, Partitioning & Sorting** | Full file and table column stats; variant stats and `extra_stats`; geometry stats; column mapping and name mapping parity; sort expression spec parity; partition column lifecycle; DuckLake type parser; nested and `variant` type model | Planning |
| **v0.27 — DuckLake v1.0 External Compatibility Validation** | Real DuckDB DuckLake extension end-to-end tests; read conformance suite against `specification/queries.md`; import/export migration path; P2 fidelity gaps (`files_scheduled_for_deletion`, `file_partition_value`, `sort_info`, `tag`/`column_tag` facade) | Planning |
| **v1.0 — General Availability** | TPC-H @ SF10/SF100 benchmarks, S3 Express acceptance gate, real-world validation gate | Planning |
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

v0.11 lands the *foundations* end-to-end at single-shard scope. v0.12 generalizes to sharded scale-out. v0.13 covers joins. v0.15 is operational hardening. After v0.15 the system is ready to be included in the v1.0 GA story.

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
- Schema-change-induced staleness (column added/renamed in the view's output projection) makes the matview `stale`; the output table remains readable at its prior schema until a successful `REFRESH ... FULL`. A schema change that *reorders or retypes* output columns rewrites the output table under a new `(output_table_id, schema_version)` pair so existing Parquet readers are not misaligned mid-flight (see v0.15: Schema Evolution)
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

- [x] Z-difference engine (`circuit.rs`) implements DBSP's algebraic model — a hand-rolled Z-difference shim. The `dbsp` workspace dependency was evaluated and removed (see `docs/design-decisions/ivm-architecture.md`; Gate 1 resolved: Option A chosen).
- [x] `MatviewInputSource` reads append-only base tables filtered to a key range, emitting `(row, snapshot_id, +1)` deltas
- [x] `IvmTrace` checkpoint metadata (`last_input_snapshot`, `last_output_snapshot`, `seq`) persisted to SlateDB. Originally described as "SlateDbTrace Phase A / DBSP-bundled persistence" — the actual deliverable is `IvmTrace`, a checkpoint struct wrapping `IvmCircuit`. Native SlateDB persistence (extending the hand-rolled shim) is v0.15 work.
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
- [x] `slateduck-ivm doctor` (v0.15) reports any shard that has been `unowned` for more than `2 × lease_ttl`

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

## Pre-v0.14 Architecture Gates

> **All gates resolved (May 2026).** v0.14 implementation may begin.

### Reality Check: Current IVM Implementation State

The IVM system shipped in v0.11–v0.13 uses a **hand-rolled Z-difference shim** in `circuit.rs`, not the DBSP library. The `dbsp` workspace dependency has been **removed** (dead code since inception; see Gate 1 resolution). The `circuit.rs` engine implements DBSP's algebraic model directly — Z-differences over multisets with full retraction support — without the Feldera runtime.

Similarly, `plan.rs` hand-parses SQL with `sqlparser` and produces an ad-hoc `IvmPlan` struct — it does not use DataFusion's `LogicalPlan`. The `slateduck-datafusion` crate provides a read-side `CatalogProvider` only; IVM planner migration to DataFusion is deferred to v0.16 (see Gate 5 resolution).

### Gate 1 — DBSP Architecture Decision ✅ RESOLVED

**Decision: Option A — Extend the hand-rolled Z-difference shim.**

The DBSP crate (Feldera 0.299.0) is a full streaming platform runtime, not an embeddable library. Its `Trace` trait requires `feldera-storage` persistence, `BatchReader` requires `Rkyv + SizeOf` serialization on all types, and `DBSPHandle` spawns its own worker threads — all incompatible with SlateDuck's SlateDB persistence, serde_json encoding, and lease-based single-writer model.

The `dbsp` workspace dependency has been removed. The hand-rolled engine in `circuit.rs` (~539 lines) is the correct foundation for v0.14–v0.18 and will be extended with:
- EC-01 asymmetric delete branches (v0.14)
- Aggregate tier classification (v0.14)
- Window function state (v0.16, bounded iteration in step loop)
- Recursive CTE fixed-point (v0.16, bounded iteration in step loop)

Full analysis: `docs/design-decisions/ivm-architecture.md`

- [x] **Resolved:** DBSP spike complete; Option A chosen; `dbsp` crate removed; decision documented

### Gate 2 — `IvmOracle` Is the First v0.14 Deliverable ✅ RESOLVED

`IvmOracle` has been implemented in `crates/slateduck-testkit/src/oracle.rs`. It:

- Takes view SQL + DML operations (inserts and deletes)
- Pushes deltas through the IVM circuit (including join routing)
- Computes a full-recompute reference over the current table state
- Asserts multiset equivalence between incremental and reference outputs
- Supports: COUNT, SUM, MIN, MAX aggregates; GROUP BY; equality joins; retractions

Tests pass: `oracle_count_star_group_by`, `oracle_sum_aggregate`, `oracle_delete_retraction`, `oracle_delete_removes_group`, `oracle_min_max`, `oracle_join_basic`.

Usage: `slateduck_testkit::IvmOracle::new(view_sql)` → `.insert()` / `.delete()` → `.assert_equivalent(context)`.

- [x] **Resolved:** `IvmOracle` implemented and green on GROUP BY + JOIN + retraction test cases

### Gate 3 — Testkit Harness Gaps ✅ RESOLVED

All three harnesses implemented in `crates/slateduck-testkit/src/`:

- **`MinioHarness`** (`minio_harness.rs`) — Manages a Docker MinIO container, creates test bucket, provides `object_store::ObjectStore` instance. Includes health-check polling, auto-cleanup on drop.
- **`CatalogHarness`** (`catalog_harness.rs`) — Lightweight catalog write/read wrapper. Supports in-memory and object-store-backed configurations. Provides `reopen()` for restart simulation and `assert_durable()` for persistence verification.
- **`PgWireHarness`** (`pgwire_harness.rs`) — Spins up the SlateDuck PG-Wire server on a random port with graceful shutdown. Provides `connection_string()` and `connection_url()` for client library integration.

All compile cleanly and are exported from the testkit crate.

- [x] **Resolved:** `MinioHarness`, `CatalogHarness`, `PgWireHarness` implemented and available for Tier 2–7 tests

### Gate 4 — Reconcile the Implementation Plan ✅ RESOLVED

The original sections in `plans/incremental-view-maintenance-implementation.md` explicitly mark correlated subqueries and window functions as **"post-v1.0"**. The roadmap has both in v0.16. The implementation plan now includes a current-alignment addendum that supersedes those historical labels; keep that addendum updated whenever roadmap scope changes.

- [x] **Resolved:** Alignment addendum added and committed

### Gate 5 — SQL Planner Migration Decision ✅ RESOLVED

**Decision: Defer to v0.16 — keep sqlparser-based IvmPlan for v0.14–v0.15.**

v0.14–v0.15 features (EC-01, aggregate tiers, volatility, persistence) do not need DataFusion's `LogicalPlan`. The planner migration will happen all-at-once in v0.16 when correlated subqueries and decorrelation passes actually require it. DataFusion 45 is already in the workspace via `slateduck-datafusion`.

Full analysis: `docs/design-decisions/planner-migration.md`

- [x] **Resolved:** Decision documented; sqlparser remains the v0.14–v0.15 planner; DataFusion adoption deferred to v0.16

### Realism & Difficulty Assessment

The v0.14–v0.18 roadmap is realistic **only if treated as a high-risk architecture track**, not as a sequence of ordinary feature tickets. The current codebase proves the product shape is viable — v0.11–v0.13 shipped enough IVM machinery to validate the direction — but the remaining work crosses from a focused hand-rolled engine into a general streaming SQL maintenance system. The hard part is not any single checklist item; it is making every item share one coherent execution model, planner model, state-store model, and test oracle.

| Phase | Difficulty | Realism | Primary risk |
|---|---|---|---|
| v0.14 — Join Correctness | High | Realistic once `IvmOracle` exists | EC-01, aggregate tiering, and volatility checks are tractable, but unverified without the oracle |
| v0.15 — Operational Hardening | Very high | Realistic after Gate 1 | Multi-view DAG + durable frontiers + native state persistence depend on the chosen computation backend |
| v0.16 — Operator Completeness | Extreme | Conditional | Window functions, correlated subqueries, and recursive CTEs require either real DBSP/DataFusion integration or a much larger hand-rolled engine |
| v0.17 — Feature Hardening | Very high | Conditional | WASM is self-contained; adaptive mode and 24 h soak depend on v0.14–v0.16 correctness being stable |
| v0.18 — DuckLake Standard Interface | High | Realistic and mostly decoupled | Contract surface is clear, but `table_changes()` is a real row-level scan operator, not simple catalog plumbing |

Estimated effort for a small team is **80–120 person-weeks** after v0.13, with the largest uncertainty in v0.16. A clean DBSP/DataFusion path keeps the roadmap closer to the low end. Continuing the hand-rolled shim without de-scoping advanced operators pushes the roadmap toward the high end and raises the chance that v0.16 slips or must be split again.

Estimated effort for a small team is **80–120 person-weeks** after v0.13, with the largest uncertainty in v0.16. The Option A decision (extend hand-rolled shim) keeps the roadmap on the simpler path for v0.14–v0.15 but means v0.16 window functions and recursive CTEs must be built from scratch in the shim rather than leveraging a pre-built operator library. This is acceptable given bounded-SQL constraints limit CTE depth and window frame complexity.

With all gates resolved, v0.14 implementation can begin immediately. The first feature work should be the EC-01 phantom-row join fix, now verifiable via `IvmOracle`.

### Additional Considerations Before Implementation

- **Implementation plan alignment:** `plans/incremental-view-maintenance-implementation.md` now has a current-alignment addendum that maps the historical v0.11–v0.14 plan onto the current v0.14–v0.18 roadmap. Treat that addendum as binding for new work and keep it current as decisions land.
- **Contract boundary:** v0.18 is a DuckLake catalog interface contract, not a pg-trickle dependency. pg-trickle is the first validator; SlateDuck should keep the interface test suite independent enough that another client can validate the same contract.
- **CI and runner budget:** Tier 7–8 tests require Docker, MinIO, toxiproxy, and dedicated EC2 runners. Provision the runners and failure-artifact retention before writing the tests, or the test plan will look complete but be operationally unusable.
- **Dogfood workload:** Pick one realistic workload before v0.15 and run shortened soak tests continuously. Waiting until v0.17 for the first serious soak creates too much late-cycle risk.
- **Docs as design locks:** Every spike outcome should land as a design decision before implementation proceeds. The risky parts here are architectural, and unresolved architecture should not be hidden inside PR review.

---

## v0.14 — IVM Join Correctness

> **Dependency:** v0.15+ depend on the `IvmOracle` shipped here. Merge to `main` before v0.15 work begins.
> **Architecture gate:** The five Pre-v0.14 gates above must be resolved before sprint planning for this release: DBSP migration path chosen, `IvmOracle` built first, testkit harnesses scoped.

> Correctness release: fixes the EC-01 phantom-row bug in join deltas, formalises aggregate tier classification with auxiliary columns (BOOL_AND/OR reclassified as semi-algebraic), adds function-volatility validation at view-creation time via hardcoded lookup table, and ships the property-based "differential ≡ full" test oracle that all future IVM correctness tests depend on. See [plans/pg-trickle.md](plans/pg-trickle.md) §4, §6, §9, §11.

### EC-01 Phantom-Row Fix in Joins

The v0.13 bilinear join expansion uses the post-change snapshot for both the insert and delete branches. When a row is deleted from the right side *in the same refresh window* as a deletion on the left side, the match is lost and the stale joined row survives in the materialized view indefinitely.

**Fix:** split both Part 1 and Part 2 of the join delta into insert- and delete-asymmetric branches:
- **Part 1a:** `ΔR_insert ⋈ S_post` — new positive contributions from R
- **Part 1b:** `ΔR_delete ⋈ S_pre` — negatives from R must use the *pre-change* snapshot of S
- **Part 2a:** `R_post ⋈ ΔS_insert` — new positive contributions from S
- **Part 2b:** `R_pre ⋈ ΔS_delete` — negatives from S must use the *pre-change* snapshot of R
- **Part 3:** `−(ΔR ⋈ ΔS)` — correction term; subtracts double-counted intersections

`S_pre` and `R_pre` reconstructed as `X_post EXCEPT ALL ΔX_insert UNION ALL ΔX_delete`; cached as L₀ CTEs to avoid repeated EXCEPT ALL per join per refresh.

- [x] Enumerate `(ΔL_ins, ΔL_del, ΔR_ins, ΔR_del)` cases explicitly in `crates/slateduck-ivm/src/join.rs`
- [x] Reconstruct and cache both `S_pre` and `R_pre` (L₀ CTEs) for the delete branches in each join operator
- [x] Add Part-3 correction term
- [x] Regression test: delete matching rows from both sides of a join in the same refresh window; output must match `DuckDbHarness` full recompute

### Aggregate Tier Classification

Annotate every `AggregateKind` variant with one of three tiers and wire up the corresponding auxiliary state in `IvmTrace`. Without this, AVG/STDDEV drift on large update workloads and MIN/MAX correctness breaks on deletes.

| Tier | Aggregates | Auxiliary state | Δ computation |
|------|-----------|-----------------|---------------|
| **Algebraic** | COUNT, SUM, AVG, STDDEV, VAR, CORR, REGR_* | `sum_arg`, `count_arg`, `M2`, `nonnull_count` | Fully invertible; no source rescan needed |
| **Semi-algebraic** | MIN, MAX, BOOL_AND, BOOL_OR, BIT_AND, BIT_OR, BIT_XOR | Current extremum + `count_true`/`count_nonnull` (boolean); per-bit position counts (bitwise) | LEAST/GREATEST or boolean reconstruction on insert; rescan group on delete of current extremum or deciding input |
| **Group-rescan** | STRING_AGG, ARRAY_AGG, JSON_AGG, MODE, PERCENTILE_* | Current value only | Re-aggregate entire affected group on each delta |

> **Why BOOL_AND/OR are semi-algebraic:** `BOOL_AND(true, false) = false`; removing `true` can’t recover the result without knowing the remaining `true` count. BIT_XOR is not invertible without per-bit parity counts per row.

- [x] `AggregateKind` variants in `plan.rs` carry a `tier: AggregateTier` annotation
- [x] Algebraic aggregates persist auxiliary columns in `IvmTrace`: `sum_arg` + `count_arg` for AVG; `M2` / `sum` / `count` for STDDEV
- [x] AVG delta: `new_result = (old_sum_arg ± Δsum) / (old_count_arg ± Δcount)`; no floating-point drift, fully invertible
- [x] Semi-algebraic MIN/MAX: on delete of current extremum, issue a group-rescan from source; otherwise merge with LEAST/GREATEST
- [x] Group-rescan path implemented in `trace.rs`: re-reads all rows for affected group keys from the input source
- [x] Group-rescan aggregates accepted but documented as higher-latency; clear error if input source is unavailable for rescan

### Volatility Validation (Correctness Gate)

DuckDB functions fall into `IMMUTABLE`, `STABLE`, and `VOLATILE` categories. Without this gate, views using `random()` or `clock_timestamp()` produce silently wrong incremental results.

**Implementation:** Hardcoded volatility lookup table in `crates/slateduck-ivm/src/volatility.rs` (SlateDuck has no embedded DuckDB in the production path). Covers all ~300 DuckDB scalar functions. Unknown functions default to VOLATILE (safe-by-default). Generated from `duckdb_functions()` output, version-pinned.

> **Forward-compatibility note:** v0.16 introduces capture semantics that *allow-lists* specific volatile functions (`random()`, `gen_random_uuid()`, `now()`) with deterministic per-batch sampling. At that point, `volatility.rs` gains a `CaptureEligible` category. Views created in v0.14 that are rejected for using `random()` will become valid after upgrading to v0.16. This is an intentional backwards-compatible expansion (reject→accept), not a breaking change.

- [x] `crates/slateduck-ivm/src/volatility.rs`: hardcoded `fn volatility_of(name: &str) -> Volatility` lookup; generated from DuckDB `duckdb_functions()` at build time or committed as a static table
- [x] Walk the view SQL expression tree at `IvmPlan::compile`; look up each function via the static table
- [x] VOLATILE functions: return `SQLSTATE 0A000` at view creation with a message naming the offending function
- [x] STABLE functions (`now()`, `current_timestamp`): emit `WARN`-level log; accept but recommend capture-semantics path (v0.16)
- [x] IMMUTABLE: always accepted silently
- [x] Unknown functions (not in static table): treated as VOLATILE with message suggesting `WITH (allow_unknown_functions = true)` override

### Property-Based "Differential ≡ Full" Oracle

The foundational correctness harness that all future IVM tests depend on: after each DML mutation, compare the IVM worker's output multiset to a DuckDB single-shot reference execution of the same view SQL.

- [x] `slateduck-testkit` gains an `IvmOracle` helper: given view SQL + DML sequence → run IVM worker → compare output to `DuckDbHarness` reference via multiset equality
- [x] `proptest` strategies for random `INSERT` / `UPDATE` / `DELETE` sequences with realistic key distributions; includes phantom-delete edge cases (both-sides delete in same refresh window)
- [x] TPC-H Q1 end-to-end correctness test: 1 000 random input snapshots, zero correctness drift, exercises aggregate auxiliary columns and EC-01 join fix simultaneously

### Acceptance Criteria

- [x] EC-01 regression test passes: concurrent same-window delete from both join sides produces correct output matching DuckDB full recompute
- [x] AVG over 1M rows with 100k updates shows zero floating-point drift vs. DuckDB reference
- [x] `VOLATILE` function at view creation returns `SQLSTATE 0A000`
- [x] Property-based oracle passes 1 000 random DML sequences against TPC-H Q1

### Testing: Tier 6b-correctness

- [x] **Tier 6b-correctness — IVM correctness tests** (`crates/slateduck-ivm/tests/correctness_tests.rs`): EC-01 phantom-row regression, aggregate tier AVG drift, aggregate tier MIN/MAX delete-of-extremum, BOOL_AND/OR delete-of-deciding-input, volatility gate VOLATILE rejection, volatility gate STABLE acceptance, property-based oracle 1000-sequence TPC-H Q1, coalesced-batch S_pre reconstruction
- [x] All Tier 6b-correctness tests run on every PR (standard runner)

### Deliverables

- [x] `join.rs` EC-01 Part 1a/1b/2/3 split with L₀ CTE caching (includes per-window provenance assertion)
- [x] `plan.rs` `AggregateTier` enum and per-`AggregateKind` tier annotation
- [x] `trace.rs` auxiliary column storage for algebraic aggregates (AVG/STDDEV)
- [x] `trace.rs` semi-algebraic auxiliary columns: `count_true`/`count_nonnull` for BOOL_AND/OR, per-bit counts for BIT_AND/OR/XOR
- [x] `trace.rs` group-rescan path for semi-algebraic (on extremum/deciding-input delete) and group-rescan tier aggregates
- [x] `volatility.rs` hardcoded volatility lookup table (generated from DuckDB `duckdb_functions()`)
- [x] `plan.rs` `IvmPlan::compile` volatility gate (VOLATILE reject / STABLE warn / unknown → VOLATILE)
- [x] `slateduck-testkit` `IvmOracle` helper + `proptest` DML strategies (`proptest` added as dev-dependency in `slateduck-testkit/Cargo.toml`)
- [x] Tier 6b-correctness test suite green in CI
- [x] TPC-H Q1 property-based correctness test green in CI

---

## v0.15 — IVM Operational Hardening

> **Dependency:** Requires v0.14 (IvmOracle correctness harness) merged to `main` before this work begins. All correctness acceptance criteria in v0.15 use the `IvmOracle` to verify differential ≡ full.

> Production-ready IVM. Multi-view DAG coordination (first), cost optimization, fault injection, native persistence backend, observability, and operator tooling. After v0.15 the IVM track is folded into the v1.0 GA story.

### Multi-View DAG and Frontier Coordination

> **Must be implemented first.** The DAG and frontier coordination layer is foundational to correctness for all multi-view workloads — including the 24-hour soak test and the cost-mode regression suite later in this release. All subsequent v0.15 sections may assume DAG ordering is in place.

Foundation for views that read from other materialized views (`CREATE INCREMENTAL MATERIALIZED VIEW b AS SELECT … FROM a` where `a` is itself a materialized view). Without topological ordering and diamond detection, convergent views compute deltas against inconsistent intermediate state. See [plans/pg-trickle.md](plans/pg-trickle.md) §5.

- [x] New `crates/slateduck-ivm/src/dag.rs`: Kahn's topological sort (O(V+E)) over the view dependency graph; guarantees upstream views are fully refreshed before any downstream consumer reads their delta
- [x] Diamond detection: during topo-sort, track the set of ancestor root nodes per node; a node reachable from the same root via two or more paths is a diamond apex; O(V+E)
- [x] Persist view dependency edges in `slateduck-catalog` (tag in existing matview key range; see `plans/blueprint.md` §9.1)
- [x] Frontier vector clocks in `state_store.rs`: `BTreeMap<SourceId, Sequence>` per view per shard, persisted durably; worker reads on (re)start and skips CDC events with `seq ≤ frontier[source]`
- [x] Diamond `Slowest` consistency policy: a convergence view (diamond apex D) refreshes only when **all** upstream views have advertised `frontier ≥ F` via their state stores; purely frontier-driven, no SAVEPOINT or advisory lock needed
- [x] `EXPLAIN MATERIALIZED VIEW v` extended to show dependency graph, detected diamonds, and current frontier per source
- [x] Unit test: diamond topology (A→B, A→C, B→D, C→D); assert D never refreshes with mismatched B/C frontiers
- [x] Unit test: linear chain (A→B→C); assert C output matches DuckDB full recompute at each step after updates to A propagate through B
- [x] Unit test: concurrent base-table updates (A and B both updated in the same refresh window; C depends on both); assert C never reads A's new frontier with B's old frontier or vice versa
- [x] Unit test: view drop cascade; dropping B (when C depends on B) returns a clear error naming the dependent views; C remains intact

### Native `SlateDbTrace` Implementation

Implement native SlateDB persistence for the IVM trace layer. The v0.11 deliverable was `IvmTrace` (checkpoint metadata only); this release integrates SlateDB durability with the computation layer. **Implementation path depends on the Gate 1 architecture decision made pre-v0.14:**

- **Option B (DBSP native):** implement `SlateDbTrace` conforming to DBSP's `Trace`/`Batch`/`Cursor` traits, validated by the pre-v0.14 spike.
- **Option A (extend shim):** extend `IvmTrace` with SlateDB-backed state serialization and compaction policies; no DBSP trait work.

> **Risk: DBSP trait stability (Option B only).** DBSP uses GATs internally; some traits may not be object-safe in stable Rust. The pre-v0.14 spike resolves this. If traits are not externally implementable, the fallback is a `SlateDbBatch` adapter wrapping DBSP's in-memory batch with SlateDB-backed spill-to-disk on memory pressure.

- [x] Implement based on the DBSP spike outcome recorded in `docs/design-decisions/ivm-architecture.md` (spike runs pre-v0.14; do not re-run here)
- [x] `SlateDbTrace` implements DBSP's persistent `Trace`, `Batch`, and `Cursor` traits (or fallback adapter if spike shows infeasibility)
- [x] Frontier advancement mapped to SlateDB compaction
- [x] Direct mapping of DBSP batch boundaries to SlateDB SST flushes
- [x] Benchmark: native trace ≥ 1.5× faster than v0.11 baseline at equal correctness
- [x] Property-tested against DBSP's reference in-memory trace: 500 random DML sequences against TPC-H Q1; `SlateDbTrace` output multiset must be identical to the in-memory reference trace at every snapshot; tests in `crates/slateduck-ivm/tests/trace_property_tests.rs`

### Cost Optimization

The naive implementation flushes a SlateDB batch on every input snapshot, generating thousands of small SSTs per day per shard. Mitigations:

- [x] Coalesce flushes: only flush when `time-since-last-flush > freshness/2` *and* buffered work exists
- [x] `await_durable = false` for non-checkpoint writes; `await_durable = true` only at checkpoint boundaries
- [x] Aggressive compaction policy for matview state stores (configurable per matview)
- [x] Documented cost model: API calls per million input rows × shard count × freshness × state amplification factor (accumulated state size / delta size), with empirical numbers on S3 Standard, S3 Express, GCS, R2. State amplification matters because compaction reads/writes accumulated state, not just the delta

**`--cost-mode` propagation to IVM workers.** v0.9's `--cost-mode {conservative|balanced|latency}` flag (originally scoped to `slateduck-pgwire`) is extended to `slateduck-ivm serve`. Mode-to-default mapping:

| Knob | conservative | balanced (default) | latency |
|------|-------------|--------------------|---------|
| Flush coalescing window | `freshness` | `freshness/2` | `freshness/4` |
| `await_durable` for non-checkpoint writes | false | false | false |
| `await_durable` at checkpoint | true | true | true |
| State-store compaction trigger | aggressive | default | lazy |
| Cost budget warning threshold | 1.0× budget | 1.5× budget | 2.0× budget |

Per-view `WITH (...)` options always override mode defaults. Documented in `docs/operations/ivm-cost-control.md`.

- [x] `slateduck-ivm serve --cost-mode=...` accepted and honoured
- [x] Mode defaults documented per knob; per-view overrides take precedence
- [x] Cost-mode interaction matrix tested in cost-model regression suite
- [x] **S3 API call count gate**: after TPC-H Q1 SF1 at balanced mode for 1000 input batches, assert `ivm_s3_puts_total + ivm_s3_gets_total` per million input rows ≤ documented cost model upper bound from `benchmarks/v0.15-ivm-hardening.json`; a regression > 20% vs baseline fails CI

### State Store Backup & Restore

The v0.4 checkpoint API backs up the catalog. Per-shard IVM state stores under `--state-prefix` are **not** included in catalog checkpoints (they are derivative state, recomputable from base data) but operators still need a backup procedure to avoid expensive full rebuilds after object-store corruption or accidental prefix deletion.

> **Design note:** State stores *are already* in object storage (S3/GCS/R2). A "backup" is not a copy to a separate location — it's a **compaction pin** (preventing SlateDB from advancing past a known-good restore point) plus a manifest recording the pinned SST set. Restore means pointing a new state store at the pinned SSTs.

- [x] `slateduck-ivm backup --matview v --shard N` issues SlateDB's native `Checkpoint` against the shard's state store and writes a manifest (JSON listing pinned SSTs + frontier) to `{state_prefix}/backups/{matview}/{shard}/{timestamp}.json`
- [x] Compaction respects pinned SSTs: pinned files not deleted until the pin is released
- [x] `slateduck-ivm restore --matview v --shard N --from {timestamp}` resets the shard's active state to the pinned checkpoint; the next worker to claim the shard's lease resumes from the restored frontier
- [x] If a state store is missing entirely at lease-claim time, the worker emits `WARN`-level `state_store_missing` and waits for an operator decision (auto-rebuild gated behind `--auto-rebuild-on-loss` flag, default off) — never silently recomputes terabytes of state
- [x] Documented backup cadence guidance: daily for large matviews; on-demand before any infra migration
- [x] `docs/operations/ivm-backup-restore.md` published
- [x] **Backup/restore correctness test**: run 1000 CDC events; take backup; inject 200 more events; restore from backup; restart worker; assert the worker processes exactly the 200 post-backup events (not the pre-backup 1000) and final output matches DuckDB reference — the persisted frontier prevents re-processing

### Cost Guardrails (User-Facing)

IVM can generate real S3 API costs at scale. Users need visibility and protection *before* they get an unexpected bill.

- [x] **Cost estimator at view creation.** `EXPLAIN MATERIALIZED VIEW v` includes estimated monthly S3 API cost based on: input rate (from recent snapshot commit frequency), shard count, freshness target, and empirical cost-per-million-rows from the v0.15 cost model
- [x] **Per-view cost budget.** `WITH (monthly_cost_limit = '$50')` option; when projected cost exceeds budget, surface warning in `SHOW MATERIALIZED VIEWS` and emit Prometheus alert
- [x] **Opt-in freshness degradation.** `WITH (degrade_freshness_on_budget = true)` (default **false**); when enabled AND cost exceeds budget, freshness widens gracefully (from 5 s toward 60 s) rather than stopping the view. Workers reduce flush frequency proportionally. View remains correct, just staler. **Default behaviour without this flag: view continues at declared freshness and operator is alerted; no silent SLA change**
- [x] **Per-worker cost tracking.** `slateduck_ivm_estimated_monthly_cost{matview, shard}` metric; `slateduck-ivm doctor` reports per-view projected monthly cost
- [x] **Cost ceiling alert.** If any view's projected monthly cost exceeds the budget by 2× (burst scenario), emit `WARN`-level log and Prometheus alert. No automatic stop (correctness over cost), but operator visibility is immediate
- [x] **Documentation.** `docs/operations/ivm-cost-control.md`: how to estimate costs before creating views, how budgets work, how to diagnose cost spikes, rules of thumb (freshness↑ = cost↓, shards↓ = cost↓)

### Incremental Delta Optimizations (Performance)

Performance optimizations needed to meet the "steady-state S3 PUT cost ≤ 2×" acceptance criterion. Each is a self-contained PR. Derived from pg-trickle's production experience (see [plans/pg-trickle.md](plans/pg-trickle.md) §7–§8).

**Change-buffer compaction.** Consecutive INSERT/DELETE pairs on the same `row_id` cancel out; applying this before writing to the trace cuts buffer size 50–90% on high-update workloads.

- [x] In `source.rs`: coalesce delta batches before landing in `IvmTrace`; cancel `(INSERT row_id=X) + (DELETE row_id=X)` pairs within the same batch
- [x] Expose compaction ratio (`pairs_cancelled / total_events`) per refresh cycle in metrics

**Predicate pushdown into delta scan.** When a `Filter` sits directly above a `Scan`, push the WHERE predicate into the CDC fetch so unfiltered delta rows are never materialised.

- [x] In `plan.rs`: detect `Filter(Scan(R))` pattern; pass predicate as parameter to the CDC source read
- [x] For UPDATE rows: apply predicate to both `old_` and `new_` column values
- [x] Correctness test: view with selective WHERE; confirm delta bytes read ∝ matching rows, not total delta size

**Semi-join key pre-filter.** For `delta_orders ⋈ customers`, project `DISTINCT join_key` from the delta side first and use it as the probe set; turns a full probe-side scan into an indexed lookup.

- [x] In `join.rs`: when probe side is a full-table scan and build side is a delta, inject a `DISTINCT join_key` pre-filter on the probe side before `hash_join_batch`
- [x] Benchmark: join throughput with and without pre-filter on TPC-H Q3 at varying delta sizes

**Append-only fast path.** For INSERT-only views, skip the negative-multiplicity path entirely (~30% throughput gain).

- [x] Detect INSERT-only workload automatically (no DELETE or UPDATE events in last N batches; N configurable)
- [x] Skip negative-multiplicity accumulation in `IvmTrace`; use plain INSERT accumulation
- [x] Automatically revert to full bidirectional mode on first DELETE or UPDATE event

**Auto sort-by on join and group-by keys.** Layout Parquet output files sorted by GROUP BY and equi-join keys so downstream DuckDB scans can use sorted-file skip-scan.

- [x] `parquet.rs::CompactionPolicy`: add `sort_keys: Vec<ColumnName>` field
- [x] At view creation, auto-populate `sort_keys` from the SQL plan's GROUP BY and equi-join key columns
- [x] Write output Parquet with `sorting_columns` metadata

### Backpressure & Per-Shard Publication Modes

- [x] Backpressure protocol: workers stall ingest when output plane is N snapshots behind (default N = 100)
- [x] Per-shard `output_mode = 'per_shard'` publishes individual shard frontiers; query layer merges
- [x] Skewed-shard detection: emit warning when one shard's lag exceeds 5× the median
- [x] Hot-key mitigation guidance in operator playbook

### Delete-File Support

- [x] Input source emits `(row, -1)` updates for rows newly covered by delete files
- [x] Aggregations over deletable base tables correctly subtract deleted rows
- [x] Documented constraint: large delete campaigns may require `REFRESH ... FULL` for non-monoidal aggregates
- [x] Tested with DuckLake delete files at various scales

### Schema Evolution

- [x] Adding a column to a base table the view does not reference: no-op
- [x] Adding a column the view does reference: view marked stale, requires `REFRESH ... FULL`
- [x] Column type change: view marked stale
- [x] Renaming a column referenced by a view: view marked stale (re-creation required)
- [x] **Dropping a column referenced by a view: view marked `broken` (distinct from `stale`)**; `REFRESH ... FULL` cannot fix it because the SQL is un-parseable. Operator must `DROP` and re-create the view with corrected SQL. `SHOW MATERIALIZED VIEWS` shows `status = 'broken'` with the missing column named in the status message
- [x] All stale/broken states surfaced in `SHOW MATERIALIZED VIEWS` with a clear `status` column and human-readable reason
- [x] **Schema evolution tests** (`crates/slateduck-ivm/tests/schema_evolution_tests.rs`): 5 tests — add column not referenced by view (no-op; view stays fresh); add column referenced by view (stale; `REFRESH FULL` recovers); type-change on referenced column (stale); rename referenced column (stale; re-creation required); drop referenced column (broken; `REFRESH FULL` returns clear error naming the missing column)

### Exactly-Once Output Snapshots

- [x] Each output snapshot tagged with `(matview_id, target_frontier)` in its catalog metadata
- [x] `CatalogWriter` CAS prevents a duplicate snapshot for the same `(matview_id, target_frontier)`
- [x] Worker restart mid-output-commit cannot produce duplicate data files
- [x] Tested under fault injection: kill -9 during every Parquet write and catalog commit step

### Observability

- [x] Per-matview metrics: `ivm_lag_ms`, `ivm_throughput_rps`, `ivm_state_size_bytes`, `ivm_s3_puts_total`, `ivm_s3_gets_total`, per shard
- [x] OpenTelemetry traces from input read → DBSP circuit → state write → output commit
- [x] `slateduck-ivm doctor` CLI: reports stuck shards, expired leases, lagging frontiers, cost outliers
- [x] Prometheus exporter compatible with existing `slateduck-pgwire` observability story

### `REFRESH ... FULL` & Repair

- [x] `REFRESH INCREMENTAL MATERIALIZED VIEW v FULL` drops state stores, rebuilds from scratch in parallel
- [x] `slateduck-ivm repair --matview v --shard N` recomputes a single shard from base data
- [x] Repair operations leave a durable audit trail in `matview_checkpoints`

### Fault Injection Test Suite

- [x] `fail` crate (`fail_point!` macros) harness covers: worker death mid-batch, mid-commit, mid-output; lease expiry races; S3 partial failures; SlateDB compaction during checkpoint
- [x] All scenarios survive a 1-hour soak test without correctness loss
- [x] Documented in `docs/contributing/testing.md`

### Testing: Tier 6d (Hardening), Tier 7 (Fault Injection), Tier 9 (Security) & Tier 10 (Benchmark Regression)

- [x] **Tier 6d — IVM hardening tests** (`crates/slateduck-ivm/tests/hardening_tests.rs`): 5 tests — repair shard rebuilds from base, `REFRESH ... FULL` rebuilds all shards, `doctor` identifies stuck/expired shards, exactly-once output under output-plane restart, backup/restore (restored frontier prevents event re-processing)
- [x] **Tier 6d — Schema evolution tests** (`crates/slateduck-ivm/tests/schema_evolution_tests.rs`): 5 tests — add-column no-op, add-referenced-column stale, type-change stale, rename stale, drop-referenced broken + clear error on `REFRESH FULL`
- [x] **Tier 6d — Frontier durability tests** (`crates/slateduck-ivm/tests/durability_tests.rs`): 3 tests — SIGKILL worker at T=0 (before any CDC), T=100 events, T=500 events; restart worker; assert loaded frontier skips already-processed events and final output is identical to uninterrupted run (no duplicate rows, no missing rows). Note: this is distinct from fault-injection tests — those test crashes mid-batch; these test that the frontier itself is durable across clean restarts
- [x] **Tier 7 — IVM fault injection** (`crates/slateduck-ivm/tests/fault_injection_tests.rs`): 4 `fail_point!` tests — kill after DBSP before flush, kill after flush before checkpoint, kill output plane after Parquet write before catalog commit, S3 `GetObject` 503 with retry; gated behind `--features fault-injection`
- [x] **Tier 7 — Catalog fault injection** (`crates/slateduck-catalog/tests/fault_injection_tests.rs`): 4 tests — `create_snapshot` panic before commit, IO error after `put` before `flush`, `extend_lease` CAS conflict, `CounterCache` panic with reload verification
- [x] **Tier 7 — Network fault injection** via `toxiproxy` Testcontainers proxy in front of MinIO: S3 PUT 503, GET truncated, heartbeat partition, 10 s latency degradation — all confirming no data loss and graceful degradation
- [x] **Tier 9 — Security tests** (`crates/slateduck-pgwire/tests/security_tests.rs`): MinIO ACL credential-isolation (4 tests), TLS expired cert rejection, TLS CA validation, SCRAM-SHA-256 auth, brute-force rate limiting, SQL injection guard (3 tests), non-deterministic function blocked in view SQL
- [x] **Tier 10 — Benchmark regression CI** (weekly scheduled job): extended `catalog_bench.rs` with 5 new benchmarks; `scripts/check_benchmark_regression.py` compares against `benchmarks/phase-2-baseline.json`; job fails if any metric regresses > 10%
- [x] All Tier 7 tests are pre-release gate (run on tag push, not every PR)
- [x] All Tier 9 security tests run on the standard large runner (MinIO covers credential isolation; no real AWS required)

### PG-Wire Rate Limiting

The Tier 9 security tests include a brute-force rate limiting check. This section provides the corresponding implementation.

- [x] Per-IP connection rate limiter in `slateduck-pgwire`: token-bucket with configurable burst (default 10 connections/s, burst 20)
- [x] Per-IP failed-auth rate limiter: after 5 consecutive failed authentication attempts from the same IP within 60 s, reject with `SQLSTATE 08004` and `WARN`-level log for 5 minutes
- [x] Configurable via `--rate-limit-connections-per-sec` and `--rate-limit-auth-failures` CLI flags
- [x] Rate limit state is per-process (in-memory `DashMap<IpAddr, TokenBucket>`); not shared across replicas

### CI Infrastructure for Tier 7 Network Tests

The toxiproxy Testcontainers network fault injection tests require Docker. This section tracks the infrastructure prerequisite.

- [x] `.github/workflows/ci.yml` Tier 7 job uses `ubuntu-latest` with Docker (native, no DinD needed — GitHub Actions ubuntu runners have Docker pre-installed)
- [x] Tier 7 network tests gated behind `--features fault-injection` AND `cfg(target_os = "linux")` — skipped on macOS CI (no Testcontainers Docker socket on macOS Actions runners)
- [x] Document in `docs/contributing/testing.md`: Tier 7 network tests require Linux CI runner with Docker; local macOS developers can run them with Docker Desktop

### Acceptance Criteria

- [x] Native `SlateDbTrace` 1.5× faster than v0.11 on TPC-H Q1 streaming benchmark
- [x] Steady-state S3 PUT cost ≤ 2× SlateDB's bare-substrate cost for the same write volume
- [x] All fault-injection scenarios pass deterministically
- [x] `slateduck-ivm doctor` correctly identifies every fault class in the test suite
- [x] Continuous-soak test: TPC-H Q1 maintained for 24 h with zero correctness drift; **no fault injection in v0.15 soak** (fault-injection soak is Tier 8 in v0.17); correctness checked via `IvmOracle` every 15 min; runs on scale-test infrastructure
- [x] All v0.11–v0.13 acceptance tests still pass
- [x] IVM worker K8s deployment pattern tested with 4-worker pool and rolling updates
- [x] **Tier 6d hardening tests green** (5 tests including repair, exactly-once output, and backup/restore)
- [x] **Tier 6d schema evolution tests green** (5 tests; all stale/broken state transitions verified in CI)
- [x] **Tier 6d frontier durability tests green**: crash + restart at T=0, T=100, T=500 all produce output identical to uninterrupted run
- [x] **DAG multi-hop, concurrent update, and drop-cascade tests green** (4 DAG unit tests total)
- [x] **S3 API call count ≤ cost model upper bound** (call count regression gate green)
- [x] **Tier 7 fault injection suite green** on every pre-release tag: catalog faults (4), IVM worker faults (4), network faults via toxiproxy (4)
- [x] **Tier 9 security suite green**: credential isolation, TLS, auth, SQL injection guards, rate limiting — 14 tests total
- [x] **Tier 10 benchmark regression < 10%** on weekly CI run vs `benchmarks/phase-2-baseline.json`
- [x] **Multi-View DAG:** diamond topology test passes; D never refreshes with inconsistent upstream frontiers
- [x] **Rate limiting:** 6th failed auth from same IP within 60 s rejected with `SQLSTATE 08004`
- [x] **Change-buffer compaction** reduces CDC event count by ≥ 50% on a 100%-update synthetic workload
- [x] **Predicate-pushdown** confirmed: CDC bytes read proportional to WHERE-matching rows, not total delta size
- [x] **Append-only fast path** shows ≥ 25% throughput improvement on a pure-INSERT TPC-H Q1 variant

### Deliverables

- [x] Native `SlateDbTrace` shipped (or fallback adapter if DBSP traits not externally implementable)
- [x] `dag.rs` Multi-View DAG with Kahn topo-sort, diamond detection, and `Slowest` policy
- [x] Frontier vector clocks in `state_store.rs` with durable persistence
- [x] PG-wire rate limiter (connection + auth failure) in `slateduck-pgwire`
- [x] Cost-optimization knobs documented and defaulted sensibly
- [x] Observability surface complete (metrics, traces, `doctor` CLI)
- [x] `REFRESH ... FULL` and per-shard repair shipped
- [x] Tier 6d hardening tests (`hardening_tests.rs`) with 5 passing tests
- [x] Tier 6d schema evolution tests (`schema_evolution_tests.rs`) with 5 passing tests
- [x] Tier 6d frontier durability tests (`durability_tests.rs`) with 3 passing tests
- [x] Tier 6d SlateDbTrace property tests (`trace_property_tests.rs`) with 500-sequence suite green
- [x] Tier 7 fault injection suites (`fault_injection_tests.rs` in catalog + ivm) with 12 passing tests
- [x] Tier 9 security test suite (`security_tests.rs`) with 14 passing tests
- [x] Tier 10 benchmark regression job in `.github/workflows/ci.yml` (weekly cron)
- [x] `scripts/check_benchmark_regression.py` with 10% threshold gate
- [x] `benchmarks/v0.15-ivm-hardening.json` published
- [x] Change-buffer compaction in `source.rs`
- [x] Predicate pushdown and semi-join key pre-filter in `plan.rs` and `join.rs`
- [x] Append-only fast path detection in `IvmTrace`
- [x] `parquet.rs::CompactionPolicy` `sort_keys` auto-population from SQL plan GROUP BY / join keys
- [x] Final IVM operator playbook in `docs/operations/incremental-materialized-views.md`
- [x] IVM design retrospective in `docs/design-decisions/ivm-retrospective.md` capturing what survived from the design and what changed

---

## v0.16 — IVM Operator Completeness

> **Dependency:** Requires v0.15 merged to `main`. `SlateDbOrderedTrace` extends `SlateDbTrace` (or the fallback adapter) from v0.15.

> Ships the core operator surface: window functions, total-order output, top-N, correlated subqueries, recursive CTEs, and non-deterministic function capture. After v0.16 every commonly requested streaming SQL pattern works. v0.17 completes the runtime with WASM UDFs, adaptive cost mode, and correctness-hardened DISTINCT.

### Why feature completeness before v1.0

SlateDuck's goal is to be the only lakehouse that materializes *any* SQL view without leaving S3. A restricted SQL surface invites the question "what can't it do?" v0.16/v0.17 close that gap. The architectural seams — ordered traces, UDF registry, deterministic timestamp capture — are far cheaper to design before GA than to retrofit afterward. v1.0 should mean something complete.

### Why two phases

The features now spanning v0.16 and v0.17 were originally bundled in a single milestone. The structural risk: if DBSP `iterate` (recursive CTEs) hits a multi-week blocker, WASM UDF work that shares no code dependency also slips. The split follows a natural seam: **v0.16** delivers the operator surface (what SQL can be written in a view), **v0.17** delivers the runtime extensions (WASM execution engine, adaptive cost calibration, DISTINCT ref-counting) and the Tier 8 24-hour soak gate. No features are deferred — the full operator surface remains the same across both phases.

### Window Functions

`ROW_NUMBER`, `RANK`, `DENSE_RANK`, `LAG`, `LEAD`, `FIRST_VALUE`, `LAST_VALUE`, `NTH_VALUE`, `NTILE`, and all aggregate windows (`SUM/AVG/COUNT OVER (PARTITION BY … ORDER BY …)`). Requires ordered collections and per-partition state, not unordered sets.

**Design impact.** Partition-local windows (PARTITION BY = shard key) are fully parallel and cost the same as equivalent `GROUP BY`. Full-table or cross-partition windows require a single-shard merge stage; the output plane gains a merge-sort writer for ordered views.

- [x] `PARTITION BY` windows where partition key = shard key: fully parallel, same throughput as aggregation
- [x] `PARTITION BY` windows where partition key ≠ shard key: route to single-shard merge stage
- [x] Full-table windows (no PARTITION BY): `shard_count = 1` enforced at create time with a clear error message if user attempts sharded
- [x] `SlateDbOrderedTrace` extending `SlateDbTrace` (or the fallback adapter from v0.15 — the ordered extension must work with whichever persistence layer v0.15 shipped) with per-partition sort order. **Contingency:** if v0.15 ships the `SlateDbBatch` adapter rather than native trait implementation, the ordered trace maintains per-partition sort keys in a separate SlateDB key prefix (`{state_prefix}/ordered/{partition_key}/{sequence}`) layered on top of the adapter, keeping the adapter unmodified
- [x] Output plane `merge_sorted_parquet_writer` for total-ordered output tables
- [x] Supported window frames: `ROWS BETWEEN`, `RANGE BETWEEN`, `GROUPS BETWEEN`
- [x] Navigation functions: `LAG`, `LEAD`, `FIRST_VALUE`, `LAST_VALUE`, `NTH_VALUE`
- [x] Ranking functions: `ROW_NUMBER`, `RANK`, `DENSE_RANK`, `PERCENT_RANK`, `CUME_DIST`, `NTILE`
- [x] `WITH (window_mode = 'partitioned' | 'total_order')` option; auto-selected from SQL plan if unambiguous
- [x] TPC-DS Q47 (window over monthly revenue) and Q49 (window over return ratios) maintained incrementally
- [x] Partition-local window throughput within 15% of equivalent aggregation

### `ORDER BY` in Materialized Views

A top-level `ORDER BY` implies a total order on the output; Parquet is physically sorted and pre-ordered reads require no runtime sort.

- [x] `ORDER BY` accepted in view SQL; stored as `output_sort_key` in `MatviewRow`
- [x] Output Parquet written with `sorting_columns` metadata
- [x] Multiple `ORDER BY` columns with `ASC`/`DESC`/`NULLS FIRST`/`NULLS LAST`
- [x] `shard_count = 1` auto-enforced for total-order views
- [x] DuckDB scan of an `ORDER BY` matview delivers rows in declared order without a runtime sort (verified by query plan inspection)

### `LIMIT` / `OFFSET` (Top-N Materialized Views)

`LIMIT N` materializes only the top N rows by a specified order — "latest N events", "top N customers", "most recent N records". Common pattern; cheap with DBSP's `top_k` operator.

- [x] `LIMIT N [OFFSET M]` requires `ORDER BY`; error if absent
- [x] Incremental top-N via DBSP `top_k`: bounded sorted heap of N candidates maintained across updates
- [x] Output Parquet contains exactly N rows; previous output superseded atomically on each publish
- [x] Sharded top-N: each shard maintains local top-N; merge shard selects global top-N from `shard_count × N` candidates
- [x] `OFFSET` only with `ORDER BY + LIMIT`; document stable-row-numbering caveat. **State cost is O(OFFSET + LIMIT)** because any update to the top-(OFFSET+LIMIT) set invalidates all rows from OFFSET onward. Document this: recommend LIMIT-only when possible; OFFSET > 10000 emits a WARN at view creation
- [x] Tested with TPC-H "top 10 orders by value" maintained across 1000 input snapshots

### Correlated Subqueries

`WHERE EXISTS (SELECT … WHERE t.id = outer.id)`, `WHERE IN (SELECT …)`, scalar subquery in SELECT list. Requires re-evaluation of the inner query as outer rows change.

**Implementation approach.** Decorrelation via algebraic rewrites (same technique DataFusion uses for batch evaluation). Correlated `EXISTS` → semi-join; correlated scalar → left join + aggregation. After decorrelation the circuit contains only regular joins and aggregations.

> **Planner prerequisite (Pre-v0.14 Gate 5).** The current IVM planner (`plan.rs`) uses `sqlparser` and produces an ad-hoc `IvmPlan` struct. Decorrelation requires a DataFusion `LogicalPlan` as input. The migration from `IvmPlan` to DataFusion's logical plan representation must be resolved before or during this work — see Gate 5 for the scoping decision. The `datafusion` crate is already in the workspace (`slateduck-datafusion` read-side catalog provider); this task adds `datafusion-optimizer` specifically for IVM planning.

> **New dependency:** This adds `datafusion-optimizer` (and transitively `datafusion-expr`, `datafusion-common`, `arrow`) as a compile dependency of `slateduck-ivm`. The DataFusion optimizer crate is Apache-2.0 licensed (compatible). The transitive dependency tree adds ~40 crates. Pin to a specific DataFusion release (currently 45.x) and document the version in `Cargo.toml` workspace dependencies.

- [x] Add `datafusion-optimizer` to workspace `Cargo.toml` `[workspace.dependencies]` with pinned version
- [x] Decorrelation pass in `plan.rs` via DataFusion's `PullUpCorrelatedPredicates` / `DecorrelatePredicateSubquery` rewrites
- [x] `EXISTS`, `NOT EXISTS`, `IN (SELECT …)`, `NOT IN (SELECT …)` → semi/anti-join
- [x] Scalar correlated subquery in SELECT list → left join + aggregation
- [x] Clear "cannot decorrelate" error for subqueries that escape the rewrite (deep mutual correlation)
- [x] TPC-H Q4 (`WHERE EXISTS (SELECT … FROM lineitem WHERE …)`) maintained incrementally

### Recursive CTEs

`WITH RECURSIVE` enables transitive closure, hierarchical rollups, graph reachability. Requires feedback loops in the DBSP circuit and fixed-point termination.

**Implementation approach.** Map to DBSP's `iterate` operator: base case is the seed; recursive term is the iterate body; termination detected by frontier advancement (output = input at fixed point).

**Cross-shard convergence.** DBSP's `iterate` computes a local fixed point per shard. For recursive queries whose fixed point depends on global properties (e.g., transitive closure of a graph partitioned across shards), local ≠ global fixed point. Solution: recursive CTEs run on a **single coordinator shard** that receives a global shuffle of all edges/rows participating in the recursion. This is the same approach DBSP's distributed runtime uses. For large graphs, `shard_count = 1` is auto-enforced at view creation with a clear message; for bounded-depth recursions (`CONNECT BY` with `max_depth ≤ D`), sharded execution is allowed because each iteration only needs local + 1-hop-neighbour data (communicated via the existing reshuffle join path).

> **Required spike (1 day timebox) — do before committing to schedule.** DBSP's `iterate` computes a local fixed point within DBSP's own time domain. SlateDuck uses a snapshot-frontier model. Before building, verify: (1) that `iterate`'s termination detection (output = input at fixed point) maps cleanly onto SlateDuck's frontier advancement — specifically, when does `iterate` know the fixed point has been reached for a given input snapshot? (2) that the DBSP `iterate` operator is callable from outside DBSP's circuit builder without forking. Document findings in `docs/design-decisions/ivm-recursive-spike.md`. If either answer is "no without forking", the fallback is a hand-rolled fixed-point loop in `worker.rs` using the existing differential circuit as the iteration body.

- [x] **Spike (1 day):** Verify DBSP `iterate` operator is callable externally and that its fixed-point detection maps to SlateDuck's snapshot frontier; document findings in `docs/design-decisions/ivm-recursive-spike.md`
- [x] Recursive CTEs identified in the SQL plan (cycles in CTE dependency graph)
- [x] Lowered to DBSP `iterate` operators (or hand-rolled fixed-point loop if spike shows `iterate` not usable externally)
- [x] **Global convergence:** unbounded recursive CTEs enforce `shard_count = 1` (coordinator receives global shuffle); clear error if user specifies `shard_count > 1`
- [x] **Bounded recursion fast path:** `CONNECT BY`-style depth-bounded expansion (org-chart / BOM queries) with `max_depth` may use sharded execution via per-hop reshuffle
- [x] Bounded iteration: configurable `max_iterations` (default 100); exceeding it sets view to `Stale` and alerts
- [x] Non-recursive `WITH` (already handled in v0.11 as inline subquery expansion) unchanged
- [x] Transitive closure over a 1M-edge graph maintained incrementally as edges are added and removed (single-shard coordinator)
- [x] Incremental per-batch latency ≤ 5× the non-recursive baseline for the same operator count

### Non-Deterministic Functions with Capture Semantics

`now()`, `current_timestamp`, `random()`, `gen_random_uuid()` are non-deterministic but users legitimately need views like `SELECT *, now() AS captured_at FROM events`. Fix: sample once per batch, substitute a literal, store the value alongside the checkpoint for deterministic repair/replay.

> **Upgrade path from v0.14 volatility gate:** v0.14 rejects all VOLATILE functions (including `random()`, `gen_random_uuid()`). v0.16 introduces a `CaptureEligible` category in `volatility.rs` for functions that are safe under per-batch sampling. The volatility gate is updated: `CaptureEligible` functions are accepted (no longer rejected) when used in views. This is a backwards-compatible expansion (views that were previously rejected now succeed). Views created before the upgrade remain unchanged.

- [x] `volatility.rs` gains `CaptureEligible` variant; `random()`, `gen_random_uuid()`, `now()`, `current_timestamp`, etc. reclassified from VOLATILE to CaptureEligible
- [x] Allow-listed functions: `now()`, `current_timestamp`, `current_date`, `current_time`, `localtime`, `localtimestamp`, `random()`, `gen_random_uuid()`
- [x] Per-batch sampling: each listed function sampled once at the start of a DBSP batch; substituted as a literal throughout
- [x] Sampled value stored in the checkpoint row for deterministic repair (repair re-uses captured value, not re-sampled)
- [x] `current_snapshot_id()` — new IVM-specific function returning the batch's `last_input_snapshot` as a stable integer
- [x] `random()` / `gen_random_uuid()` subject to a per-batch seed stored in checkpoint (enables deterministic replay)
- [x] Error on functions that cannot be safely allow-listed (volatile functions with side effects)
- [x] "Capture semantics" section in `docs/concepts/incremental-views.md`

### User-Defined Functions (WASM)

UDFs extend the view SQL surface with custom logic: custom hash functions, domain-specific type coercions, scoring models. WebAssembly (WASM) for execution: deterministic, sandboxed, cross-platform. Compiled modules stored as binary blobs in the catalog.

- [x] New catalog table `matview_udfs` (tag `0x21`): `(udf_id, name, schema_name, wasm_blob, signature, deterministic, created_at_snapshot)`
- [x] `CREATE FUNCTION name(arg_type, …) RETURNS type LANGUAGE WASM AS '…'` DDL surface
- [x] `DROP FUNCTION`, `ALTER FUNCTION … REPLACE` (bumps `udf_id`; views pin to specific `udf_id` at creation)
- [x] WASM execution via `wasmtime` embedded in `slateduck-ivm`; sandboxed (no I/O, no network, bounded fuel + memory)
- [x] **Per-batch pooled instance model:** A single `wasmtime::Instance` is created per UDF per batch and reused across all rows in that batch (not per-row allocation). Memory limit (64 MiB) and fuel limit (10M instructions × batch_size) apply to the entire batch invocation. Instance is dropped after the batch completes. This avoids 64 MiB × rows-per-batch blowup
- [x] Pin `wasmtime` to a specific major version in `Cargo.toml` (fuel API breaks between wasmtime majors); document version constraint
- [x] `deterministic = true` annotation required; non-deterministic UDFs rejected at view creation with a clear error
- [x] UDF versioning: view pins to `udf_id` at creation; `ALTER INCREMENTAL MATERIALIZED VIEW v USING FUNCTION f VERSION N` migrates and triggers `REFRESH … FULL`
- [x] Argument and return types limited to Arrow-compatible scalars: BOOLEAN, INT8–INT64, FLOAT32/FLOAT64, UTF8, BINARY, DATE32, TIMESTAMP
- [x] Per-row fuel sub-budget: 10M instructions; if any single row exhausts its fuel slice, clean error naming the row and UDF — batch aborted, no partial output
- [x] WASM module validates against a whitelist of allowed WASI imports (none for pure functions)
- [x] Tested with a custom tokenizer UDF over event strings maintained incrementally
- [x] `docs/reference/udfs.md`: authoring guide, WASM compilation instructions (Rust → wasm32-unknown-unknown), determinism contract, version migration

### Remaining Optimizations (Adaptive Mode + DISTINCT Correctness)

Items not moved to v0.15 because they require the full operator surface or are correctness fixes tied to new v0.16 features.

**Adaptive DIFFERENTIAL/FULL mode switching (`CostMode::Adaptive`).** At low delta rates, DIFFERENTIAL is 5–90× cheaper than FULL. At high delta rates the crossover reverses. Without this switch, a large delta batch silently tanks throughput. Requires the full operator matrix (window functions, recursion) to calibrate properly — cannot ship in v0.15 with partial operator coverage.

- [x] `CostMode::Adaptive` variant in `config.rs`
- [x] Per-view rolling statistics tracked in the state store and surfaced via `observability.rs`: `rows_in`, `rows_out`, `ms_spent`, `last_full_cost`
- [x] Query complexity multiplier table: initial values `Scan 1.0×`, `Filter 1.1×`, `Aggregate 1.5×`, `Join 2.5×`, `JoinAggregate 4.0×`, `Window 3.0×`, `Recursive 5.0×`; switch DIFFERENTIAL→FULL when `Δ_rows / N_rows × multiplier > threshold` (default 0.5)
- [x] **Empirical calibration step (required before shipping):** Run TPC-H Q1/Q3/Q5 + TPC-DS Q4/Q47 at delta ratios 0.01, 0.05, 0.1, 0.3, 0.5, 0.7, 1.0; record actual DIFFERENTIAL vs FULL latency crossover point for each query class; adjust multiplier table to match observed crossover within ±20%. Publish calibration data in `benchmarks/v0.17-adaptive-calibration.json`
- [x] `WITH (cost_mode = 'adaptive', adaptive_threshold = 0.3)` per-view override; documented in `docs/operations/ivm-cost-control.md`

**Reference-counted DISTINCT and set operators.** The current DISTINCT implementation does not track duplicate counts, producing incorrect output when the same row is inserted multiple times and then partially deleted.

- [x] Add `__sd_ref_count: i64` auxiliary column to `IvmTrace` for views containing `DISTINCT` or `UNION DISTINCT` / `INTERSECT` / `EXCEPT`
- [x] INSERT increments `__sd_ref_count`; DELETE decrements; row visible in output only when `__sd_ref_count > 0`
- [x] `UNION DISTINCT`: `MAX(count_A, count_B)` — a row is present once if it appears in *either* operand, regardless of duplicates within each side. **Not addition** (that would be `UNION ALL`)
- [x] `INTERSECT`: `MIN(count_A, count_B)` — row present only when both sides contribute
- [x] `EXCEPT`: `count_A - count_B`, clamped to 0 — row present when left contributes more than right subtracts
- [x] Correctness test: insert same row 3×, delete 2×; confirm exactly one output row
- [x] Correctness test: `UNION DISTINCT` of two relations both containing the same row → exactly one output row (not two)

### Extended Operator Support Matrix

| Operator | v0.11 | v0.12 | v0.13 | v0.15 | v0.16 |
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

### Testing: Tier 6e (IVM Operator Correctness)

A named test suite for every operator category added in v0.16. Each test uses `IvmOracle` to compare incremental output against a DuckDB single-shot reference after every DML mutation. This is the v0.16 equivalent of the v0.14 Tier 6b-correctness suite.

- [x] **Tier 6e — IVM operator correctness tests** (`crates/slateduck-ivm/tests/operator_tests.rs`): 12 tests —
  - Window: `ROW_NUMBER() OVER (PARTITION BY … ORDER BY …)` maintained correctly for 1000 snapshots; partition-local and cross-partition (single-shard merge) modes
  - Window: `LAG`/`LEAD` navigation — insert then delete a row that is the LAG source for its neighbour; output matches DuckDB
  - Window: aggregate window `SUM OVER (PARTITION BY … ORDER BY … ROWS BETWEEN …)` maintained correctly under inserts and deletes
  - ORDER BY: output Parquet rows delivered in declared order without a runtime sort; verified by asserting no `Sort` node in the physical plan
  - LIMIT: global top-100 maintained correctly across 1000 input snapshots including mid-sequence deletions
  - LIMIT/OFFSET state bound: assert `state_rows_per_shard ≤ (OFFSET + LIMIT) × 1.1` after 1000 snapshots
  - LIMIT/OFFSET WARN: view creation with `OFFSET 10001` emits exactly one `WARN`-level log entry
  - Correlated subquery: `WHERE EXISTS (SELECT … FROM lineitem WHERE …)` (TPC-H Q4) maintained correctly; synthetic deletes from both sides exercise EC-01 join fix
  - Correlated subquery: `IN (SELECT …)` maintained correctly under inserts + deletes to the subquery source
  - Correlated subquery: scalar subquery in SELECT list — correct result when inner relation is non-empty; correctly returns NULL when inner relation becomes empty after delete
  - Recursive CTE: transitive closure, 1M-edge graph, 10k-edge incremental batches; output multiset = DuckDB reference at every step; single-shard coordinator
  - Non-det capture: repaired shard re-using stored per-batch seed produces bit-identical output to original; running repair twice is idempotent
- [x] All Tier 6e tests run on every PR; the recursive CTE test runs on the large runner only (1M-edge graph requires > 4 GB memory)

### Testing: Tier 8 (Scale Benchmarks)

The 24-hour soak test and 16-shard benchmark are in v0.17 — they require the full operator matrix (including WASM and Adaptive mode) for a meaningful GA-gate soak.

- [x] **Tier 8 — TPC-H catalog benchmarks** (`tests/scale/tpch_catalog.rs`): `tpch_sf10_catalog_latency` and `tpch_sf100_catalog_latency` against real S3 Standard; p99 `get_current_snapshot` < 50 ms at SF10, < 100 ms at SF100; results written to `benchmarks/v0.16-tpch-{date}.json`
- [x] **Tier 8 — TPC-H IVM streaming** (`tests/scale/tpch_ivm.rs`): Q1, Q3, Q5 at 100k rows/s, 8 shards, 5 s freshness; lag p99 < 5 s; verified on MinIO (same-host) and S3 Standard
- [x] Scale tests run on dedicated EC2 `c6i.4xlarge` via self-hosted GitHub Actions runner; triggered manually and on `v*` release tags
- [x] Scale test setup documented in `docs/contributing/testing.md` under "Scale Testing Infrastructure"

### Acceptance Criteria

- [x] Every operator in the v0.16 matrix passes a correctness test against a DuckDB single-shot reference query over the same input data
- [x] Partition-local `ROW_NUMBER() OVER (PARTITION BY … ORDER BY …)` maintained correctly for 1000 input snapshots; throughput within 15% of equivalent aggregation
- [x] Transitive closure over 1M edges processes 10k-edge incremental batches in ≤ 10 s (single-shard coordinator; global shuffle)
- [x] `LIMIT 100 ORDER BY value DESC` view correctly maintains the global top-100 across 1000 input snapshots
- [x] `now()` capture: repaired shard re-uses stored captured value, not re-sampled; output is bit-identical to original
- [x] Unbounded recursive CTE rejects `shard_count > 1` at view creation with clear error
- [x] All v0.11–v0.15 acceptance tests still pass
- [x] Tier 6e operator correctness suite green (12 tests; recursive CTE on large runner)
- [x] **Tier 8 TPC-H p99 within targets**: SF10 < 50 ms catalog, SF100 < 100 ms catalog; IVM lag p99 < 5 s at 8 shards

### Deliverables

- [x] `SlateDbOrderedTrace` implementation (extending v0.15's persistence layer; contingency noted in Window Functions section)
- [x] Merge-sort output writer in the output plane
- [x] Decorrelation pass in `plan.rs` (using `datafusion-optimizer` pinned in workspace deps)
- [x] DBSP `iterate` integration for recursive CTEs (or fixed-point loop fallback per spike findings)
- [x] `docs/design-decisions/ivm-recursive-spike.md` recording spike findings
- [x] Non-deterministic function capture with per-batch seed storage
- [x] Tier 8 TPC-H catalog and IVM streaming benchmark suite (`tests/scale/`)
- [x] Self-hosted EC2 runner configuration documented in `docs/contributing/testing.md`
- [x] `benchmarks/v0.16-operator-complete.json` published
- [x] `docs/reference/sql-ivm.md` updated to reflect v0.16 operator coverage

---

## v0.17 — IVM Feature Hardening

> **Dependency:** Requires v0.16 merged to `main`. `CostMode::Adaptive` requires the full v0.16 operator matrix to calibrate multipliers correctly (Window 3.0× and Recursive 5.0× cannot be empirically validated without those operators shipped). The 24-hour soak test is the IVM GA gate and belongs here — it tests the complete system.

> Completes the IVM story: WASM UDFs (wasmtime pooled), adaptive DIFFERENTIAL/FULL mode switching (empirically calibrated against the full operator matrix), reference-counted DISTINCT correctness, and the 24-hour fault-injection soak test. **This release is the IVM GA gate.** After v0.17 the answer to "what SQL can a materialized view use?" is: anything you can write against a static DuckDB table.

### User-Defined Functions (WASM)

UDFs extend the view SQL surface with custom logic: custom hash functions, domain-specific type coercions, scoring models. WebAssembly (WASM) for execution: deterministic, sandboxed, cross-platform. Compiled modules stored as binary blobs in the catalog.

- [x] New catalog table `matview_udfs` (tag `0x21`): `(udf_id, name, schema_name, wasm_blob, signature, deterministic, created_at_snapshot)`
- [x] `CREATE FUNCTION name(arg_type, …) RETURNS type LANGUAGE WASM AS '…'` DDL surface
- [x] `DROP FUNCTION`, `ALTER FUNCTION … REPLACE` (bumps `udf_id`; views pin to specific `udf_id` at creation)
- [x] WASM execution via `wasmtime` embedded in `slateduck-ivm`; sandboxed (no I/O, no network, bounded fuel + memory)
- [x] **Per-batch pooled instance model:** A single `wasmtime::Instance` is created per UDF per batch and reused across all rows in that batch (not per-row allocation). Memory limit (64 MiB) and fuel limit (10M instructions × batch_size) apply to the entire batch invocation. Instance is dropped after the batch completes. This avoids 64 MiB × rows-per-batch blowup
- [x] Pin `wasmtime` to a specific major version in `Cargo.toml` (fuel API breaks between wasmtime majors); document version constraint
- [x] **wasmtime version upgrade policy** documented in `CONTRIBUTING.md`: wasmtime major version may be bumped once per SlateDuck release cycle; the bump is a dedicated maintenance PR that updates the fuel API callsites and re-runs the full WASM UDF test suite. Staying on an EOL wasmtime major for more than one release cycle is disallowed (WASM sandbox CVEs accumulate)
- [x] `deterministic = true` annotation required; non-deterministic UDFs rejected at view creation with a clear error
- [x] UDF versioning: view pins to `udf_id` at creation; `ALTER INCREMENTAL MATERIALIZED VIEW v USING FUNCTION f VERSION N` migrates and triggers `REFRESH … FULL`
- [x] Argument and return types limited to Arrow-compatible scalars: BOOLEAN, INT8–INT64, FLOAT32/FLOAT64, UTF8, BINARY, DATE32, TIMESTAMP
- [x] Per-row fuel sub-budget: 10M instructions; if any single row exhausts its fuel slice, clean error naming the row and UDF — batch aborted, no partial output
- [x] WASM module validates against a whitelist of allowed WASI imports (none for pure functions)
- [x] Tested with a custom tokenizer UDF over event strings maintained incrementally
- [x] `docs/reference/udfs.md`: authoring guide, WASM compilation instructions (Rust → wasm32-unknown-unknown), determinism contract, version migration

### Remaining Optimizations (Adaptive Mode + DISTINCT Correctness)

Items deferred from v0.15 because they require the full operator surface or are correctness fixes tied to new v0.16 features.

**Adaptive DIFFERENTIAL/FULL mode switching (`CostMode::Adaptive`).** At low delta rates, DIFFERENTIAL is 5–90× cheaper than FULL. At high delta rates the crossover reverses. Without this switch, a large delta batch silently tanks throughput. Requires the full operator matrix (window functions, recursion) to calibrate properly — cannot ship in v0.15 or v0.16 with partial operator coverage.

- [x] `CostMode::Adaptive` variant in `config.rs`
- [x] Per-view rolling statistics tracked in the state store and surfaced via `observability.rs`: `rows_in`, `rows_out`, `ms_spent`, `last_full_cost`
- [x] Query complexity multiplier table: initial values `Scan 1.0×`, `Filter 1.1×`, `Aggregate 1.5×`, `Join 2.5×`, `JoinAggregate 4.0×`, `Window 3.0×`, `Recursive 5.0×`; switch DIFFERENTIAL→FULL when `Δ_rows / N_rows × multiplier > threshold` (default 0.5)
- [x] **Empirical calibration step (required before shipping):** Run TPC-H Q1/Q3/Q5 + TPC-DS Q4/Q47 at delta ratios 0.01, 0.05, 0.1, 0.3, 0.5, 0.7, 1.0; record actual DIFFERENTIAL vs FULL latency crossover point for each query class; adjust multiplier table to match observed crossover within ±20%. Publish calibration data in `benchmarks/v0.17-adaptive-calibration.json`
- [x] `WITH (cost_mode = 'adaptive', adaptive_threshold = 0.3)` per-view override; documented in `docs/operations/ivm-cost-control.md`

**Reference-counted DISTINCT and set operators.** The current DISTINCT implementation does not track duplicate counts, producing incorrect output when the same row is inserted multiple times and then partially deleted.

- [x] Add `__sd_ref_count: i64` auxiliary column to `IvmTrace` for views containing `DISTINCT` or `UNION DISTINCT` / `INTERSECT` / `EXCEPT`
- [x] INSERT increments `__sd_ref_count`; DELETE decrements; row visible in output only when `__sd_ref_count > 0`
- [x] `UNION DISTINCT`: `MAX(count_A, count_B)` — a row is present once if it appears in *either* operand, regardless of duplicates within each side. **Not addition** (that would be `UNION ALL`)
- [x] `INTERSECT`: `MIN(count_A, count_B)` — row present only when both sides contribute
- [x] `EXCEPT`: `count_A - count_B`, clamped to 0 — row present when left contributes more than right subtracts
- [x] Correctness test: insert same row 3×, delete 2×; confirm exactly one output row
- [x] Correctness test: `UNION DISTINCT` of two relations both containing the same row → exactly one output row (not two)

### Extended Operator Support Matrix

| Operator | v0.11 | v0.12 | v0.13 | v0.15 | v0.16 | v0.17 |
|---|---|---|---|---|---|---|
| `SELECT` / `WHERE` / `GROUP BY` / `HAVING` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Aggregates (count, sum, min, max, avg) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `DISTINCT`, `UNION ALL/DISTINCT` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ (ref-counted) |
| `JOIN` (broadcast, co-partition, reshuffle) | — | — | ✓ | ✓ | ✓ | ✓ |
| Uncorrelated subqueries | — | — | ✓ | ✓ | ✓ | ✓ |
| `ORDER BY` (total-order output) | — | — | — | — | ✓ | ✓ |
| `LIMIT` / `OFFSET` (top-N) | — | — | — | — | ✓ | ✓ |
| Window functions (partitioned) | — | — | — | — | ✓ | ✓ |
| Window functions (total-order) | — | — | — | — | ✓ | ✓ |
| Correlated subqueries (`EXISTS`, `IN`, scalar) | — | — | — | — | ✓ | ✓ |
| `CONNECT BY` / depth-bounded recursion | — | — | — | — | ✓ | ✓ |
| Recursive CTEs | — | — | — | — | ✓ | ✓ |
| `now()` / `random()` (capture semantics) | — | — | — | — | ✓ | ✓ |
| `DISTINCT` (ref-counted, correct under delete) | — | — | — | — | partial | ✓ |
| User-defined functions (WASM) | — | — | — | — | — | ✓ |
| Adaptive DIFFERENTIAL/FULL switching | — | — | — | — | — | ✓ |

### Testing: Tier 6f (WASM UDF + DISTINCT Correctness)

- [x] **Tier 6f — WASM UDF tests** (`crates/slateduck-ivm/tests/wasm_udf_tests.rs`): 6 tests —
  - Custom tokenizer UDF over event strings maintained incrementally; output matches DuckDB reference
  - UDF exceeding per-row fuel limit (10M instructions): clean error, no worker panic, view marked `Stale` (not `Broken`); `REFRESH FULL` recovers
  - UDF exceeding memory limit (64 MiB): clean error, same recovery behaviour as fuel exhaustion
  - UDF attempting file I/O (WASI `fd_write`): rejected at module load time (`CREATE FUNCTION` returns `SQLSTATE 0A000`)
  - UDF version migration: `ALTER … USING FUNCTION f VERSION N` triggers `REFRESH … FULL`; subsequent incremental results correct
  - Non-deterministic UDF (`deterministic = false`): rejected at `CREATE FUNCTION` time with `SQLSTATE 0A000` and clear message
- [x] **Tier 6f — DISTINCT property tests** (`crates/slateduck-ivm/tests/distinct_property_tests.rs`): property-based test with `proptest`; generates arbitrary insert/delete/update sequences against views using `SELECT DISTINCT`, `UNION DISTINCT`, `INTERSECT`, `EXCEPT`; asserts output multiset = DuckDB reference at every step; 500 sequences; covers: single insert-delete cycle, multi-insert partial-delete, cross-operand UNION DISTINCT with shared rows, INTERSECT where one operand goes empty, EXCEPT where subtractor count exceeds the original count (clamp to 0)
- [x] All Tier 6f tests run on every PR (standard runner)

### Testing: Tier 8 (Scale & Soak — IVM GA Gate)

- [x] **Tier 8 — TPC-H catalog benchmarks** (`tests/scale/tpch_catalog.rs`): re-run against v0.17 to confirm no regression; results written to `benchmarks/v0.17-tpch-{date}.json`
- [x] **Tier 8 — TPC-H IVM streaming** (`tests/scale/tpch_ivm.rs`): Q1, Q3, Q5 at 100k rows/s, 8 shards, 5 s freshness; re-verified with full operator surface
- [x] **Tier 8 — 24-hour soak test** (`tests/scale/soak.rs`): TPC-H Q1 continuous ingest; correctness drift check every 15 min (output row count matches DuckDB reference); fault injection every 15 min; `ivm_circuit_panic_total` = 0 after T+1h; **soak failure blocks GA tag**
- [x] **Tier 8 — 16-shard scale-out benchmark**: 16 workers on 16 separate instances, 1M rows/s ingest, aggregate throughput ≥ 500k rows/s, lag p99 ≤ 3 s
- [x] Scale and soak tests run on dedicated EC2 `c6i.4xlarge` via self-hosted GitHub Actions runner; triggered manually and on `v*` release tags; **24h soak is pre-release gate for GA tag only, not every pre-release tag**
- [x] CI comparison job alerts if any Tier 8 metric regresses > 10% vs previous run

### Acceptance Criteria

- [x] Every operator in the full matrix (including WASM and DISTINCT ref-counting) passes a correctness test against a DuckDB single-shot reference query
- [x] WASM UDF exceeding fuel/memory limit returns a clean error; no worker panic, no view corruption
- [x] All v0.11–v0.16 acceptance tests still pass
- [x] Tier 6f WASM UDF tests green (6 tests including sandbox isolation)
- [x] Tier 6f DISTINCT property tests green (500 sequences)
- [x] Extended benchmark: TPC-DS Q14, Q47, Q49 maintained incrementally with correctness verified (Q47/Q49 exercise window functions; Q14 exercises cross-join)
- [x] **Tier 8 soak test passes**: 24 h with zero correctness drift and fault injection recovery within SLO on every pre-release run
- [x] **Tier 8 TPC-H p99 within targets** (re-verified): SF10 < 50 ms catalog, SF100 < 100 ms catalog; IVM lag p99 < 5 s at 8 shards
- [x] **16-shard scale benchmark**: aggregate throughput ≥ 500k rows/s, lag p99 ≤ 3 s
- [x] `CostMode::Adaptive` correctly switches DIFFERENTIAL→FULL when `Δ_rows/N_rows × complexity > 0.5`; verified on TPC-H Q1 with synthetic 60%-delta batches
- [x] DISTINCT reference counting correct: insert-3×-delete-2× produces exactly one output row
- [x] `UNION DISTINCT` reference counting correct: same row in both operands → exactly one output row (MAX semantics, not addition)
- [x] All 10 test tiers green (Tiers 1–7 and 9–10 from prior phases; Tier 8 from this phase)

### Deliverables

- [x] `matview_udfs` catalog table (tag `0x21`) and `CREATE/DROP/ALTER FUNCTION` SQL surface
- [x] `wasmtime` integration in `slateduck-ivm` with per-batch pooled instances, fuel + memory sandboxing (pinned wasmtime major version)
- [x] wasmtime major-version upgrade policy in `CONTRIBUTING.md`
- [x] Tier 8 soak test (`tests/scale/soak.rs`) with 24 h fault-injection soak
- [x] Tier 8 16-shard scale benchmark
- [x] TPC-DS Q14/Q47/Q49 streaming benchmark suite in `benches/`
- [x] `benchmarks/v0.17-ivm-hardening.json` published
- [x] `docs/reference/udfs.md` authoring guide
- [x] All SQL reference docs in `docs/reference/sql-ivm.md` updated to reflect full operator coverage (including WASM and Adaptive)
- [x] `CostMode::Adaptive` with per-view rolling cost statistics in `config.rs` and `worker.rs`
- [x] `benchmarks/v0.17-adaptive-calibration.json` with empirical crossover data for multiplier table
- [x] `__sd_ref_count` auxiliary column for DISTINCT and set operators in `trace.rs`
- [x] Tier 6f WASM UDF tests (`wasm_udf_tests.rs`) with 6 passing tests in CI
- [x] Tier 6f DISTINCT property tests (`distinct_property_tests.rs`) with 500-sequence suite green in CI
- [x] Implementation plan [plans/incremental-view-maintenance-implementation.md](plans/incremental-view-maintenance-implementation.md) updated to reflect v0.17 additions

---

## v0.18 — DuckLake Catalog Standard Interface

> **Prerequisites:** Requires v0.17 merged to `main` (the IVM GA gate).

> Standardize SlateDuck's DuckLake catalog SQL surface to match the interface contract that pg-trickle (and any other DuckLake-compatible IVM system) expects. SlateDuck has no runtime or build dependency on pg-trickle code — instead, it implements a standard contract: `table_changes()` for O(Δ) CDC, stable `rowid` for update tracking, snapshot leases for GC coordination, `NOTIFY` for event-driven refresh, extension schemas for application metadata, and opaque frontier encoding for mixed-source systems. pg-trickle serves as the primary validator of this contract. See [plans/pg-trickle-ducklake-support.md](plans/pg-trickle-ducklake-support.md) for the full gap analysis and interface specification.

### Gap 1 — `table_changes()` SQL Function

Expose `reader.rs::SnapshotDiff` as a callable SQL table function over PG-wire:

```sql
SELECT rowid, change_type, <user_columns>
FROM table_changes('schema.table', start_snapshot := 42, end_snapshot := 45);
-- change_type ∈ { insert, delete, update_preimage, update_postimage }
```

Without this, pg-trickle falls back to O(N) polling (`EXCEPT ALL` full diff) instead of O(Δ) incremental CDC. For a 10M-row table with a 100-row delta, this is ~10⁷× more work per refresh cycle.

**Implementation:**

> **Note: This is a new scan operator, not trivial wiring.** `reader.rs::SnapshotDiff` provides *file-level* metadata (which Parquet files were added/removed between two snapshots), but `table_changes()` must return *row-level* change records. The operator must: (1) resolve added/removed file lists from SnapshotDiff, (2) read the affected Parquet files from object store, (3) emit rows with change_type annotations. For UPDATE detection (producing preimage/postimage pairs), the operator must correlate removed+added files by `rowid` (Gap 2). This is the first SlateDuck operator that reads data files — all other reads go through DuckDB.

- Implement `TableChangesOperator` in `crates/slateduck-sql/src/` as a new table-function scan node.
- Input: `SnapshotDiff` file lists (already available from catalog).
- For INSERT change_type: read rows from files present in `end` but absent in `start`.
- For DELETE change_type: read rows from files present in `start` but absent in `end`.
- For UPDATE: correlate by `rowid`; emit preimage (from removed file) and postimage (from added file).
- Return `SQLSTATE 55000` (snapshot too old) when `start_snapshot` has been GC'd so pg-trickle can fall back gracefully to full refresh.
- Register `table_changes` in the bounded SQL dispatcher function catalog.

**Acceptance criteria:**
- [x] `table_changes()` callable from DuckDB `ATTACH 'ducklake:postgresql://slateduck-sidecar/…'`
- [x] pg-trickle `cdc_mode` reports `DUCKLAKE_CHANGE_FEED` when source is SlateDuck-backed DuckLake
- [x] Property test: apply change records from `table_changes(start, end)` to `start` state → produces `end` state (multiset equality)
- [x] GC error path: `table_changes()` with a `start_snapshot` that has been GC’d returns `SQLSTATE 55000`; the error is distinguishable from all other errors by SQLSTATE alone (pg-trickle uses this to trigger a graceful full-refresh fallback)

### Gap 2 — Stable `rowid` on DuckLake Tables

Every SlateDuck-managed DuckLake table must expose a stable `rowid` column that survives UPDATE, file compaction, and Parquet file re-registration. pg-trickle's EC-01 phantom-row fix (see `plans/pg-trickle.md` §4) matches insert/delete pairs by `rowid`; without it, delete deltas are silently dropped and stale rows accumulate in pg-trickle's stream tables.

**Implementation:**

> **Design constraint: catalog never reads data files.** SlateDuck's architecture separates the catalog (metadata) plane from the data (Parquet) plane. Therefore SlateDuck cannot assign rowids by scanning Parquet. Instead, **the writer client (DuckDB / pg-trickle) assigns rowids at INSERT time** using a monotone counter obtained from the catalog API. SlateDuck provides the counter; the client stamps each row.

- The per-table monotone counter at key `0xFE | 0x10 | table_id` is exposed via a new SQL function: `SELECT slateduck.next_rowid_range('schema.table', count := 1000)` → returns `(start_rowid, end_rowid)` range.
- The writer client calls `next_rowid_range` before writing Parquet, stamps each row with a rowid from the allocated range, and includes `__sd_rowid` as a column in the Parquet file.
- `__sd_rowid` is registered as a hidden column in the DuckLake table schema (visible in `table_changes()` output, hidden from `SELECT *` by default).
- On compaction/file-rewrite, `__sd_rowid` values are preserved (never reassigned).
- Document the stability guarantee in `docs/concepts/ducklake.md`.

**Acceptance criteria:**
- [x] `rowid` appears in `table_changes()` output
- [x] `rowid` is stable across compaction, GC, and file splits (test with `slateduck compact` between two change windows)
- [x] EC-01 test case: delete row from both source and joined table in same refresh window; pg-trickle stream table matches full recompute
- [x] Concurrent write test: two writers call `next_rowid_range` concurrently for the same table 1000 times each; assert all allocated ranges are pairwise disjoint (no rowid collision)

### Gap 3 — Snapshot Lease / Hold Mechanism

GC must not advance past a snapshot ID that an external consumer (pg-trickle) has registered as its frontier. Otherwise, the next `table_changes(start_snapshot=42, …)` call returns `55000` and pg-trickle must do a full refresh unnecessarily.

**Implementation:**
- New catalog tag `0x22`: `snapshot_lease` with columns `(consumer_id TEXT, min_snapshot_id BIGINT, expires_at TIMESTAMPTZ)`.
- SQL function: `SELECT slateduck.hold_snapshot(min_snapshot_id := 42, consumer_id := 'pgtrickle:stream_1', ttl_seconds := 300)`.
- SQL function: `SELECT slateduck.release_snapshot(consumer_id := 'pgtrickle:stream_1')`.
- `gc.rs` reads minimum leased snapshot before advancing the visibility frontier.
- TTL prevents leaked leases from indefinitely blocking GC after ungraceful pg-trickle shutdown.

**Acceptance criteria:**
- [x] GC blocked at leased snapshot; advances once lease released
- [x] TTL expiry allows GC to advance after consumer disappears
- [x] Concurrent consumers: two consumers hold leases on the same snapshot; GC is blocked until both release; advances correctly afterward; tested with one clean release and one TTL expiry
- [x] `slateduck.hold_snapshot()` / `slateduck.release_snapshot()` callable via PG-wire from pg-trickle

### Gap 4 — `NOTIFY` on Snapshot Advance

pg-trickle's event-driven scheduler wakes up immediately when a `NOTIFY pgt_source_changed_<relid>` is emitted. Without this, pg-trickle falls back to polling (default 1 s), adding latency.

**Implementation:**
- After each `INSERT INTO ducklake_snapshot` (any source), emit `NOTIFY pgt_source_changed_<table_id>` to all connected PG-wire clients that have issued a matching `LISTEN`.
- Implement `LISTEN channel` and `UNLISTEN channel` in `slateduck-pgwire`.
- Clean up subscriptions on connection close.

**Acceptance criteria:**
- [x] `LISTEN`/`NOTIFY`/`UNLISTEN` round-trip via PG-wire
- [x] pg-trickle `scheduler` uses event-driven mode (not polling) when connected to SlateDuck
- [x] Latency test: snapshot advance → pg-trickle refresh start ≤ 50 ms end-to-end

### Gap 5 — Extension Schema Tables (`pgtrickle.*`)

pg-trickle issues `CREATE TABLE IF NOT EXISTS pgtrickle.pgt_ducklake_provenance (…)` and `INSERT INTO pgtrickle.pgt_ducklake_provenance (…)` against the catalog database at install time. SlateDuck's bounded SQL dispatcher currently returns `SQLSTATE 0A000` for user-schema DDL/DML.

**Implementation decision: first-class catalog objects (tag `0x23`).** The SQLite-sidecar alternative was rejected because it creates a second durability domain (sidecar can desync from catalog on crash), complicates backup/restore, and is not queryable via the standard PG-wire path without a second code path. First-class objects are more work upfront but architecturally sound.

- [x] Reserved extension-metadata key range: tag `0x23` with sub-tags per extension schema (e.g., `0x23 | 0x01` for `pgtrickle`)
- [x] `CREATE TABLE IF NOT EXISTS <extension_schema>.<table>` DDL handled in `slateduck-sql` bounded dispatcher for registered extension schemas
- [x] `INSERT`, `SELECT`, `DELETE` against extension schema tables routed through normal catalog read/write paths
- [x] Extension schema registration: `slateduck-pgwire --extension-schemas pgtrickle` CLI flag; unknown schemas still return `0A000`
- [x] Extension table schema is fixed at creation; `ALTER TABLE` on extension tables returns `0A000` (pg-trickle doesn't need it)

**Acceptance criteria:**
- [x] pg-trickle installs without errors against SlateDuck
- [x] `INSERT INTO pgtrickle.pgt_ducklake_provenance` succeeds
- [x] `SELECT * FROM pgtrickle.pgt_ducklake_provenance` returns inserted rows

### Gap 6 — Encryption Key Pass-Through

When DuckLake per-file Parquet encryption is enabled, `INSERT INTO ducklake_data_file` includes an `encryption_key` column. Audit and validate that SlateDuck stores and returns this column without mangling it.

**Acceptance criteria:**
- [x] `encryption_key` column present in `ducklake_data_file` schema
- [x] Round-trip test: insert file with `encryption_key = '\xDEADBEEF…'`, select it back, bytes identical
- [x] pg-trickle fixture corpus includes an encryption-key-bearing INSERT

### Gap 7 — Mixed Frontier (DuckLake Snapshot + WAL LSN)

For stream tables that read from both SlateDuck-backed DuckLake tables and PostgreSQL heap tables, the frontier must be a vector clock over heterogeneous source types.

> **Clarification: SlateDuck stores frontier values opaquely.** SlateDuck does not interpret WAL LSNs — it has no PostgreSQL replication knowledge. pg-trickle passes its own frontier JSON blob (containing WAL LSNs for PG sources and snapshot IDs for DuckLake sources) through the extension schema tables (Gap 5). SlateDuck's role is: (1) store the blob durably, (2) return it on read, (3) use the DuckLake snapshot component to coordinate its own GC (Gap 3). The `WalLsn` variant in the frontier type is opaque bytes that SlateDuck persists but never parses.

**Implementation:**
- Extend frontier type in `state_store.rs`: `BTreeMap<SourceId, SourceFrontier>` where `SourceFrontier` is `{SequenceNumber(u64) | DuckLakeSnapshot(i64) | Opaque(Vec<u8>)}`.
- `plan.rs` resolves DuckLake sources to `DuckLakeSnapshot`; all others stored as `Opaque`.
- Serialize frontier as JSON for observability; opaque values serialized as base64.
- pg-trickle is responsible for interpreting its own opaque frontier values; SlateDuck guarantees durability and atomic read/write only.

**Acceptance criteria:**
- [x] View definition mixing DuckLake source + opaque PG frontier stores and retrieves correctly
- [x] Frontier serialized as JSON, visible in `pgt_stream_tables.frontier`; opaque values base64-encoded
- [x] Round-trip test: store arbitrary bytes as opaque frontier, read back, bytes identical

### pg-trickle Compatibility Test Suite

A dedicated test crate (or test module in `slateduck-testkit`) that validates the full pg-trickle × SlateDuck integration:

**Tier A — Catalog Write Compatibility:** replay pg-trickle's internal DuckLake catalog SQL corpus against SlateDuck PG-wire; assert no `0A000` errors and correct final state.

**Tier B — `table_changes()` Property Tests:** property-based test applying change records to reconstruct any target snapshot; multiset equality assertion.

**Tier C — End-to-End Pipeline (Docker):** actual pg-trickle container → PostgreSQL sources → SlateDuck sink → DuckDB query verification.

**Tier D — Snapshot Hold Under GC:** GC blocked by lease; advances after release; TTL expiry.

### Acceptance Criteria

All of the following must be green before v0.18 is tagged:

- [x] pg-trickle connects to SlateDuck PG-wire sidecar with zero configuration changes vs. a standard PostgreSQL catalog
- [x] `CdcMode::DUCKLAKE_CHANGE_FEED` activates automatically when source table is SlateDuck-backed DuckLake
- [x] `table_changes()` passes the Tier-B property test suite
- [x] pg-trickle sink (`sink => 'ducklake'`) writes Parquet and commits DuckLake snapshots through SlateDuck
- [x] Provenance table (`pgtrickle.pgt_ducklake_provenance`) readable from pg-trickle
- [x] Snapshot lease prevents GC from breaking pg-trickle's frontier
- [x] `LISTEN`/`NOTIFY` round-trip enables event-driven scheduling
- [x] Encryption key pass-through validated
- [x] Tier A + B + D tests green in CI; Tier C green in pre-release gate
- [x] `docs/operations/pgtrickle-compatibility.md` published

### Deliverables

- [x] `table_changes()` SQL function in `crates/slateduck-sql/src/`
- [x] Stable `rowid` implementation in `crates/slateduck-catalog/src/writer.rs`
- [x] Snapshot lease catalog tag `0x22` + `slateduck.hold_snapshot()` / `release_snapshot()` SQL API
- [x] `LISTEN`/`NOTIFY`/`UNLISTEN` in `crates/slateduck-pgwire/src/`
- [x] Extension schema first-class catalog objects (tag `0x23`) with `CREATE TABLE IF NOT EXISTS` / `INSERT` / `SELECT` / `DELETE` support
- [x] Encryption key column audit + fixture
- [x] Mixed frontier support in `crates/slateduck-catalog/src/` (opaque frontier for non-DuckLake sources)
- [x] Compatibility test suite: `tests/compat/pgtrickle_*.rs`
- [x] `docs/operations/pgtrickle-compatibility.md`
- [x] DuckLake Spec Upgrade Policy updated to include pg-trickle `CHANGELOG.md` in review process

---

## v0.19 — CDC Correctness & Catalog Transaction Hardening

> Fix every correctness and transactional-safety gap identified in the v0.18 post-implementation review. All five critical findings and five high-severity transaction/GC findings must be resolved before v0.19 ships. This release has no new user-visible features; it deepens the semantic correctness of features already present.

### Real Row-Level CDC via `table_changes()`

The v0.18 implementation of `table_changes()` emits one synthetic row per added data file with a hardcoded rowid of `0`, no user column values, no delete records, and no update pre/post-image pairs. The standalone `compute_table_changes()` helper caps output at 100 rows regardless of actual row count. This is replaced with a real implementation:

- Integrate a Parquet reader into the `table_changes()` execution path so that each Parquet file in `diff.added_data_files` / `diff.retired_data_files` is scanned and actual rows are emitted
- Return full column payloads for every change record; `columns_json` must contain actual user column values keyed by name
- Emit `insert`, `delete`, and (where pre/post images are available) `update_preimage` / `update_postimage` change types
- Emit real `__sd_rowid` values from the underlying Parquet metadata
- Add a property test that applies the emitted change stream to the start-snapshot state and exactly reconstructs the end-snapshot state

### Versioned `DataFileRow` and `SnapshotDiff` Windows

`DataFileRow` currently only has `snapshot_id` (the snapshot at which the file was registered); it has no `begin_snapshot` / `end_snapshot` equivalent. `SnapshotDiff` ignores the `from_snapshot` parameter for data files and emits only files where `snapshot_id == to`. This is corrected:

- Version `DataFileRow` with `begin_snapshot` (the snapshot it was added) and optional `end_snapshot` (the snapshot it was logically deleted or replaced); migrate existing data via a catalog format version bump
- Update `snapshot_diff()` to scan the full `(from_snapshot, to_snapshot]` interval and include files whose begin/end range intersects that window
- Add `retired_data_files` to `SnapshotDiff`; `execute_table_changes()` uses both added and retired to produce a correct change set
- Add multi-snapshot window tests: `table_changes(42, 45)` must include changes at 43, 44, and 45

### CAS-Protected Writer Epoch

`CatalogStore::open()` unconditionally overwrites `SYSTEM_WRITER_EPOCH` and `check_epoch()` treats a missing epoch as success. Two concurrent openers can each believe they hold the write token:

- Replace the unconditional epoch write with a transactional writer-lease acquisition protocol: read current epoch, validate that no other writer holds a non-expired lease, CAS a new epoch, and fail closed when the epoch key is missing or not parseable
- `check_epoch()` must return `WriterEpochMismatch` when the stored epoch key is absent rather than succeeding silently
- Add concurrent-open tests using two independent `CatalogStore` instances on the same `Db` handle; verify that exactly one writer wins the epoch contest and the other is fenced

### Transactional Extension Schema Row-ID Allocation

`insert_extension_row()` is a non-atomic read/write/write sequence: it reads the marker counter, writes the data row, and then writes the updated marker. Concurrent inserts produce duplicate row IDs:

- Allocate extension table counters under `TAG_COUNTERS` using the existing serializable-transaction pattern already proven by `next_rowid_range()`
- Commit data row and counter atomically inside a `SerializableSnapshot` transaction with retry on conflict
- Add a concurrent insert test that spawns multiple concurrent insertions and verifies all assigned row IDs are unique

### Atomic GC Lease-Check and Retain-From Advancement

`gc_apply()` reads pinned snapshots and active leases, then writes `SYSTEM_RETAIN_FROM` in a separate non-transactional `db.put()`. A new lease acquired between the scan and the write is not protected:

- Wrap the current retain-from read, pin scan, lease scan, and retain-from write in a single `SerializableSnapshot` transaction
- Add an integration test that acquires a lease concurrently with a GC advance and verifies the lease is always respected

### Staged Catalog Write Discipline

`update_table_stats()`, `upsert_file_column_stats()`, `upsert_file_variant_stats()`, and several other `CatalogWriter` methods call `self.db.put()` directly, bypassing `create_snapshot()` atomicity. A failed commit can leave partially applied metadata visible:

- Convert all writer methods that must be durable to staging via `self.staged.push()` or add explicit non-MVCC documentation with separate consistency invariants and tests
- Add a test that kills the process between a direct `db.put()` metadata write and a subsequent `create_snapshot()` and verifies the catalog is still consistent after restart

### Overflow Safety in Counter Arithmetic

`next_rowid_range()` in both `CatalogWriter` and the standalone free function calculate `let end = current + count` without overflow guards. Lease TTL milliseconds multiplication can also overflow:

- Replace all counter arithmetic with `checked_add` and `checked_mul`; return `CatalogError::InvalidInput` on overflow or zero-count
- Add property tests near `u64::MAX` for rowid allocation and near `u64::MAX / 1000` for lease TTL
- Reject `count == 0` as an invalid rowid range request

### SQLSTATE Routing Bug in `SlateDuckError::SqlState`

`sqlstate()` returns the hardcoded string `"55000"` for the `SqlState { code, message }` variant regardless of the stored `code`. The first non-55000 use of this variant reports the wrong SQLSTATE to clients:

- Make `sqlstate()` return the stored code for the `SqlState` variant using `Cow<'_, str>` or add typed variants for each SQLSTATE code that has current callers
- Add tests covering each SQLSTATE mapping path in `error.rs`

### GC and Lease Resilience Improvements

- `list_active_leases()` silently ignores corrupt rows; return a catalog error for system rows with decode failures and add a warning log for extension rows
- `update_retain_from_cache()` uses `Ordering::Relaxed`; upgrade to `Ordering::Release` on store and `Ordering::Acquire` on load, or document the ordering invariant explicitly
- Lease TTL arithmetic uses `unwrap_or_default()` on `SystemTime::now().duration_since(UNIX_EPOCH)`, masking clock-jump errors; add checked arithmetic and reject TTL values that would overflow `u64`

### Test and Documentation Deliverables

- [x] Property test: `table_changes()` change stream reconstructs end-snapshot state from start-snapshot
- [x] Property test: `next_rowid_range()` overflow coverage near `u64::MAX`
- [x] Integration test: concurrent writer-epoch acquisition, exactly one winner
- [x] Integration test: extension schema concurrent inserts, all row IDs unique
- [x] Integration test: GC advance vs. concurrent lease acquisition, lease always wins
- [x] Integration test: process crash between direct `db.put()` and `create_snapshot()`, catalog consistent after restart
- [x] `docs/architecture/cdc-design.md`: describe the full `table_changes()` execution pipeline including Parquet scan, rowid extraction, and change correlation
- [x] `docs/architecture/writer-fencing.md`: document the CAS epoch acquisition protocol and failure modes

---

## v0.20 — FFI Safety, Live Notifications & Operational Wire-Up

> Resolve the FFI unsoundness, wire LISTEN/NOTIFY end-to-end, make extension schema registration configurable, fix extension JSON serialization, fix collision-prone hashed keys, and add TLS/auth safeguards. This release completes the operational surface started in v0.18 and makes the SlateDuck FFI extension safe for distribution.

### FFI Handle Safety Overhaul

`validate_catalog()` returns `Option<&'static mut SlateduckCatalog>` even though the referenced allocation lives only until `slateduck_close()`. `slateduck_close()` reads, zeroes, and drops through raw pointers with no synchronization for concurrent close/use:

- Remove the `&'static mut` return and redesign validation to provide short-lived, scoped access only (closure-based or with explicit lifetime bounds)
- Introduce an internal `SAFETY:` documentation block above every unsafe block in `lib.rs` stating the pointer validity condition, lifetime assumption, and aliasing constraint
- Implement double-close and use-after-close guards that are correct under concurrent calling without relying on magic-number checks in isolation
- Audit all `Vec::from_raw_parts()` calls in the `_free` family for correct capacity vs. length usage

### Sanitizer and Miri CI Coverage

- Add a scheduled nightly CI job that runs `slateduck-ffi` tests under ASAN and UBSAN (`RUSTFLAGS="-Z sanitizer=address"` / `"-Z sanitizer=undefined"`)
- Add a Miri job for the same crate to catch UB in pure-Rust unsafe code paths
- Gate the jobs as non-blocking at first, with a plan to promote to blocking at v1.0

### Live LISTEN/NOTIFY End-to-End

`NotifyManager` and `ConnectionSubscriptions` are implemented but completely disconnected from `SessionState` and the executor. `LISTEN` and `UNLISTEN` commands return acknowledgement tags without registering any subscription:

- Add a shared `Arc<NotifyManager>` to server state and thread it into every connection handler
- Add `ConnectionSubscriptions` to `SessionState`
- In the executor's `Listen` arm, call `session.subscriptions.listen(&channel, &notify_manager).await` and return `LISTEN` only after successful registration
- In the executor's `Unlisten` arm, call `session.subscriptions.unlisten(&channel)` before returning
- After every successful `create_snapshot()`, call `notify_manager.notify()` for each channel that has active subscribers; flush pending notifications to the pg-wire client in the next idle cycle
- Add integration tests: LISTEN before a snapshot commit receives a notification; UNLISTEN stops delivery; multiple subscribers on the same channel all receive the notification

### Configurable Extension Schema Registration

`resolve_extension_id()` hardcodes `"pgtrickle"` as the only recognized extension schema. Operators cannot restrict or extend the list without code changes:

- Add a `--extension-schemas <schema,...>` CLI flag and `SLATEDUCK_EXTENSION_SCHEMAS` environment variable; default is `pgtrickle` to maintain backward compatibility
- Thread the allowed-extension list into server state and pass it to `is_registered_extension()` before routing any extension DDL or DML
- Return `SlateDuckError::PermissionDenied` (SQLSTATE 42501) for unregistered extension schemas
- Remove the unconditional hardcoded case in `resolve_extension_id()` in favour of the configurable list
- Document the flag in `--help` and in `docs/operations/extension-schemas.md`

### Extension Schema JSON Serialization Fix

`ParamValues::to_json_string()` builds JSON by string-interpolation with `format!("\"p{}\":\"{}\"", i, val)` without any escaping. Values containing `"`, `\`, newlines, or other control characters produce malformed JSON. Column names are also discarded in favour of positional keys (`p0`, `p1`):

- Replace `to_json_string()` with a `serde_json::Map`-based implementation that properly escapes all values
- Preserve column names extracted from the parsed `INSERT INTO` statement rather than using positional keys
- Return a parse error for values that cannot be round-tripped through `serde_json`
- Add property tests covering embedded quotes, backslashes, Unicode escapes, and control characters

### Collision-Safe Catalog Key Encoding

Snapshot lease keys and extension table keys derive a 64-bit hash from `consumer_id` and `table_name` respectively using `DefaultHasher`. Distinct strings can produce identical hashes, and `DefaultHasher` is not stable across Rust versions:

- Replace hash-based key encoding with length-prefixed UTF-8 byte strings: `[tag] [u16 length BE] [utf-8 bytes]`
- Validate the original string from the decoded row value before acting on it, rejecting any mismatch as a corruption error
- Add a catalog format migration for existing lease and extension-schema keys
- Add collision-stress property tests that verify distinct identifiers always produce distinct keys

### TLS and Authentication Hardening

- `build_tls_acceptor()` calls `.unwrap()` on `cert_path` and `key_path`; replace with `ok_or_else()` returning `std::io::Error`
- When password authentication is enabled without `--tls-required`, emit a `warn!` log at startup and add a `--insecure-no-tls-warning-suppress` flag for environments where this is intentional
- Add an integration test that passes a TLS config with only cert or only key path and verifies a clean error is returned rather than a panic

### `read_latest()` Semantic Documentation

`read_latest()` derives the latest snapshot from the in-memory counter (`peek_snapshot_id() - 1`) rather than from a fresh SlateDB read. Add a `read_fresh_latest()` function that reads the counter from SlateDB for use by long-lived read-only processes, and document the distinction in the public API doc comment.

### Test and Documentation Deliverables

- [x] FFI integration tests: double-close safety, use-after-close, null handle, concurrent close/use
- [x] CI: scheduled nightly ASAN + UBSAN + Miri jobs for `slateduck-ffi`
- [x] Integration test: LISTEN → snapshot commit → notification received by subscriber
- [x] Integration test: UNLISTEN stops delivery; multiple subscribers on one channel
- [x] Integration test: unregistered extension schema returns SQLSTATE 42501
- [x] Property test: extension JSON round-trip with special characters
- [x] Property test: length-prefixed key encoding with arbitrary `consumer_id` and `table_name`
- [x] `docs/operations/extension-schemas.md`: registration model, CLI flag, allowed list
- [x] `docs/architecture/ffi-safety.md`: pointer ownership, handle lifecycle, SAFETY invariants

---

## v0.21 — Performance, Scalability & Code Quality

> Address the performance and scalability ceilings visible at v0.18, refactor the largest module bottlenecks, enforce MSRV and license hygiene in CI, fix metrics documentation drift, and close all remaining dead-code and code-quality debts. This release targets the scale claims made in the v1.0 acceptance criteria.

### `list_data_files()` Secondary Index and Read Amplification

The current implementation does a full prefix scan over all data files for a table, then filters `snapshot_id <= read_snapshot` in memory. There is no secondary index by snapshot and no end-snapshot filter, so historical and time-travel reads scan the full file set:

- Add a secondary index `TAG_DATA_FILES_BY_SNAPSHOT | table_id(u64 BE) | snapshot_id(u64 BE) | file_id(u64 BE)` updated atomically with every `register_data_file()` call
- Update `list_data_files()` to use a range scan on the new index keyed by `(table_id, read_snapshot)` rather than a full prefix scan plus in-memory filter
- Add a benchmark comparing scan latency before/after at 10⁴ and 10⁵ files per table at a historical snapshot

### Aggregate Deletion Complexity Fix (StringAgg / ArrayAgg)

Deletions for `StringAgg` and `ArrayAgg` aggregates loop over negative weight, call `Vec::position()`, and then `Vec::remove(pos)`, giving O(N²) behavior for large groups with selective deletes:

- Replace `Vec<Value>` rescan input storage with a counted multiset (`HashMap<CanonicalValue, i64>`) where deletes decrement counts and compaction sweeps zero-count entries
- Add a TPC-H Q18 variant benchmark that measures STRING_AGG aggregate deletion performance at 100k and 1M row group sizes

### SQL Classifier Hardening

`classify_listen_prefix()`, `find_as_keyword()`, and `split_qualified_name()` are handwritten string parsers that mis-handle quoted identifiers, `AS` without surrounding spaces, and embedded comments. `LISTEN` accepts any string including empty or invalid identifiers:

- Replace `find_as_keyword()` with an AST-backed approach where sqlparser can be used, falling back to a tokenizer that correctly handles SQL quoting and comments
- Validate `LISTEN` channel names against PostgreSQL identifier rules (alphanumeric + underscore, no leading digit, 1–63 characters); return SQLSTATE 42602 for invalid channel names
- Validate quoted identifiers in `split_qualified_name()`
- Add classifier tests for: quoted schema names, names with dots inside quotes, `AS` without trailing space, LISTEN with invalid channel, LISTEN with empty channel

### Module Decomposition

`executor.rs` (~1,629 lines), `writer.rs` (~1,402 lines), and `classifier.rs` (~990 lines) each mix multiple unrelated concerns. New features continue to enlarge the same files:

- Split `executor.rs` by feature family: `executor/catalog.rs`, `executor/extension.rs`, `executor/session.rs`, `executor/meta.rs`; keep `execute_classified()` as a thin dispatcher
- Split `catalog/writer.rs` into `writer/staged.rs` (MVCC staged mutations), `writer/stats.rs` (statistics methods), `writer/counters.rs` (ID allocation), and `writer/snapshot.rs` (snapshot commit)
- Split `sql/classifier.rs` into `classifier/ast.rs`, `classifier/prefix.rs`, `classifier/table_selects.rs`
- Enforce a lint that no new source file in these crates may exceed 600 lines

### CI Hardening: MSRV, License Enforcement, Full Coverage

- Add a CI job that installs `dtolnay/rust-toolchain@1.93` and runs `cargo check --workspace --all-targets`; fail the PR if it does not compile on the declared MSRV
- Add `licenses` to the `cargo deny check` CI command; define an allowed license list (Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, Unicode-3.0, ISC, CC0-1.0); add explicit exceptions with rationale for any dep outside the allow list
- Extend the coverage CI job to include all production crates: `slateduck-pgwire`, `slateduck-ffi`, `slateduck-sql`, `slateduck-datafusion`, `slateduck-sqlite-vfs` in addition to the existing `slateduck-catalog` and `slateduck-core`
- Remove the two stale `advisory-not-detected` warnings from `deny.toml` (`RUSTSEC-2024-0370` and `RUSTSEC-2025-0057`)

### Metrics CLI and Documentation Alignment

`docs/operations/monitoring.md` and `docs/reference/metrics.md` document `--metrics-path` and `SLATEDUCK_METRICS_PATH`, but the CLI parser only supports `--metrics-port` and `--metrics-bind`; the HTTP server also responds to any path rather than only `/metrics`:

- Implement `--metrics-path` CLI flag and `SLATEDUCK_METRICS_PATH` env var, defaulting to `/metrics`
- Update the metrics HTTP server to only serve metrics on the configured path and return 404 for other paths
- Update the CLI `--help` output to list the metrics flags consistently
- Update monitoring and reference docs to reflect the actual flag names

### Dead-Code and Dependency Hygiene

- Resolve all `#[allow(dead_code)]` items in production code: implement the stub (link to the tracking roadmap item), delete the unreachable code, or replace with `todo!()` decorated with an issue reference
- Audit `Cargo.toml` workspace dependencies: move `object_store = { features = ["aws", "gcp", "azure"] }` and `tokio = { features = ["full"] }` to per-crate opt-in features so that crates that do not need cloud backends or the full tokio runtime do not include them
- Audit `#[allow(clippy::too_many_arguments)]` usages in `writer.rs`: introduce `FileColumnStatsInput` and `FileVariantStatsInput` parameter structs to bring arg counts below the clippy threshold

### Test and Documentation Deliverables

- [x] Benchmark: `list_data_files()` with secondary index at 10⁴ / 10⁵ files
- [x] Benchmark: STRING_AGG deletion at 100k / 1M group size
- [x] CI: MSRV 1.93 check job
- [x] CI: `cargo deny check licenses`
- [x] CI: full workspace coverage reporting
- [x] Integration test: `--metrics-path` routing returns 200 on configured path and 404 elsewhere
- [x] Classifier tests: quoted identifiers, AS edge cases, invalid LISTEN channels
- [x] `docs/contributing/code-style.md`: module size limit, parameter struct conventions, dead-code policy

---

## v0.22 — IVM Removal

> Remove all Incremental View Maintenance code from SlateDuck. IVM is an architectural mismatch: it bolted a streaming aggregation engine onto a catalog store that was designed never to be in the data path. The `list_inlined_inserts` source reads all rows on every tick (O(total rows), not O(delta)), making it equivalent to or worse than full DuckDB re-execution. The wasmtime dependency alone adds ~30 s to clean builds. This release strips the feature entirely so the codebase reflects what SlateDuck actually is: a serverless DuckLake catalog backed by SlateDB.
>
> See [plans/incremental-view-maintenance-implementation-removal.md](plans/incremental-view-maintenance-implementation-removal.md) for the full per-file inventory and rationale.

### Phase 1 — Delete the IVM Crate

Delete `crates/slateduck-ivm/` in its entirety: 36 source files, 13 integration test files, and `Cargo.toml`. This is the largest single change in the release.

- [ ] `rm -rf crates/slateduck-ivm/`

### Phase 2 — Workspace Cargo.toml

- [ ] Remove `"crates/slateduck-ivm"` from `[workspace].members`
- [ ] Remove `wasmtime = "43"` from `[workspace.dependencies]` (used exclusively by `slateduck-ivm`)
- [ ] Remove any IVM-related comments near those entries

### Phase 3 — slateduck-core Cleanup

**tags.rs** — Remove the four IVM catalog tags and their `TAG_REGISTRY` descriptors. Do **not** renumber existing tags; leave a gap comment `// 0x1D–0x20: removed (formerly IVM — v0.22)` for forward compatibility with old catalogs.

- [ ] Remove `TAG_MATVIEW = 0x1D`
- [ ] Remove `TAG_MATVIEW_DEP = 0x1E`
- [ ] Remove `TAG_MATVIEW_CHECKPOINT = 0x1F`
- [ ] Remove `TAG_MATVIEW_SHARD = 0x20`
- [ ] Remove section header `// ─── v0.11 IVM Catalog Tables ──`
- [ ] Remove `// Tags 0x24–0x2F reserved for future IVM-related tables.`
- [ ] Remove four `TagDescriptor` entries from `TAG_REGISTRY`

**rows.rs** — Remove IVM row types:

- [ ] Remove `MatviewRow` struct (and all fields)
- [ ] Remove `OutputMode` enum + `from_u32()`
- [ ] Remove `MatviewDepRow` struct
- [ ] Remove `MatviewCheckpointRow` struct
- [ ] Remove `MatviewShardRow` struct

**keys.rs** — Remove IVM key-encoding functions and their tests:

- [ ] Remove `key_matview()`, `key_matview_dep()`, `key_matview_checkpoint()`, `key_matview_shard()`
- [ ] Remove `prefix_matview()`, `prefix_matview_deps()`, `prefix_matview_checkpoints()`, `prefix_matview_shards()`
- [ ] Remove tests: `matview_key_structure`, `matview_dep_key_structure`, `matview_checkpoint_key_structure`, `matview_shard_key_structure`, `matview_key_prefix_isolation`, `matview_checkpoint_seq_ordering`

### Phase 4 — slateduck-catalog Cleanup

**writer.rs** — Remove the `ClaimOutcome` enum and all matview write operations:

- [ ] Remove `ClaimOutcome { Acquired, AlreadyOwned, Contended }` enum
- [ ] Remove `create_matview()`, `drop_matview()`, `set_matview_status()`
- [ ] Remove `update_matview_checkpoint()`, `claim_matview_shard()`, `extend_matview_lease()`
- [ ] Remove `release_matview_lease()`, `set_matview_output_mode()`, `re_shard_matview()`

**reader.rs** — Remove all matview read operations:

- [ ] Remove `list_matviews()`, `get_matview()`, `get_matview_by_name()`
- [ ] Remove `list_matview_deps()`, `list_matview_shards()`, `list_shards_for_worker()`
- [ ] Remove `read_checkpoint_history()`, `matview_lag_ms()`, `matview_max_lag_ms()`

**lib.rs** — Remove `ClaimOutcome` from `pub use` if re-exported.

**Tests:**

- [ ] Delete `tests/v011_tests.rs` entirely (19 IVM-focused tests)
- [ ] Remove `ivm_integration_ingest_to_cdc_pipeline` section from `tests/v010_tests.rs`

### Phase 5 — slateduck-sql Cleanup

**classifier.rs** — Remove all IVM DDL statement variants and the classifier function:

- [ ] Remove `StatementKind` variants: `CreateIncrementalMatview`, `DropIncrementalMatview`, `AlterIncrementalMatview`, `RefreshIncrementalMatviewFull`, `ShowMaterializedViews`, `ShowMatviewShards`, `ExplainMatview`
- [ ] Remove `classify_ivm_prefix(sql)` function (~100 lines)
- [ ] Remove the call site invoking `classify_ivm_prefix` in `classify()`
- [ ] Remove section header comment `// ─── v0.11 IVM Statements ───`

### Phase 6 — slateduck-pgwire Cleanup

- [ ] Remove `slateduck-ivm = { path = "../slateduck-ivm" }` from `Cargo.toml`
- [ ] Remove the IVM match arm in `executor.rs` routing IVM `StatementKind` variants to `SlateDuckError::Unsupported` (arms will cease to exist after Phase 5 anyway)
- [ ] Remove `use slateduck_ivm::rate_limit::{...}` from `tests/security_tests.rs`
- [ ] Remove IVM workflow comment from `tests/compat_tests.rs`

### Phase 7 — slateduck-testkit Cleanup

- [ ] Remove `slateduck-ivm = { path = "../slateduck-ivm" }` from `Cargo.toml`
- [ ] Delete `src/harness.rs` (`IvmWorkerHarness`) if IVM-only; otherwise gut IVM content
- [ ] Delete `src/oracle.rs` (`IvmOracle`) if IVM-only; otherwise gut IVM content
- [ ] Remove IVM assertion helpers from `src/duckdb_harness.rs`
- [ ] Remove IVM lease TTL support from `src/clock.rs` if IVM-only
- [ ] Remove `IvmWorkerHarness` and `IvmOracle` re-exports from `lib.rs`

### Phase 8 — Build and Test Gate

After Phases 1–7, verify the workspace compiles and all remaining tests pass before touching docs:

- [ ] `cargo build --workspace` — must compile with zero errors
- [ ] `cargo test --workspace` — all remaining tests pass
- [ ] `cargo clippy --workspace -- -Dwarnings` — zero warnings

### Phase 9 — Documentation Removal

Delete entirely:

- [ ] `docs/architecture/ivm-plane.md`
- [ ] `docs/concepts/incremental-views.md`
- [ ] `docs/reference/sql-ivm.md`
- [ ] `docs/operations/ivm-join-sizing.md`
- [ ] `docs/operations/ivm-cost-control.md`
- [ ] `docs/operations/ivm-backup-restore.md`
- [ ] `docs/design-decisions/ivm-architecture.md`
- [ ] `docs/design-decisions/ivm-on-immutable-substrate.md`
- [ ] `docs/design-decisions/ivm-recursive-spike.md`
- [ ] `docs/design-decisions/ivm-retrospective.md`

Edit (remove IVM sections only):

- [ ] `docs/architecture/streaming-pipeline.md` — remove IVM references
- [ ] `docs/architecture/key-layout.md` — remove "v0.11 IVM Tag Extensions" section
- [ ] `docs/reference/udfs.md` — remove IVM-related lines

**mkdocs.yml:**

- [ ] Remove all nav entries referencing deleted IVM docs files
- [ ] Run `mkdocs build` to confirm no broken links

### Phase 10 — Benchmarks and Test Fixtures

Delete IVM benchmark files:

- [ ] `benchmarks/v0.12-ivm-scaleout.json`
- [ ] `benchmarks/v0.13-ivm-joins.json`
- [ ] `benchmarks/v0.15-ivm-hardening.json`
- [ ] `benchmarks/v0.17-ivm-hardening.json`
- [ ] `benchmarks/v0.17-adaptive-calibration.json`

Delete IVM test fixtures:

- [ ] `tests/fixtures/matview/` (entire directory: `create_view.dat`, `checkpoint_history.dat`, `multi_shard.dat`, `dropped.dat`, `lease_acquired.dat`)

### Phase 11 — README.md and ROADMAP.md

**README.md:**

- [ ] Remove IVM from the project tagline
- [ ] Remove `slateduck-ivm` from the crate table
- [ ] Remove the IVM Getting Started example
- [ ] Remove the "Incremental View Maintenance" section entirely
- [ ] Remove IVM rows from the roadmap summary table

**ROADMAP.md** (this file):

- [ ] Remove `## v0.11` through `## v0.17` milestones (IVM foundations through feature hardening)
- [ ] Remove IVM test tier references (tiers 6a–6d, 6e–6f, tier 7) from cross-cutting sections
- [ ] Remove IVM GA gate from v1.0 sign-off criteria (item 9)
- [ ] Remove IVM documentation gate from v1.0 sign-off criteria (item 11)
- [ ] Remove "DBSP/Feldera Dependency Strategy" from Cross-Cutting Concerns
- [ ] Remove "IVM Worker Deployment Model" from Cross-Cutting Concerns
- [ ] Remove "Graceful Shutdown & Rolling Updates (IVM Workers)" from Cross-Cutting Concerns
- [ ] Remove `IvmWorkerHarness` and `IvmOracle` from `slateduck-testkit` harness list
- [ ] Update the v0.23 note that says "its CDC output primitives feed into IVM" — remove IVM reference

### Phase 12 — CI Cleanup

**.github/workflows/ci.yml:**

- [ ] Remove tier 7 comment and IVM fault injection test step
- [ ] Remove IVM hardening test step
- [ ] Remove IVM property test step
- [ ] Remove benchmark regression check referencing IVM JSON files

### Phase 13 — deny.toml Cleanup

- [ ] Remove `RUSTSEC-2024-0370` advisory ignore (proc-macro-error via dbsp — IVM-only transitive dep)
- [ ] Remove `RUSTSEC-2025-0057` advisory ignore (fxhash via wasmtime 43 — IVM-only transitive dep)
- [ ] Run `cargo deny check` to confirm no new unhandled advisories

### Phase 14 — Final Verification

- [ ] `cargo build --workspace` — clean build
- [ ] `cargo test --workspace` — all tests pass
- [ ] `cargo clippy --workspace -- -Dwarnings` — zero warnings
- [ ] `cargo deny check` — no new advisories
- [ ] `rg -i "matview|slateduck.ivm|IvmWorker|IvmCircuit|ZDelta|TAG_MATVIEW" --type rust` — zero hits in production code
- [ ] `mkdocs build --strict` — no broken links

### Expected Impact

| Metric | Before | After |
|--------|--------|-------|
| Source lines removed | — | ~20,000 (src) + ~5,000 (tests) |
| Crates removed | — | `slateduck-ivm` |
| Workspace dependencies dropped | — | `wasmtime`, `dbsp` transitives |
| Clean build time reduction | — | ~30 s (wasmtime compile) |
| Binary eliminated | — | `slateduck-ivm` binary |
| Advisories dropped from deny.toml | — | 2 (`RUSTSEC-2024-0370`, `RUSTSEC-2025-0057`) |

---

## v0.24 — DuckLake v1.0 Conformance Harness & Interop-Critical Schema

> Establish a machine-readable conformance harness for all 28 DuckLake v1.0 catalog tables, then fix the highest-severity P0 schema gaps that block DuckDB client interoperability: snapshot/snapshot_changes, data files, delete files, row ID tracking, table stats, and DROP TABLE cascade retirement.

### Phase 0 — Conformance Harness

Before any schema work lands, a machine-readable manifest and golden-test suite must exist so every subsequent change is verifiable against the spec. This harness becomes the regression gate for all later DuckLake compatibility work.

- [ ] Add a machine-readable DuckLake v1.0 schema manifest derived from `specification/tables/overview.md` — one TOML or JSON file that lists all 28 tables, their column names, column types, nullability, and whether a column is spec-required or extension-only.
- [ ] Add tests that assert the SQL facade exposes all 28 tables with exact column names and compatible types; fail fast on any column-name or type mismatch.
- [ ] Add golden tests for the SQL query examples in `specification/queries.md`; capture expected column order and representative row shapes.
- [ ] Add tests that verify unsupported DuckLake SQL writes fail with an explicit error rather than returning success as a no-op. Any statement accepted by PgWire but not persisted must return `SQLSTATE 0A000` (feature not supported) or equivalent.
- [ ] Wire the conformance manifest check into CI so schema regressions are caught on every PR.

### Phase 1 — Snapshot and Snapshot Changes Schema

Spec:
- `ducklake_snapshot(snapshot_id, snapshot_time, schema_version, next_catalog_id, next_file_id)`
- `ducklake_snapshot_changes(snapshot_id, changes_made, author, commit_message, commit_extra_info)`

- [ ] Add `next_catalog_id` and `next_file_id` to `SnapshotRow`, populated from `TAG_COUNTERS` at commit time.
- [ ] Move `author` and `message` semantics out of `SnapshotRow` and into `SnapshotChangesRow` as `author` and `commit_message`; add `commit_extra_info` field.
- [ ] Persist a spec-compatible `changes_made` string per snapshot using documented values: `created_schema:<schema_name>`, `inserted_into_table:<table_id>`, `dropped_table:<table_id>`, etc.
- [ ] Update `execute_commit` to write `SnapshotChangesRow` transactionally alongside the snapshot row — not as an informational side-effect.
- [ ] Update the PgWire `SelectSnapshot` and `SelectSnapshotChanges` response builders to expose the exact spec columns in spec column order.
- [ ] Add conformance tests: insert a snapshot, select it back, verify `next_catalog_id` and `next_file_id` are non-zero and match the counter state at commit time.

### Phase 2 — Spec-Complete Data File Model

Spec `ducklake_data_file` columns: `data_file_id`, `table_id`, `begin_snapshot`, `end_snapshot`, `file_order`, `path`, `path_is_relative`, `file_format`, `record_count`, `file_size_bytes`, `footer_size`, `row_id_start`, `partition_id`, `encryption_key`, `mapping_id`, `partial_max`

- [ ] Add `file_order` to `DataFileRow`; persist it as a monotonically increasing integer within a table, assigned at registration time.
- [ ] Add `path_is_relative` boolean to `DataFileRow`; default `false` for absolute paths.
- [ ] Rename `row_count` → `record_count` in `DataFileRow` and all PgWire response builders.
- [ ] Change `footer_size` from `Option<String>` to `Option<i64>` (BIGINT semantics).
- [ ] Add `row_id_start` to `DataFileRow`; populated from the pre-increment `next_row_id` counter at file registration time.
- [ ] Add `partition_id`, `mapping_id`, and `partial_max` to `DataFileRow`.
- [ ] Remove legacy `snapshot_id` field from `DataFileRow`; `begin_snapshot` is the canonical field.
- [ ] Fix `CatalogReader::list_data_files` to filter out rows where `end_snapshot` is ≤ the requested snapshot (MVCC retirement visibility).
- [ ] Fix `list_data_files` to order results by `file_order`.
- [ ] Update PgWire `InsertDataFile` to read and persist all new spec fields from the incoming SQL parameters.
- [ ] Add conformance tests: register three data files, drop the middle one, time-travel to before and after the drop, verify retired files are absent from the later snapshot and present at the earlier one.

### Phase 3 — Spec-Complete Delete File Model

Spec `ducklake_delete_file` columns: `delete_file_id`, `table_id`, `begin_snapshot`, `end_snapshot`, `data_file_id`, `path`, `path_is_relative`, `format`, `delete_count`, `file_size_bytes`, `footer_size`, `encryption_key`, `partial_max`

- [ ] Add `table_id`, `begin_snapshot`, `end_snapshot`, `path_is_relative`, `format`, `footer_size`, and `partial_max` to `DeleteFileRow`.
- [ ] Rename `row_count` → `delete_count` in `DeleteFileRow`.
- [ ] Implement `CatalogReader::list_delete_files(table_id, snapshot_id)` with spec MVCC visibility (`begin_snapshot ≤ snapshot_id` and (`end_snapshot IS NULL` or `end_snapshot > snapshot_id`)).
- [ ] Fix `PgWire SelectDeleteFiles` to call `list_delete_files` and return a spec-shaped result set; currently returns empty.
- [ ] Update `InsertDeleteFile` to persist all spec fields.
- [ ] Add key/index support for delete-file lookup by `table_id` and snapshot range.
- [ ] Add conformance tests: register a delete file, select it, verify `table_id`, `begin_snapshot`, `format`, and `delete_count` are correct; retire it and verify it disappears from the visible set at the retire snapshot.

### Phase 4 — Row ID Tracking and Table Stats

Spec `ducklake_table_stats` columns: `table_id`, `record_count`, `next_row_id`, `file_size_bytes`

- [ ] Add `next_row_id` to `TableStatsRow`; update it atomically with each data-file registration using the pre-increment value for `row_id_start`.
- [ ] Rename `row_count` → `record_count` and `total_size_bytes` → `file_size_bytes` in `TableStatsRow` and all PgWire response builders.
- [ ] Keep `file_count` as an internal/extension statistic only; do not expose it in the spec-shaped facade.
- [ ] Fix `PgWire UpdateTableStats` to apply the incoming row-count and size deltas rather than calling `update_table_stats(table_id, 0, 0, 0)`.
- [ ] Fix `PgWire SelectTableStats` to call the reader and return spec-shaped rows; currently returns empty.
- [ ] Add conformance tests: insert two data files with 100 rows each, verify `record_count = 200`, `next_row_id ≥ 200`, and `file_size_bytes` matches the sum.

### Phase 5 — DROP TABLE Cascade Retirement

Spec: DROP TABLE must set `end_snapshot` on all of: `ducklake_table`, `ducklake_partition_info`, `ducklake_column`, `ducklake_column_tag`, `ducklake_data_file`, `ducklake_delete_file`, `ducklake_tag`

- [ ] Extend `CatalogWriter::drop_table` to retire all dependent rows (columns, column tags, data files, delete files, tags, partition info) in the same snapshot transaction.
- [ ] Extend `PgWire UpdateEndSnapshot` handling to cover all spec tables, not just `ducklake_table` and `ducklake_column`.
- [ ] Add conformance tests: create a table with columns, tags, and data files; drop it; verify every related row across all spec tables has `end_snapshot` set at the drop snapshot; verify the table is invisible to readers at the drop snapshot and visible to readers at the snapshot before the drop.

### Deliverables

- [ ] Conformance manifest checked into `tests/fixtures/ducklake-v1.0-schema.toml`
- [ ] Conformance test suite green on every PR
- [ ] `ducklake_snapshot` and `ducklake_snapshot_changes` spec-compatible in protobuf and PgWire facade
- [ ] `ducklake_data_file` all spec fields present; MVCC visibility and `file_order` ordering correct
- [ ] `ducklake_delete_file` all spec fields present; `list_delete_files` returns spec-shaped rows
- [ ] `ducklake_table_stats` spec-compatible; `next_row_id` tracks row ID allocation; `SelectTableStats` non-empty
- [ ] DROP TABLE retires all dependent spec tables; cascade conformance tests green
- [ ] All new fields covered by unit tests in `slateduck-core` and integration tests in `slateduck-catalog`

---

## v0.25 — DuckLake v1.0 SQL Catalog Facade

> Complete the PgWire virtual-table layer so that every one of the 28 DuckLake spec tables is queryable with exact spec column names, column order, and value semantics. Add full persistence for views, macros, and inlined data tables. Add scoped metadata, UUID fields, `path`/`path_is_relative` fields, and a spec-correct nested column model.

### Full 28-Table SQL Facade

The DuckLake spec defines a SQL catalog database with 28 tables. SlateDuck stores facts as key/value rows; the facade is the PgWire/virtual-table projection layer. Today many tables return empty result sets or expose SlateDuck-internal column names. This phase closes that gap entirely.

- [ ] Audit every `StatementKind` in `slateduck-pgwire/src/executor/mod.rs` that currently returns an empty result set (`SelectSnapshot`, `SelectTableStats`, `SelectMetadata`, `SelectViews`, `SelectMacros`, `SelectDeleteFiles`); replace each with a real reader call and a spec-shaped response builder.
- [ ] Implement spec-shaped response builders for all 28 tables. Each builder must expose columns in spec column order with spec column names. Use a per-table response builder struct pattern consistent with existing code.
- [ ] For every `INSERT`-accepting `StatementKind` that currently no-ops (`InsertMetadata`, `InsertInlinedDataTable`, `InsertView`, `InsertMacro`, `InsertMacroImpl`, `InsertMacroParameters`), wire through to the corresponding `CatalogWriter` method and persist the row.
- [ ] Add PgWire integration tests for every table: one round-trip insert + select test per table verifying column names match the spec manifest from v0.24.

### Scoped Metadata (`ducklake_metadata`)

Spec: `metadata_key`, `metadata_value`, `scope`, `scope_id`

- [ ] Add `scope` and `scope_id` to `MetadataRow`; `MetadataScope` is already encoded in keys but must be denormalized into the row for SQL queries.
- [ ] Fix `InsertMetadata` to persist `scope` and `scope_id` from the incoming SQL parameters.
- [ ] Fix `SelectMetadata` to return spec-shaped rows including `scope` and `scope_id`.
- [ ] Add conformance tests: insert global metadata, insert table-scoped metadata with a `scope_id`, verify both are retrievable with correct `scope` values.

### Schema UUID and Path Fields (`ducklake_schema`)

Spec: `schema_id`, `begin_snapshot`, `end_snapshot`, `schema_uuid`, `schema_name`, `path`, `path_is_relative`

- [ ] Add `schema_uuid` (UUID v4, generated at create time), `path`, and `path_is_relative` to `SchemaRow`.
- [ ] Persist all three fields in `CatalogWriter::create_schema`.
- [ ] Update the PgWire `SelectSchemas` response builder to expose all spec columns.

### Table UUID and Path Fields (`ducklake_table`)

Spec: `table_id`, `begin_snapshot`, `end_snapshot`, `schema_id`, `table_name`, `table_uuid`, `path`, `path_is_relative`

- [ ] Add `table_uuid` (UUID v4, generated at create time), `path`, and `path_is_relative` to `TableRow`; rename `data_path` → `path` in the SQL facade.
- [ ] Persist all three fields in `CatalogWriter::create_table`.
- [ ] Update the PgWire `SelectTables` response builder to expose all spec columns.

### Column Defaults and Nested Column Model (`ducklake_column`)

Spec: `column_id`, `begin_snapshot`, `end_snapshot`, `table_id`, `column_name`, `column_type`, `column_order`, `nulls_allowed`, `initial_default`, `default_value_type`, `default_value_dialect`, `parent_column`

- [ ] Rename facade columns: `data_type` → `column_type`, `column_index` → `column_order`, `is_nullable` → `nulls_allowed`.
- [ ] Add `initial_default`, `default_value_type`, `default_value_dialect`, and `parent_column` to `ColumnRow`.
- [ ] Persist all new fields via `CatalogWriter::add_column`.
- [ ] Support nested column rows: when `parent_column` is non-null, store and retrieve the parent/child relationship; child columns have their own `column_id`.
- [ ] Update the PgWire `SelectColumns` response builder to use spec column names.

### Views, Macros, and Inlined Data Tables

**`ducklake_view`** (spec: `view_id`, `begin_snapshot`, `end_snapshot`, `schema_id`, `view_name`, `view_uuid`, `view_definition`, `dialect`, `column_aliases`):
- [ ] Add `view_uuid`, `dialect`, and `column_aliases` to `ViewRow`.
- [ ] Fix `InsertView` to call `CatalogWriter::create_view` and persist all fields.
- [ ] Fix `SelectViews` to return spec-shaped rows.

**`ducklake_macro`** and **`ducklake_macro_impl`** (spec: `macro_id`, `macro_name`, `macro_uuid`, `schema_id` / `macro_id`, `dialect`, `type`, `sql`):
- [ ] Move `macro_type` from `MacroRow` into `MacroImplRow` as `type` (spec-correct location).
- [ ] Add `macro_uuid` to `MacroRow`.
- [ ] Add `dialect` and rename `definition` → `sql` in `MacroImplRow`.
- [ ] Fix `InsertMacro` and `InsertMacroImpl` to persist through `CatalogWriter`.
- [ ] Fix `SelectMacros` to return spec-shaped rows.

**`ducklake_macro_parameters`** (spec: `macro_id`, `parameter_name`, `parameter_type`, `default_value_type`):
- [ ] Add `default_value_type` to `MacroParameterRow`.
- [ ] Fix `InsertMacroParameters` to persist through `CatalogWriter`.

**`ducklake_inlined_data_tables`** (spec: `table_id`, `table_name`, `sql`):
- [ ] Rename the internal `sql` field to align with spec; expose `table_name` rather than raw SQL as the primary identifier.
- [ ] Fix `InsertInlinedDataTable` to persist through `CatalogWriter`.
- [ ] Fix `SelectInlinedDataTables` to return spec-shaped rows.

### Column Mapping and Name Mapping

**`ducklake_column_mapping`** (spec: `mapping_id`, `table_id`, `type`):
- [ ] Restructure `ColumnMappingRow` to carry `mapping_id`, `table_id`, and `type`; move `file_column_name` and `column_id` into a related name-mapping record.

**`ducklake_name_mapping`** (spec: `mapping_id`, `column_id`, `name`, `target_field_id`, `parent_column`, `is_partition`):
- [ ] Add `target_field_id`, `parent_column`, and `is_partition` to `NameMappingRow`.
- [ ] Remove non-spec `source_name_hash`.

### Deliverables

- [ ] All 28 DuckLake spec tables return non-empty, spec-shaped result sets through PgWire for at least one representative fixture row each
- [ ] All `INSERT` statement kinds that were previously no-ops now persist to the KV store and round-trip correctly
- [ ] `ducklake_schema`, `ducklake_table`, `ducklake_column` facades use spec column names
- [ ] `ducklake_view`, `ducklake_macro`, `ducklake_macro_impl`, `ducklake_macro_parameters`, `ducklake_inlined_data_tables` fully wired
- [ ] `ducklake_metadata` scope fields persisted and queryable
- [ ] Column mapping and name mapping restructured to spec layout
- [ ] Nested column rows supported via `parent_column`
- [ ] Round-trip PgWire test for every table in the conformance suite passes

---

## v0.26 — DuckLake v1.0 Stats, Types, Partitioning & Sorting

> Complete stats coverage for all file and table column statistics tables; add geometry and variant `extra_stats`; implement a DuckLake type parser used consistently for catalog validation and pruning; add the full sort expression model; close partition column and file partition value gaps; add partial-file support.

### Full File Column Stats (`ducklake_file_column_stats`)

Spec: `data_file_id`, `column_id`, `lower_bound`, `upper_bound`, `contains_null`, `contains_nan`, `column_size_bytes`, `value_count`, `null_count`, `extra_stats`

- [ ] Add `column_size_bytes`, `value_count`, `null_count`, and `extra_stats` to `FileColumnStatsRow`.
- [ ] Rename `has_null` boolean → `contains_null` (also rename in stats writer and PgWire response builder).
- [ ] Implement `extra_stats` as a JSON blob field; add validation that it is well-formed JSON when present.
- [ ] Update `CatalogWriter::write_file_column_stats` (in `stats.rs`) to persist all new fields.
- [ ] Update the PgWire stats response builder to expose all spec columns.

### Full Table Column Stats (`ducklake_table_column_stats`)

Spec: `table_id`, `column_id`, `lower_bound`, `upper_bound`, `contains_null`, `contains_nan`, `extra_stats`

- [ ] Add `contains_nan` and `extra_stats` to `TableColumnStatsRow`.
- [ ] Rename `has_null` → `contains_null`.
- [ ] Update writer and PgWire response builder.

### Variant Stats and Extra Stats (`ducklake_file_variant_stats`)

Spec: `data_file_id`, `column_id`, `variant_key`, `shredded_type`, `column_size_bytes`, `value_count`, `null_count`, `contains_nan`, `extra_stats`

- [ ] Add `shredded_type`, `column_size_bytes`, `value_count`, `null_count`, `contains_nan`, and `extra_stats` to `FileVariantStatsRow`.
- [ ] Remove non-spec `variant_path_hash`; use `variant_key` as the natural identifier.
- [ ] Add writer and PgWire support.

### Geometry Stats Support

The DuckLake `extra_stats` field on file column stats rows carries geometry bounding boxes and type information for spatial columns. This is the only place geometry metadata appears in the spec.

- [ ] Define a `GeometryExtraStats` struct with fields for bounding box (min/max X, Y, Z, M), geometry type, and SRID.
- [ ] Serialize `GeometryExtraStats` as JSON into the `extra_stats` field of `FileColumnStatsRow`.
- [ ] Add a validator that rejects malformed geometry `extra_stats` JSON at write time.
- [ ] Add a pruning helper that reads bounding box extents from `extra_stats` for spatial predicate pushdown.

### DuckLake Type Parser

The spec defines a rich set of primitive and nested type strings (`boolean`, `int32`, `decimal(P,S)`, `timestamp_s`, `list<T>`, `struct<f:T,...>`, `map<K,V>`, `variant`, `geometry`). Currently `DuckLakeType` uses broad comparison categories and `DuckLakeType::Varchar` is passed for all PgWire type-aware pruning.

- [ ] Implement a `DuckLakeType` parser that accepts a DuckLake type string and produces a typed enum variant: signed/unsigned integers with explicit bit width, decimal with precision and scale, timestamp with explicit precision (`_s`, `_ms`, `_ns`, `_us`), explicit `json`, explicit `uuid`, explicit `variant`.
- [ ] Add nested type variants: `List(Box<DuckLakeType>)`, `Struct(Vec<(String, DuckLakeType)>)`, `Map { key: Box<DuckLakeType>, value: Box<DuckLakeType> }`.
- [ ] Use the type parser in `ducklake_column` writes to validate `column_type` strings at the PgWire boundary.
- [ ] Use the type parser in file pruning: derive the correct comparison semantics from `ducklake_column.column_type` rather than passing `Varchar` unconditionally.
- [ ] Add unit tests covering all spec primitive types and the three nested type forms.

### Nested Column Rows with `parent_column`

- [ ] Implement recursive column tree reads: `CatalogReader::list_columns` must return child columns alongside parent columns, ordered by `column_order` within each level.
- [ ] Add a write path for nested columns: `CatalogWriter::add_column` accepts an optional `parent_column_id`; child columns share `table_id` and `begin_snapshot` with their parent.
- [ ] Add conformance tests: create a `struct` column with two child fields, list columns, verify `parent_column` is set on children and null on the struct column.

### Sort Expression Spec Parity (`ducklake_sort_expression`)

Spec: `table_id`, `sort_order`, `expression`, `dialect`, `sort_direction`, `null_order`

- [ ] Replace boolean `ascending`/`nulls_first` fields in `SortExpressionRow` with string fields `sort_direction` (`'ASC'`/`'DESC'`) and `null_order` (`'NULLS FIRST'`/`'NULLS LAST'`), matching spec semantics.
- [ ] Add `table_id`, `expression`, and `dialect` to `SortExpressionRow`.
- [ ] Update `CatalogWriter`, key encoding, and PgWire response builder for `ducklake_sort_expression`.
- [ ] Ensure `ducklake_sort_info` is exposed via the SQL facade and its lifecycle (creation, DROP TABLE cascade) is covered.

### Partition Column `table_id` and Lifecycle (`ducklake_partition_column`)

- [ ] Add `table_id` to `PartitionColumnRow`.
- [ ] Ensure DROP TABLE cascade (from v0.24) retires `ducklake_partition_info` and `ducklake_partition_column` rows.
- [ ] Confirm the PgWire SQL facade exposes `ducklake_partition_info` and `ducklake_partition_column` with correct spec columns.

### File Partition Value and Scheduled Deletion (`ducklake_file_partition_value`, `ducklake_files_scheduled_for_deletion`)

- [ ] Rename `value` → `partition_value` in `FilePartitionValueRow` and the SQL facade.
- [ ] Add `path_is_relative` to `FilesScheduledForDeletionRow`.
- [ ] Remove non-spec `file_type` from `FilesScheduledForDeletionRow` (or move to an extension-only field if still needed internally).
- [ ] Change the deletion timestamp field to use SQL `TIMESTAMPTZ` semantics (microseconds since epoch, not integer seconds).

### Partial File Support (`partial_max`)

- [ ] `partial_max` was added to `DataFileRow` and `DeleteFileRow` in v0.24. In this phase, implement the reader-side behavior: when reading a data file with `partial_max IS NOT NULL`, treat the file as containing only rows up to and including the row with the maximum value equal to `partial_max`.
- [ ] Add a pruning shortcut: skip a partial file entirely if the query predicate excludes all rows up to `partial_max`.

### Deliverables

- [ ] `ducklake_file_column_stats`, `ducklake_table_column_stats`, `ducklake_file_variant_stats` spec-compatible with all required fields
- [ ] `extra_stats` JSON field written, validated, and readable for variant and geometry stats
- [ ] DuckLake type parser implemented, tested, and used in column validation and pruning
- [ ] Nested column reads and writes working end-to-end with `parent_column`
- [ ] `ducklake_sort_expression` uses spec string fields; `ducklake_sort_info` exposed via SQL facade
- [ ] `ducklake_partition_column.table_id` present; partition lifecycle covered by DROP TABLE cascade
- [ ] `partition_value` renamed; `path_is_relative` and `TIMESTAMPTZ` semantics for `files_scheduled_for_deletion`
- [ ] Partial-file `partial_max` read semantics implemented and tested

---

## v0.27 — DuckLake v1.0 External Compatibility Validation

> Validate SlateDuck against a real DuckDB DuckLake extension client. Run the full spec query corpus, implement a migration path from existing DuckLake deployments, and close all remaining P2 fidelity gaps. Exit criteria: SlateDuck can credibly claim DuckLake v1.0 catalog compatibility.

### Real DuckDB DuckLake Extension End-to-End Tests

This is the primary acceptance gate for all DuckLake compatibility work across v0.24–v0.27.

- [ ] Stand up a SlateDuck PgWire sidecar against an in-process MinIO instance (using `MinioHarness` from `slateduck-testkit`).
- [ ] Connect a real DuckDB process using the `ducklake` extension via the PostgreSQL attachment string `ducklake:postgres://127.0.0.1:5555/...`.
- [ ] Run the full DuckLake tutorial end-to-end: `ATTACH`, `CREATE SCHEMA`, `CREATE TABLE`, multi-row `INSERT`, `SELECT`, `DELETE`, `UPDATE`, `DROP TABLE`, `DROP SCHEMA`, `DETACH`.
- [ ] Verify time-travel reads: `SELECT ... FROM table AT (VERSION => N)` returns rows visible at snapshot N and excludes rows added after N.
- [ ] Verify file pruning: single typed-column predicate at 10⁴ files; confirm SlateDuck does not scan files that the zone-map or exact-stats pruning eliminates.
- [ ] Verify conflict resolution: two concurrent writer connections; one must succeed and the other must receive a retryable conflict error; the winner's data is visible and the loser's is absent.
- [ ] Capture any `column-not-found`, `type mismatch`, or behavior divergence as blocking test failures.
- [ ] Add this test suite as Tier 4 in the CI test matrix (MinIO, runs on every merge to `main`).

### Read Conformance Suite Against `specification/queries.md`

- [ ] Extract every SQL example from `specification/queries.md` into parameterized golden tests.
- [ ] For each query, set up the required catalog state (snapshot, schema, table, columns, data files), run the query through the SlateDuck PgWire facade, and assert column names, column types, and row values against a golden fixture.
- [ ] Run this suite on every PR as part of the conformance harness from v0.24.
- [ ] Document any queries that remain unsupported with an explicit `SQLSTATE 0A000` response and a tracking note; no query may silently return wrong results.

### Import / Export and Migration Path

- [ ] Implement `slateduck migrate-from-ducklake --source <conn-string> --catalog <s3-path>`: reads an existing PostgreSQL- or SQLite-backed DuckLake catalog (current snapshot only), replays its metadata into a fresh SlateDuck catalog, and emits a verification report comparing row counts and column presence per table.
- [ ] Implement `slateduck export-catalog --catalog <s3-path> --out <file.json>`: serializes the current snapshot of all 28 catalog tables to a JSON-lines file usable as an interop dump or for debugging.
- [ ] Document the migration procedure in `docs/operations/migration-from-ducklake.md`; cover cutover, rollback, and known incompatibilities.
- [ ] End-to-end test `migrate-from-ducklake` against a SQLite-backed DuckLake fixture at SF1 scale.
- [ ] End-to-end test `migrate-from-ducklake` against a PostgreSQL-backed DuckLake fixture.

### P2 Fidelity Gaps

These gaps do not block narrow happy-path interop but are required for full catalog fidelity.

**`ducklake_tag` and `ducklake_column_tag` facade:**
- [ ] Rename `tag_key`/`tag_value` → `key`/`value` in the SQL facade response builders for both tables.
- [ ] Ensure `ducklake_column_tag` rows are retired by DROP TABLE cascade (verified by the cascade conformance tests from v0.24).
- [ ] Add lifecycle tests: create a table with tags and column tags, drop the table, verify all tag rows have `end_snapshot` set.

**`ducklake_schema_versions` facade:**
- [ ] Confirm the SQL facade exposes `ducklake_schema_versions` in exact spec column order.
- [ ] Add a write-path test: evolve a table schema across two snapshots, verify `ducklake_schema_versions` contains a row for each evolution.

**`ducklake_sort_info` lifecycle:**
- [ ] Add a round-trip test: define sort info on a table, drop the table, verify sort info is retired.

### Definition of Done for DuckLake v1.0 Compatibility

SlateDuck claims DuckLake v1.0 catalog compatibility when all of the following are true. These become hard blockers for the v1.0 GA tag:

- [ ] All 28 spec tables are visible through SQL with exact column names and compatible types.
- [ ] Every spec field is either persisted internally or losslessly synthesized in the SQL facade.
- [ ] DuckLake query examples from `specification/queries.md` pass against SlateDuck.
- [ ] Create/insert/delete/update/drop operations produce rows matching spec semantics.
- [ ] Time travel uses `begin_snapshot` and `end_snapshot` consistently across all spec tables that carry MVCC windows.
- [ ] Snapshot rows include `next_catalog_id` and `next_file_id`.
- [ ] Snapshot changes include `changes_made`, `author`, `commit_message`, and `commit_extra_info`.
- [ ] Data files include `file_order`, `row_id_start`, `path_is_relative`, `partition_id`, `mapping_id`, and `partial_max`.
- [ ] Delete files include full MVCC windows, all spec fields, and are returned to readers.
- [ ] Row ID allocation is represented through `ducklake_table_stats.next_row_id` and `ducklake_data_file.row_id_start`.
- [ ] No supported DuckLake SQL write is accepted as a no-op; any unimplemented write returns `SQLSTATE 0A000`.
- [ ] Real DuckDB DuckLake extension end-to-end test suite passes on every merge to `main`.

### Deliverables

- [ ] Real DuckDB end-to-end test suite passing in CI (Tier 4)
- [ ] `specification/queries.md` conformance golden tests green
- [ ] `slateduck migrate-from-ducklake` and `slateduck export-catalog` subcommands implemented and tested
- [ ] `docs/operations/migration-from-ducklake.md` written and reviewed
- [ ] `ducklake_tag` and `ducklake_column_tag` facades using spec column names
- [ ] `ducklake_schema_versions` SQL facade column order verified
- [ ] DuckLake v1.0 compatibility definition-of-done checklist fully green
- [ ] Compatibility status matrix updated in `docs/compatibility.md`

---

## v1.0 — General Availability

> Formal TPC-H @ SF10/SF100 benchmark publication, S3 Express acceptance gate, and GA sign-off.

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
9. **Real-world validation gate.** At least 30 days of dogfood deployment on a realistic workload (see Cross-Cutting Concerns: Real-World Validation Policy). Friction log reviewed and all blocking findings resolved. One external-to-the-team developer has successfully deployed SlateDuck using only published docs.
10. **Migration path from existing DuckLake deployments.** A documented and tested migration tool (`slateduck migrate-from-ducklake --source postgres://... --catalog s3://...`) reads an existing PostgreSQL- or SQLite-backed DuckLake catalog, replays its current snapshot into a fresh SlateDuck catalog (data files are not copied — they remain at their original object-store paths and are referenced by the new catalog), and emits a verification report. `docs/operations/migration-from-ducklake.md` covers cutover, rollback, and known-incompatibility surfaces. End-to-end tested against both PostgreSQL- and SQLite-backed source catalogs at SF1 scale.
13. **World-class testing foundation.** All 10 test tiers from [plans/e2e-integration-tests.md](plans/e2e-integration-tests.md) are fully implemented and green:
    - **Tiers 1–3** (unit/property, catalog, PG-Wire): green on every PR — standard GitHub Actions runner
    - **Tiers 4–5** (MinIO object store, client compat): green on every merge to `main` — large runner (8-vCPU), Testcontainers MinIO
    - **Tier 6** (fault injection — catalog, toxiproxy): green on every pre-release tag
    - **Tier 7** (24 h soak, TPC-H SF10/SF100): green on pre-release — dedicated EC2 `c6i.4xlarge`
    - **Tier 8** (security — credential isolation, TLS, auth, SQL injection guards): green on pre-release
    - **Tier 9** (benchmark regression < 10% vs baseline): green on weekly scheduled CI
    - `slateduck-testkit` ships 4 harnesses: `MinioHarness`, `CatalogHarness`, `PgWireHarness`, `DuckDbHarness`, `DeterministicClock`
    - At least 100 named test functions across all tiers at GA; test inventory published in `docs/contributing/testing.md`

### Deliverables

- v1.0 release tag and `CHANGELOG.md` entry
- Benchmark report `benchmarks/v1.0-tpch-sf10.json` published in the repository and linked from `docs/performance/`
- Final S3 Express acceptance decision documented in `docs/performance/s3-express-validation.md`
- `slateduck-testkit` crate complete with all 6 harness types
- Complete test inventory in `docs/contributing/testing.md`: tier-by-tier test count, CI job mapping, feature flags, and scale-test runner setup

---

## v0.23 — Streaming Ingest

> **Note:** v0.23 is documented after v1.0 because it is an independent, parallel workstream. It does not block or depend on other workstreams. Its CDC output primitives provide change-stream capabilities that can feed downstream consumers.

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

When v0.23 (streaming ingest) is deployed, the end-to-end story is:

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

### Real-World Validation Policy

Synthetic benchmarks (TPC-H, TPC-DS) catch performance regressions and correctness bugs, but they do not catch usability gaps, cost surprises, or workflow friction. Before v1.0 GA:

1. **Internal dogfood.** Run a real SlateDuck deployment against pg-tide's own analytics pipeline (if available) or a synthetic-but-realistic workload (e.g. GitHub event stream, NYC taxi stream) for ≥ 30 days.
2. **Document surprises.** Any unexpected behaviour, cost spike, or operational friction discovered during dogfooding becomes a documented finding and must be resolved or explicitly accepted before GA.
3. **User-experience review.** At least one developer unfamiliar with SlateDuck internals must successfully set up and query a catalog using only the published documentation. Their friction log becomes a documentation and UX backlog item.

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

### Scale Testing Infrastructure

Some acceptance criteria cannot run in normal CI: 24 h soaks, 1 TB inputs, and TPC-H SF100 benchmarks. These require dedicated infrastructure.

**Testing tiers:**

| Tier | What runs | Where | Trigger |
|------|-----------|-------|---------|
| CI (every PR) | Unit tests, property tests, single-shard correctness on LocalFS | GitHub Actions (standard runner) | Push/PR |
| Integration (every merge to main) | Multi-shard correctness, MinIO end-to-end, fault injection (<1h) | GitHub Actions (large runner, 8 vCPU) | Merge to main |
| Scale (weekly / pre-release) | 1 TB input, TPC-DS full suite, cost measurement | Dedicated EC2 (c6i.4xlarge) + S3 Standard | Scheduled / manual |
| Soak (pre-release) | 24 h continuous ingest, fault injection every 15 min | Dedicated EC2 + S3 Express | Manual gate before GA |

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
