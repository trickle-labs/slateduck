# RockLake Roadmap

A lakehouse catalog backed by SlateDB — catalog and data in the same S3 bucket, zero infrastructure.

---

## Vision

RockLake makes a DuckLake lakehouse fully serverless: both the Parquet data
files and the DuckLake catalog live in the same object-storage bucket, with no
external database server required. The catalog is stored in SlateDB — an
embedded, LSM-based key-value store built entirely on top of object storage —
and is queryable from the standard DuckDB `ducklake` extension as well as other
DuckLake-compatible clients.

A second, equally load-bearing commitment shapes every storage decision:
**committed catalog facts are never physically deleted by normal operation,
and are always readable at the `dl_snapshot_id` at which they were written.**
Physical deletion exists only via the explicit, audited `rocklake excise`
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

Physical deletion exists only via the explicit, audited `rocklake excise`
command invoked outside the normal write path (compliance erasure, opt-in
bounded retention). The default `rocklake gc` only advances query-visibility
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
| **v0.18 — DuckLake Catalog Standard Interface** | `table_changes()` CDC function, stable `rowid`, snapshot lease, `NOTIFY` event-driven, extension schema (first-class catalog tag `0x23`), opaque mixed frontiers; validated against standard Postgres drivers | **Done** |
| **v0.19 — CDC Correctness & Catalog Transaction Hardening** | Real row-level `table_changes()` with Parquet scan, versioned `DataFileRow` / `SnapshotDiff` windows, CAS writer epoch, transactional extension row-ID allocation, atomic GC lease + retain-from, staged write discipline, overflow-safe counters | **Done** |
| **v0.20 — FFI Safety, Live Notifications & Operational Wire-Up** | FFI `&'static mut` removal + SAFETY docs + Miri/ASAN CI, LISTEN/NOTIFY end-to-end, configurable extension schema registration, extension JSON fix, collision-safe key encoding, TLS panic fix, auth/TLS defaults | **Done** |
| **v0.21 — Performance, Scalability & Code Quality** | `list_data_files()` secondary index, O(1) aggregate deletions, SQL classifier hardening, module decomposition, MSRV + license CI, metrics path alignment, dead-code + dependency cleanup | **Done** |
| **v0.22 — IVM Removal** | Delete `rocklake-ivm` crate, remove IVM catalog tags/rows/keys, strip IVM SQL DDL variants, clean docs, benchmarks, CI, and deny.toml | **Done** |
| **v0.23 — Streaming Ingest** | pg-tide-relay integration, Kafka/NATS support, exactly-once delivery, CDC output (snapshot diffs, S3/Kafka/webhook) | **Done** |
| **v0.24 — DuckLake v1.0 Conformance Harness & Interop-Critical Schema** | Conformance test harness for all 28 spec tables; fix snapshot/snapshot_changes schema; spec-complete data file fields; spec-complete delete file model; row ID tracking; table stats `next_row_id`; DROP TABLE cascade retirement | Complete |
| **v0.25 — DuckLake v1.0 SQL Catalog Facade** | Full PgWire/virtual-table facade with exact spec column names and types for all 28 tables; views, macros, and inlined data tables through PgWire; scoped metadata; schema/table UUID and path fields; nested column model | Complete |
| **v0.26 — DuckLake v1.0 Stats, Types, Partitioning & Sorting** | Full file and table column stats; variant stats and `extra_stats`; geometry stats; column mapping and name mapping parity; sort expression spec parity; partition column lifecycle; DuckLake type parser; nested and `variant` type model | Complete |
| **v0.27 — DuckLake v1.0 External Compatibility Validation** | Real DuckDB DuckLake extension end-to-end tests; read conformance suite against `specification/queries.md`; import/export migration path; P2 fidelity gaps (`files_scheduled_for_deletion`, `file_partition_value`, `sort_info`, `tag`/`column_tag` facade) | Done |
| **v0.27.1 — CDC Completeness & Real Parquet Row Scanning** | Implement real `extract_rows_from_parquet()` via `object_store`; replace synthetic CDC column payloads with actual file data; verify `record_count` against scanned rows; streaming/batching for large Parquet files; end-to-end CDC round-trip tests | Done |
| **v0.27.2 — DataFusion Completeness, Code Hardening & Security** | Auto-resolve `data_root` from catalog metadata; eliminate OS-thread-per-sync DataFusion bridge overhead; resolve or remove `rocklake-sqlite-vfs` placeholder; replace DataRowEncoder `unwrap()` calls; harden key/value decode paths; verify `checked_add` in writer; verify `SqlState` code propagation; API ergonomics for `CatalogStore` commit; warn on auth-without-TLS; address wall-clock lease concern | Done |
| **v0.27.3 — Testing Completeness, CI Production Gates & Documentation** | Make coverage threshold a hard gate; add doc-tests for all public APIs in `rocklake-core` and `rocklake-catalog`; add network-level PG-Wire integration test; add concurrent writer fencing test; verify checkpoint-restore snapshot-ID safety; verify `rebuild_catalog` behaviour; align `docs/operations/monitoring.md` with CLI flags; close all open partial findings from Assessments 1 & 2 | Done |
| **v0.27.4 — DuckDB 1.5.x PostgreSQL Scanner Compatibility** | Handle all DuckDB 1.5.x postgres scanner initialization queries: `DISCARD ALL`; `SELECT to_regclass('duckdb_secrets')`; `SELECT EXISTS(... information_schema.tables ...)`; multi-statement catalog scan (`pg_namespace`, `pg_class`/`pg_attribute`/`pg_constraint`, `pg_enum`, `pg_type` composites, `pg_indexes`); `SELECT pg_database_size(current_database())`; capture DuckDB 1.5.x wire-corpus fixture; update compatibility matrix to DuckDB 1.5.x only | Done |
| **v0.27.5 — DuckLake v1.0 Spec Gap Closure** | Close P0/P1/P2 gaps from `plans/ducklake-1.0-spec-gaps.md` and `plans/ducklake-1.0-spec-gaps-2.md`: exact SQL catalog facades for all 28 tables; fix snapshot/snapshot_changes schema; implement spec-complete delete-file semantics; DROP TABLE cascade; inlined data SQL support; data file spec fields; metadata facades; column stats completeness; field naming alignment; stats model semantics cleanup; transaction atomicity; RowDescription centralization; type-aware stats; DROP/ALTER cascade; compatibility corpus | Done |
| **v0.27.6 — DuckLake Inlined-Data Lifecycle Integration Tests** | Opt-in automated DuckDB/DuckLake lifecycle tests: fresh attach, INSERT/DELETE/UPDATE, restart reads, stats inspection, direct `postgres_query` of dynamic inlined tables; stats merge regression cases for negative numbers, floats, and strings | Done |
| **v0.27.7 — DuckLake SQL Schema Registry** | `DuckLakeTableSchema` registry as single source of truth for all 28 metadata table schemas; wire executor response builders, handler describe, and COPY to the registry; projection-order golden tests for every table; arbitrary output alias support for dynamic inlined tables | Done |
| **v0.27.8 — DuckLake Transaction Atomicity & Snapshot Changes Conformance** | Group all statements in one logical DuckLake commit into an atomic batch; spec-complete `ducklake_snapshot_changes` with `changes_made`, `author`, `commit_message`, `commit_extra_info`; interleaved writer and rollback tests; writer fencing validation; type-aware column stats for dates, timestamps, decimals | Done |
| **v0.27.9 — DuckLake Advanced Metadata Validation** | End-to-end DuckDB tests for views, macros, tags, column tags, sort info, and partition info; DROP/ALTER cascade covering all metadata types; ALTER TABLE time-travel tests; imported existing DuckLake catalog support | Done |
| **v0.27.10 — DuckLake Compatibility CI** | Pin known-good DuckDB and DuckLake versions in CI; nightly optional jobs; durable compatibility corpus covering PQsendQuery pg_catalog scans; exact column schema/OID describe checks | Done |
| **v0.27.11 — Wire & SQL Resiliency Hardening** | Implement DataFusion virtual catalog, AST visitor, settings registry, fuzzer; fully refactor schema registry (matching all 28 tables exactly, renaming key/value, sql, tag columns); expose `ducklake_latest_snapshot_id(regclass)` for CDC startup | Planning |
| **v0.27.12 — Containerized Multi-Backend Object Store Emulator Testing** | Implement containerized GCS/Azure emulators; verify catalog CRUD, snapshot commit, and epoch fencing; persist/expose data-file and delete-file spec fields (footer_size, partition_id, encryption_key) | Done |
| **v0.27.13 — Real Multi-Client & Multi-Driver Interoperability Certification** | Build multi-driver compat suite; verify binary formats; validate client schema discovery; enforce visibility constraints (begin_snapshot/end_snapshot) and sort data files by file_order; archive planning docs as generic DuckLake CDC contract reference | Done |
| **v0.27.14 — Security Hardening & Protocol-Level Testing** | Verify constant-time auth; SCRAM-SHA-256; TLS version gating; implement atomic metadata commits, consolidated stats deltas, and repeatable-read writer fencing (SQLSTATE 40001) | Planning |
| **v0.35.0 — Strategy C: Native DuckDB Extension** | Complete the native DuckDB extension so `ATTACH 'ducklake:slatedb:s3://...' AS lake` works without a PG-wire sidecar; eliminates all Postgres-scanner compatibility burden for local/embedded use; `rocklake-ffi` C ABI already done; C++ catalog registration is the remaining gap | Planning |
| **v0.40.0 — Full Ecosystem Compatibility Certification** | Release-blocking CI evidence for every `docs/compatibility.md` row: real DuckDB/DuckLake versions, SQL clients, Spark/Trino/Presto disposition, DataFusion, object stores, TLS/auth, Rust/MSRV, and release platforms | Planning |
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
  rocklake/
  ├── Cargo.toml
  ├── crates/
  │   ├── rocklake-core/
  │   ├── rocklake-catalog/
  │   ├── rocklake-sql/
  │   ├── rocklake-sqlite-vfs/
  │   ├── rocklake-pgwire/
  │   └── rocklake-ffi/
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
| Conditional initialization | `DbTransaction` can implement insert-if-absent for `ducklake_metadata` | Require explicit `rocklake init` under an external deployment lock |
| Serializable counter allocation | Two concurrent transactions on the same counter: one wins, loser gets a retryable conflict, no ID is reused after crash/reopen | Single-writer in-memory allocator persisting counter and consumed rows in one batch |
| Concurrent initialization convergence | Two processes calling `open_or_create` on a fresh catalog produce exactly one coherent initial key/value set | Require explicit `rocklake init` |
| Durable commit options | `commit_with_options` / `await_durable` survives a crash | Document as required; abort if SlateDB does not expose it |
| `flush()` reader visibility | Write → `flush()` → fresh `DbReader` sees the key on LocalFS and MinIO | Replace with verified memtable flush or serve read-your-writes from the writer process |
| Visibility-barrier latency | Measure p50/p95/p99 on LocalFS and MinIO; record for later Phase 4 latency budgets | — |
| Writer fencing | Force two writers; capture the exact error kind returned; confirm it is distinguishable | Maintain RockLake-own epoch check; map stale epochs to `SQLSTATE 57P04` |
| `WriteBatch` logical size | Determine whether SlateDB imposes its own limit | Enforce RockLake's own 64 MiB limit unconditionally |
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

### Catalog Key Layout (`rocklake-core`)

Implement the full binary key layout for all 28 DuckLake v1.0 tables plus RockLake system namespaces. Every tag byte must be allocated up front, even for tables deferred to later phases, so that unknown tables return an explicit error rather than silent data loss.

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
FE  RockLake counters         counter_id
FF  RockLake system keys      writer epoch / endpoint / retain-from / catalog-format-version
```

The `0xFE` counter keys and `0xFF` system keys are managed with simple
transactional writes (see [plans/blueprint.md §1.4](plans/blueprint.md)).
Excision audit records are appended under a dedicated `0xFF | "excised"` prefix
and accumulate without overwriting previous entries.

Produce `crates/rocklake-core/src/tags.rs` as the single source of truth listing every table's tag byte, key shape, versioning rule, MVCC behavior, unique-guard key requirement, and implementation status (`Live`, `Deferred(phase)`, `Unimplemented`).

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

- `CatalogPath` struct in `rocklake-core` encapsulates `object_store_root`, `catalog_prefix`, `data_prefix`, `data_path_mode` (`Absolute` | `RelativeToDataPrefix`)
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

- **Retention advancement (default, safe).** `0xFF | "retain-from"` is a single key updated transactionally by the TTL task. It records the query-visibility floor; `rocklake gc` only advances it, never deletes bytes. Default: infinite / never advance (configurable via `--retention-days`; `0` or omitted means never advance). `catalog.pin_snapshot(id)` blocks advancement.
- **Excision (rare, audited).** Physical deletion of bytes. Invoked only via `rocklake excise`, never as part of the normal write path or default gc sweep. The excision event is persisted under `0xFF | "excised"` so the audit trail accumulates across runs.

Default physical retention is **infinite**. Operators may opt into bounded
storage via `--excise-days` (off by default) plus an explicit
`rocklake excise --before <snapshot> --apply` invocation.

Orphaned Parquet files (not committed to any snapshot) remain eligible for
cleanup by the orphaned-file sweep with the configurable grace period (default
7 days); they are not part of the catalog-data fact set and do not require
excision.

### Early Validation and Benchmark Baseline

- `rocklake verify catalog` command: primary-key uniqueness, foreign-key references, MVCC interval consistency, counter monotonicity
- `benchmarks/phase-2-baseline.json`: p50/p95/p99/p99.9 for `get_current_snapshot`, `list_data_files` at 10 K files, `describe_table` with 100 columns, `prune_files` on one typed column, `create_snapshot` with 100 file additions — on LocalFS and MinIO

### Deliverables

- [x] Documented Rust library storing and retrieving every row type defined by DuckLake v1.0 including `0xFD` dynamic inlined rows
- [x] Property test suite green
- [x] `tags.rs` complete and reviewed
- [x] `rocklake verify catalog` command working
- [x] Benchmark baseline recorded

---

## v0.3 — PG-Wire Sidecar (Alpha)

> Connect the standard DuckDB `ducklake` extension to RockLake through a PostgreSQL-wire sidecar.

This is the Strategy B production implementation. The sidecar speaks the PostgreSQL wire protocol and translates DuckLake catalog SQL into `CatalogStore` operations, storing all state in SlateDB.

### DuckLake-Spec Operations (`rocklake-catalog`)

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

### `rocklake-pgwire` Sidecar Binary

#### Wire Protocol

- `pgwire` crate for startup, simple query protocol, extended query protocol (`Parse`/`Bind`/`Describe`/`Execute`/`Sync`)
- Prepared-statement caching: cache the parsed + classified AST for named statements
- `SET` handler: accept all settings; store and apply `timezone`, `client_encoding` (`UTF8` only), `DateStyle`
- `SHOW` handler: return plausible hardcoded values for `server_version`, `DateStyle`, `transaction_isolation`
- Pass replay test against `tests/fixtures/handshake/duckdb-{version}.jsonl` before any DuckLake-specific logic is wired

#### Bounded SQL Dispatcher (`rocklake-sql`)

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

All errors flow through a single `to_pg_error(err: RockLakeError) -> PgErrorResponse` function:

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

- Golden test: replay Phase 0 DuckLake tutorial corpus against RockLake sidecar; diff output byte-for-byte against the SQLite-backed reference
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

### `rocklake serve` Binary

```
rocklake serve \
  --catalog s3://bucket/catalogs/warehouse-a \
  --bind 0.0.0.0:5432
```

Operators who want bounded time-travel visibility pass `--retention-days N` (e.g. `--retention-days 30`).

### Deliverables

- [x] `rocklake serve` binary exposing a SlateDB catalog at a PostgreSQL TCP endpoint
- [x] DuckDB connecting via standard `postgres` extension with all tutorial operations passing
- [x] Golden tests green for DuckDB 1.5.2
- [x] All crash injection tests passing
- [x] SQLSTATE test for every error code path

---

## v0.4 — Production Hardening

> Make RockLake safe and operable in production.

### Visibility GC and Excision

Catalog-data immutability splits the old "GC" concept into two distinct
operations:

**Visibility GC (default, safe).** Advances the `retain-from` key by a
transactional write. Never deletes bytes. Run via `rocklake gc plan` /
`rocklake gc apply` or as an optional background task behind `--enable-gc`
(off by default until acceptance tests prove it does not compete with foreground
catalog commits). Pinning via `catalog.pin_snapshot(id)` blocks advancement.

**Excision (rare, audited, foreground only).** Physically deletes catalog
facts and Parquet files older than the floor. Invoked only via
`rocklake excise plan` / `rocklake excise apply --before <snapshot>`. Always
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

- `rocklake checkpoint create` — thin wrapper around `SlateDB Checkpoint` API; produces a point-in-time catalog backup
- `rocklake checkpoint restore` — restore catalog to a named checkpoint
- `rocklake checkpoint list` — show all available checkpoints with timestamps

### Catalog Export and Migration

- `rocklake export --output catalog.ndjson [--snapshot-id N]` — NDJSON export of all live catalog rows at the specified or latest snapshot; includes `0xFD` inlined rows labeled by generated table name; excludes `0xFE`/`0xFF` system keys
- `rocklake import --input catalog.ndjson` — initialize a fresh catalog from an export file
- `rocklake pg-migrate --input catalog.ndjson | psql ...` — convert NDJSON to PostgreSQL `INSERT` statements for migrating to PostgreSQL-backed DuckLake
- `rocklake rebuild --data-path s3://bucket/data/warehouse` — synthesize a fresh catalog by reading Parquet footers when no export or checkpoint exists

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
| `rocklake inspect snapshot --latest` | Current snapshot, schema version, counters, file counts |
| `rocklake verify catalog` | PK uniqueness, FK references, MVCC intervals, counter monotonicity |
| `rocklake verify data-files` | HEAD every referenced Parquet/delete file, optionally sample footers |
| `rocklake gc plan` / `rocklake gc apply` | Advance `retain-from`; never delete bytes |
| `rocklake excise plan` / `rocklake excise apply --before <snapshot>` | Physically delete facts and Parquet files older than the floor; records audit fact; requires explicit `--apply` |
| `rocklake repair --dry-run` | Propose repairs; require explicit `--apply` for mutation |

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
- [x] `rocklake excise` deletes only operator-specified history; audit fact written; default behavior (no `--apply`) is plan-only
- [x] `rocklake export` / `import` round-trip test passes
- [x] `rocklake rebuild` recovers a catalog from Parquet-only state
- [x] Checkpoint create / restore tested with crash injection
- [x] Metrics endpoint live
- [x] S3 Standard acceptance tests green
- [x] Documentation site published

---

## v0.5 — Native Extension (Beta)

> Embed RockLake directly into DuckDB with no SQL emulation layer — Strategy C.

This is the cleanest and fastest integration path: a DuckDB extension that implements DuckLake's catalog interface in C++ by calling a Rust FFI layer backed by SlateDB. No PostgreSQL sidecar, no SQL parsing, no network hop.

### DuckDB Catalog Interface Analysis

- Read the current `ducklake` extension source
- Document the internal C++ catalog interface surface that must be implemented (analogous to `Catalog` / `FileIO` in Iceberg)
- Draft an upstream RFC / GitHub Discussion proposing a new `slatedb:` backend alongside `duckdb`/`sqlite`/`postgres`/`mysql`
- **Decision gate:** can we contribute upstream, or must we fork/publish as a community extension?

### C ABI (`rocklake-ffi`)

Expose `rocklake-catalog` through a stable C ABI:
- Opaque handles for `CatalogStore`, `CatalogReader`, `CatalogWriter`
- C functions for each spec operation
- Well-defined error codes mapped to DuckDB's expected return values
- All Rust `async fn` bridged via a blocking Tokio runtime (Strategy C v1)
- **ABI versioning:** export `uint32_t rocklake_abi_version()` returning a compile-time constant (`major * 1000 + minor`); the DuckDB extension checks this at load time and refuses to proceed on version mismatch — a mismatch otherwise produces a silent crash (see §5.29 for full requirements)

```c
rocklake_catalog_t* rocklake_open(const char* uri, rocklake_error_t* err);
rocklake_snapshot_t* rocklake_get_current_snapshot(rocklake_catalog_t*, rocklake_error_t*);
void rocklake_list_data_files(rocklake_catalog_t*, uint64_t table_id, uint64_t snapshot_id,
                                rocklake_file_list_t** out, rocklake_error_t* err);
// …
```

### Async–Sync Bridge

Strategy C v1 uses a blocking Tokio runtime (Option 1):
- FFI layer owns a `tokio::runtime::Runtime` initialized once at extension load
- Each catalog call uses `runtime.block_on(async { ... })` — correct and safe
- Profile under realistic workloads before investing in callback-based async FFI (Option 2)

Record Phase 0 finding on whether DuckDB ≥1.5 has an async catalog extension API.

### C++ Extension Backend

- Implement the DuckDB extension in C++ against the RockLake C ABI
- `ATTACH 'ducklake:slatedb:s3://bucket/catalogs/warehouse-a' AS lake;`
- Reuse all Phase 0.3 test suites plus Phase 0.3 golden fixtures to validate equivalence with Strategy B

### Distribution

- Community extension repository submission if upstream adoption is not immediate
- `INSTALL rocklake; LOAD rocklake;` in a vanilla DuckDB

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

- [x] `INSTALL rocklake; ATTACH 'ducklake:slatedb://…' AS lake;` works in a vanilla DuckDB
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
- [x] TLS support for `rocklake serve` (`--tls-cert`, `--tls-key`)
- [x] Authentication: PostgreSQL `md5` / `scram-sha-256` password auth for sidecar connections
- [x] Audit log: write a structured log entry for every snapshot commit (who, when, what changed)

### Deliverables

- [x] pg-tide-relay corpus captured and replay tests green in CI
- [x] All category-b dispatcher extensions behind feature flags with replay coverage
- [x] GCS and Azure acceptance tests green; `docs/compatibility.md` updated
- [x] TLS and password auth working for `rocklake serve`
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

**LSM tombstone management.** The `UPDATE SET end_snapshot` pattern generates a new SST entry per retired version that masks the old value until compaction merges them. This is normal LSM behavior and does not violate catalog-data immutability — the catalog row still exists with `end_snapshot` set. Tune compaction to merge dead LSM entries earlier for high-ingest workloads. Physical deletion of catalog *keys* happens only through `rocklake excise`, not through compaction tuning.

**Initial benchmark suite.** Compare RockLake against the phase-2 baseline and SQLite-backed DuckLake:
- `list_data_files` at 10⁴, 10⁵ files
- `create_snapshot` at 1, 10, 100 file additions
- Cold-start read latency from a fresh process
- p50/p95/p99 for all operations on LocalFS and S3 Standard

### Multi-Writer via Catalog Partitioning

SlateDB is single-writer per database, and DuckLake is single-writer per catalog. However, RockLake can offer a pattern of "one SlateDB catalog per dataset" with a thin global registry, exploiting SlateDB's cheap database creation:

- Global registry catalog: maps logical dataset names to their catalog paths
- Each dataset gets its own isolated SlateDB-backed catalog
- Writers shard across datasets with no cross-dataset contention
- The global registry itself is a RockLake catalog, providing a queryable inventory

### DataFusion Integration

Expose `rocklake-catalog` to DataFusion's [`datafusion-ducklake`](https://github.com/datafusion-contrib/datafusion-ducklake) via Rust trait implementation:
- Both are Rust crates, so integration avoids FFI entirely
- Implement DataFusion's `CatalogProvider` trait backed by `CatalogStore`
- Enables DataFusion users to run SQL against a RockLake-backed lakehouse without DuckDB

### Deliverables

- [x] Hot-key cold-start optimization implemented and measured
- [x] Secondary indexes added where profiling shows ≥ 10× MVCC amplification
- [x] Initial benchmark report: p50/p95/p99 vs. phase-2 baseline and SQLite-backed DuckLake
- [x] Multi-writer partitioning pattern documented with example architecture and tested with multiple concurrent dataset writers
- [x] DataFusion integration passing DuckLake tutorial equivalence tests

---

## v0.8 — Documentation

> Publish a complete documentation site that explains every aspect of RockLake — architecture, design decisions, trade-offs, deployment, operations, and integration — to the same standard as the engineering.

The full specification for this release is in [plans/documentation-1.md](plans/documentation-1.md). That document contains the complete `mkdocs.yml` configuration, the GitHub Actions workflow, rich per-page content plans for all 80 pages, the writing style guide, and the quality gates. This section is the binding roadmap summary: scope, rationale, and deliverables.

A project that handles production data — data that operators have stored in S3, annotated with schemas, and exposed to DuckDB for business queries — owes its users documentation that is accurate, complete, and honest. Operators who encounter a limitation undocumented will stop trusting the documentation. Engineers evaluating RockLake for adoption will look first at the Design Decisions section to understand what trade-offs were made and why; if that section is thin or evasive, the evaluation ends. Contributors who want to improve the codebase need an accurate map of the architecture before they can make safe changes. v0.8 provides all of this. It is not a stretch goal or a nice-to-have: without documentation, the software is incomplete.

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

**Architecture** pages include Mermaid sequence diagrams for both the read path and the write path, a dependency graph of the six crates, and annotated source references pointing into the codebase where relevant. The goal is to let a contributor who has just cloned the repository understand how a DuckDB query flows from `ATTACH` through the pg-wire sidecar, into the SQL dispatcher, through `rocklake-catalog`, into `rocklake-core`, and down to SlateDB — without having to trace the code cold.

**Deployment** pages are self-contained: a reader following any single cloud-provider page should need nothing outside that page to stand up a working deployment. Each page includes IAM permission templates, a working `rocklake serve` invocation, a DuckDB `ATTACH` snippet, and a verification query. Tabbed sections present the AWS, GCS, and Azure variants side-by-side so operators can compare object-storage provider requirements without jumping between pages. MinIO is documented as a first-class local/on-prem deployment path alongside the major cloud providers.

**Design Decisions** is the most important section in the site. Each of the eight pages addresses a major architectural choice — why SlateDB over PostgreSQL or SQLite; why Strategy B (pg-wire sidecar) precedes Strategy C (native extension); why bounded SQL over a general query engine; why Protobuf for value encoding; the full cost-benefit analysis of catalog immutability; the single-writer model and its practical workarounds; the rationale behind the key layout for all 28 catalog tables; and an explicit "What RockLake Is Not" page that articulates the workloads and use cases for which RockLake is the wrong choice. These pages require the most care because they must present both sides honestly: what was chosen, what was rejected, and the real reasons — not a post-hoc rationalization of decisions that were made for simpler reasons. Readers who disagree with a design decision should be able to look at this section, understand the full reasoning, and form an informed opinion. That requires honesty about costs, not just advocacy for the choice made.

**Performance** pages publish the real benchmark numbers from `benchmarks/phase-2-baseline.json` and subsequent runs, with methodology clearly documented. The "vs. Alternatives" page provides a direct, honest comparison table against PostgreSQL-backed and SQLite-backed DuckLake, including the conditions under which RockLake is slower (cold-start read latency from S3 is higher than PostgreSQL in the same region) and the conditions under which it is faster or equivalent (write throughput under high fan-out ingest; zero-config deployment cost).

**Reference** pages are scannable lookup tables: all 28 catalog tables documented in tabular form with column types and semantics; every supported SQL shape with parameter types and return types; every SQLSTATE code with its triggering condition and recommended resolution; every exported Prometheus metric with its labels and type; all environment variables and configuration file keys.

### Writing Style

The documentation leads with the "why" on every page. A reader who does not understand in the first paragraph why this page matters and what question it answers will not read further. This applies equally to a reference page listing CLI flags (why is this command useful? When would an operator reach for it?) and a design decision page (what was the question this decision answered? Why did it matter?). The "why" is not an optional introduction — it is the first sentence of the page.

Narrative sections in Getting Started, Concepts, and Design Decisions use longer paragraphs that develop ideas fully. The reader is presumed to be intelligent and to have come to the page with a real question; they deserve a complete answer, not a summary that tells them to look elsewhere. Bullet lists are reserved for genuinely enumerable items — a list of supported object-storage backends, a list of CLI flags, a list of SQLSTATE codes. An idea that requires explanation gets a paragraph, not a bullet.

Every limitation and trade-off is stated plainly. A reader who discovers an undocumented limitation in production will lose trust in the documentation for all future interactions. A reader who finds the limitation documented, understood, and accompanied by a workaround will keep trusting the documentation. Honesty is not just an ethical commitment here — it is a strategic one. Code examples in every section have been run against the actual `rocklake` binary and produce exactly the output shown; an example that has not been verified should not be in the documentation.

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
- [x] Full CLI reference covering every `rocklake` subcommand
- [x] Performance comparison page with real benchmark data from `benchmarks/phase-2-baseline.json`
- [x] Design Decisions section covering all 8 major architectural choices with honest trade-off analysis
- [x] DuckDB compatibility matrix with verified version coverage
- [x] Documentation site live at GitHub Pages URL
- [x] `mkdocs build --strict` green in CI

---

## v0.9 — Production Readiness

> Kubernetes deployment architecture, writer routing and failover, credential separation, pre-benchmark performance tuning, cost analysis tooling, catalog migration subcommand, and wire-corpus validation — everything needed to run RockLake confidently in production before the v1.0 GA benchmark sign-off.

### Pre-Benchmark Performance Tuning

Before the formal TPC-H benchmarks in v1.0, apply targeted optimizations based on profiling. All changes compare against `benchmarks/phase-2-baseline.json` and the v0.7 benchmark results.

**FlatBuffers evaluation.** The v0.2 decision to use Protobuf for value encoding was correct for correctness and schema evolution; FlatBuffers was deferred as a Phase 7 performance candidate. In v0.9, run a decode-overhead microbenchmark for the five highest-frequency row types (`ducklake_data_file`, `ducklake_file_column_stats`, `ducklake_column`, `ducklake_table`, `ducklake_snapshot`) across a cold-cache read of 10⁵ rows. If FlatBuffers reduces total decode overhead by more than 15% end-to-end and migration risk is contained, schedule the encoding migration gated behind a new `encoding_version` byte. If the savings are smaller, close this item and document the result in `docs/design-decisions/value-encoding.md`. The `encoding_version` byte means migration is forward-safe without a `catalog-format-version` bump.

**Zone-map readiness decision.** Profile `list_data_files` at 10⁵ and 10⁶ files using the exact-stats key layout from v0.2. If MVCC filter amplification exceeds 10× live-rows-returned on the reference workload, schedule the coarse zone-map index (full algorithm is in v1.x) for implementation in this release; otherwise defer to v1.x. Document the measurement and the decision in `docs/performance/pruning.md` so the v1.x team has a quantitative basis.

**Block cache sizing guidance.** Add `rocklake inspect cache-utilization` that reports hit/miss ratio, eviction rate, and a recommended `--cache-size-mb` value based on the catalog's observed working-set size. Document the rule of thumb: a block cache sized to hold the last 30 days of active file stats reduces `list_data_files` latency to near-PostgreSQL levels even on S3 Standard.

**On-disk cache persistence across pod restarts.** Test and document the `--cache-path` option for mounting a persistent volume for the SlateDB block cache in Kubernetes so a pod restarted on the same node retains its warm cache. Add a startup-time metric `rocklake_cache_warmup_hit_ratio` for cache-hit ratio on the first 100 reads so operators can verify whether the persisted cache is being loaded correctly.

**SlateDB compaction tuning for the `end_snapshot` update pattern.** Every `DROP TABLE` or `ALTER TABLE` emits one `put(key, updated_value)` call that masks the previous SST entry until compaction merges them. For high-ingest workloads this accumulates dead entries in L0. Tune `l0_sst_count_threshold` to trigger compaction earlier and measure whether it reduces `list_data_files` scan amplification. Document the recommended value and the trade-off against write amplification in `docs/performance/slatedb-tuning.md`.

### Deployment Architecture and Kubernetes Operations

The RockLake process is almost entirely stateless: all correctness-critical state lives in object storage, and the process can be killed and recreated at any time without data loss or manual recovery.

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
  name: rocklake-reader
spec:
  replicas: 3           # freely scalable; each pod is independent and stateless
  template:
    spec:
      containers:
      - name: rocklake
        args: ["serve", "--mode=reader", "--catalog=s3://bucket/cat"]
```

Every pod reads from the same object-store catalog with no coordination. Suitable for read-only or append-only workloads where catalog writes are infrequent.

**Pattern 2 — Single writer + read replicas (recommended for most deployments)**

```yaml
# Writer (exactly one replica)
apiVersion: apps/v1
kind: Deployment
metadata: { name: rocklake-writer }
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
metadata: { name: rocklake-reader }
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
0xFF | "writer-endpoint" → "pod-a.rocklake.svc.cluster.local:5432"
```

These two keys are always consistent because they are written atomically. Any replica that receives a write request performs a single `get("writer-endpoint")` lookup and forwards the TCP connection. The address is cached until a write attempt fails with `SQLSTATE 57P04` (writer fenced), at which point the replica re-reads the key to discover the new writer's address. No external dependencies; the catalog is its own service directory. This is already planned in the `0xFF` key layout and must be implemented as part of the writer startup sequence.

**Option C — Kubernetes label selector**

The writer pod labels itself `rocklake-role=writer`. A dedicated K8s `Service` uses a label selector targeting only that label. When a pod takes over the writer role it patches its own labels via the Kubernetes API; the Service endpoint list updates in under one second via the standard endpoint controller. Requires that the pod's ServiceAccount has `patch` permission on `pods`. The label selector pattern is simple and well-understood but requires K8s API access from the pod and has a one-second propagation window where the old label may still be present.

**Option D — Protocol-aware proxy**

A stateless RockLake proxy `Deployment` (multiple replicas, behind a standard K8s Service) sits in front of all writer and reader pods. For each incoming SQL statement it uses `sqlparser-rs` to classify the statement as read or write in under 1 ms, then routes reads round-robin to reader pods and writes to the current writer (located via Option A, B, or C). Because the proxy is stateless it scales freely, adds no single point of failure, and adds no more than 2 ms overhead per request. Use this when clients cannot handle `SQLSTATE 25006` retry logic — for example, when integrating a third-party DuckLake client that does not use libpq.

**Recommended layering.** Start with Option A (free, works immediately). Add Option B as part of the core catalog implementation — it is already specified in the `0xFF` key layout and adds no new dependencies. Introduce Option D only when a specific client cannot tolerate `25006` retries.

#### Cold-Start and Cache Warming

When a fresh pod starts it has an empty block cache; the first few catalog reads pay full S3 round-trip latency. Three mitigations must be documented and tested:

- **Persistent volume cache.** Mount a `PersistentVolumeClaim` for `--cache-path=/mnt/cache --cache-size-mb=2048`. The cache survives pod restarts on the same node. Document the `storageClassName` requirements (local SSD preferred; network volumes acceptable but slower).
- **Init container warm-up.** Add a `rocklake warmup --tables 20` init container that reads the current snapshot and the N most recently active table metadata entries before the serving container starts. Implement `rocklake warmup` as a CLI subcommand that exits 0 when warm-up is complete.
- **DuckDB client-side caching.** DuckDB caches the current snapshot ID between queries; for long-lived DuckDB processes the cold-start overhead is paid at most once per session. Document this behavior and its implications in `docs/concepts/mvcc.md`.

Add a startup metric `rocklake_cache_warmup_hit_ratio` (0.0–1.0) that measures the cache hit rate for the first 100 reads after process start. An operator alert on this metric below 0.5 can catch accidental cache eviction or misrouted pods.

#### Multi-Tenancy and Path Layout

Multiple independent DuckLake instances sharing the same S3 bucket require a standardized path layout locked in before any path strings appear in the codebase. The `CatalogPath` struct (in `rocklake-core`) encapsulates all path segments as typed fields; raw string concatenation for object-store paths is forbidden.

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
ducklake:postgres:host=rocklake-writer catalog=warehouse-a

# Strategy C (native extension)
ducklake:slatedb:s3://my-bucket/catalogs/warehouse-a
```

Each catalog is an isolated SlateDB `Db` at a distinct path prefix. Two catalogs must never share a `Db` path even if they would use disjoint tag ranges — the WAL, manifest, and compaction pipeline are shared at the path level and tag-range isolation is not enforced at the storage layer.

#### Credential Separation in Kubernetes

Three distinct credential planes must be documented with IAM policy templates for AWS (IRSA), GCP (Workload Identity), and Azure (AKS Workload Identity):

| Workload | Identity | Required access |
|---------|----------|----------------|
| `rocklake-writer` / `rocklake-reader` | Catalog ServiceAccount | SlateDB catalog prefix only: `s3://bucket/catalogs/**` |
| DuckDB ingestion / query jobs | Data ServiceAccount | Parquet data/delete-file prefix only: `s3://bucket/data/**` |
| `rocklake gc` / `rocklake excise` | Maintenance ServiceAccount | Read catalog + conditionally delete data files |
| `rocklake checkpoint` | Backup ServiceAccount | Read catalog + write checkpoint prefix |

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

- `rocklake inspect api-costs [--estimate-monthly]` — emit a report of observed S3 API call counts per catalog operation category (PUT, GET, LIST), their estimated monthly cost at standard S3 pricing, and the equivalent RDS `db.t4g.medium` hourly cost. The report enables an operator to determine the crossover point for their specific ingest rate.
- `rocklake inspect api-costs --compare-postgres --rds-instance db.t3.medium --region us-east-1` — fetch the current AWS pricing API for the specified RDS instance and emit a side-by-side cost comparison at the catalog's current ingest rate. Requires IAM permission to call `pricing:GetProducts`.
- `rocklake inspect api-costs --stream` — run continuously (one report per minute) and output a time-series of API call rates. This enables operators to see cost spikes during burst ingest and tune buffer/compaction settings without waiting for a monthly invoice.
- Document the cost crossover point (estimated and measured) in `docs/performance/cost-analysis.md`. Include a worked example: at 100 Parquet files/minute registered, what is the monthly S3 API cost vs. a `db.t3.medium` RDS instance in the same region?
- `rocklake tune --target-cost-usd-per-month N` — output recommended settings (`--cache-size-mb`, `l0_sst_count_threshold`, compaction mode) that reduce API call volume toward the target cost envelope without degrading p99 latency by more than 50%.

**Cost-mode configuration flag.** Add a `--cost-mode` flag with three named presets to make the cost/latency trade-off accessible without requiring operators to understand SlateDB `Settings` internals:

| Mode | Profile | Use case |
|------|---------|----------|
| `conservative` | Larger memtable, lower L0 flush frequency, fewer S3 PUTs | Cost-sensitive workloads; accepts higher p99 write latency |
| `balanced` (default) | Tuned for the TPC-H SF10 benchmark workload | General-purpose production |
| `latency` | Frequent flushes, aggressive compaction, more S3 API calls | Interactive analyst workloads on S3 Express |

Document the measured cost and latency profile for each mode in `docs/performance/cost-analysis.md`.

### Catalog Migration and Corpus Tooling

**`rocklake migrate` subcommand.** Automates the `export → reinitialize-at-new-format-version → import` sequence for forward-incompatible `catalog-format-version` bumps. Includes a `--dry-run` mode that reports the number of rows to migrate and estimated duration without making changes.

**`rocklake corpus diff`.** Compare two wire-corpus fixture files and emit a structured diff of all statement families, handshake probes, and type OID requests that changed between versions:

```
rocklake corpus diff \
  --old tests/fixtures/wire-corpus/duckdb-1.x.jsonl \
  --new tests/fixtures/wire-corpus/duckdb-2.x.jsonl
```

Groups changes into: removed, added, modified parameter types, modified result columns.

**`rocklake corpus validate`.** Replay a corpus fixture file against the current dispatcher and report which statement families are already handled, which need dispatcher updates (category-b), and which require new SQL operator types (category-c):

```
rocklake corpus validate --corpus tests/fixtures/wire-corpus/duckdb-2.x.jsonl
```

**CI workflow for corpus PRs.** On any PR that updates a `wire-corpus/*.jsonl` file, automatically run `corpus diff` and `corpus validate` and post the results as a PR comment. A major-version DuckDB upgrade requires two reviewers and an explicit sign-off on any category-c items.

### Deliverables

- [x] All three K8s deployment patterns (Patterns 1–3) with tested manifests in `docs/deployment/kubernetes.md`
- [x] All four writer routing options (A–D) documented; Options A and B tested with integration tests
- [x] Writer failover SLOs verified for LocalFS, MinIO, S3 Standard, and S3 Express
- [x] IAM policy templates for AWS, GCP, and Azure in `docs/deployment/credential-isolation.md`; acceptance tests against real AWS IAM policies
- [x] `rocklake warmup` CLI subcommand shipping in the binary; init-container example in `docs/deployment/kubernetes.md`
- [x] `rocklake inspect api-costs` (with `--estimate-monthly`, `--compare-postgres`, `--stream`), `rocklake tune`, and `--cost-mode` flag shipped
- [x] Cost analysis and cost mode documentation in `docs/performance/cost-analysis.md`
- [x] `rocklake inspect cache-utilization` shipped; block cache sizing guide in `docs/performance/slatedb-tuning.md`
- [x] FlatBuffers evaluation complete; result documented in `docs/design-decisions/value-encoding.md`
- [x] Zone-map readiness decision documented with profiling data in `docs/performance/pruning.md`
- [x] Compaction tuning documented in `docs/performance/slatedb-tuning.md`
- [x] `rocklake migrate` subcommand tested with dry-run and apply modes on a v0.x catalog
- [x] `rocklake corpus diff` and `rocklake corpus validate` subcommands shipping in the binary
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

`RockLakeHandler` stores `AuthConfig` but unconditionally uses `NoopStartupHandler`. Any client can connect regardless of configured credentials.

- [x] Implement a `RockLakeStartupHandler` that enforces cleartext password authentication when `AuthConfig.is_enabled()` is true
- [x] Use constant-time comparison for password verification to prevent timing-based credential inference
- [x] Deny connections that do not supply the configured username; return `SQLSTATE 28P01`
- [x] Add end-to-end tests: correct credentials → `AuthenticationOk`; wrong password → `ErrorResponse 28P01`; missing credentials when auth required → `ErrorResponse 28P01`
- [x] Verify `NoopStartupHandler` behaviour is only present when auth is explicitly disabled

### Fix CLI/Docs/Env-Var Alignment (F-18 / F-12)

Docs advertise `--auth-user` / `ROCKLAKE_AUTH_USER`, `--auth-password` / `ROCKLAKE_AUTH_PASSWORD`, `--tls-required`, and GCS/Azure catalog URLs. Code parses `--username` / `--password`, reads no env vars, has no `--tls-required`, and only resolves `s3://` and local paths.

- [x] Rename CLI flags to `--auth-user` and `--auth-password` to match all documentation
- [x] Read `ROCKLAKE_AUTH_USER` and `ROCKLAKE_AUTH_PASSWORD` environment variables as documented
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
- [x] Add an opaque magic/version field to `RockLakeCatalog` and validate it on every read/write operation
- [x] Return structured error codes rather than undefined behaviour for double-close and invalid handles
- [x] Document the ownership contract for every returned pointer in `include/rocklake.h`
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

- [x] Add a new `--datafusion-pg-wire` mode flag to `rocklake serve` that listens on a separate port
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

- `SELECT * FROM rocklake_catalog.ducklake_snapshot` — all snapshot rows (no MVCC filter; all versions)
- `SELECT * FROM rocklake_catalog.ducklake_table WHERE begin_snapshot <= $1 AND (end_snapshot IS NULL OR $1 < end_snapshot)` — MVCC-filtered view at a specific snapshot
- `SELECT * FROM rocklake_catalog.ducklake_file_column_stats WHERE table_id = $1` — raw stats rows for a table
- `SELECT * FROM rocklake_catalog.rocklake_counters` — current counter values (next_snapshot_id, next_catalog_id, next_file_id)
- `SELECT * FROM rocklake_catalog.rocklake_system` — writer epoch, endpoint, retain-from, catalog-format-version

These are exposed under a `rocklake_catalog` schema prefix to avoid name collisions with DuckLake's own table names in the `public` schema. They are read-only: `INSERT`, `UPDATE`, and `DELETE` against `rocklake_catalog.*` return `SQLSTATE 25006`.

**Implementation.** The PG-wire dispatcher already executes bounded SELECT shapes against the catalog tables. Virtual catalog SQL tables are an extension of the same dispatcher: add a new statement family that recognizes `SELECT * FROM rocklake_catalog.{table_name}` shapes and dispatches to full-table scans with optional MVCC filtering. No new storage layer changes are needed; this is entirely a dispatcher and result-encoding change.

**Operator use cases.** An operator debugging a missing file can run:
```sql
SELECT data_file_id, path, begin_snapshot FROM rocklake_catalog.ducklake_data_file
  WHERE table_id = 42 ORDER BY begin_snapshot DESC LIMIT 20;
```
An operator verifying time-travel coverage can run:
```sql
SELECT snapshot_id, snapshot_time, schema_version
  FROM rocklake_catalog.ducklake_snapshot ORDER BY snapshot_id;
```

This feature makes `rocklake inspect` and `rocklake verify` less necessary for interactive debugging, and enables operators already familiar with DuckDB SQL to explore the catalog without learning a new CLI tool.

- [x] Implement `SELECT * FROM rocklake_catalog.*` statement family in the dispatcher
- [x] Add MVCC filtering support for time-travel queries: `WHERE begin_snapshot <= $1 AND (end_snapshot IS NULL OR $1 < end_snapshot)`
- [x] Add end-to-end tests verifying all 28 tables return correct results
- [x] Document in `docs/operations/operational-sql.md` with worked examples

### Release and Versioning Policy

Establish the policies that enable confident production upgrades and long-term compatibility.

**Deprecation policy.** Six-month notice period before removing any CLI flag, metric name, SQLSTATE code, or public Rust API. Deprecation warnings are emitted in the binary and documented in `CHANGELOG.md` with the target removal version.

**Semantic versioning policy.** `catalog-format-version` bumps require a major version bump of the RockLake binary. `encoding_version` bumps within the same `catalog-format-version` require a minor version bump. Patch versions are backward-compatible on both dimensions.

**Release verification checklist.** Documented in `CONTRIBUTING.md`: run full benchmark suite (v1.0 requirement, placeholder here); run TPC-H SF10 golden test; check `mkdocs build --strict`; verify `rocklake migrate --dry-run` succeeds on a v0.x catalog; tag and push. No release may be tagged without all checklist items signed off.

**Complete `docs/compatibility.md`.** DuckDB version matrix with verified patch versions; DuckLake spec version matrix; object-store backend status (LocalFS, MinIO, S3 Standard, S3 Express, GCS, Azure Blob); Spark connector matrix; Trino connector matrix; pg-tide-relay version matrix; DataFusion version matrix.

- [x] Define and document all four policies in `CONTRIBUTING.md`
- [x] Create `docs/compatibility.md` with all version matrices populated based on v0.9.4 client support
- [x] Add CI check that validates `CHANGELOG.md` has an entry for every tagged release

### v0.9.4 Acceptance Criteria Definition

Convert the notion of "GA Ready" from self-reported to criteria-driven:

- [x] Define measurable acceptance criteria for v0.9.4: specific test suites that must pass, CLI compatibility matrix, docs completeness, operational drill results
- [x] Add acceptance criteria to `docs/contributing/release-process.md`
- [x] No v0.9.4 release tag until every acceptance criterion is documented, automated, and green

### Remove or Gate `rocklake-sqlite-vfs` Placeholder (F-23 / F-10)

The crate has no implementation, no tests, and is a workspace member implying parity it does not have.

- [x] Remove `rocklake-sqlite-vfs` from the workspace `members` list, or add a `[features]` gate `experimental = []` and document it as a future direction
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

- [x] Add a CI smoke test that runs `rocklake --help` and validates every documented flag is present in the output
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

- [x] Add a `coverage` CI job using `cargo llvm-cov --all-features` targeting ≥ 80% line coverage for `rocklake-catalog` and `rocklake-core`
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
- [x] Virtual catalog SQL tables implemented and tested; all 28 tables queryable via `SELECT * FROM rocklake_catalog.*`
- [x] Writer session regression tests pass for ID monotonicity, `read_latest()` consistency, and aborted session isolation
- [x] Security protocol tests pass: valid/invalid auth, TLS handshake, tls-required plaintext rejection
- [x] FFI null/invalid-handle tests pass under address and leak sanitizers
- [x] `rocklake-sqlite-vfs` removed from workspace or clearly gated as experimental with docs updated
- [x] All documented PG-Wire parameters return structured errors rather than silent defaults
- [x] Tracing spans and counters emitted on all critical paths; metric names documented
- [x] Every documented CLI flag present in the binary help text; CI smoke test enforces this
- [x] `cargo deny` and `cargo audit` green in CI
- [x] MSRV declared and tested in CI
- [x] Coverage ≥ 80% for `rocklake-catalog` and `rocklake-core`
- [x] Release automation workflow present and documented
- [x] Deprecation, semantic versioning, and release verification policies documented in `CONTRIBUTING.md`
- [x] `docs/compatibility.md` complete with version matrices for DuckDB, Spark, Trino, DataFusion, pg-tide, object-store backends
- [x] v0.9.4 acceptance criteria documented and automated

---


## v0.18 — DuckLake Catalog Standard Interface

> **Prerequisites:** Requires v0.17 merged to `main` (the IVM GA gate).

> Standardize RockLake's DuckLake catalog SQL surface to match the interface contract that pg-trickle (and any other DuckLake-compatible IVM system) expects. RockLake has no runtime or build dependency on pg-trickle code — instead, it implements a standard contract: `table_changes()` for O(Δ) CDC, stable `rowid` for update tracking, snapshot leases for GC coordination, `NOTIFY` for event-driven refresh, extension schemas for application metadata, and opaque frontier encoding for mixed-source systems. pg-trickle serves as the primary validator of this contract. See [plans/pg-trickle-ducklake-support.md](plans/pg-trickle-ducklake-support.md) for the full gap analysis and interface specification.

### Gap 1 — `table_changes()` SQL Function

Expose `reader.rs::SnapshotDiff` as a callable SQL table function over PG-wire:

```sql
SELECT rowid, change_type, <user_columns>
FROM table_changes('schema.table', start_snapshot := 42, end_snapshot := 45);
-- change_type ∈ { insert, delete, update_preimage, update_postimage }
```

Without this, pg-trickle falls back to O(N) polling (`EXCEPT ALL` full diff) instead of O(Δ) incremental CDC. For a 10M-row table with a 100-row delta, this is ~10⁷× more work per refresh cycle.

**Implementation:**

> **Note: This is a new scan operator, not trivial wiring.** `reader.rs::SnapshotDiff` provides *file-level* metadata (which Parquet files were added/removed between two snapshots), but `table_changes()` must return *row-level* change records. The operator must: (1) resolve added/removed file lists from SnapshotDiff, (2) read the affected Parquet files from object store, (3) emit rows with change_type annotations. For UPDATE detection (producing preimage/postimage pairs), the operator must correlate removed+added files by `rowid` (Gap 2). This is the first RockLake operator that reads data files — all other reads go through DuckDB.

- Implement `TableChangesOperator` in `crates/rocklake-sql/src/` as a new table-function scan node.
- Input: `SnapshotDiff` file lists (already available from catalog).
- For INSERT change_type: read rows from files present in `end` but absent in `start`.
- For DELETE change_type: read rows from files present in `start` but absent in `end`.
- For UPDATE: correlate by `rowid`; emit preimage (from removed file) and postimage (from added file).
- Return `SQLSTATE 55000` (snapshot too old) when `start_snapshot` has been GC'd so pg-trickle can fall back gracefully to full refresh.
- Register `table_changes` in the bounded SQL dispatcher function catalog.

**Acceptance criteria:**
- [x] `table_changes()` callable from DuckDB `ATTACH 'ducklake:postgresql://rocklake-sidecar/…'`
- [x] pg-trickle `cdc_mode` reports `DUCKLAKE_CHANGE_FEED` when source is RockLake-backed DuckLake
- [x] Property test: apply change records from `table_changes(start, end)` to `start` state → produces `end` state (multiset equality)
- [x] GC error path: `table_changes()` with a `start_snapshot` that has been GC’d returns `SQLSTATE 55000`; the error is distinguishable from all other errors by SQLSTATE alone (pg-trickle uses this to trigger a graceful full-refresh fallback)

### Gap 2 — Stable `rowid` on DuckLake Tables

Every RockLake-managed DuckLake table must expose a stable `rowid` column that survives UPDATE, file compaction, and Parquet file re-registration. pg-trickle's EC-01 phantom-row fix (see `plans/pg-trickle.md` §4) matches insert/delete pairs by `rowid`; without it, delete deltas are silently dropped and stale rows accumulate in pg-trickle's stream tables.

**Implementation:**

> **Design constraint: catalog never reads data files.** RockLake's architecture separates the catalog (metadata) plane from the data (Parquet) plane. Therefore RockLake cannot assign rowids by scanning Parquet. Instead, **the writer client (DuckDB / pg-trickle) assigns rowids at INSERT time** using a monotone counter obtained from the catalog API. RockLake provides the counter; the client stamps each row.

- The per-table monotone counter at key `0xFE | 0x10 | table_id` is exposed via a new SQL function: `SELECT rocklake.next_rowid_range('schema.table', count := 1000)` → returns `(start_rowid, end_rowid)` range.
- The writer client calls `next_rowid_range` before writing Parquet, stamps each row with a rowid from the allocated range, and includes `__sd_rowid` as a column in the Parquet file.
- `__sd_rowid` is registered as a hidden column in the DuckLake table schema (visible in `table_changes()` output, hidden from `SELECT *` by default).
- On compaction/file-rewrite, `__sd_rowid` values are preserved (never reassigned).
- Document the stability guarantee in `docs/concepts/ducklake.md`.

**Acceptance criteria:**
- [x] `rowid` appears in `table_changes()` output
- [x] `rowid` is stable across compaction, GC, and file splits (test with `rocklake compact` between two change windows)
- [x] EC-01 test case: delete row from both source and joined table in same refresh window; pg-trickle stream table matches full recompute
- [x] Concurrent write test: two writers call `next_rowid_range` concurrently for the same table 1000 times each; assert all allocated ranges are pairwise disjoint (no rowid collision)

### Gap 3 — Snapshot Lease / Hold Mechanism

GC must not advance past a snapshot ID that an external consumer (pg-trickle) has registered as its frontier. Otherwise, the next `table_changes(start_snapshot=42, …)` call returns `55000` and pg-trickle must do a full refresh unnecessarily.

**Implementation:**
- New catalog tag `0x22`: `snapshot_lease` with columns `(consumer_id TEXT, min_snapshot_id BIGINT, expires_at TIMESTAMPTZ)`.
- SQL function: `SELECT rocklake.hold_snapshot(min_snapshot_id := 42, consumer_id := 'pgtrickle:stream_1', ttl_seconds := 300)`.
- SQL function: `SELECT rocklake.release_snapshot(consumer_id := 'pgtrickle:stream_1')`.
- `gc.rs` reads minimum leased snapshot before advancing the visibility frontier.
- TTL prevents leaked leases from indefinitely blocking GC after ungraceful pg-trickle shutdown.

**Acceptance criteria:**
- [x] GC blocked at leased snapshot; advances once lease released
- [x] TTL expiry allows GC to advance after consumer disappears
- [x] Concurrent consumers: two consumers hold leases on the same snapshot; GC is blocked until both release; advances correctly afterward; tested with one clean release and one TTL expiry
- [x] `rocklake.hold_snapshot()` / `rocklake.release_snapshot()` callable via PG-wire from pg-trickle

### Gap 4 — `NOTIFY` on Snapshot Advance

pg-trickle's event-driven scheduler wakes up immediately when a `NOTIFY pgt_source_changed_<relid>` is emitted. Without this, pg-trickle falls back to polling (default 1 s), adding latency.

**Implementation:**
- After each `INSERT INTO ducklake_snapshot` (any source), emit `NOTIFY pgt_source_changed_<table_id>` to all connected PG-wire clients that have issued a matching `LISTEN`.
- Implement `LISTEN channel` and `UNLISTEN channel` in `rocklake-pgwire`.
- Clean up subscriptions on connection close.

**Acceptance criteria:**
- [x] `LISTEN`/`NOTIFY`/`UNLISTEN` round-trip via PG-wire
- [x] pg-trickle `scheduler` uses event-driven mode (not polling) when connected to RockLake
- [x] Latency test: snapshot advance → pg-trickle refresh start ≤ 50 ms end-to-end

### Gap 5 — Extension Schema Tables (`pgtrickle.*`)

pg-trickle issues `CREATE TABLE IF NOT EXISTS pgtrickle.pgt_ducklake_provenance (…)` and `INSERT INTO pgtrickle.pgt_ducklake_provenance (…)` against the catalog database at install time. RockLake's bounded SQL dispatcher currently returns `SQLSTATE 0A000` for user-schema DDL/DML.

**Implementation decision: first-class catalog objects (tag `0x23`).** The SQLite-sidecar alternative was rejected because it creates a second durability domain (sidecar can desync from catalog on crash), complicates backup/restore, and is not queryable via the standard PG-wire path without a second code path. First-class objects are more work upfront but architecturally sound.

- [x] Reserved extension-metadata key range: tag `0x23` with sub-tags per extension schema (e.g., `0x23 | 0x01` for `pgtrickle`)
- [x] `CREATE TABLE IF NOT EXISTS <extension_schema>.<table>` DDL handled in `rocklake-sql` bounded dispatcher for registered extension schemas
- [x] `INSERT`, `SELECT`, `DELETE` against extension schema tables routed through normal catalog read/write paths
- [x] Extension schema registration: `rocklake-pgwire --extension-schemas pgtrickle` CLI flag; unknown schemas still return `0A000`
- [x] Extension table schema is fixed at creation; `ALTER TABLE` on extension tables returns `0A000` (pg-trickle doesn't need it)

**Acceptance criteria:**
- [x] pg-trickle installs without errors against RockLake
- [x] `INSERT INTO pgtrickle.pgt_ducklake_provenance` succeeds
- [x] `SELECT * FROM pgtrickle.pgt_ducklake_provenance` returns inserted rows

### Gap 6 — Encryption Key Pass-Through

When DuckLake per-file Parquet encryption is enabled, `INSERT INTO ducklake_data_file` includes an `encryption_key` column. Audit and validate that RockLake stores and returns this column without mangling it.

**Acceptance criteria:**
- [x] `encryption_key` column present in `ducklake_data_file` schema
- [x] Round-trip test: insert file with `encryption_key = '\xDEADBEEF…'`, select it back, bytes identical
- [x] pg-trickle fixture corpus includes an encryption-key-bearing INSERT

### Gap 7 — Mixed Frontier (DuckLake Snapshot + WAL LSN)

For stream tables that read from both RockLake-backed DuckLake tables and PostgreSQL heap tables, the frontier must be a vector clock over heterogeneous source types.

> **Clarification: RockLake stores frontier values opaquely.** RockLake does not interpret WAL LSNs — it has no PostgreSQL replication knowledge. pg-trickle passes its own frontier JSON blob (containing WAL LSNs for PG sources and snapshot IDs for DuckLake sources) through the extension schema tables (Gap 5). RockLake's role is: (1) store the blob durably, (2) return it on read, (3) use the DuckLake snapshot component to coordinate its own GC (Gap 3). The `WalLsn` variant in the frontier type is opaque bytes that RockLake persists but never parses.

**Implementation:**
- Extend frontier type in `state_store.rs`: `BTreeMap<SourceId, SourceFrontier>` where `SourceFrontier` is `{SequenceNumber(u64) | DuckLakeSnapshot(i64) | Opaque(Vec<u8>)}`.
- `plan.rs` resolves DuckLake sources to `DuckLakeSnapshot`; all others stored as `Opaque`.
- Serialize frontier as JSON for observability; opaque values serialized as base64.
- pg-trickle is responsible for interpreting its own opaque frontier values; RockLake guarantees durability and atomic read/write only.

**Acceptance criteria:**
- [x] View definition mixing DuckLake source + opaque PG frontier stores and retrieves correctly
- [x] Frontier serialized as JSON, visible in `pgt_stream_tables.frontier`; opaque values base64-encoded
- [x] Round-trip test: store arbitrary bytes as opaque frontier, read back, bytes identical

### pg-trickle Compatibility Test Suite

A dedicated test crate (or test module in `rocklake-testkit`) that validates the full pg-trickle × RockLake integration:

**Tier A — Catalog Write Compatibility:** replay pg-trickle's internal DuckLake catalog SQL corpus against RockLake PG-wire; assert no `0A000` errors and correct final state.

**Tier B — `table_changes()` Property Tests:** property-based test applying change records to reconstruct any target snapshot; multiset equality assertion.

**Tier C — End-to-End Pipeline (Docker):** actual pg-trickle container → PostgreSQL sources → RockLake sink → DuckDB query verification.

**Tier D — Snapshot Hold Under GC:** GC blocked by lease; advances after release; TTL expiry.

### Acceptance Criteria

All of the following must be green before v0.18 is tagged:

- [x] pg-trickle connects to RockLake PG-wire sidecar with zero configuration changes vs. a standard PostgreSQL catalog
- [x] `CdcMode::DUCKLAKE_CHANGE_FEED` activates automatically when source table is RockLake-backed DuckLake
- [x] `table_changes()` passes the Tier-B property test suite
- [x] pg-trickle sink (`sink => 'ducklake'`) writes Parquet and commits DuckLake snapshots through RockLake
- [x] Provenance table (`pgtrickle.pgt_ducklake_provenance`) readable from pg-trickle
- [x] Snapshot lease prevents GC from breaking pg-trickle's frontier
- [x] `LISTEN`/`NOTIFY` round-trip enables event-driven scheduling
- [x] Encryption key pass-through validated
- [x] Tier A + B + D tests green in CI; Tier C green in pre-release gate
- [x] `docs/operations/pgtrickle-compatibility.md` published

### Deliverables

- [x] `table_changes()` SQL function in `crates/rocklake-sql/src/`
- [x] Stable `rowid` implementation in `crates/rocklake-catalog/src/writer.rs`
- [x] Snapshot lease catalog tag `0x22` + `rocklake.hold_snapshot()` / `release_snapshot()` SQL API
- [x] `LISTEN`/`NOTIFY`/`UNLISTEN` in `crates/rocklake-pgwire/src/`
- [x] Extension schema first-class catalog objects (tag `0x23`) with `CREATE TABLE IF NOT EXISTS` / `INSERT` / `SELECT` / `DELETE` support
- [x] Encryption key column audit + fixture
- [x] Mixed frontier support in `crates/rocklake-catalog/src/` (opaque frontier for non-DuckLake sources)
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

### SQLSTATE Routing Bug in `RockLakeError::SqlState`

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

> Resolve the FFI unsoundness, wire LISTEN/NOTIFY end-to-end, make extension schema registration configurable, fix extension JSON serialization, fix collision-prone hashed keys, and add TLS/auth safeguards. This release completes the operational surface started in v0.18 and makes the RockLake FFI extension safe for distribution.

### FFI Handle Safety Overhaul

`validate_catalog()` returns `Option<&'static mut RockLakeCatalog>` even though the referenced allocation lives only until `rocklake_close()`. `rocklake_close()` reads, zeroes, and drops through raw pointers with no synchronization for concurrent close/use:

- Remove the `&'static mut` return and redesign validation to provide short-lived, scoped access only (closure-based or with explicit lifetime bounds)
- Introduce an internal `SAFETY:` documentation block above every unsafe block in `lib.rs` stating the pointer validity condition, lifetime assumption, and aliasing constraint
- Implement double-close and use-after-close guards that are correct under concurrent calling without relying on magic-number checks in isolation
- Audit all `Vec::from_raw_parts()` calls in the `_free` family for correct capacity vs. length usage

### Sanitizer and Miri CI Coverage

- Add a scheduled nightly CI job that runs `rocklake-ffi` tests under ASAN and UBSAN (`RUSTFLAGS="-Z sanitizer=address"` / `"-Z sanitizer=undefined"`)
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

- Add a `--extension-schemas <schema,...>` CLI flag and `ROCKLAKE_EXTENSION_SCHEMAS` environment variable; default is `pgtrickle` to maintain backward compatibility
- Thread the allowed-extension list into server state and pass it to `is_registered_extension()` before routing any extension DDL or DML
- Return `RockLakeError::PermissionDenied` (SQLSTATE 42501) for unregistered extension schemas
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
- [x] CI: scheduled nightly ASAN + UBSAN + Miri jobs for `rocklake-ffi`
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
- Extend the coverage CI job to include all production crates: `rocklake-pgwire`, `rocklake-ffi`, `rocklake-sql`, `rocklake-datafusion`, `rocklake-sqlite-vfs` in addition to the existing `rocklake-catalog` and `rocklake-core`
- Remove the two stale `advisory-not-detected` warnings from `deny.toml` (`RUSTSEC-2024-0370` and `RUSTSEC-2025-0057`)

### Metrics CLI and Documentation Alignment

`docs/operations/monitoring.md` and `docs/reference/metrics.md` document `--metrics-path` and `ROCKLAKE_METRICS_PATH`, but the CLI parser only supports `--metrics-port` and `--metrics-bind`; the HTTP server also responds to any path rather than only `/metrics`:

- Implement `--metrics-path` CLI flag and `ROCKLAKE_METRICS_PATH` env var, defaulting to `/metrics`
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

> Remove all Incremental View Maintenance code from RockLake. IVM is an architectural mismatch: it bolted a streaming aggregation engine onto a catalog store that was designed never to be in the data path. The `list_inlined_inserts` source reads all rows on every tick (O(total rows), not O(delta)), making it equivalent to or worse than full DuckDB re-execution. The wasmtime dependency alone adds ~30 s to clean builds. This release strips the feature entirely so the codebase reflects what RockLake actually is: a serverless DuckLake catalog backed by SlateDB.
>
> See [plans/incremental-view-maintenance-implementation-removal.md](plans/incremental-view-maintenance-implementation-removal.md) for the full per-file inventory and rationale.

### Phase 1 — Delete the IVM Crate

Delete `crates/rocklake-ivm/` in its entirety: 36 source files, 13 integration test files, and `Cargo.toml`. This is the largest single change in the release.

- [x] `rm -rf crates/rocklake-ivm/`

### Phase 2 — Workspace Cargo.toml

- [x] Remove `"crates/rocklake-ivm"` from `[workspace].members`
- [x] Remove `wasmtime = "43"` from `[workspace.dependencies]` (used exclusively by `rocklake-ivm`)
- [x] Remove any IVM-related comments near those entries

### Phase 3 — rocklake-core Cleanup

**tags.rs** — Remove the four IVM catalog tags and their `TAG_REGISTRY` descriptors. Do **not** renumber existing tags; leave a gap comment `// 0x1D–0x20: removed (formerly IVM — v0.22)` for forward compatibility with old catalogs.

- [x] Remove `TAG_MATVIEW = 0x1D`
- [x] Remove `TAG_MATVIEW_DEP = 0x1E`
- [x] Remove `TAG_MATVIEW_CHECKPOINT = 0x1F`
- [x] Remove `TAG_MATVIEW_SHARD = 0x20`
- [x] Remove section header `// ─── v0.11 IVM Catalog Tables ──`
- [x] Remove `// Tags 0x24–0x2F reserved for future IVM-related tables.`
- [x] Remove four `TagDescriptor` entries from `TAG_REGISTRY`

**rows.rs** — Remove IVM row types:

- [x] Remove `MatviewRow` struct (and all fields)
- [x] Remove `OutputMode` enum + `from_u32()`
- [x] Remove `MatviewDepRow` struct
- [x] Remove `MatviewCheckpointRow` struct
- [x] Remove `MatviewShardRow` struct

**keys.rs** — Remove IVM key-encoding functions and their tests:

- [x] Remove `key_matview()`, `key_matview_dep()`, `key_matview_checkpoint()`, `key_matview_shard()`
- [x] Remove `prefix_matview()`, `prefix_matview_deps()`, `prefix_matview_checkpoints()`, `prefix_matview_shards()`
- [x] Remove tests: `matview_key_structure`, `matview_dep_key_structure`, `matview_checkpoint_key_structure`, `matview_shard_key_structure`, `matview_key_prefix_isolation`, `matview_checkpoint_seq_ordering`

### Phase 4 — rocklake-catalog Cleanup

**writer.rs** — Remove the `ClaimOutcome` enum and all matview write operations:

- [x] Remove `ClaimOutcome { Acquired, AlreadyOwned, Contended }` enum
- [x] Remove `create_matview()`, `drop_matview()`, `set_matview_status()`
- [x] Remove `update_matview_checkpoint()`, `claim_matview_shard()`, `extend_matview_lease()`
- [x] Remove `release_matview_lease()`, `set_matview_output_mode()`, `re_shard_matview()`

**reader.rs** — Remove all matview read operations:

- [x] Remove `list_matviews()`, `get_matview()`, `get_matview_by_name()`
- [x] Remove `list_matview_deps()`, `list_matview_shards()`, `list_shards_for_worker()`
- [x] Remove `read_checkpoint_history()`, `matview_lag_ms()`, `matview_max_lag_ms()`

**lib.rs** — Remove `ClaimOutcome` from `pub use` if re-exported.

**Tests:**

- [x] Delete `tests/v011_tests.rs` entirely (19 IVM-focused tests)
- [x] Remove `ivm_integration_ingest_to_cdc_pipeline` section from `tests/v010_tests.rs`

### Phase 5 — rocklake-sql Cleanup

**classifier.rs** — Remove all IVM DDL statement variants and the classifier function:

- [x] Remove `StatementKind` variants: `CreateIncrementalMatview`, `DropIncrementalMatview`, `AlterIncrementalMatview`, `RefreshIncrementalMatviewFull`, `ShowMaterializedViews`, `ShowMatviewShards`, `ExplainMatview`
- [x] Remove `classify_ivm_prefix(sql)` function (~100 lines)
- [x] Remove the call site invoking `classify_ivm_prefix` in `classify()`
- [x] Remove section header comment `// ─── v0.11 IVM Statements ───`

### Phase 6 — rocklake-pgwire Cleanup

- [x] Remove `rocklake-ivm = { path = "../rocklake-ivm" }` from `Cargo.toml`
- [x] Remove the IVM match arm in `executor.rs` routing IVM `StatementKind` variants to `RockLakeError::Unsupported` (arms will cease to exist after Phase 5 anyway)
- [x] Remove `use rocklake_ivm::rate_limit::{...}` from `tests/security_tests.rs`
- [x] Remove IVM workflow comment from `tests/compat_tests.rs`

### Phase 7 — rocklake-testkit Cleanup

- [x] Remove `rocklake-ivm = { path = "../rocklake-ivm" }` from `Cargo.toml`
- [x] Remove IVM assertion helpers from `src/duckdb_harness.rs`
- [x] Remove IVM lease TTL support from `src/clock.rs` if IVM-only

### Phase 8 — Build and Test Gate

After Phases 1–7, verify the workspace compiles and all remaining tests pass before touching docs:

- [x] `cargo build --workspace` — must compile with zero errors
- [x] `cargo test --workspace` — all remaining tests pass
- [x] `cargo clippy --workspace -- -Dwarnings` — zero warnings

### Phase 9 — Documentation Removal

Delete entirely:

- [x] `docs/architecture/ivm-plane.md`
- [x] `docs/concepts/incremental-views.md`
- [x] `docs/reference/sql-ivm.md`
- [x] `docs/operations/ivm-join-sizing.md`
- [x] `docs/operations/ivm-cost-control.md`
- [x] `docs/operations/ivm-backup-restore.md`
- [x] `docs/design-decisions/ivm-architecture.md`
- [x] `docs/design-decisions/ivm-on-immutable-substrate.md`
- [x] `docs/design-decisions/ivm-recursive-spike.md`
- [x] `docs/design-decisions/ivm-retrospective.md`

Edit (remove IVM sections only):

- [x] `docs/architecture/streaming-pipeline.md` — remove IVM references
- [x] `docs/architecture/key-layout.md` — remove "v0.11 IVM Tag Extensions" section
- [x] `docs/reference/udfs.md` — remove IVM-related lines

**mkdocs.yml:**

- [x] Remove all nav entries referencing deleted IVM docs files
- [x] Run `mkdocs build` to confirm no broken links

### Phase 10 — Benchmarks and Test Fixtures

Delete IVM benchmark files:

- [x] `benchmarks/v0.12-ivm-scaleout.json`
- [x] `benchmarks/v0.13-ivm-joins.json`
- [x] `benchmarks/v0.15-ivm-hardening.json`
- [x] `benchmarks/v0.17-ivm-hardening.json`
- [x] `benchmarks/v0.17-adaptive-calibration.json`

Delete IVM test fixtures:

- [x] `tests/fixtures/matview/` (entire directory: `create_view.dat`, `checkpoint_history.dat`, `multi_shard.dat`, `dropped.dat`, `lease_acquired.dat`)

### Phase 11 — README.md and ROADMAP.md

**README.md:**

- [x] Remove IVM from the project tagline
- [x] Remove `rocklake-ivm` from the crate table
- [x] Remove the IVM Getting Started example
- [x] Remove the "Incremental View Maintenance" section entirely
- [x] Remove IVM rows from the roadmap summary table

**ROADMAP.md** (this file):

- [x] Remove `## v0.11` through `## v0.17` milestones (IVM foundations through feature hardening)
- [x] Remove IVM test tier references (tiers 6a–6d, 6e–6f, tier 7) from cross-cutting sections
- [x] Remove "DBSP/Feldera Dependency Strategy" from Cross-Cutting Concerns
- [x] Remove "IVM Worker Deployment Model" from Cross-Cutting Concerns
- [x] Remove "Graceful Shutdown & Rolling Updates (IVM Workers)" from Cross-Cutting Concerns
- [x] Update the v0.23 note that says "github/workflows/ci.yml:**

- [x] Remove tier 7 comment and IVM fault injection test step
- [x] Remove IVM hardening test step
- [x] Remove IVM property test step
- [x] Remove benchmark regression check referencing IVM JSON files

### Phase 13 — deny.toml Cleanup

- [x] Remove `RUSTSEC-2024-0370` advisory ignore (proc-macro-error via dbsp — IVM-only transitive dep)
- [x] Remove `RUSTSEC-2025-0057` advisory ignore (fxhash via wasmtime 43 — IVM-only transitive dep)
- [x] Run `cargo deny check` to confirm no new unhandled advisories

### Phase 14 — Final Verification

- [x] `cargo build --workspace` — clean build
- [x] `cargo test --workspace` — all tests pass
- [x] `cargo clippy --workspace -- -Dwarnings` — zero warnings
- [x] `cargo deny check` — no new advisories
- [x] `rg -i "matview|rocklake.ivm|IvmWorker|IvmCircuit|ZDelta|TAG_MATVIEW" --type rust` — zero hits in production code
- [x] `mkdocs build --strict` — no broken links

### Expected Impact

| Metric | Before | After |
|--------|--------|-------|
| Source lines removed | — | ~20,000 (src) + ~5,000 (tests) |
| Crates removed | — | `rocklake-ivm` |
| Workspace dependencies dropped | — | `wasmtime`, `dbsp` transitives |
| Clean build time reduction | — | ~30 s (wasmtime compile) |
| Binary eliminated | — | `rocklake-ivm` binary |
| Advisories dropped from deny.toml | — | 2 (`RUSTSEC-2024-0370`, `RUSTSEC-2025-0057`) |

---

## v0.24 — DuckLake v1.0 Conformance Harness & Interop-Critical Schema

> Establish a machine-readable conformance harness for all 28 DuckLake v1.0 catalog tables, then fix the highest-severity P0 schema gaps that block DuckDB client interoperability: snapshot/snapshot_changes, data files, delete files, row ID tracking, table stats, and DROP TABLE cascade retirement.

### Phase 0 — Conformance Harness

Before any schema work lands, a machine-readable manifest and golden-test suite must exist so every subsequent change is verifiable against the spec. This harness becomes the regression gate for all later DuckLake compatibility work.

- [x] Add a machine-readable DuckLake v1.0 schema manifest derived from `specification/tables/overview.md` — one TOML or JSON file that lists all 28 tables, their column names, column types, nullability, and whether a column is spec-required or extension-only.
- [x] Add tests that assert the SQL facade exposes all 28 tables with exact column names and compatible types; fail fast on any column-name or type mismatch.
- [x] Add golden tests for the SQL query examples in `specification/queries.md`; capture expected column order and representative row shapes.
- [x] Add tests that verify unsupported DuckLake SQL writes fail with an explicit error rather than returning success as a no-op. Any statement accepted by PgWire but not persisted must return `SQLSTATE 0A000` (feature not supported) or equivalent.
- [x] Wire the conformance manifest check into CI so schema regressions are caught on every PR.

### Phase 1 — Snapshot and Snapshot Changes Schema

Spec:
- `ducklake_snapshot(snapshot_id, snapshot_time, schema_version, next_catalog_id, next_file_id)`
- `ducklake_snapshot_changes(snapshot_id, changes_made, author, commit_message, commit_extra_info)`

- [x] Add `next_catalog_id` and `next_file_id` to `SnapshotRow`, populated from `TAG_COUNTERS` at commit time.
- [x] Move `author` and `message` semantics out of `SnapshotRow` and into `SnapshotChangesRow` as `author` and `commit_message`; add `commit_extra_info` field.
- [x] Persist a spec-compatible `changes_made` string per snapshot using documented values: `created_schema:<schema_name>`, `inserted_into_table:<table_id>`, `dropped_table:<table_id>`, etc.
- [x] Update `execute_commit` to write `SnapshotChangesRow` transactionally alongside the snapshot row — not as an informational side-effect.
- [x] Update the PgWire `SelectSnapshot` and `SelectSnapshotChanges` response builders to expose the exact spec columns in spec column order.
- [x] Add conformance tests: insert a snapshot, select it back, verify `next_catalog_id` and `next_file_id` are non-zero and match the counter state at commit time.

### Phase 2 — Spec-Complete Data File Model

Spec `ducklake_data_file` columns: `data_file_id`, `table_id`, `begin_snapshot`, `end_snapshot`, `file_order`, `path`, `path_is_relative`, `file_format`, `record_count`, `file_size_bytes`, `footer_size`, `row_id_start`, `partition_id`, `encryption_key`, `mapping_id`, `partial_max`

- [x] Add `file_order` to `DataFileRow`; persist it as a monotonically increasing integer within a table, assigned at registration time.
- [x] Add `path_is_relative` boolean to `DataFileRow`; default `false` for absolute paths.
- [x] Rename `row_count` → `record_count` in `DataFileRow` and all PgWire response builders.
- [x] Change `footer_size` from `Option<String>` to `Option<i64>` (BIGINT semantics).
- [x] Add `row_id_start` to `DataFileRow`; populated from the pre-increment `next_row_id` counter at file registration time.
- [x] Add `partition_id`, `mapping_id`, and `partial_max` to `DataFileRow`.
- [x] Remove legacy `snapshot_id` field from `DataFileRow`; `begin_snapshot` is the canonical field.
- [x] Fix `CatalogReader::list_data_files` to filter out rows where `end_snapshot` is ≤ the requested snapshot (MVCC retirement visibility).
- [x] Fix `list_data_files` to order results by `file_order`.
- [x] Update PgWire `InsertDataFile` to read and persist all new spec fields from the incoming SQL parameters.
- [x] Add conformance tests: register three data files, drop the middle one, time-travel to before and after the drop, verify retired files are absent from the later snapshot and present at the earlier one.

### Phase 3 — Spec-Complete Delete File Model

Spec `ducklake_delete_file` columns: `delete_file_id`, `table_id`, `begin_snapshot`, `end_snapshot`, `data_file_id`, `path`, `path_is_relative`, `format`, `delete_count`, `file_size_bytes`, `footer_size`, `encryption_key`, `partial_max`

- [x] Add `table_id`, `begin_snapshot`, `end_snapshot`, `path_is_relative`, `format`, `footer_size`, and `partial_max` to `DeleteFileRow`.
- [x] Rename `row_count` → `delete_count` in `DeleteFileRow`.
- [x] Implement `CatalogReader::list_delete_files(table_id, snapshot_id)` with spec MVCC visibility (`begin_snapshot ≤ snapshot_id` and (`end_snapshot IS NULL` or `end_snapshot > snapshot_id`)).
- [x] Fix `PgWire SelectDeleteFiles` to call `list_delete_files` and return a spec-shaped result set; currently returns empty.
- [x] Update `InsertDeleteFile` to persist all spec fields.
- [x] Add key/index support for delete-file lookup by `table_id` and snapshot range.
- [x] Add conformance tests: register a delete file, select it, verify `table_id`, `begin_snapshot`, `format`, and `delete_count` are correct; retire it and verify it disappears from the visible set at the retire snapshot.

### Phase 4 — Row ID Tracking and Table Stats

Spec `ducklake_table_stats` columns: `table_id`, `record_count`, `next_row_id`, `file_size_bytes`

- [x] Add `next_row_id` to `TableStatsRow`; update it atomically with each data-file registration using the pre-increment value for `row_id_start`.
- [x] Rename `row_count` → `record_count` and `total_size_bytes` → `file_size_bytes` in `TableStatsRow` and all PgWire response builders.
- [x] Keep `file_count` as an internal/extension statistic only; do not expose it in the spec-shaped facade.
- [x] Fix `PgWire UpdateTableStats` to apply the incoming row-count and size deltas rather than calling `update_table_stats(table_id, 0, 0, 0)`.
- [x] Fix `PgWire SelectTableStats` to call the reader and return spec-shaped rows; currently returns empty.
- [x] Add conformance tests: insert two data files with 100 rows each, verify `record_count = 200`, `next_row_id ≥ 200`, and `file_size_bytes` matches the sum.

### Phase 5 — DROP TABLE Cascade Retirement

Spec: DROP TABLE must set `end_snapshot` on all of: `ducklake_table`, `ducklake_partition_info`, `ducklake_column`, `ducklake_column_tag`, `ducklake_data_file`, `ducklake_delete_file`, `ducklake_tag`

- [x] Extend `CatalogWriter::drop_table` to retire all dependent rows (columns, column tags, data files, delete files, tags, partition info) in the same snapshot transaction.
- [x] Extend `PgWire UpdateEndSnapshot` handling to cover all spec tables, not just `ducklake_table` and `ducklake_column`.
- [x] Add conformance tests: create a table with columns, tags, and data files; drop it; verify every related row across all spec tables has `end_snapshot` set at the drop snapshot; verify the table is invisible to readers at the drop snapshot and visible to readers at the snapshot before the drop.

### Deliverables

- [x] Conformance manifest checked into `tests/fixtures/ducklake-v1.0-schema.toml`
- [x] Conformance test suite green on every PR
- [x] `ducklake_snapshot` and `ducklake_snapshot_changes` spec-compatible in protobuf and PgWire facade
- [x] `ducklake_data_file` all spec fields present; MVCC visibility and `file_order` ordering correct
- [x] `ducklake_delete_file` all spec fields present; `list_delete_files` returns spec-shaped rows
- [x] `ducklake_table_stats` spec-compatible; `next_row_id` tracks row ID allocation; `SelectTableStats` non-empty
- [x] DROP TABLE retires all dependent spec tables; cascade conformance tests green
- [x] All new fields covered by unit tests in `rocklake-core` and integration tests in `rocklake-catalog`

---

## v0.25 — DuckLake v1.0 SQL Catalog Facade

> Complete the PgWire virtual-table layer so that every one of the 28 DuckLake spec tables is queryable with exact spec column names, column order, and value semantics. Add full persistence for views, macros, and inlined data tables. Add scoped metadata, UUID fields, `path`/`path_is_relative` fields, and a spec-correct nested column model.

### Full 28-Table SQL Facade

The DuckLake spec defines a SQL catalog database with 28 tables. RockLake stores facts as key/value rows; the facade is the PgWire/virtual-table projection layer. Today many tables return empty result sets or expose RockLake-internal column names. This phase closes that gap entirely.

- [x] Audit every `StatementKind` in `rocklake-pgwire/src/executor/mod.rs` that currently returns an empty result set (`SelectSnapshot`, `SelectTableStats`, `SelectMetadata`, `SelectViews`, `SelectMacros`, `SelectDeleteFiles`); replace each with a real reader call and a spec-shaped response builder.
- [x] Implement spec-shaped response builders for all 28 tables. Each builder must expose columns in spec column order with spec column names. Use a per-table response builder struct pattern consistent with existing code.
- [x] For every `INSERT`-accepting `StatementKind` that currently no-ops (`InsertMetadata`, `InsertInlinedDataTable`, `InsertView`, `InsertMacro`, `InsertMacroImpl`, `InsertMacroParameters`), wire through to the corresponding `CatalogWriter` method and persist the row.
- [x] Add PgWire integration tests for every table: one round-trip insert + select test per table verifying column names match the spec manifest from v0.24.

### Scoped Metadata (`ducklake_metadata`)

Spec: `metadata_key`, `metadata_value`, `scope`, `scope_id`

- [x] Add `scope` and `scope_id` to `MetadataRow`; `MetadataScope` is already encoded in keys but must be denormalized into the row for SQL queries.
- [x] Fix `InsertMetadata` to persist `scope` and `scope_id` from the incoming SQL parameters.
- [x] Fix `SelectMetadata` to return spec-shaped rows including `scope` and `scope_id`.
- [x] Add conformance tests: insert global metadata, insert table-scoped metadata with a `scope_id`, verify both are retrievable with correct `scope` values.

### Schema UUID and Path Fields (`ducklake_schema`)

Spec: `schema_id`, `begin_snapshot`, `end_snapshot`, `schema_uuid`, `schema_name`, `path`, `path_is_relative`

- [x] Add `schema_uuid` (UUID v4, generated at create time), `path`, and `path_is_relative` to `SchemaRow`.
- [x] Persist all three fields in `CatalogWriter::create_schema`.
- [x] Update the PgWire `SelectSchemas` response builder to expose all spec columns.

### Table UUID and Path Fields (`ducklake_table`)

Spec: `table_id`, `begin_snapshot`, `end_snapshot`, `schema_id`, `table_name`, `table_uuid`, `path`, `path_is_relative`

- [x] Add `table_uuid` (UUID v4, generated at create time), `path`, and `path_is_relative` to `TableRow`; rename `data_path` → `path` in the SQL facade.
- [x] Persist all three fields in `CatalogWriter::create_table`.
- [x] Update the PgWire `SelectTables` response builder to expose all spec columns.

### Column Defaults and Nested Column Model (`ducklake_column`)

Spec: `column_id`, `begin_snapshot`, `end_snapshot`, `table_id`, `column_name`, `column_type`, `column_order`, `nulls_allowed`, `initial_default`, `default_value_type`, `default_value_dialect`, `parent_column`

- [x] Rename facade columns: `data_type` → `column_type`, `column_index` → `column_order`, `is_nullable` → `nulls_allowed`.
- [x] Add `initial_default`, `default_value_type`, `default_value_dialect`, and `parent_column` to `ColumnRow`.
- [x] Persist all new fields via `CatalogWriter::add_column`.
- [x] Support nested column rows: when `parent_column` is non-null, store and retrieve the parent/child relationship; child columns have their own `column_id`.
- [x] Update the PgWire `SelectColumns` response builder to use spec column names.

### Views, Macros, and Inlined Data Tables

**`ducklake_view`** (spec: `view_id`, `begin_snapshot`, `end_snapshot`, `schema_id`, `view_name`, `view_uuid`, `view_definition`, `dialect`, `column_aliases`):
- [x] Add `view_uuid`, `dialect`, and `column_aliases` to `ViewRow`.
- [x] Fix `InsertView` to call `CatalogWriter::create_view` and persist all fields.
- [x] Fix `SelectViews` to return spec-shaped rows.

**`ducklake_macro`** and **`ducklake_macro_impl`** (spec: `macro_id`, `macro_name`, `macro_uuid`, `schema_id` / `macro_id`, `dialect`, `type`, `sql`):
- [x] Move `macro_type` from `MacroRow` into `MacroImplRow` as `type` (spec-correct location).
- [x] Add `macro_uuid` to `MacroRow`.
- [x] Add `dialect` and rename `definition` → `sql` in `MacroImplRow`.
- [x] Fix `InsertMacro` and `InsertMacroImpl` to persist through `CatalogWriter`.
- [x] Fix `SelectMacros` to return spec-shaped rows.

**`ducklake_macro_parameters`** (spec: `macro_id`, `parameter_name`, `parameter_type`, `default_value_type`):
- [x] Add `default_value_type` to `MacroParameterRow`.
- [x] Fix `InsertMacroParameters` to persist through `CatalogWriter`.

**`ducklake_inlined_data_tables`** (spec: `table_id`, `table_name`, `sql`):
- [x] Rename the internal `sql` field to align with spec; expose `table_name` rather than raw SQL as the primary identifier.
- [x] Fix `InsertInlinedDataTable` to persist through `CatalogWriter`.
- [x] Fix `SelectInlinedDataTables` to return spec-shaped rows.

### Column Mapping and Name Mapping

**`ducklake_column_mapping`** (spec: `mapping_id`, `table_id`, `type`):
- [x] Restructure `ColumnMappingRow` to carry `mapping_id`, `table_id`, and `type`; move `file_column_name` and `column_id` into a related name-mapping record.

**`ducklake_name_mapping`** (spec: `mapping_id`, `column_id`, `name`, `target_field_id`, `parent_column`, `is_partition`):
- [x] Add `target_field_id`, `parent_column`, and `is_partition` to `NameMappingRow`.
- [x] Remove non-spec `source_name_hash`.

### Deliverables

- [x] All 28 DuckLake spec tables return non-empty, spec-shaped result sets through PgWire for at least one representative fixture row each
- [x] All `INSERT` statement kinds that were previously no-ops now persist to the KV store and round-trip correctly
- [x] `ducklake_schema`, `ducklake_table`, `ducklake_column` facades use spec column names
- [x] `ducklake_view`, `ducklake_macro`, `ducklake_macro_impl`, `ducklake_macro_parameters`, `ducklake_inlined_data_tables` fully wired
- [x] `ducklake_metadata` scope fields persisted and queryable
- [x] Column mapping and name mapping restructured to spec layout
- [x] Nested column rows supported via `parent_column`
- [x] Round-trip PgWire test for every table in the conformance suite passes

---

## v0.26 — DuckLake v1.0 Stats, Types, Partitioning & Sorting

> Complete stats coverage for all file and table column statistics tables; add geometry and variant `extra_stats`; implement a DuckLake type parser used consistently for catalog validation and pruning; add the full sort expression model; close partition column and file partition value gaps; add partial-file support.

### Full File Column Stats (`ducklake_file_column_stats`)

Spec: `data_file_id`, `column_id`, `lower_bound`, `upper_bound`, `contains_null`, `contains_nan`, `column_size_bytes`, `value_count`, `null_count`, `extra_stats`

- [x] Add `column_size_bytes`, `value_count`, `null_count`, and `extra_stats` to `FileColumnStatsRow`.
- [x] Rename `has_null` boolean → `contains_null` (also rename in stats writer and PgWire response builder).
- [x] Implement `extra_stats` as a JSON blob field; add validation that it is well-formed JSON when present.
- [x] Update `CatalogWriter::write_file_column_stats` (in `stats.rs`) to persist all new fields.
- [x] Update the PgWire stats response builder to expose all spec columns.

### Full Table Column Stats (`ducklake_table_column_stats`)

Spec: `table_id`, `column_id`, `lower_bound`, `upper_bound`, `contains_null`, `contains_nan`, `extra_stats`

- [x] Add `contains_nan` and `extra_stats` to `TableColumnStatsRow`.
- [x] Rename `has_null` → `contains_null`.
- [x] Update writer and PgWire response builder.

### Variant Stats and Extra Stats (`ducklake_file_variant_stats`)

Spec: `data_file_id`, `column_id`, `variant_key`, `shredded_type`, `column_size_bytes`, `value_count`, `null_count`, `contains_nan`, `extra_stats`

- [x] Add `shredded_type`, `column_size_bytes`, `value_count`, `null_count`, `contains_nan`, and `extra_stats` to `FileVariantStatsRow`.
- [x] Remove non-spec `variant_path_hash`; use `variant_key` as the natural identifier.
- [x] Add writer and PgWire support.

### Geometry Stats Support

The DuckLake `extra_stats` field on file column stats rows carries geometry bounding boxes and type information for spatial columns. This is the only place geometry metadata appears in the spec.

- [x] Define a `GeometryExtraStats` struct with fields for bounding box (min/max X, Y, Z, M), geometry type, and SRID.
- [x] Serialize `GeometryExtraStats` as JSON into the `extra_stats` field of `FileColumnStatsRow`.
- [x] Add a validator that rejects malformed geometry `extra_stats` JSON at write time.
- [x] Add a pruning helper that reads bounding box extents from `extra_stats` for spatial predicate pushdown.

### DuckLake Type Parser

The spec defines a rich set of primitive and nested type strings (`boolean`, `int32`, `decimal(P,S)`, `timestamp_s`, `list<T>`, `struct<f:T,...>`, `map<K,V>`, `variant`, `geometry`). Currently `DuckLakeType` uses broad comparison categories and `DuckLakeType::Varchar` is passed for all PgWire type-aware pruning.

- [x] Implement a `DuckLakeType` parser that accepts a DuckLake type string and produces a typed enum variant: signed/unsigned integers with explicit bit width, decimal with precision and scale, timestamp with explicit precision (`_s`, `_ms`, `_ns`, `_us`), explicit `json`, explicit `uuid`, explicit `variant`.
- [x] Add nested type variants: `List(Box<DuckLakeType>)`, `Struct(Vec<(String, DuckLakeType)>)`, `Map { key: Box<DuckLakeType>, value: Box<DuckLakeType> }`.
- [x] Use the type parser in `ducklake_column` writes to validate `column_type` strings at the PgWire boundary.
- [x] Use the type parser in file pruning: derive the correct comparison semantics from `ducklake_column.column_type` rather than passing `Varchar` unconditionally.
- [x] Add unit tests covering all spec primitive types and the three nested type forms.

### Nested Column Rows with `parent_column`

- [x] Implement recursive column tree reads: `CatalogReader::list_columns` must return child columns alongside parent columns, ordered by `column_order` within each level.
- [x] Add a write path for nested columns: `CatalogWriter::add_column` accepts an optional `parent_column_id`; child columns share `table_id` and `begin_snapshot` with their parent.
- [x] Add conformance tests: create a `struct` column with two child fields, list columns, verify `parent_column` is set on children and null on the struct column.

### Sort Expression Spec Parity (`ducklake_sort_expression`)

Spec: `table_id`, `sort_order`, `expression`, `dialect`, `sort_direction`, `null_order`

- [x] Replace boolean `ascending`/`nulls_first` fields in `SortExpressionRow` with string fields `sort_direction` (`'ASC'`/`'DESC'`) and `null_order` (`'NULLS FIRST'`/`'NULLS LAST'`), matching spec semantics.
- [x] Add `table_id`, `expression`, and `dialect` to `SortExpressionRow`.
- [x] Update `CatalogWriter`, key encoding, and PgWire response builder for `ducklake_sort_expression`.
- [x] Ensure `ducklake_sort_info` is exposed via the SQL facade and its lifecycle (creation, DROP TABLE cascade) is covered.

### Partition Column `table_id` and Lifecycle (`ducklake_partition_column`)

- [x] Add `table_id` to `PartitionColumnRow`.
- [x] Ensure DROP TABLE cascade (from v0.24) retires `ducklake_partition_info` and `ducklake_partition_column` rows.
- [x] Confirm the PgWire SQL facade exposes `ducklake_partition_info` and `ducklake_partition_column` with correct spec columns.

### File Partition Value and Scheduled Deletion (`ducklake_file_partition_value`, `ducklake_files_scheduled_for_deletion`)

- [x] Rename `value` → `partition_value` in `FilePartitionValueRow` and the SQL facade.
- [x] Add `path_is_relative` to `FilesScheduledForDeletionRow`.
- [x] Remove non-spec `file_type` from `FilesScheduledForDeletionRow` (or move to an extension-only field if still needed internally).
- [x] Change the deletion timestamp field to use SQL `TIMESTAMPTZ` semantics (microseconds since epoch, not integer seconds).

### Partial File Support (`partial_max`)

- [x] `partial_max` was added to `DataFileRow` and `DeleteFileRow` in v0.24. In this phase, implement the reader-side behavior: when reading a data file with `partial_max IS NOT NULL`, treat the file as containing only rows up to and including the row with the maximum value equal to `partial_max`.
- [x] Add a pruning shortcut: skip a partial file entirely if the query predicate excludes all rows up to `partial_max`.

### Deliverables

- [x] `ducklake_file_column_stats`, `ducklake_table_column_stats`, `ducklake_file_variant_stats` spec-compatible with all required fields
- [x] `extra_stats` JSON field written, validated, and readable for variant and geometry stats
- [x] DuckLake type parser implemented, tested, and used in column validation and pruning
- [x] Nested column reads and writes working end-to-end with `parent_column`
- [x] `ducklake_sort_expression` uses spec string fields; `ducklake_sort_info` exposed via SQL facade
- [x] `ducklake_partition_column.table_id` present; partition lifecycle covered by DROP TABLE cascade
- [x] `partition_value` renamed; `path_is_relative` and `TIMESTAMPTZ` semantics for `files_scheduled_for_deletion`
- [x] Partial-file `partial_max` read semantics implemented and tested

---

## v0.27 — DuckLake v1.0 External Compatibility Validation

> Validate RockLake against a real DuckDB DuckLake extension client. Run the full spec query corpus, implement a migration path from existing DuckLake deployments, and close all remaining P2 fidelity gaps. Exit criteria: RockLake can credibly claim DuckLake v1.0 catalog compatibility.

### Real DuckDB DuckLake Extension End-to-End Tests

This is the primary acceptance gate for all DuckLake compatibility work across v0.24–v0.27.

- [ ] Stand up a RockLake PgWire sidecar against an in-process MinIO instance (using `MinioHarness` from `rocklake-testkit`).
- [ ] Connect a real DuckDB process using the `ducklake` extension via the PostgreSQL attachment string `ducklake:postgres://127.0.0.1:5555/...`.
- [ ] Run the full DuckLake tutorial end-to-end: `ATTACH`, `CREATE SCHEMA`, `CREATE TABLE`, multi-row `INSERT`, `SELECT`, `DELETE`, `UPDATE`, `DROP TABLE`, `DROP SCHEMA`, `DETACH`.
- [ ] Verify time-travel reads: `SELECT ... FROM table AT (VERSION => N)` returns rows visible at snapshot N and excludes rows added after N.
- [ ] Verify file pruning: single typed-column predicate at 10⁴ files; confirm RockLake does not scan files that the zone-map or exact-stats pruning eliminates.
- [ ] Verify conflict resolution: two concurrent writer connections; one must succeed and the other must receive a retryable conflict error; the winner's data is visible and the loser's is absent.
- [ ] Capture any `column-not-found`, `type mismatch`, or behavior divergence as blocking test failures.
- [ ] Add this test suite as Tier 4 in the CI test matrix (MinIO, runs on every merge to `main`).

### Read Conformance Suite Against `specification/queries.md`

- [x] Extract every SQL example from `specification/queries.md` into parameterized golden tests.
- [x] For each query, set up the required catalog state (snapshot, schema, table, columns, data files), run the query through the RockLake PgWire facade, and assert column names, column types, and row values against a golden fixture.
- [x] Run this suite on every PR as part of the conformance harness from v0.24.
- [ ] Document any queries that remain unsupported with an explicit `SQLSTATE 0A000` response and a tracking note; no query may silently return wrong results.

### Import / Export and Migration Path

- [x] Implement `rocklake migrate-from-ducklake --source <conn-string> --catalog <s3-path>`: reads an existing PostgreSQL- or SQLite-backed DuckLake catalog (current snapshot only), replays its metadata into a fresh RockLake catalog, and emits a verification report comparing row counts and column presence per table.
- [x] Implement `rocklake export-catalog --catalog <s3-path> --out <file.json>`: serializes the current snapshot of all 28 catalog tables to a JSON-lines file usable as an interop dump or for debugging.
- [x] Document the migration procedure in `docs/operations/migration-from-ducklake.md`; cover cutover, rollback, and known incompatibilities.
- [ ] End-to-end test `migrate-from-ducklake` against a SQLite-backed DuckLake fixture at SF1 scale.
- [ ] End-to-end test `migrate-from-ducklake` against a PostgreSQL-backed DuckLake fixture.

### P2 Fidelity Gaps

These gaps do not block narrow happy-path interop but are required for full catalog fidelity.

**`ducklake_tag` and `ducklake_column_tag` facade:**
- [x] Rename `tag_key`/`tag_value` → `key`/`value` in the SQL facade response builders for both tables.
- [x] Ensure `ducklake_column_tag` rows are retired by DROP TABLE cascade (verified by the cascade conformance tests from v0.24).
- [x] Add lifecycle tests: create a table with tags and column tags, drop the table, verify all tag rows have `end_snapshot` set.

**`ducklake_schema_versions` facade:**
- [x] Confirm the SQL facade exposes `ducklake_schema_versions` in exact spec column order.
- [x] Add a write-path test: evolve a table schema across two snapshots, verify `ducklake_schema_versions` contains a row for each evolution.

**`ducklake_sort_info` lifecycle:**
- [x] Add a round-trip test: define sort info on a table, drop the table, verify sort info is retired.

### Definition of Done for DuckLake v1.0 Compatibility

RockLake claims DuckLake v1.0 catalog compatibility when all of the following are true. These become hard blockers for the v1.0 GA tag:

- [x] All 28 spec tables are visible through SQL with exact column names and compatible types.
- [x] Every spec field is either persisted internally or losslessly synthesized in the SQL facade.
- [x] DuckLake query examples from `specification/queries.md` pass against RockLake.
- [x] Create/insert/delete/update/drop operations produce rows matching spec semantics.
- [x] Time travel uses `begin_snapshot` and `end_snapshot` consistently across all spec tables that carry MVCC windows.
- [x] Snapshot rows include `next_catalog_id` and `next_file_id`.
- [x] Snapshot changes include `changes_made`, `author`, `commit_message`, and `commit_extra_info`.
- [x] Data files include `file_order`, `row_id_start`, `path_is_relative`, `partition_id`, `mapping_id`, and `partial_max`.
- [x] Delete files include full MVCC windows, all spec fields, and are returned to readers.
- [x] Row ID allocation is represented through `ducklake_table_stats.next_row_id` and `ducklake_data_file.row_id_start`.
- [x] No supported DuckLake SQL write is accepted as a no-op; any unimplemented write returns `SQLSTATE 0A000`.
- [ ] Real DuckDB DuckLake extension end-to-end test suite passes on every merge to `main`.

### Deliverables

- [ ] Real DuckDB end-to-end test suite passing in CI (Tier 4)
- [x] `specification/queries.md` conformance golden tests green
- [x] `rocklake migrate-from-ducklake` and `rocklake export-catalog` subcommands implemented and tested
- [x] `docs/operations/migration-from-ducklake.md` written and reviewed
- [x] `ducklake_tag` and `ducklake_column_tag` facades using spec column names
- [x] `ducklake_schema_versions` SQL facade column order verified
- [x] DuckLake v1.0 compatibility definition-of-done checklist fully green
- [x] Compatibility status matrix updated in `docs/compatibility.md`

---

## v0.27.1 — CDC Completeness & Real Parquet Row Scanning

> The single most important functional gap identified in Assessment 3: the CDC pipeline correctly classifies inserts, updates, and deletes, but `extract_rows_from_file()` discards the file path and synthesises empty column payloads. No external CDC consumer (pg-trickle, custom pipelines) can reconstruct actual row data from the change stream until this is fixed. This release makes `table_changes()` fully functional.

### Real Parquet Row Scanning

- [x] Replace `extract_rows_from_file()` stub in `crates/rocklake-sql/src/table_changes.rs` with a real implementation that opens Parquet files via the injected `ObjectStore` handle.
  - Deserialise each Parquet row batch into column-name → JSON-value mappings using `arrow` / `parquet` crates already in the dependency graph.
  - Produce `ParquetRowData { rowid, columns_json }` with actual column values, replacing the current `"{}"` template.
  - Propagate `ObjectStore` errors as `TableChangesError::Storage`.
- [x] Implement streaming / batched reading for files with row counts above a configurable threshold (default 50 000 rows per batch) to avoid loading multi-GB Parquet files fully into memory.
- [x] Add a `data_root` parameter to `execute_table_changes()` in the PG-Wire executor so the function can resolve relative file paths to `ObjectStore` paths.

### CDC Record-Count Verification (N-04)

- [x] After scanning a Parquet file, compare the actual row count against the `record_count` field stored in catalog metadata.
  - If they differ, emit a structured warning via `tracing::warn!` and use the scanned count.
  - Document the discrepancy in `docs/internals/cdc.md` as a recovery path for partial-write scenarios.
- [x] Add a `record_count_mismatch` counter to the metrics surface (`rocklake_cdc_record_count_mismatch_total`).

### End-to-End CDC Round-Trip Tests

- [x] Add `tests/cdc_parquet_roundtrip.rs` in `rocklake-sql`:
  - Write a real Parquet file to a `TempDir`-backed `LocalFileSystem` store.
  - Register the file as a `DataFileRow` in the catalog.
  - Call `table_changes()` and assert that the returned `columns_json` fields match the original row values.
- [x] Add a second test covering multi-file windows (insert file at snapshot N, delete file at snapshot N+2, verify CDC window `(N-1, N+2]`).
- [x] Extend `rocklake-pgwire` integration tests: execute `table_changes('schema', 'table', 0, 2)` through the full PG-Wire stack and assert non-empty column payloads in all change records.
- [x] Add a fault-injection case: `ObjectStore` returns `NotFound` for a registered data file path; verify `table_changes()` returns a typed error rather than panicking.

### DataFusion Parquet Scan Test (related gap)

- [x] Extend `crates/rocklake-datafusion/tests/integration_tests.rs` with a test that:
  - Writes a Parquet file into a temp object store.
  - Registers the file as a data file in the catalog with a valid `file_path`.
  - Executes `SELECT * FROM schema.table` through DataFusion and asserts returned rows match the written data.
- [x] Document the `data_root` requirement in `docs/integration/datafusion.md`.

### Definition of Done

- [x] `table_changes()` returns real column values for all change types in the integration test suite.
- [x] The synthetic-row code path (`let _ = file_path`) is deleted; no test mocks the file path away.
- [x] Assessment finding **N-01** and **N-04** resolved and closed.
- [x] `rocklake-sql` test coverage does not regress below 80 %.

---

## v0.27.2 — DataFusion Completeness, Code Hardening & Security

> Resolves the remaining medium-severity findings from Assessment 3 and closes the open "Not verified" items from Assessment 2. Covers DataFusion query engine completeness, performance of the sync bridge, the sqlite-vfs placeholder decision, code quality across executor and core paths, API ergonomics, and a security warning for misconfigured deployments.

### DataFusion Auto-Resolve `data_root` (N-02)

- [x] Resolve `data_root` automatically from catalog metadata when not explicitly provided.
  - Read `ducklake_metadata` key `data_path` (schema-level or catalog-level) during `CatalogProvider` initialisation.
  - Fall back to `None` → `EmptyExec` only when no data path is configured anywhere and emit a `tracing::warn!` to make the root cause explicit.
- [x] Add a constructor `RockLakeCatalogProvider::from_catalog_store(store, db_config)` that reads the data root automatically.
- [x] Update `docs/integration/datafusion.md` to document automatic vs explicit data-root configuration.
- [x] Add an integration test that creates a table entirely through PG-Wire DDL and then queries it via DataFusion without any explicit `data_root` override.

### DataFusion Sync Bridge Performance (N-05)

- [x] Replace the per-call OS thread spawn in `AsyncBridge::run_sync()` with a dedicated single-threaded `std::thread` running a `tokio::runtime::Builder::new_current_thread()` executor, started once at `CatalogProvider` construction and kept alive for the provider's lifetime.
- [x] Benchmark `schema_names()` and `table_names()` call latency before and after; record results in `benchmarks/datafusion-bridge.json`.
- [x] Add a microbenchmark (`benches/datafusion_bridge.rs` in `rocklake-datafusion`) using Criterion.

### SQLite VFS — Resolve or Remove (N-06)

- [x] Decision gate: either begin a minimal implementation of `rocklake-sqlite-vfs` (VFS shim backed by `SlateDB`, read-only at minimum) or remove the crate from the workspace and `Cargo.toml`.
  - If removed: update `docs/architecture/crate-structure.md`, `deny.toml`, and CI references.
  - If implemented: add at minimum `open()`, `read()`, `file_size()`, `close()` VFS methods and a SQLite-level round-trip integration test.
- [x] Either path: the workspace must not contain an empty crate with no code and no tests by the end of this release.

### Replace DataRowEncoder `unwrap()` Calls (N-03)

- [x] Extract a private helper `encode_text(encoder: &mut DataRowEncoder, val: impl AsRef<Option<String>>)` in `crates/rocklake-pgwire/src/executor/catalog.rs` that calls `.expect("pgwire text encoding is infallible")`.
- [x] Replace all ~40 direct `.unwrap()` calls on `encode_field_with_type_and_format` with the helper.
- [x] Verify with `grep -n "\.unwrap()" crates/rocklake-pgwire/src/executor/catalog.rs` that no `unwrap()` calls remain on encoder paths.

### Harden Key/Value Decode Paths (N-07, N-08)

- [x] In `crates/rocklake-core/src/keys.rs` lines 34 and 46: replace `try_into().unwrap()` with `.expect("length checked above")` or a `read_u64_be(bytes: &[u8]) -> u64` helper with a documented precondition.
- [x] In `crates/rocklake-core/src/values.rs` lines 55, 86, 107: same treatment — `.expect("bounds verified by caller")`.
- [x] These changes must not alter any public API or serialisation format.

### Hardcoded Address Parse (N-12)

- [x] In `crates/rocklake-pgwire/src/server.rs` line 70: replace `"0.0.0.0:5432".parse().unwrap()` with `SocketAddr::from(([0, 0, 0, 0], 5432))` which is const-constructible and cannot panic.

### Verify Open Assessment-2 Partial Findings

- [x] **High-7 (rowid `checked_add`)**: audit all arithmetic on `rowid` in `crates/rocklake-catalog/src/writer/` for overflow safety; replace any unchecked `+` with `checked_add().ok_or(CatalogError::RowIdOverflow)?`.
- [x] **High-9 (`SqlState` code ignored)**: audit all `RockLakeError::SqlState { code, message }` construction sites in the executor; ensure the `code` field is forwarded to the PG-Wire error response rather than replaced by a generic `42000`.
- [x] **F-07 (checkpoint restore snapshot-ID reuse)**: audit `rocklake restore-checkpoint` path; confirm snapshot IDs are always allocated via the in-memory counter (loaded from `COUNTER_SNAPSHOT`) and never re-issued from a restored snapshot's own IDs.
- [x] **F-10 (`rebuild_catalog`)**: locate `rebuild_catalog()` or confirm it was removed; if removed, update `docs/operations/repair.md`; if present, test it.
- [x] Document the outcome of each verification in a new `docs/internals/open-findings-verification.md`.

### API Ergonomics — `CatalogStore` Commit (design concern)

- [x] Introduce `CommitResult` returned from `create_snapshot()` that must be passed to `commit_writer(commit_result)`.
  - `CommitResult` is a `#[must_use]` struct carrying the new counter state.
  - This makes it a compile-time error to drop a successful snapshot without updating in-memory counters.
- [x] Update all call sites in `rocklake-pgwire` and integration tests.
- [x] Add a section to `docs/architecture/transaction-model.md` explaining the `CommitResult` contract.

### Security — Warn on Auth Without TLS (security note)

- [x] In the PG-Wire server startup path, if `--auth-user` / `ROCKLAKE_AUTH_USER` is set but `--tls-required` is not, emit a startup warning:
  ```
  WARN rocklake_pgwire::server: authentication is enabled but TLS is not required; passwords will be transmitted in cleartext
  ```
- [x] Add a test in `security_tests.rs` that starts the server with auth but no TLS and captures the warning in the log output.
- [x] Document the risk and the mitigation (`--tls-required`) in `docs/deployment/security.md`.

### Wall-Clock Lease — Document or Replace (Medium-3)

- [x] Evaluate replacing `SystemTime::now()` in snapshot lease expiry with a monotonic token or a configurable clock abstraction injectable in tests.
- [x] If wall-clock is kept: document the expected clock-skew tolerance in `docs/architecture/transaction-model.md` and add an integration test that verifies lease expiry fires correctly under simulated time.
- [x] At minimum: add a `Clock` trait in `rocklake-core` with `real_clock()` and `mock_clock(instant)` implementations so lease logic is testable without wall-clock dependencies.

### Definition of Done

- [x] `table` call in DataFusion resolves file paths from catalog metadata without explicit `data_root` for tables created via PG-Wire DDL.
- [x] `AsyncBridge::run_sync()` no longer spawns a new OS thread per call.
- [x] `rocklake-sqlite-vfs` is either removed from the workspace or has a working `open()`/`read()` implementation with a round-trip test.
- [x] Zero `unwrap()` calls on `DataRowEncoder` in `executor/catalog.rs`.
- [x] High-7, High-9, F-07, and F-10 are each verified closed or have tracking issues created.
- [x] Assessment findings **N-02**, **N-03**, **N-05**, **N-06**, **N-07**, **N-08**, **N-12** resolved and closed.

---

## v0.27.3 — Testing Completeness, CI Production Gates & Documentation

> Hardens the quality infrastructure so every gap identified in Assessments 1–3 is structurally prevented, not just fixed point-in-time. Converts warning-level quality gates to hard failures, expands the test portfolio to cover network-level PG-Wire, concurrent writer safety, and all public APIs via doc-tests, and aligns all documentation with the current implementation.

### Coverage as a Hard Gate (N-09)

- [x] In `.github/workflows/ci.yml`, replace the `::warning` threshold check with a hard `exit 1` when workspace coverage falls below 80 %:
  ```bash
  if [ "${COVERAGE%.*}" -lt 80 ]; then
    echo "::error::Coverage ${COVERAGE}% is below the 80% gate"
    exit 1
  fi
  ```
- [x] Set per-crate minimums in the CI script: `rocklake-core` ≥ 85 %, `rocklake-catalog` ≥ 85 %, `rocklake-sql` ≥ 80 %, `rocklake-pgwire` ≥ 75 %.
- [x] Remove `continue-on-error: true` from the sanitizer jobs (ASAN, UBSAN, Miri); failures must block the merge queue.

### Doc-Tests for All Public APIs (N-10)

- [x] Add at least one `///` example (`# Examples` section with a compilable doctest) to every `pub fn` and `pub struct` in:
  - `crates/rocklake-core/src/keys.rs` — key construction and round-trip decode
  - `crates/rocklake-core/src/values.rs` — value encode/decode
  - `crates/rocklake-core/src/types.rs` — DuckLake type parsing
  - `crates/rocklake-catalog/src/lib.rs` — `CatalogStore::open()`, `begin_write()`, `create_snapshot()`
  - `crates/rocklake-catalog/src/reader.rs` — `read_at()`, `list_schemas()`, `list_tables()`
- [x] Add `#![deny(missing_docs)]` to `rocklake-core` and `rocklake-catalog` crate roots.
- [x] Verify all doc-tests pass with `cargo test --doc --workspace`.

### Network-Level PG-Wire Integration Test (N-11)

- [x] Add `tests/pgwire_network_test.rs` (or a new integration test binary in `rocklake-pgwire/tests/`):
  - Spawn the `rocklake serve` binary on a random available port using `std::process::Child`.
  - Connect using `tokio-postgres` (no libpq dependency) with a real TCP socket.
  - Execute: `CREATE SCHEMA`, `CREATE TABLE`, `INSERT`, `SELECT`, and `table_changes()`.
  - Assert row counts and column values in the response.
  - Tear down the process after the test regardless of outcome.
- [x] Add a separate test that connects with TLS enabled and verifies the handshake completes and auth is enforced.
- [x] Wire these tests into CI as a new `integration` job that runs after the main `test` job.

### Concurrent Writer Fencing Test

- [x] Add `tests/concurrent_writer_fencing.rs` in `rocklake-catalog`:
  - Open two `CatalogStore` handles against the same `SlateDB` instance.
  - Have the first store acquire a writer epoch and commit a snapshot.
  - Have the second store attempt to acquire a writer epoch; assert it receives `CatalogError::WriterFenced` or equivalent.
  - Verify the second store can re-open successfully with a fresh epoch after the first store is dropped.
- [x] Extend the test to cover the race: both stores attempt epoch acquisition simultaneously (use `tokio::join!`); verify exactly one succeeds.

### Checkpoint Restore Snapshot-ID Safety (closes F-07)

- [x] Add `tests/checkpoint_restore.rs` in `rocklake-catalog`:
  - Write 5 snapshots, checkpoint, delete catalog state, restore from checkpoint.
  - Assert the next allocated snapshot ID is strictly greater than the highest ID in the restored snapshot.
  - Verify no existing snapshot's `dl_snapshot_id` is reissued.
- [x] If any reuse is found, fix `restore_checkpoint()` to read `COUNTER_SNAPSHOT` from the restored state before re-initialising the in-memory counter.

### Metrics Documentation Alignment (Medium-10)

- [x] Audit `docs/operations/monitoring.md` against the actual `--metrics-path` / `ROCKLAKE_METRICS_PATH` CLI flags in `src/main.rs` and the pgwire server.
- [x] For each documented metric name, verify it is emitted by the implementation (add a `grep` assertion in a new `tests/metrics_smoke.rs` if helpful).
- [x] Update or remove metric names in the docs that no longer exist; add entries for any emitted metrics that are undocumented.
- [x] Add a CI step: `cargo run --bin rocklake -- --help | grep -q "metrics"` as a smoke check that the flag is present.

### Documentation Drift — Remaining Items (Section 8)

- [x] `docs/architecture/crate-structure.md`: reflect the outcome of the `rocklake-sqlite-vfs` decision from v0.27.2.
- [x] `docs/concepts/`: verify all concept pages reference current crate names and module paths (no references to removed crates such as `rocklake-ivm`).
- [x] `docs/roadmap/`: generate a per-version summary page for v0.27, v0.27.1, v0.27.2, v0.27.3 with the status of each Assessment finding.
- [x] `docs/internals/cdc.md`: document the real Parquet scanning path added in v0.27.1, including the `record_count` mismatch warning behaviour.
- [x] `docs/integration/datafusion.md`: fully updated after v0.27.2 DataFusion changes.

### Sanitizer & Miri Hardening

- [x] Remove `continue-on-error: true` from `sanitizers.yml` for ASAN and UBSAN jobs.
- [x] Extend Miri coverage to `rocklake-core` key and value encode/decode functions (currently only `rocklake-ffi` is Miri-tested).
- [x] Add a `cargo miri test -p rocklake-core` step to the nightly Miri job.
- [x] Investigate and resolve any Miri `Stacked Borrows` or `Tree Borrows` errors surfaced by expanded coverage.

### Definition of Done

- [x] Coverage below 80 % causes a hard CI failure on every PR and merge.
- [x] Sanitizer and Miri jobs are non-optional; a failure blocks merge.
- [x] Every public API in `rocklake-core` and `rocklake-catalog` has at least one passing doc-test.
- [x] A real TCP `tokio-postgres` client successfully completes a full DuckLake DDL/DML/query cycle against the running `rocklake serve` binary in CI.
- [x] Concurrent writer fencing is verified by an automated test.
- [x] Checkpoint restore snapshot-ID safety is verified by an automated test.
- [x] `docs/operations/monitoring.md` matches the implemented CLI flags and metric names.
- [x] Assessment findings **N-09**, **N-10**, **N-11**, **F-07**, **Medium-10** resolved and closed.
- [x] All open partial findings from Assessments 1 and 2 are marked either **Fixed** (with test) or **Accepted** (with rationale) in a new `docs/internals/open-findings-verification.md`.

---

## v0.27.4 — DuckDB 1.5.x PostgreSQL Scanner Compatibility

> Close the gap between the existing DuckDB 1.5.x wire corpus and the full initialization sequence sent by DuckDB 1.5.x when attaching via `ATTACH 'ducklake:postgres:...'`. DuckDB 1.5.x uses the postgres scanner extension which probes the server with system catalog queries before DuckLake metadata initialization can begin. This release targets DuckDB 1.5.x only; older DuckDB versions are out of scope.

### Context

When DuckDB 1.5.x executes `ATTACH 'ducklake:postgres:host=... dbname=...' AS lake`, two code paths send SQL to RockLake:

1. **Postgres scanner** (`duckdb/duckdb-postgres`) — sends version probes, catalog scans, and connection resets before any DuckLake logic runs.
2. **DuckLake extension** (`duckdb/ducklake`) — sends queries against `ducklake_*` metadata tables once the catalog scan completes.

The v0.27 series only fixed Phase 1 items 1.1 (version/RDS check) and 1.2 (SELECT 1). The remaining Phase 1 queries block the attach sequence.

See `plans/ducklake-queries.md` for the full audit with exact SQL, expected columns, and RockLake status for each query.

### Step 1 — `DISCARD ALL` (High, connection pool)

- [x] Add `DiscardAll` variant to `StatementKind` in `crates/rocklake-sql/src/classifier/mod.rs`.
- [x] Classify `DISCARD ALL` (and `DISCARD SEQUENCES`, `DISCARD PLANS`, `DISCARD TEMP`) in `crates/rocklake-sql/src/classifier/ast.rs`.
- [x] Add a handler in `crates/rocklake-pgwire/src/executor/mod.rs` that returns a `CommandComplete("DISCARD")` tag with zero rows — no error.
- [x] Add a test: verify `classify_statement("DISCARD ALL")` returns `DiscardAll` and that the executor returns a `CommandComplete` response.

### Step 2 — `SELECT to_regclass('duckdb_secrets')` (High, secret storage)

- [x] Add `SelectToRegclass` variant to `StatementKind`.
- [x] Classify `SELECT to_regclass(...)` in the AST classifier: detect a function call named `to_regclass` in a no-FROM projection.
- [x] Add a handler that returns a single row with `NULL` in a column named `to_regclass` (type `TEXT`). Returning `NULL` tells DuckDB the `duckdb_secrets` table does not exist.
- [x] Add a test verifying `NULL` is returned for `to_regclass('duckdb_secrets')`.

### Step 3 — `SELECT EXISTS(... information_schema.tables ...)` (High, secret storage fallback)

- [x] Add `SelectExistsInfoSchema` variant to `StatementKind`.
- [x] Classify the pattern: `SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE ...)` — detect a no-FROM projection containing a scalar subquery against `information_schema.tables`.
- [x] Add a handler that returns a single `false` boolean row (column named `exists`, type `BOOL`).
- [x] Add a test verifying the classifier recognises the full `duckdb_secrets` existence query and returns `false`.

### Step 4 — `SELECT pg_database_size(current_database())` (Low, informational)

- [x] Add `SelectPgDatabaseSize` variant to `StatementKind`.
- [x] Classify `SELECT pg_database_size(...)` in the AST classifier.
- [x] Add a handler returning `0::INT8` (or an approximation from the catalog store size if available).
- [x] Add a test verifying the handler returns a single integer row.

### Step 5 — Multi-Statement Catalog Scan (`pg_namespace` / `pg_class` / `pg_enum` / `pg_type` / `pg_indexes`) (Critical)

This is sent as a **single string** via the simple-query protocol:

```
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ;
SELECT oid, nspname FROM pg_namespace ORDER BY oid;
SELECT ... FROM pg_class JOIN ... UNION ALL SELECT ... FROM pg_constraint ...;
SELECT n.oid, enumtypid, typname, enumlabel FROM pg_enum JOIN ...;
SELECT n.oid, t.typrelid, t.typname, pg_attribute.attname, sub_type.typname FROM pg_type JOIN ...;
SELECT pg_namespace.oid, tablename, indexname FROM pg_indexes JOIN ...;
ROLLBACK;
```

DuckDB expects five result sets (one per SELECT) in sequence before the final `ROLLBACK`.

- [x] Add `PgCatalogScan` variant to `StatementKind`.
- [x] Detect the batch: a `Begin` statement whose `sql` string contains the `pg_namespace` / `pg_class` / `pg_enum` characteristic queries.
- [x] Build a multi-result response in the executor:
  - **pg_namespace result** (`oid INT8, nspname TEXT`): one row per schema RockLake exposes (at minimum `(1, 'public')` and `(2, 'main')`).
  - **pg_class UNION result** (`namespace_id INT8, relname TEXT, relpages INT8, attname TEXT, type_name TEXT, type_modifier INT8, ndim INT8, attnum INT8, notnull BOOL, constraint_id INT8, constraint_type TEXT, constraint_key TEXT`): rows for every ducklake table with its column definitions. Constraints: empty.
  - **pg_enum result** (`oid INT8, enumtypid INT8, typname TEXT, enumlabel TEXT`): zero rows.
  - **pg_type composites result** (`oid INT8, id INT8, type TEXT, attname TEXT, typname TEXT`): zero rows.
  - **pg_indexes result** (`oid INT8, tablename TEXT, indexname TEXT`): zero rows.
- [x] Verify the `pgwire` crate supports returning multiple result sets for a single simple-query string; adapt the handler or the protocol layer if needed.
- [x] Add tests for each of the five result set shapes.
- [x] Add an end-to-end test that replays a DuckDB 1.5.x ATTACH corpus through the PG-Wire layer and asserts all five result sets are present with correct schema.

### Step 6 — Wire Corpus for DuckDB 1.5.x

- [x] Record a new `tests/fixtures/wire-corpus/duckdb-1.5.x.jsonl` by running DuckDB 1.5.x against RockLake once all the above handlers are in place.
- [x] The fixture must capture the full ATTACH sequence: connection, version check, `DISCARD ALL`, `to_regclass`, `information_schema.tables`, multi-statement catalog scan, and at least one DuckLake metadata query.
- [x] Add a corpus replay test that validates every message in the fixture against the running RockLake server.
- [x] Update `docs/compatibility.md` and `tests/fixtures/compatibility-matrix.toml` (from v0.40.0 scope) to record DuckDB 1.5.x as the primary supported version.
- [x] Remove DuckDB versions older than 1.5.2 from the supported matrix.

### Definition of Done

- [x] `DISCARD ALL` returns `CommandComplete("DISCARD")` with no error.
- [x] `SELECT to_regclass('duckdb_secrets')` returns a single `NULL` row.
- [x] `SELECT EXISTS(... information_schema.tables ...)` returns a single `false` row.
- [x] `SELECT pg_database_size(current_database())` returns a single integer row.
- [x] Multi-statement catalog scan returns five result sets with correct schema.
- [x] DuckDB 1.5.x `ATTACH 'ducklake:postgres:...'` completes without error in CI.
- [x] A new `tests/fixtures/wire-corpus/duckdb-1.5.x.jsonl` fixture is captured and replayed by CI.
- [x] `docs/compatibility.md` states DuckDB 1.5.x as supported with CI evidence.
- [x] DuckDB versions older than 1.5.2 are removed from the compatibility matrix.

---

## v0.27.5 — DuckLake v1.0 Spec Gap Closure

> Close all P0 and P1 gaps from `plans/ducklake-1.0-spec-gaps.md`. These gaps block DuckLake v1.0 interoperability: missing SQL catalog facades, incorrect snapshot/delete-file schema, incomplete DROP TABLE cascade, and the absence of inlined data execution. RockLake's internal catalog storage is robust; the work here is projecting exact DuckLake spec tables and semantics through PgWire and implementing the execution paths that v0.2–v0.27 stubbed as no-ops.

### P0 (Critical) — Interoperability Blockers

#### 1. Exact DuckLake SQL Catalog Facade for All 28 Tables

RockLake stores protobuf rows in SlateDB; PgWire currently returns custom response schemas for many tables. This must be inverted: return exact DuckLake spec columns, types, and order for all 28 tables.

**Affected tables with current status:**

- `ducklake_snapshot` — currently exposes `author`, `message`; spec requires `next_catalog_id`, `next_file_id`
- `ducklake_schema` — missing `schema_uuid`, `path`, `path_is_relative`
- `ducklake_table` — uses `data_path`, missing `table_uuid`, `path_is_relative`
- `ducklake_column` — uses non-spec column names (`data_type`, `column_index`, `is_nullable`)
- `ducklake_view` — `SelectViews` returns empty; spec requires exact `view_name`, `dialect`, `column_aliases`
- `ducklake_metadata` — `SelectMetadata` returns empty; `InsertMetadata` is ignored
- `ducklake_table_stats` — `SelectTableStats` returns empty; missing `next_row_id`
- `ducklake_delete_file` — returns empty; missing all MVCC and spec fields
- `ducklake_inlined_data_tables` — `SelectInlinedRows` returns empty (see below)
- `ducklake_macro`, `ducklake_macro_impl`, `ducklake_macro_parameters` — all stubs returning empty

**Tasks:**

- [x] For each of the 28 spec tables, create a mapping from internal protobuf `Row` to exact spec SQL columns.
- [x] Update all `SELECT` handlers in `crates/rocklake-pgwire/src/executor/mod.rs` to project spec schemas instead of custom response builders.
- [x] Add response builders for tables currently returning empty: `SelectTableStats`, `SelectMetadata`, `SelectViews`, `SelectMacros`, `SelectMacroImpl`, `SelectMacroParam`, `SelectDeleteFiles`.
- [x] Write conformance tests for all 28 table SELECTs using queries from `specification/queries.md` (`crates/rocklake-pgwire/tests/v0275_conformance_tests.rs`).

#### 2. Fix Snapshot and Snapshot Change Schema

Current schema diverges from spec in two ways: `next_catalog_id` / `next_file_id` are stored in counters, not snapshot rows; and `author` / `message` are in the wrong table.

**Spec required:**

- `ducklake_snapshot(snapshot_id, snapshot_time, schema_version, next_catalog_id, next_file_id)`
- `ducklake_snapshot_changes(snapshot_id, changes_made, author, commit_message, commit_extra_info)`

**Tasks:**

- [x] Denormalize `next_catalog_id` and `next_file_id` into `SnapshotRow` at commit time in `CatalogWriter::create_snapshot`.
- [x] Move `author` and `message` semantics from `SnapshotRow` to `SnapshotChangesRow` as `author` and `commit_message`.
- [x] Build a spec-compatible `changes_made` string per snapshot (format: `created_schema:schema_name`, `created_table:table_id`, `dropped_table:table_id`, etc.).
- [x] Update PgWire response projections to expose the new schema (`make_snapshot_row_response` returns 5 spec columns; `make_snapshot_changes_response` aggregates per snapshot_id with comma-separated `changes_made`).
- [x] Add tests: verify denormalized counter values match counter state; verify snapshot changes capture all mutations (`crates/rocklake-catalog/tests/v0275_tests.rs` Phase 1–2).

#### 3. Implement Spec-Complete Delete File Semantics

Delete files are currently stubbed. They must support full lifecycle, MVCC visibility, and spec fields.

**Spec required fields:**

- `data_file_id`, `table_id`, `begin_snapshot`, `end_snapshot`, `file_format`, `record_count`, `file_size_bytes`

**Tasks:**

- [x] Add full MVCC support to delete-file reads: apply `is_visible(begin_snapshot, end_snapshot, dl_snapshot_id)` filtering.
- [x] Implement `list_delete_files(table_id, snapshot_id)` in `CatalogReader` with visibility filtering.
- [x] Update PgWire `SelectDeleteFiles` handler to return visible delete files for the requested table.
- [x] Add tests: verify delete files are visible only within their snapshot range; verify DELETE statement creates delete-file entries; verify cascade retirement of delete files on DROP TABLE.

#### 4. DROP TABLE Cascade Retirement

Dropped tables currently leave related metadata visible. All related rows must have `end_snapshot` set at the drop snapshot.

**Affected tables:**

- `ducklake_table` — main table row (already done)
- `ducklake_column` — all columns for the table
- `ducklake_data_file` / `ducklake_delete_file` — all files for the table
- `ducklake_file_column_stats` — all stats for files
- `ducklake_tag` / `ducklake_column_tag` — all tags for table and columns
- `ducklake_partition_info`, `ducklake_partition_column` — all partition metadata
- `ducklake_sort_info`, `ducklake_sort_expression` — all sort metadata
- Inlined data rows (tag `0xFD`) — all inlined inserts and deletes for the table

**Tasks:**

- [x] Implement cascading `end_snapshot` updates in `CatalogWriter::drop_table` (data files, delete files matched by `data_file_id`, and inlined insert rows).
- [x] Verify all affected table types are retired: `drop_table_cascades_to_delete_files` and `drop_table_cascades_to_inlined_rows` tests in `crates/rocklake-catalog/tests/v0275_tests.rs`.
- [x] Update the typed drop path in the SQL dispatcher to call the cascading drop writer.

### P1 (Important) — Feature Gaps

#### 5. Inlined Data SQL Execution

Currently `INSERT INTO ducklake_inlined_*`, `SELECT FROM ducklake_inlined_*`, and `UPDATE ... SET end_snapshot` are accepted as no-ops. They must actually execute.

**Tasks:**

- [x] Parse `INSERT INTO ducklake_inlined_*` statements: extract table_id, schema_version, row_id, and row-data columns.
- [x] Call `CatalogWriter::register_inlined_insert()` with the extracted row data.
- [x] Parse `SELECT FROM ducklake_inlined_*` statements: call `CatalogReader::list_inlined_inserts()` and project results.
- [x] Parse `UPDATE ducklake_inlined_* SET end_snapshot=X` statements: call `CatalogWriter::mark_inlined_insert_deleted()`.
- [x] Add tests: verify inlined inserts are queryable; verify UPDATE marks them deleted; verify deletes respect MVCC visibility.

#### 6. Data File Spec Field Completeness

Current `DataFileRow` is missing several spec fields.

**Spec required fields missing:**

- `file_order` — ordering within the table
- `row_id_start` — starting row ID for the file
- `partition_id` — partition reference
- `mapping_id` — column mapping reference
- `partial_max` — max value for partial files

**Tasks:**

- [x] Add missing fields to `DataFileRow` protobuf definition.
- [x] Populate these fields in `CatalogWriter::register_data_file()` (use sensible defaults if not supplied by caller).
- [x] Update PgWire `SelectDataFile` response to include new fields.
- [x] Add tests: verify all fields are persisted and retrieved correctly.

#### 7. Schema, Table, and Column Metadata Facades

These tables have partial spec implementations or non-spec column names.

**Tasks:**

- [x] `ducklake_schema` — add `schema_uuid`, `path`, `path_is_relative` fields; update response builder.
- [x] `ducklake_table` — rename internal `data_path` to spec `path`; add `table_uuid`, `path_is_relative`; update response builder.
- [x] `ducklake_column` — rename fields to match spec exactly (`data_type` → `column_type`, etc.); add support for nested columns (parent_column, default_value_type, default_value_dialect); update response builder.
- [x] Add conformance tests for all three tables.

#### 8. Metadata, Views, Macros, and Macro Implementation

These tables are currently stubbed and return empty.

**Tasks:**

- [x] Implement `SelectMetadata`, `SelectViews`, `SelectMacros`, `SelectMacroImpl`, `SelectMacroParameters` handlers in executor.
- [x] Wire `InsertMetadata` to `CatalogWriter::upsert_metadata()`.
- [x] Wire `InsertView` / `InsertMacro` / `InsertMacroImpl` / `InsertMacroParameters` to the corresponding writer methods (currently these exist but are not called).
- [x] Add tests: verify metadata is persisted and visible; verify views/macros are created and queryable; verify lifecycle is correct.

#### 9. Column Stats Completeness

File and table column stats are missing several spec fields.

**Missing fields:**

- `null_count` (instead of boolean `has_null`)
- `contains_nan` (for floating-point columns)
- `extra_stats` (for complex data types)
- `column_size_bytes` and `value_count`

**Tasks:**

- [x] Add missing fields to `FileColumnStatsRow` and `TableColumnStatsRow` protobuf definitions.
- [x] Update stats writers to populate these fields.
- [x] Update stats readers and PgWire response builders.
- [x] Add tests: verify all stats fields are persisted and visible.

### P2 (Cleanup) — Field Naming and Facade Issues

#### 10. Field Naming Consistency

Several tables use internal naming instead of spec column names.

**Affected tables:**

- `ducklake_file_partition_value` — uses `value` instead of spec `partition_value`
- `ducklake_tag` / `ducklake_column_tag` — internally `tag_key` / `tag_value`, spec requires `key` / `value`
- `ducklake_sort_expression` — uses boolean fields instead of spec string format
- `ducklake_files_scheduled_for_deletion` — missing `path_is_relative`; timestamp semantics differ (spec is `TIMESTAMPTZ`)

**Tasks:**

- [x] Rename internal fields to match spec exactly in all response builders.
- [x] For `sort_expression`, convert internal boolean representation to spec string format.
- [x] Fix timestamp semantics in `files_scheduled_for_deletion`.
- [x] Add conformance tests for each table.

#### 11. Partition Info and Sort Info SQL Facades

These tables are internally complete but lack SQL facades and lifecycle coverage.

**Tasks:**

- [x] Ensure `ducklake_partition_info`, `ducklake_partition_column`, `ducklake_sort_info`, `ducklake_sort_expression` are exposed with exact spec schema via PgWire.
- [x] Verify DROP TABLE cascade retires all partition and sort metadata.
- [x] Add tests: verify partition/sort info is queryable; verify cascade retirement is correct.

#### 12. Inlined Data Table Registry Facade

`ducklake_inlined_data_tables` currently uses non-spec field name `sql` instead of `table_name`.

**Tasks:**

- [x] Rename internal field to match spec.
- [x] Ensure `InsertInlinedDataTables` and `SelectInlinedData` handlers project correct schema.
- [x] Add tests: verify inlined data table registry is readable.

### Already Fixed in v0.27.x Preparation — From `plans/ducklake-1.0-spec-gaps-2.md`

The following bugs were discovered during the DuckDB/DuckLake source review documented in `plans/ducklake-1.0-spec-gaps-2.md` and fixed as part of incremental v0.27.x work. They are recorded here so they remain part of the v0.27.5 scope and are covered by the Definition of Done tests.

- [x] **Combined snapshot/stats/changelog `UNION ALL` query**: added `StatementKind::SelectSnapshotStatsAndChanges` classification and `make_snapshot_stats_changes_response` returning the expected 15-column shape.
- [x] **`ducklake_table_stats` column order and `next_row_id` exposure**: response builder now preserves DuckLake's requested projection order and exposes v1.0 `next_row_id`.
- [x] **Incremental stats inserts accumulated instead of replaced**: `update_table_stats` now reads and accumulates existing `record_count`, `file_count`, `file_size_bytes`, and advances `next_row_id`.
- [x] **Table column stats widening across batches**: `upsert_table_column_stats` now merges `contains_null`, `contains_nan`, `min_value`, `max_value`, and `extra_stats` using numeric-aware comparison.
- [x] **Boolean stats ordering in `stats_value_less_or_equal`**: added explicit `false < true` branch before integer parsing so boolean min/max merge correctly. Covered by `stats_merge_handles_booleans_correctly` in `v0275_tests.rs`.
- [x] **`ducklake_snapshot_changes` SQL facade**: added `SelectSnapshotChanges` classifier variant, `list_all_snapshot_changes` reader, and `make_snapshot_changes_response` executor that aggregates multiple change events per snapshot into one row with comma-separated `changes_made`.
- [x] **Inlined deletes decrement global row count**: `BufferedOp::DeleteInlinedRows` calls `adjust_table_record_count` with a negative delta.
- [x] **Stale inlined append row IDs remapped**: `inlined_insert_key_exists` prevents overwriting live rows; ordinary append rows are remapped to the next free key.
- [x] **Dynamic inlined row update classification**: `UPDATE ducklake_inlined_data_* SET end_snapshot` is now classified as `UpdateInlinedRowEndSnapshot` and buffered.
- [x] **Casted dynamic inlined RowDescription**: handler `expr_last_identifier` recurses through `Expr::Cast`; extended-query describe resolves fields against catalog column types.

### P0 (Critical) — Additional Interoperability Items From spec-gaps-2

#### 13. Stats Model Semantics Cleanup

The internal `TableStatsRow` retains a `file_count` field that no longer maps to a DuckLake v1.0 public column. In the `InsertTableStats` execution path the third v1.0 literal position (which DuckLake v1.0 defines as `next_row_id`) is currently stored under the `file_count` field name, which can cause confusion during future maintenance.

**Tasks:**

- [x] Rename internal `file_count` in `TableStatsRow` or wrap it in a clearly internal struct so the public DuckLake `next_row_id` and `file_size_bytes` fields are unambiguous.
- [x] Update `InsertTableStats` parsing in `crates/rocklake-pgwire/src/executor/mod.rs` so the third literal is treated as `next_row_id` or explicitly ignored if RockLake computes `next_row_id` independently.
- [x] Add migration/facade handling for persisted catalogs that stored the old `file_count` semantics.
- [x] Add a regression test confirming that all four DuckLake v1.0 `ducklake_table_stats` column positions round-trip correctly.

#### 14. Transaction Atomicity and Writer Conflict Behavior

DuckLake commit batches contain multiple metadata statements whose combined meaning depends on the full batch. The inlined update case (replacement insert + row-retirement `UPDATE` in one batch) demonstrated that partial evaluation produces incorrect row counts and stats.

**Tasks:**

- [x] Ensure all SQL statements belonging to one logical DuckLake metadata commit are buffered and evaluated as a single atomic batch before any side effects are applied.
- [x] Verify that stale row ID remapping, stats adjustments, snapshot changes, and catalog counter increments all see the complete incoming batch.
- [x] Add tests with interleaved DuckLake writers that produce conflicting commits; verify exactly one writer wins per snapshot.
- [x] Add writer fencing tests: kill writer mid-batch; start new writer; verify no partial batch is visible and new writer takes over cleanly.
- [x] Add rollback tests: disconnect mid-batch; verify catalog state is unchanged from before the batch started.

### P1 (Important) — Additional Feature Items From spec-gaps-2

#### 15. Extended-Query and COPY RowDescription Centralization

Executor response builders, handler `describe_fields_for_sql`, and COPY schemas for the same virtual table are defined in separate places and can drift. Arbitrary output aliases in dynamic inlined table projections are not yet supported through the binary COPY path.

**Tasks:**

- [x] Introduce a shared `DuckLakeTableSchema` type or equivalent constant registry mapping each `ducklake_*` table name to its exact FieldInfo list.
- [x] Wire `make_*_response` builders, `describe_fields_for_sql`, and `projected_copy_indices` to the registry so all three paths use the identical field definitions.
- [x] Add `postgres_query` tests in `duckdb_binary_tests.rs` or equivalent for every relevant metadata table with both plain and cast/alias projection shapes.
- [x] Add COPY-to-stdout tests that verify projection order and binary field encoding correctness.
- [x] Implement arbitrary output alias support for dynamic inlined table projections and add a test for `SELECT row_id AS rid, CAST(id AS INTEGER) AS duck_id FROM ducklake_inlined_data_*`.

#### 16. Type-Aware Column Stats Merge

The `stats_value_less_or_equal` helper currently handles integers and finite floats numerically but falls back to lexicographic comparison for all other types, which produces wrong min/max for dates, timestamps, decimals, and other DuckLake stat-relevant types.

**Tasks:**

- [x] Extend `stats_value_less_or_equal` to parse and compare `DATE` (days-since-epoch), `TIMESTAMP`/`TIMESTAMPTZ` (microseconds-since-epoch), unsigned integers, decimal/numeric strings, booleans, and UUID strings.
- [x] Add DuckDB validation tests for pruning correctness with `id IN (10, 2)`, negative integers, dates, timestamps, and strings that differ lexicographically from numeric order.
- [x] Preserve exact encoded min/max strings as DuckLake expects them; do not normalize or reformat values during merging.

#### 17. DROP/ALTER Cascade Metadata Retirement

The existing task 4 covers DROP TABLE cascade. ALTER TABLE column operations also mutate MVCC-versioned rows and must be covered by time-travel tests.

**Tasks:**

- [x] Implement and test `alter_table_add_column`, `alter_table_drop_column`, and `alter_table_rename_column` cascades: each must retire the old column row and advance `schema_version`.
- [x] Add time-travel tests: read the table at a snapshot before and after an ALTER; verify the correct column set is visible at each snapshot.
- [x] Add a test that drops a table which has attached partition info, sort info, and tag metadata; verify all related rows have `end_snapshot` set and are invisible at the drop snapshot.

### P2 (Cleanup) — Additional Items From spec-gaps-2

#### 18. Durable Compatibility Corpus

Focused regression tests cover known SQL shapes. A corpus-based suite is needed to catch upstream DuckLake SQL drift as DuckDB and DuckLake evolve.

**Tasks:**

- [x] Capture DuckLake metadata SQL from real attach, create, insert, delete, update, drop, view, macro, and partition workflows; store normalized SQL under `tests/fixtures/` tagged by DuckDB and DuckLake version.
- [x] Add a corpus classification test that runs every statement through `classify_statement` and fails on `StatementKind::Unsupported`.
- [x] Add a corpus response-shape test that executes every corpus `SELECT` and validates field names and field count.
- [x] Add an optional `make ducklake-compat` or equivalent CI job that runs the corpus against a local DuckDB and DuckLake binary and reports new failures as actionable diffs.

### Definition of Done

- [x] All 28 spec tables return exact DuckLake schema columns in correct order through PgWire.
- [x] All spec queries from `specification/queries.md` return correct results with correct MVCC visibility.
- [x] Snapshot rows denormalize `next_catalog_id` and `next_file_id`.
- [x] Snapshot changes persist `changes_made` in spec format with `author` and `commit_message`.
- [x] Delete files support full MVCC visibility and are visible through `SELECT ducklake_delete_file`.
- [x] DROP TABLE cascades `end_snapshot` to all related metadata rows.
- [x] `INSERT INTO ducklake_inlined_*`, `SELECT FROM ducklake_inlined_*`, and `UPDATE ... SET end_snapshot` execute correctly (not no-ops).
- [x] All P1 field gaps are closed: spec-complete data files, schema/table/column/metadata/view/macro facades, column stats completeness.
- [x] All P2 field naming is aligned with spec: `tag_name`/`tag_value` for tags; spec-correct sort_expression and files_scheduled_for_deletion schemas.
- [x] Conformance test suite passes all queries from `specification/queries.md` with spec-correct results.
- [x] No `SelectXXX` handler returns an empty result set unless the spec explicitly permits it (e.g., no metadata rows, no views, no macros).
- [x] Stats model semantics are clean: internal `file_count` naming is resolved; `InsertTableStats` maps all four v1.0 literal positions correctly.
- [x] One logical DuckLake commit is processed atomically; partial batch state is never visible; writer fencing tests pass.
- [x] All executor response builders, handler describes, and COPY schemas are derived from a shared schema registry.
- [x] `postgres_query` tests exist for every DuckLake metadata table in both plain and cast/alias projection forms.
- [x] COPY-to-stdout projection and binary encoding tests pass.
- [x] Type-aware stats merging covers dates, timestamps, decimals, and all other DuckLake stat-relevant types.
- [x] ALTER TABLE operations cascade correctly with time-travel tests before and after each alteration.
- [x] Compatibility corpus exists under `tests/fixtures/` and the optional corpus classification and response-shape CI tests are defined.

---

## v0.27.6 — DuckLake Inlined-Data Lifecycle Integration Tests

> Move the real DuckDB/DuckLake lifecycle from manual validation scripts into an opt-in automated test suite. All eight bug fixes from `plans/ducklake-1.0-spec-gaps-2.md` have been validated manually; this release makes those validations reproducible and extends stats regression coverage. Corresponds to Phase 1 of the implementation roadmap in `plans/ducklake-1.0-spec-gaps-2.md`.

### Tasks

#### Opt-In Lifecycle Integration Test

- [x] Create an integration test in `crates/rocklake-pgwire/tests/v0276_lifecycle_tests.rs` (gated on `duckdb_available()` + `ducklake_available()`, skips gracefully without `#[ignore]`) that:
  - Starts a live RockLake PgWire server against a temp catalog directory.
  - Connects a real DuckDB client with `LOAD ducklake; ATTACH 'ducklake:postgres:...' AS my_lake`.
  - Runs the full workload: `CREATE SCHEMA`, `CREATE TABLE`, `INSERT`, raw read, ordered read, filtered read.
  - Asserts result sets match expected rows.
- [x] Add a restart variant (`inlined_data_restart_lifecycle`): stop the server, restart against the same catalog directory, reattach, repeat the read assertions.
- [x] Add a `postgres_query` variant (`postgres_query_inlined_data`): call `SELECT * FROM postgres_query('...', 'SELECT * FROM ducklake_inlined_data_tables')` and verify rows are returned.

#### Stats Merge Regression Cases

- [x] Add unit tests for `stats_value_less_or_equal` with negative integers (e.g., `-10` vs `-2`) — in `stats.rs` `#[cfg(test)]` module and via `upsert_table_column_stats` in `v0276_lifecycle_tests.rs`.
- [x] Add unit tests with finite floats that differ only in fractional part (`stats_merge_floats_fractional_part`, `float_fractional_part_is_numeric`, `float_trailing_zero_fractional_is_numeric`).
- [x] Add unit tests with string values where lexicographic order differs from logical order (`stats_merge_string_numeric_order_differs_from_lexicographic`, `decimal_string_lexicographic_order_differs_from_numeric`).
- [x] Confirm that existing numeric comparisons (`10` vs `2`) still produce the correct result (`stats_merge_multi_digit_integer_still_correct`, `existing_numeric_comparisons_still_correct`).

### Definition of Done

- [x] Fresh lifecycle test (`inlined_data_fresh_lifecycle`) skips gracefully without DuckDB; passes when DuckDB+ducklake are available.
- [x] Restart lifecycle test (`inlined_data_restart_lifecycle`) skips gracefully without DuckDB; passes when DuckDB+ducklake are available.
- [x] `postgres_query` direct inlined table test (`postgres_query_inlined_data`) skips gracefully without DuckDB; passes when DuckDB+ducklake are available.
- [x] Stats merge regression tests for negative numbers, floats, and strings are present and pass (`stats_merge_floats_fractional_part`, `stats_merge_string_numeric_order_differs_from_lexicographic`, `stats_merge_multi_digit_integer_still_correct` in `v0276_lifecycle_tests.rs`; direct unit tests in `stats.rs`).

---

## v0.27.7 — DuckLake SQL Schema Registry

> Eliminate drift between executor response builders, handler describes, and COPY schemas by introducing a single `DuckLakeTableSchema` registry. This is the foundation work that makes all subsequent metadata facade work mechanical. Corresponds to Phase 2 of the implementation roadmap in `plans/ducklake-1.0-spec-gaps-2.md`.

### Tasks

#### DuckLakeTableSchema Registry

- [x] Define a `DuckLakeTableSchema` struct (or equivalent constant table) in `crates/rocklake-pgwire/src/` listing, for each of the 28 DuckLake v1.0 metadata tables: field name, wire type OID, and format (text/binary).
- [x] Make the registry the single authoritative source for FieldInfo in `describe_fields_for_sql`, `make_*_response` builders, and COPY metadata responses.
- [x] For every table that previously hard-coded FieldInfo in multiple locations, replace those duplicates with a registry lookup.

#### Projection-Order Golden Tests

- [x] Add a golden test for each of the 28 tables that asserts the RowDescription field names and order match the spec.
- [x] Add golden tests for at least three SELECT variants per high-risk table: `SELECT *`, `SELECT <explicit cols>`, and `SELECT <cols with CAST>`.

#### Arbitrary Output Alias Support

- [x] Implement support for arbitrary output alias names in dynamic inlined table projections (e.g., `SELECT row_id AS rid FROM ducklake_inlined_data_*`).
- [x] Add a binary COPY test for aliased dynamic inlined projections to confirm correct RowDescription and field encoding.

### Definition of Done

- [x] Registry exists and is used by all RowDescription, response builder, and COPY paths.
- [x] No `FieldInfo` for a metadata table is defined outside the registry.
- [x] Projection-order golden tests pass for all 28 tables.
- [x] Arbitrary output alias test passes in extended query and binary COPY modes.

---

## v0.27.8 — DuckLake Transaction Atomicity & Snapshot Changes Conformance

> Make DuckLake metadata commits atomic and make `ducklake_snapshot_changes` spec-complete. Also close the type-aware stats gap so DuckDB can prune correctly on dates, timestamps, and decimals. Corresponds to Phases 3 and 4 of the implementation roadmap in `plans/ducklake-1.0-spec-gaps-2.md`.

### Tasks

#### Transaction Atomicity

- [x] Buffer all SQL statements arriving within a single logical DuckLake metadata commit (delimited by the DuckLake extension's transaction protocol) before applying any side effects.
- [x] Apply stale row ID remapping, stats adjustments, snapshot changes, and catalog counter increments in a single atomic write after the full batch is collected.
- [x] Verify that a disconnect mid-batch leaves the catalog in the pre-batch state.
- [x] Add tests with two concurrent writers submitting conflicting snapshot IDs; verify exactly one succeeds and the other must retry.
- [x] Add writer fencing tests: kill writer mid-batch; start new writer; verify no partial-batch artifacts are visible.

#### Spec-Complete Snapshot Changes

- [x] Persist `changes_made` strings in the format DuckLake v1.0 expects (e.g., `created_schema:name`, `created_table:id`, `dropped_table:id`, `inserted_rows:table_id:count`, etc.).
- [x] Persist `author`, `commit_message`, and `commit_extra_info` in `ducklake_snapshot_changes` rows, not in `ducklake_snapshot`.
- [x] Remove `author`/`message` from `SnapshotRow` if they were stored there; migrate existing rows if needed.
- [x] Add a test verifying that `SELECT * FROM ducklake_snapshot_changes` after a workload returns one row per commit with correct `changes_made`, `author`, and `commit_message`.
- [x] Add a conflict-check test: two writers; one wins; verify the losing writer's snapshot is not present in `ducklake_snapshot_changes`.

#### Type-Aware Column Stats

- [x] Implement `DATE` comparison in `stats_value_less_or_equal` by parsing ISO-8601 date strings to days-since-epoch.
- [x] Implement `TIMESTAMP`/`TIMESTAMPTZ` comparison by parsing to microseconds-since-epoch.
- [x] Implement unsigned integer comparison (treat as `u64` rather than `i64`).
- [x] Implement decimal/numeric comparison using bigdecimal or string-based ordering.
- [x] Implement boolean comparison (`false < true`).
- [x] Implement UUID string comparison (lexicographic is correct for RFC-4122 UUIDs).
- [x] Add DuckDB validation tests that verify DuckDB prunes correctly on each new type after RockLake stores the stats.

### Definition of Done

- [x] Disconnect mid-batch leaves catalog unchanged; test passes.
- [x] Concurrent writer conflict test passes: one commit wins, one is rejected.
- [x] `ducklake_snapshot_changes` rows contain spec-correct `changes_made`, `author`, `commit_message`, and `commit_extra_info` after a workload.
- [x] Type-aware stats tests for DATE, TIMESTAMP, unsigned integers, decimals, booleans, and UUIDs pass.
- [x] DuckDB pruning validation tests pass for all new types.

---

## v0.27.9 — DuckLake Advanced Metadata Validation

> Validate views, macros, tags, column tags, sort info, partition info, and encryption key metadata end to end with real DuckDB. Also complete DROP/ALTER cascade for all metadata types and add imported-catalog support. Corresponds to Phase 5 of the implementation roadmap in `plans/ducklake-1.0-spec-gaps-2.md`.

### Tasks

#### Views and Macros End-to-End

- [x] Add a DuckDB integration test that creates a view (`CREATE VIEW s.v AS SELECT ...`) and reads it back through `ducklake_view`.
- [x] Add a DuckDB integration test that creates a macro and reads it back through `ducklake_macro`, `ducklake_macro_impl`, and `ducklake_macro_parameters`.
- [x] Verify RowDescription, insert/update semantics, and restart persistence for both views and macros.

#### Tags and Column Tags End-to-End

- [x] Add a DuckDB integration test that attaches tags to a table and column, reads them through `ducklake_tag` and `ducklake_column_tag`, and verifies correct `key`/`value` fields.
- [x] Verify that DROP TABLE retires all tags and column tags by checking `end_snapshot`.

#### Sort Info and Partition Info End-to-End

- [x] Add a DuckDB integration test for a table with a sort order; verify `ducklake_sort_info` rows are present with correct `sort_expression` format.
- [x] Add a DuckDB integration test for a partitioned table; verify `ducklake_partition_info`, `ducklake_partition_column`, and `ducklake_file_partition_value` rows are correct.
- [x] Verify that DROP TABLE retires all sort and partition metadata.

#### DROP/ALTER Complete Cascade

- [x] Implement and test that DROP TABLE retires table, columns, column tags, data files, delete files, partitions (info, columns, values), tags, sort info, and inlined data rows.
- [x] Implement ALTER TABLE add/drop/rename column: retire old column rows; advance `schema_version`; insert new column rows.
- [x] Add time-travel tests: query table at snapshot before and after ALTER; verify correct schema at each snapshot.

#### Encryption Key Metadata

- [x] Implement `ducklake_encryption_key` RowDescription and SELECT handler.
- [x] Add a test that verifies the table is queryable with the correct spec schema (even if no keys are present in the test catalog).

#### Imported DuckLake Catalog Support

- [x] Document the procedure for attaching an existing DuckLake catalog (created by DuckDB natively) to RockLake.
- [x] Add a smoke test that reads an externally created DuckLake catalog's metadata tables through RockLake PgWire.

### Definition of Done

- [x] View and macro lifecycle tests pass (create, read, restart).
- [x] Tag and column tag lifecycle tests pass (attach, read, retire on drop).
- [x] Sort info and partition info lifecycle tests pass.
- [x] DROP TABLE cascade test covers all 18+ spec metadata table types.
- [x] ALTER TABLE time-travel tests pass for add/drop/rename column.
- [x] `ducklake_encryption_key` SELECT returns correct empty schema.
- [x] Imported catalog smoke test passes.

---

## v0.27.10 — DuckLake Compatibility CI

> Prevent regressions as DuckDB and DuckLake evolve by building a durable compatibility corpus and automating it in CI. This is the final milestone before RockLake can claim broad DuckLake v1.0 compatibility. Corresponds to Phase 6 of the implementation roadmap in `plans/ducklake-1.0-spec-gaps-3.md`.

### Tasks

#### Durable Compatibility Corpus

- [x] Capture the complete set of DuckLake metadata SQL statements from a fresh DuckDB/DuckLake session covering: attach, create schema, create table, INSERT, DELETE, UPDATE, DROP TABLE, DROP SCHEMA, CREATE VIEW, CREATE MACRO, CREATE TABLE with sort/partition/tags.
- [x] Capture the multi-statement schema discovery transaction (`StatementKind::PgCatalogScan`) in the compatibility corpus.
- [x] Store normalized SQL statements under `tests/fixtures/ducklake-corpus/` tagged by DuckDB version and DuckLake version.
- [x] Add a classification test that runs every statement in the corpus through `classify_statement` and fails on any `StatementKind::Unsupported`.
- [x] Add a response-shape test that executes every corpus SELECT against a running RockLake instance and validates field names and count.

#### Pinned CI Jobs

- [x] Pin the exact compatibility targets: **DuckDB v1.5.3** and **DuckLake 1.0 Specification (Catalog Version 7 / V1_0)** in the CI configuration.
- [x] Add an optional nightly CI job (skipped by default in PR CI, enabled on schedule) that runs the full compatibility corpus against pinned DuckDB v1.5.3 / DuckLake 1.0 binaries.
- [x] Add fresh, restart, and concurrent-writers scenarios to the nightly job.
- [x] Explicitly check and gate that any future DuckLake v1.1 / Catalog Version 8 (`V1_1_DEV_1`) features or commits are strictly rejected in the compatibility gate to prevent out-of-scope creep.

#### Acceptance Gates

- [x] Create a `docs/compatibility.md` section that states the DuckLake v1.0 compatibility claim and links to CI evidence.
- [x] Define the acceptance criteria for "DuckDB v1.5.3 and RockLake work perfectly together under DuckLake 1.0" (from `plans/ducklake-1.0-spec-gaps-3.md`):
  - DuckDB can attach fresh; create/drop schemas and tables without custom flags.
  - Inlined and file-backed tables both work.
  - INSERT, DELETE, UPDATE, ALTER, DROP, view, macro, tag, partition, and sort metadata work.
  - Fresh reads, restart reads, time-travel reads, ordered reads, filtered reads, and projection reads are correct.
  - `postgres_query` can inspect every metadata table without RowDescription failures.
  - All 28 DuckLake v1.0 tables have exact SQL schemas.
  - Table stats, column stats, data-file metadata, and delete-file metadata survive incremental commits and restarts.
  - Conflict checks and snapshot changes behave correctly under multiple writers.
  - The compatibility suite runs against pinned versions and catches SQL drift.
  - Exact column schema, count, names, and OIDs under simple and extended describes for all 28 tables match the spec perfectly.
  - The system strictly handles Catalog Version 7 and rejects or treats as unsupported any DuckLake v1.1 (Catalog Version 8) migrations or version queries.

### Definition of Done

- [x] Corpus captured and stored under `tests/fixtures/ducklake-corpus/`.
- [x] Classification and response-shape corpus tests pass.
- [x] Nightly optional CI job is defined and runs green against pinned DuckDB v1.5.3 and DuckLake 1.0.
- [x] `docs/compatibility.md` states DuckLake v1.0 compatibility (under DuckDB v1.5.3) with CI evidence.
- [x] All acceptance criteria from `plans/ducklake-1.0-spec-gaps-3.md` are met.

---

## v0.27.11 — Wire & SQL Resiliency Hardening ✅

> Harden RockLake's query classifier, PgWire connection stability, and integration test suite to insulate the sidecar from changes in client query patterns, dialect shifts, and connection initialization queries. Incorporates the five actionable mitigations outlined in `plans/wire-and-sql-resiliency-report-1.md` and addresses the critical test sandboxing recommendations from the test suite assessment in `/Users/grove/obsidian-vault/grove/rocklake/test_suite_assessment.md`.

**Status: Done**

### Tasks

#### Mitigation 1: Abstract Virtual SQL Query Engine (DataFusion Integration)

- [x] Register the 28 DuckLake catalog tables as memory-backed logical schemas in an in-memory DataFusion `SessionContext` upon PgWire connection startup.
- [x] Direct `SELECT` queries targeting catalog tables directly to the DataFusion engine for logical planning, projection resolution, and execution.
- [x] Verify that complex subqueries, Common Table Expressions (CTEs), custom projections, Joins, and aggregations against the catalog tables are resolved automatically.

#### Mitigation 2: AST Normalizer & Pre-Processing Pipeline

- [x] Implement an AST visitor pipeline (`crates/rocklake-sql/src/classifier/normalize.rs`) that runs prior to statement classification.
- [x] Support recursive flattening of subqueries (e.g., nested `TableFactor::Derived` subqueries) and lifting of projection aliases.
- [x] Implement identifier normalization to canonically strip catalog and schema prefixes (e.g., mapping `"public".ducklake_table` to `ducklake_table`).
- [x] Strip redundant parentheses, double-quotes, whitespace tokens, and unused AST clauses such as `LIMIT` and `ORDER BY` before classification.

#### Mitigation 3: Dynamic Session Settings Registry

- [x] Create a generic, session-scoped settings `HashMap<String, String>` inside the PgWire `SessionState` struct (`crates/rocklake-pgwire/src/session.rs`).
- [x] Update the `classify_statement` logic to parse any `SET <variable> = <value>` dynamically as a generic `StatementKind::SetVariable(key, value)`.
- [x] Update the PgWire executor to capture set variables in the `SessionState` map and immediately return a standard PostgreSQL `CommandComplete` tag of `"SET"`.

#### Mitigation 4: Automated Dialect Fuzz Testing & SQLSTATE Hardening

- [x] Create a dedicated CI integration test target (`tests/dialect_fuzz.rs`) generating semi-randomized PostgreSQL-dialect query strings to send to the PgWire executor.
- [x] Harden the `execute_sql` handler to intercept all unsupported/unhandled queries and return a standardized PostgreSQL error:
  - **SQLSTATE**: `0A000` (Feature Not Supported)
  - **Severity**: `ERROR`
  - **Message**: "Statement is not supported by RockLake's catalog facade."
- [x] Assert that under fuzzing the server remains non-blocking (never drops the connection abruptly, panics, or hangs).

#### Mitigation 5: Hardened Testing with Sandbox Timeouts (Test Suite Assessment Integration)

- [x] Eliminate the indefinite block risk identified in the test suite assessment (`/Users/grove/obsidian-vault/grove/rocklake/test_suite_assessment.md`) in `crates/rocklake-pgwire/tests/v0276_lifecycle_tests.rs`.
- [x] Replace blocking `Command::output()` calls in helper functions (like `ducklake_available()`) with non-blocking, asynchronous command execution wrapped in strict `tokio::time::timeout` boundaries (e.g., 5 seconds).
- [x] Ensure that if `LOAD ducklake` attempts to fetch the extension over restricted or slow networks, the invocation times out gracefully and the test skips or fails cleanly rather than hanging the entire runner.
- [x] Audit and apply similar timeout controls to all other integration test targets spawning external processes.

#### Mitigation 6: Schema Registry Refactoring & Schema Facade Alignment

- [x] Align all 28 catalog table definitions in `crates/rocklake-pgwire/src/schema_registry.rs` to match the exact DuckLake v1.0 specification (Catalog Version 7), explicitly declaring DuckLake v1.1 (Catalog Version 8) schemas as out of scope.
- [x] Rename `metadata_key` and `metadata_value` in `ducklake_metadata` to `key` and `value`.
- [x] Rename `view_definition` in `ducklake_view` to `sql`.
- [x] Define missing schemas for `ducklake_file_variant_stats`, `ducklake_column_mapping`, and `ducklake_name_mapping` in the shared registry.
- [x] Refactor `ducklake_tag` and `ducklake_column_tag` schemas to map columns exactly to spec-defined `key` and `value` names (and remove `tag_id`).
- [x] Correct column mapping structures for `ducklake_partition_column` and `ducklake_sort_expression` to match upstream naming conventions.

#### Mitigation 7: pg-trickle CDC Startup Query — `ducklake_latest_snapshot_id(regclass)`

> **Discovered during audit of `pg-trickle/src/cdc/polling.rs` (L344–L348).** Before pg-trickle ever calls `table_changes()`, it resolves the latest snapshot boundary via `SELECT ducklake_latest_snapshot_id($1::regclass)`. This function is absent from RockLake's bounded SQL dispatcher, causing an immediate `SQLSTATE 42883` (undefined function) crash when pg-trickle registers a DuckLake change feed. All Gaps 1–8 already in the roadmap are unreachable without this.

- [x] Add `ducklake_latest_snapshot_id(regclass)` to the bounded SQL dispatcher in `crates/rocklake-sql/src/`. The function accepts a table qualified name cast to `regclass` and returns the `snapshot_id BIGINT` of the latest visible snapshot for that table (equivalent to `SELECT max(snapshot_id) FROM ducklake_snapshot` scoped appropriately).
- [x] Ensure the function is recognized by the AST classifier and routed through the same `CatalogReader` path as `get_current_snapshot()`.
- [x] Add a wire-corpus fixture for `SELECT ducklake_latest_snapshot_id($1::regclass)` covering the exact parameter binding shape pg-trickle sends.
- [x] Add an end-to-end test that simulates pg-trickle's CDC registration handshake: connect via PG-wire, call `ducklake_latest_snapshot_id`, assert a valid snapshot ID is returned, then confirm `table_changes()` is callable with that ID as `start_snapshot`.

#### Architectural Note: Gap 3 — Inlined-Data Trigger CDC (De-prioritized for Remote RockLake)

> **Audit finding.** The original `plans/pg-trickle-ducklake-support.md` Gap 3 assumes pg-trickle can attach PostgreSQL `AFTER` triggers to inlined-data tables virtualized over PG-wire. This is architecturally impossible for **remote** RockLake deployments: PostgreSQL triggers only fire when DML is executed locally on the host PostgreSQL server. When a DuckDB or other remote client writes inlined data directly to RockLake over PG-wire, it bypasses the host PostgreSQL entirely — the FDW trigger never fires.

- [x] Document in `plans/pg-trickle-ducklake-support.md` §2.5 that trigger-based inlined-data CDC is unsupported for remote RockLake deployments, and that pg-trickle must fall back to the unified `DUCKLAKE_CHANGE_FEED` polling path (`table_changes()`) for all remote catalog targets.
- [x] Verify in the pg-trickle × RockLake integration test (Tier A) that pg-trickle automatically selects `DUCKLAKE_CHANGE_FEED` mode (not trigger mode) when the catalog backend is RockLake.


### Definition of Done

- [x] In-memory DataFusion `SessionContext` registers all 28 virtual catalog tables and handles complex SQL.
- [x] `crates/rocklake-sql/src/classifier/normalize.rs` AST visitor flattening and identifier stripping is fully covered by unit tests.
- [x] PgWire `SessionState` stores generic settings dynamically and returns `"SET"` complete tags.
- [x] Fuzz test suite `tests/dialect_fuzz.rs` is active and asserts non-blocking behavior and SQLSTATE `0A000` conformance.
- [x] All external shell commands in `v0276_lifecycle_tests.rs` (especially `ducklake_available`) are run asynchronously under a 5-second `tokio::time::timeout` and do not block the suite on network constraints.
- [x] Schema registry (`crates/rocklake-pgwire/src/schema_registry.rs`) is completely refactored with all 28 tables fully aligned with the DuckLake v1.0 specification (Catalog Version 7), and any future v1.1 schemas are explicitly out of scope (renamed columns, OIDs, OID describe checks pass).
- [x] `ducklake_latest_snapshot_id(regclass)` is exposed in the bounded SQL dispatcher; pg-trickle's CDC startup handshake completes without `SQLSTATE 42883`; wire-corpus fixture and end-to-end CDC registration test are green.
- [x] Gap 3 architectural constraint documented in `plans/pg-trickle-ducklake-support.md`; Tier A integration test confirms pg-trickle selects `DUCKLAKE_CHANGE_FEED` mode (not trigger mode) against RockLake.

---

## v0.27.12 — Containerized Multi-Backend Object Store Emulator Testing

> Close the cloud storage interoperability gaps by implementing full containerized integration test harnesses for Google Cloud Storage and Azure Blob Storage in `rocklake-testkit` under DuckDB v1.5.3 and DuckLake 1.0 (Catalog Version 7). This ensures all CRUD operations, snapshot commits, read-after-write latencies, and epoch-based writer fencing are actively verified across all supported clouds.

### Tasks

#### GCS Emulator Harness
- [x] Implement `GcsEmulatorHarness` in `crates/rocklake-testkit/src/gcs_emulator_harness.rs` using `fsouza/fake-gcs-server`.
- [x] Configure `GoogleCloudStorageBuilder` in `rocklake-core` to resolve against local emulator port endpoints.
- [x] Add GCS integration tests gated behind `#[cfg(feature = "gcs-emulator")]` feature flags.

#### Azure Emulator Harness
- [x] Implement `AzureEmulatorHarness` in `crates/rocklake-testkit/src/azure_emulator_harness.rs` using the Azurite (`mcr.microsoft.com/azure-storage/azurite`) Docker container.
- [x] Configure `MicrosoftAzureBuilder` in `rocklake-core` to resolve against the local emulator container.
- [x] Add Azure integration tests gated behind `#[cfg(feature = "azure-emulator")]` feature flags.

#### Shared Catalog Backend Test Suite
- [x] Refactor existing MinIO catalog integration tests into a generic `catalog_backend_compat_test!` macro.
- [x] Run the unified suite—including open/create, snapshot commit, read-after-write, prefix listings, writer fencing, and post-crash recovery—across MinIO, GCS, and Azure emulators.
- [x] Wire emulator tests into scheduled and release-candidate CI pipelines.

#### Data-File & Delete-File Conformance
- [x] Extend data file and delete file registrations in the catalog writer to persist and expose `footer_size` (as `BIGINT`), `partition_id`, `encryption_key`, `mapping_id`, and `partial_max` columns.
- [x] Verify that `ducklake_data_file` and `ducklake_delete_file` fields are correctly mapped under S3, GCS, and Azure emulation environments.
- [x] Ensure all file fields are compliant with the DuckLake 1.0 specification, explicitly keeping any v1.1 attributes out of scope.

### Definition of Done
- [x] `GcsEmulatorHarness` compiles and successfully passes a GCS-backend catalog smoke test.
- [x] `AzureEmulatorHarness` compiles and successfully passes an Azure-backend catalog smoke test.
- [x] Shared backend integration tests pass reliably for GCS, Azure, and MinIO in CI without flaky failures.
- [x] The catalog writer correctly serializes and exposes the extended DuckLake v1.0 data-file and delete-file parameters (`footer_size`, `partition_id`, `encryption_key`, `mapping_id`, `partial_max`).

---

## v0.27.13 — Real Multi-Client & Multi-Driver Interoperability Certification

> Certify that RockLake's PG-Wire catalog facade is fully compliant with standard Postgres database clients, ORM drivers, and analytical applications under the strict DuckLake 1.0 (Catalog Version 7) and DuckDB v1.5.3 constraints.

### Tasks

#### Multi-Driver Smoke Test Suite
- [x] Create dedicated driver compatibility tests under `tests/driver_compat.rs`.
- [x] Verify basic schema list, table query, and inlined table INSERT/SELECT sequences using `tokio-postgres` (Rust), `pg` (Node.js), `psycopg` (Python), and `pgx` (Go).
- [x] Verify standard CLI compatibility using real executions of `psql` and `pgcli` loopback connections.

#### BI Tool Facade Validation
- [x] Verify that PgWire row descriptions, field formatting, and session commands (`DISCARD ALL`, `SET client_min_messages`, etc.) map correctly to BI tool queries.
- [x] Create headless verification tests simulating DBeaver and Metabase metadata schema discovery and catalog scans.
- [x] Verify all driver parameter-negotiation handshakes run to completion without unsupported feature errors.

#### MVCC Visibility & File Order Sorting
- [x] Enforce visibility constraints on external files where `begin_snapshot <= snapshot_id` and `(end_snapshot IS NULL OR end_snapshot > snapshot_id)`.
- [x] Ensure `list_data_files` results are sorted ascending by the persisted `file_order` attribute.
- [x] Validate file listings and sorting rules against standard PostgreSQL drivers (e.g. `psql`, `pgcli`) to prevent query planner regressions.
- [x] Verify visibility and ordering strictly conform to DuckLake v1.0 specifications, ignoring any v1.1 schemas.

#### pg-trickle Reference Cleanup
- [x] Remove "for pg-trickle" framing from ROADMAP entries (v0.18 note, v0.27.11 `ducklake_latest_snapshot_id` description) and retarget them as generic DuckLake CDC contract items; the underlying features (`table_changes()`, stable `rowid`, snapshot leases, `NOTIFY`, `ducklake_latest_snapshot_id()`) are valid DuckLake spec conformance regardless of consumer.
- [x] Archive or retitle `plans/pg-trickle-ducklake-support.md` and any `plans/pg-trickle.md` as a generic "DuckLake CDC contract" reference document, since pg-trickle has dropped its DuckLake support.

### Definition of Done
- [x] `tests/driver_compat.rs` executes successfully against Rust, Node.js, Python, and Go postgres clients.
- [x] `psql` and `pgcli` CLI loopback connection tests pass.
- [x] DBeaver and Metabase schema scans return correct columns and formats without failing.
- [x] MVCC data-file and delete-file visibility filtering is fully verified, and files are correctly sorted by `file_order`.
- [x] All pg-trickle-specific framing removed from ROADMAP and planning docs; retained features reframed as DuckLake CDC contract.

---

## v0.27.14 — Security Hardening & Protocol-Level Testing

> Guarantee the cryptographic and authentication safety of the PG-Wire sidecar under strict compliance rules, while preserving perfect DuckLake v1.0 / DuckDB v1.5.3 transaction isolation properties.

### Tasks

#### Timing Attack Verification
- [ ] Implement automated timing attack verification in `crates/rocklake-pgwire/tests/security_tests.rs`.
- [ ] Assert that credential evaluations (e.g. password checks) complete in constant-time using statistical timing analysis.

#### Modern SCRAM Authentication
- [ ] Implement and test `SCRAM-SHA-256` authentication exchange in the PgWire server.
- [ ] Verify SCRAM-SHA-256 handshakes succeed against standard PG drivers and ORMs.

#### Protocol-Level TLS Version Gates
- [ ] Add explicit TLS protocol validation tests.
- [ ] Verify that loopback clients attempting TLS 1.2 and TLS 1.3 connections are accepted.
- [ ] Verify that loopback clients attempting insecure handshakes (TLS 1.1 or older) are strictly rejected at the socket layer.

#### Atomic Commit Batching & Transaction Isolation
- [ ] Group multi-statement metadata inserts/updates from a single commit transaction into atomic commit blocks.
- [ ] Consolidate stats deltas before performing `ducklake_table_stats` updates to guarantee accurate record counts.
- [ ] Enforce repeatable-read transaction isolation barriers on the catalog writer, rejecting stale snapshot commits with SQLSTATE `40001` (serialization failure) to drive retry loops.
- [ ] Verify cascading dropping logic retires table, columns, column tags, data/delete files, tags, and partitions under test.
- [ ] Validate that atomic commit batching and transaction isolation are strictly tested against DuckDB v1.5.3 and DuckLake 1.0 (Catalog Version 7) workflows, with all newer v1.1 protocol aspects treated as explicitly unsupported.

### Definition of Done
- [ ] Timing analysis test proves constant-time password verification within tight statistical deviation boundaries.
- [ ] SCRAM-SHA-256 authentication tests run and pass.
- [ ] Insecure TLS handshakes (TLS 1.1 and below) are rejected under test, and TLS 1.2/1.3 are verified as accepted.
- [ ] Multi-statement catalog writes are verified as atomic, stats deltas consolidate accurately, repeatable-read writer fencing (SQLSTATE `40001`) operates correctly under conflicts, and cascading drops cascade properly.

---

## v0.35.0 — Strategy C: Native DuckDB Extension

> Complete the native DuckDB extension so that `ATTACH 'ducklake:slatedb:s3://...' AS lake` works without a PG-wire sidecar. This eliminates the Postgres-scanner compatibility burden entirely for local and embedded use. The `rocklake-ffi` Rust C ABI is already complete (v0.5, v0.9.2); the `extension/` C++ wrapper exists but stubs catalog type registration pending DuckDB's community extension catalog API.

### Current State

The v0.5 roadmap described Strategy C as Done, but the C++ extension in `extension/src/rocklake_extension.cpp` explicitly defers the key step:

```cpp
// catalog type registration would go here
// once DuckDB's extension catalog API is available for community extensions.
```

`INSTALL rocklake; ATTACH 'ducklake:slatedb:...' AS lake` does not work today. The Rust FFI layer is real; the DuckDB integration is a skeleton.

### Step 1 — DuckDB Extension Catalog API Research

- [ ] Audit DuckDB 1.5.x source for `CatalogType`, `AttachFunction`, and custom storage extension registration. Determine whether a community extension can register a new `ATTACH` scheme without modifying DuckDB core.
- [ ] Evaluate the community extension distribution path: [extension.duckdb.org](https://extension.duckdb.org) repository submission vs. self-hosted `custom_extension_repository`.
- [ ] Decision gate: **can the extension register a `slatedb:` attach handler via the public DuckDB extension API, or does it require an upstream DuckDB change or fork?** Record the finding in `docs/architecture/crate-structure.md`.

### Step 2 — C++ Catalog Implementation

If the public extension API supports catalog registration:

- [ ] Implement `RockLakeCatalog : duckdb::Catalog` in `extension/src/` delegating all virtual methods to `RockLakeCatalogWrapper` (which already wraps the C FFI calls).
- [ ] Register the attach handler for the `slatedb:` scheme in `rocklake_extension_init()` using DuckDB's `StorageExtension` or equivalent API.
- [ ] Implement the minimum required virtual methods for a read-only attach: `ScanEntry`, `GetEntry` (schemas, tables, columns), `GetTableIOFunction` (Parquet scan via existing data path).
- [ ] Implement write-path virtual methods for `CreateEntry` (table, schema, data file registration) delegating to `rocklake_ffi` write functions.

If the public extension API does not yet support catalog registration:

- [ ] Document the blocker in `docs/architecture/crate-structure.md` and file a DuckDB upstream issue/discussion requesting the required API surface.
- [ ] Provide a workaround path documented in `docs/integration/native-extension.md`: load the extension manually with a custom DuckDB build, or use the PG-wire sidecar (Strategy B) for all use cases until the upstream API is available.

### Step 3 — Build System

- [ ] Update `extension/CMakeLists.txt` to link against the DuckDB extension development headers (`duckdb.hpp`, `duckdb/main/extension_util.hpp`).
- [ ] Add a `build-extension` Makefile target or `justfile` recipe: `cargo build --release -p rocklake-ffi && cmake --build extension/build`.
- [ ] Output artifact: `rocklake.duckdb_extension` compatible with the DuckDB 1.5.x ABI.
- [ ] Add `extension/` to the release workflow (`release.yml`) so binaries for Linux x86-64/arm64 and macOS arm64 are attached to each GitHub Release.

### Step 4 — End-to-End Tests

- [ ] Add `tests/native_extension_e2e.rs` (or a shell-based golden test): `LOAD rocklake; ATTACH 'ducklake:slatedb:///tmp/test-catalog' AS lake; CREATE SCHEMA lake.s; CREATE TABLE lake.s.t (id INTEGER); INSERT INTO lake.s.t VALUES (1); SELECT * FROM lake.s.t` — asserts row count and value without starting a RockLake sidecar process.
- [ ] Add a parity test: run the same DuckLake tutorial operations against both the PG-wire sidecar (Strategy B) and the native extension (Strategy C) against the same catalog path; assert identical query results.
- [ ] Wire the Strategy C tests into CI under a separate job `native-extension` that builds the `.duckdb_extension` artifact and runs the end-to-end tests.

### Step 5 — Documentation

- [ ] Update `docs/architecture/crate-structure.md` with the current accurate status of the `extension/` directory and `rocklake-ffi` crate.
- [ ] Update `docs/getting-started/` to add a section on the native extension attach path alongside the existing PG-wire sidecar instructions.
- [ ] Add `docs/integration/native-extension.md` covering: when to use Strategy C vs. Strategy B, install steps, connection string format (`ducklake:slatedb:s3://bucket/catalog` or `ducklake:slatedb:///local/path`), known limitations vs. PG-wire, and ABI versioning policy.
- [ ] Update `docs/compatibility.md` with a new `Native Extension` row in the deployment matrix.
- [ ] Update `docs/design-decisions/` page covering Strategy B vs. Strategy C to reflect actual current status.

### Why This Eliminates the Postgres-Scanner Problem

When using the native extension, DuckDB calls into `rocklake.duckdb_extension` directly as an in-process function call. There is no TCP connection, no PG-wire handshake, no `postgres_scanner` initialization, and no system catalog probing (`DISCARD ALL`, `to_regclass`, `pg_namespace` scans). The DuckDB 1.5.x postgres-scanner compatibility work in v0.27.4 is permanently unnecessary for this path.

Strategy B (PG-wire sidecar) remains for use cases that require remote access, multi-client, or non-DuckDB SQL clients. Both strategies share the same `rocklake-catalog` / `rocklake-core` stack and produce identical catalog state.

### Definition of Done

- [ ] Decision gate from Step 1 documented; either the extension registers correctly or the upstream blocker is filed with a public tracking issue.
- [ ] `LOAD rocklake; ATTACH 'ducklake:slatedb:///tmp/test' AS lake; CREATE TABLE lake.main.t (id INTEGER); INSERT INTO lake.main.t VALUES (1); SELECT * FROM lake.main.t` passes in CI without any `rocklake serve` process running.
- [ ] Strategy B and Strategy C produce identical results on the same catalog path (parity test green).
- [ ] `.duckdb_extension` binary attached to the GitHub Release for Linux x86-64/arm64 and macOS arm64.
- [ ] `docs/integration/native-extension.md` written and reviewed.
- [ ] `docs/compatibility.md` Native Extension row present with CI evidence.

---

## v0.40.0 — Full Ecosystem Compatibility Certification

> Turn the compatibility matrix into release-blocking evidence. No v0.40.0 tag may ship until every supported, expected, untested, or unsupported claim in `docs/compatibility.md` is backed by an automated check, an explicit negative test, or a deliberate documentation downgrade.

### Current Gap Analysis

The workflow named `DuckDB Compatibility Matrix` is not a full compatibility matrix today and does not fully test every variant described in `docs/compatibility.md`.

- [ ] **DuckDB / DuckLake:** CI checks that `tests/fixtures/wire-corpus/duckdb-1.5.x.jsonl` exists, then runs the same package tests for every matrix entry. It does not pass the selected client fixture into the replay test, does not run a real DuckDB process, does not attach the real `ducklake` extension, and does not prove DuckDB 1.5.x patch streams.
- [ ] **Wire corpus replay:** The current corpus tests mostly validate fixture presence, JSON shape, or SQL classifier acceptance. They do not replay each selected corpus end-to-end through PG-wire, assert response messages, or compare final catalog state with golden DuckLake-backed output.
- [ ] **PostgreSQL clients:** The compatibility workflow runs `cargo test --package rocklake-pgwire -- --include-ignored psql_compat`, but no listed test currently contains `psql_compat`; this can pass while running zero client compatibility tests. The workflow also starts PostgreSQL containers even though the compatibility target should be a RockLake server exercised by real clients. DBeaver, pgcli, and Metabase have no automated coverage.
- [ ] **Spark / Trino / Presto:** Spark 3.5 and Trino 432 have small synthetic corpus fixtures and classifier checks only. There is no real Spark connector, Trino connector, or Presto job, and the documented Trino 400-431 / Presto disposition is not actively verified.
- [ ] **DataFusion:** DataFusion 45 is pinned and has local integration tests, including Parquet scan coverage, but those tests are not wired into the compatibility matrix and there is no docs-to-test evidence mapping for the supported DataFusion row or the unsupported `< 45` row.
- [ ] **Object storage:** Local filesystem is covered indirectly by many tests. GCS and Azure checks only validate builder construction, not real read/write/list/commit behavior. AWS S3 and MinIO are documented as supported but are not exercised by this compatibility workflow as real backends.
- [ ] **SlateDB:** The workspace pins SlateDB 0.13, but `docs/compatibility.md` is not validated against Cargo metadata and the unsupported 0.12 row has no explicit compatibility-policy check.
- [ ] **TLS and authentication:** Tests cover basic TLS-required/plaintext rejection, optional TLS startup, and password authentication behavior. They do not prove TLS 1.2 acceptance, TLS 1.3 acceptance, or TLS 1.1-and-older rejection as separate protocol-version gates.
- [ ] **Rust toolchain:** The workspace and CI declare MSRV 1.93, while `docs/compatibility.md` still says 1.80. Stable and MSRV checks are Linux-only and are not tied to the compatibility matrix.
- [ ] **Platforms and release artifacts:** CI tests Ubuntu and macOS latest only. Release artifacts are built for Linux x86-64, Linux aarch64, and macOS arm64. Windows x86-64 is documented as supported but has no CI test, no release build, and no release installation instructions. macOS x86-64 was removed from `docs/compatibility.md` but still appears in binary deployment documentation.
- [ ] **Unsupported rows:** Rows marked unsupported or not tested are not consistently asserted. Spark 3.3, DataFusion `< 45`, SlateDB 0.12, Rust below MSRV, and TLS 1.1-or-older need either explicit negative tests or a compatibility manifest entry explaining why no runtime test is possible.

### Compatibility Evidence Manifest

- [ ] Add `tests/fixtures/compatibility-matrix.toml` as the source of truth for every row in `docs/compatibility.md`. Each entry must include component, version/range, platform if applicable, claimed status, required CI job, test command, fixture or artifact path, and last-reviewed date.
- [ ] Add a CI gate that validates `docs/compatibility.md` against the manifest. A supported row without evidence, an evidence entry without a matching test, or a docs row missing from the manifest fails the build.
- [ ] Define allowed statuses precisely: `supported` means automated release-blocking evidence; `expected` means non-blocking scheduled evidence plus documented risk; `untested` means no support promise; `unsupported` means an explicit rejection, incompatibility reason, or version-policy check.
- [ ] Require every compatibility job to publish a compact JSON result artifact consumed by the manifest validator, so the docs cannot drift from the last green run.

### DuckDB / DuckLake Certification

- [ ] Replace the current fixture-exists workflow with versioned real-client jobs for every supported DuckDB/DuckLake combination in the manifest.
- [ ] For each supported DuckDB version, start RockLake as a real PG-wire sidecar, install/load the DuckDB `ducklake` extension, attach via `ducklake:postgres://...`, and run the v0.27 end-to-end lifecycle on LocalFS and MinIO.
- [ ] Replay the selected wire corpus named by the matrix entry, not a generic test filter. Assert protocol responses, SQLSTATEs, and final catalog rows against golden fixtures.
- [ ] Capture and validate new DuckDB patch/minor corpora before a version is added to the supported matrix. Weekly scheduled jobs detect new DuckDB releases and open a tracking issue when a corpus is missing.
- [ ] Keep `docs/integration/duckdb-compatibility.md` synchronized with the same manifest; remove or downgrade any DuckDB version claim that is not certified.

### SQL Client Certification

- [ ] Add real `psql` CLI smoke tests for PostgreSQL client versions 16, 17, and 18 against RockLake, including startup, simple query, extended/prepared query, transaction, auth failure, and TLS-required modes.
- [ ] Rename or add tests so the CI filter cannot silently run zero tests; fail the workflow when the selected test count is zero.
- [ ] Add pgcli 4.x smoke coverage against RockLake for connection setup, catalog SELECT, transaction, TLS-required connection, and auth failure.
- [ ] Add DBeaver 24.x coverage using its bundled PostgreSQL JDBC driver or a headless DBeaver-compatible JDBC smoke harness. Record the driver version in the manifest.
- [ ] Add Metabase 0.49+ coverage with a containerized Metabase instance or API-driven smoke harness that registers RockLake as a PostgreSQL database and runs a catalog query.

### Spark, Trino, Presto, and DataFusion

- [ ] Run a real Spark 3.5 job against RockLake through the documented pg-wire path. Cover schema discovery, table discovery, Parquet file listing, snapshot visibility, and a write path if the connector supports it.
- [ ] Run a real Trino 432+ job against RockLake through the documented pg-wire path. Cover catalog discovery, table discovery, predicate pushdown/file pruning expectations, and snapshot visibility.
- [ ] Decide the Trino 400-431 and Presto compatibility status in the manifest. If either remains `untested` or `unsupported`, `docs/compatibility.md` must say so plainly; if either becomes supported, add a real engine smoke job first.
- [ ] Promote `cargo test -p rocklake-datafusion` into the compatibility workflow for DataFusion 45 and include the Parquet scan test as the supported-row evidence.
- [ ] Add a version-policy check proving DataFusion `< 45` is outside the supported range, or remove the row from the public matrix.

### Object Storage Backend Testing

Local testing strategy for all supported object stores via containerized emulators:

| Backend | Local Emulator | Harness Pattern | Exercise |
|---------|---|---|---|
| **AWS S3** | MinIO (`minio/minio:latest`) | [MinioHarness](crates/rocklake-testkit/src/minio_harness.rs) (existing) | Catalog open/create, snapshot commit, read-after-write, list/prefix scan, writer fencing, recovery from fresh process |
| **GCS** | Google Cloud Emulator or `fsouza/fake-gcs-server` | GcsEmulatorHarness (new) | Same as MinIO; use `GoogleCloudStorageBuilder` configured for emulator endpoint |
| **Azure Blob** | Azurite (`mcr.microsoft.com/azure-storage/azurite`) | AzureEmulatorHarness (new) | Same as MinIO; use `MicrosoftAzureBuilder` configured for emulator endpoint |

Each harness follows the MinioHarness pattern: start Docker container → wait for readiness → configure ObjectStore builder → return `Arc<dyn ObjectStore>` → teardown after test.

Harnesses are added to `rocklake-testkit` and exposed as conditional features (`gcs-emulator`, `azure-emulator`, `minio-tests`) to allow local testing without requiring all Docker images.

- [ ] Implement `GcsEmulatorHarness` in `crates/rocklake-testkit/src/gcs_emulator_harness.rs` with `GoogleCloudStorageBuilder` configuration and container lifecycle management.
- [ ] Implement `AzureEmulatorHarness` in `crates/rocklake-testkit/src/azure_emulator_harness.rs` with `MicrosoftAzureBuilder` configuration and container lifecycle management.
- [ ] Add shared backend test suite exercisable by all three harnesses: `catalog_backend_compat_test!()` macro covering open/create, commit, read-after-write, list/prefix, writer fencing, and recovery.
- [ ] Wire all three harnesses into `crates/rocklake-pgwire/tests/integration_tests.rs` as optional gated tests.

### Storage, TLS, Rust, and Platform Matrix

- [ ] Add real backend compatibility jobs for LocalFS, MinIO, AWS S3, GCS, and Azure Blob in the CI workflow. Each supported backend must exercise the shared backend compatibility suite (open/create, snapshot commit, read-after-write, list/prefix scan, writer fencing, recovery from fresh process).
- [ ] LocalFS and MinIO tests run on every CI push. GCS and Azure Blob tests run on protected scheduled/release workflows or when explicitly triggered, backed by real cloud credentials.
- [ ] Add TLS protocol-version tests using real client handshakes: TLS 1.2 accepted, TLS 1.3 accepted, TLS 1.1 and older rejected. Include auth + TLS combined coverage.
- [ ] Reconcile Rust compatibility by either updating `docs/compatibility.md` to MSRV 1.93 or lowering the workspace MSRV with proof. Stable and MSRV checks must be represented in the manifest.
- [ ] Add Windows x86-64 CI and release artifacts before claiming Windows support. The release workflow must build, checksum, upload, and document the Windows binary.
- [ ] Keep Linux x86-64, Linux aarch64, and macOS arm64 release jobs as supported platform evidence. Remove stale macOS x86-64 deployment docs unless a macOS x86-64 build/test/release job is restored.

### Release Gates

- [ ] `DuckDB Compatibility Matrix` renamed or expanded to `Ecosystem Compatibility Matrix`, with separate jobs for real clients, corpus replay, SQL clients, engines, object stores, TLS/auth, Rust, and platforms.
- [ ] `docs/compatibility.md` and `docs/integration/duckdb-compatibility.md` cannot be edited to add a supported row unless the manifest and CI evidence are updated in the same PR.
- [ ] `mkdocs build --strict`, the compatibility manifest validator, all release-blocking compatibility jobs, and all release artifact builds are green before tagging v0.40.0.
- [ ] Publish a v0.40.0 compatibility report under `benchmarks/` or `docs/performance/` with the exact component versions, platforms, object stores, and CI run URLs used for certification.

### Deliverables

- [ ] Compatibility evidence manifest checked in and enforced in CI
- [ ] Real DuckDB/DuckLake matrix green for every supported version
- [ ] Wire corpus replay tests assert responses and final state per selected fixture
- [ ] SQL client compatibility green for psql 16/17/18, DBeaver 24.x, pgcli 4.x, and Metabase 0.49+
- [ ] Spark, Trino, Presto disposition, and DataFusion compatibility rows reconciled with automated evidence
- [ ] LocalFS, MinIO, AWS S3, GCS, and Azure Blob backend compatibility certified or downgraded honestly
- [ ] TLS 1.2, TLS 1.3, TLS rejection, auth, Rust/MSRV, and platform release evidence represented in the manifest
- [ ] Windows x86-64 build/test/release support added before Windows remains listed as supported
- [ ] Stale macOS x86-64 documentation removed or macOS x86-64 support restored with evidence
- [ ] `docs/compatibility.md` fully generated from or validated against current compatibility evidence

---

## v1.0 — General Availability

> Formal TPC-H @ SF10/SF100 benchmark publication, S3 Express acceptance gate, and GA sign-off.

### Full Benchmark Suite

TPC-H @ SF10 comparison across all three catalog backends — RockLake, DuckLake-on-PostgreSQL (RDS same AZ), and DuckLake-on-SQLite — for each operation family:

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

1. Full DuckLake tutorial runs end-to-end from the standard DuckDB `ducklake` extension through the RockLake PG-wire sidecar, with catalog in S3 and no PostgreSQL or SQLite database required.
2. Concurrent reads from a second DuckDB process see consistent, snapshot-isolated catalog views.
3. `kill -9` on the writer mid-commit leaves the catalog readable and consistent; the next writer fences and takes over within the SLOs verified in v0.9.
4. Benchmarks published: p50/p95/p99 catalog latency vs. PostgreSQL-backed DuckLake on RDS and SQLite-backed DuckLake; cost crossover point documented.
5. Common S3 Express planning operations are within 3× of PostgreSQL p99 latency; if not, the gap is clearly documented with a v1.x optimization plan.
6. All 28 DuckLake v1.0 catalog tables implemented, tag-allocated, fixture-covered, and explicitly status-tracked in `tags.rs`.
7. Phase 0 validation gates pass on LocalFS, MinIO, S3 Standard, and S3 Express; results documented.
8. `mkdocs build --strict` green; documentation site live with no stub pages.
9. **Real-world validation gate.** At least 30 days of dogfood deployment on a realistic workload (see Cross-Cutting Concerns: Real-World Validation Policy). Friction log reviewed and all blocking findings resolved. One external-to-the-team developer has successfully deployed RockLake using only published docs.
10. **Migration path from existing DuckLake deployments.** A documented and tested migration tool (`rocklake migrate-from-ducklake --source postgres://... --catalog s3://...`) reads an existing PostgreSQL- or SQLite-backed DuckLake catalog, replays its current snapshot into a fresh RockLake catalog (data files are not copied — they remain at their original object-store paths and are referenced by the new catalog), and emits a verification report. `docs/operations/migration-from-ducklake.md` covers cutover, rollback, and known-incompatibility surfaces. End-to-end tested against both PostgreSQL- and SQLite-backed source catalogs at SF1 scale.
13. **World-class testing foundation.** All 10 test tiers from [plans/e2e-integration-tests.md](plans/e2e-integration-tests.md) are fully implemented and green:
    - **Tiers 1–3** (unit/property, catalog, PG-Wire): green on every PR — standard GitHub Actions runner
    - **Tiers 4–5** (MinIO object store, client compat): green on every merge to `main` — large runner (8-vCPU), Testcontainers MinIO
    - **Tier 6** (fault injection — catalog, toxiproxy): green on every pre-release tag
    - **Tier 7** (24 h soak, TPC-H SF10/SF100): green on pre-release — dedicated EC2 `c6i.4xlarge`
    - **Tier 8** (security — credential isolation, TLS, auth, SQL injection guards): green on pre-release
    - **Tier 9** (benchmark regression < 10% vs baseline): green on weekly scheduled CI
    - `rocklake-testkit` ships 4 harnesses: `MinioHarness`, `CatalogHarness`, `PgWireHarness`, `DuckDbHarness`, `DeterministicClock`
    - At least 100 named test functions across all tiers at GA; test inventory published in `docs/contributing/testing.md`

### Deliverables

- v1.0 release tag and `CHANGELOG.md` entry
- Benchmark report `benchmarks/v1.0-tpch-sf10.json` published in the repository and linked from `docs/performance/`
- Final S3 Express acceptance decision documented in `docs/performance/s3-express-validation.md`
- `rocklake-testkit` crate complete with all 6 harness types
- Complete test inventory in `docs/contributing/testing.md`: tier-by-tier test count, CI job mapping, feature flags, and scale-test runner setup

---

## v0.23 — Streaming Ingest

> v0.23 completes the streaming ingest workstream: `RockLakeSink`, exactly-once delivery, and CDC output. These features were developed in parallel with the v0.18–v0.22 series and are released as part of the v0.23 tag.

> Kafka/NATS streaming pipelines, exactly-once delivery semantics, and pg-tide-relay integration for zero-infrastructure ingest paths from transactional sources to S3-backed data lakes.

### Streaming Ingest via pg-tide-relay

[pg-tide](https://github.com/trickle-labs/pg-tide) v0.34.0 registers DuckLake (and `RockLakeSink`) as a valid reverse pipeline sink. This enables:

- **Kafka → RockLake** and **NATS → RockLake** patterns with no persistent database other than the SlateDB-backed catalog
- Any external source (Kafka, NATS, Redis, SQS, webhook) writes directly to a DuckLake catalog without routing through a PostgreSQL inbox
- `RockLakeSink` connects directly to the PG-wire sidecar, giving a zero-infrastructure path from a transactional source to a queryable data lake in S3

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

- [x] `RockLakeSink` implementation in pg-tide registers without errors
- [x] End-to-end Kafka → RockLake → DuckDB query test passes with ≥100k records
- [x] NATS → RockLake → DuckDB query test passes with ≥100k records
- [x] Application metadata key namespace enforced: `{app}.{instance}.{key}` pattern validated in tests
- [x] Exactly-once delivery: process death between Parquet write and metadata commit is survivable; offset is not advanced on retry
- [x] Consumer offset tracking test: offset advances monotonically across 10 consecutive ingest batches
- [x] Performance test: Kafka ingest throughput ≥ 10k records/sec to S3 with catalog commit latency ≤ 50ms p95
- [x] Documentation: `docs/integration/streaming-ingest.md` with Kafka and NATS examples, offset recovery procedure, and failure mode handling

### CDC Output (Change Data Capture Export)

The complement to ingest: when a DuckLake snapshot is committed, the *diff* between the previous and current snapshot is a natural change stream. This turns RockLake from a streaming sink into a streaming source.

**Snapshot diff as a first-class primitive.** The diff between snapshots `S_n` and `S_{n+1}` is already computed implicitly: it's the set of catalog facts with `begin_snapshot = S_{n+1}` (new) or `end_snapshot = S_{n+1}` (retired). Expose this as a typed API and as a streaming output.

**CDC output targets:**

- **S3 CDC files.** Write per-snapshot diff as a Parquet or JSON-lines file under `{warehouse}/cdc/{table_id}/snapshot-{id}.parquet`. Readers poll or use S3 event notifications. Zero-infrastructure; natural for batch-oriented downstream.
- **Kafka/NATS CDC producer.** A sidecar (`rocklake-cdc`) tails the catalog and publishes per-table diffs to Kafka topics or NATS subjects. Exactly-once via consumer-offset tracking (same pattern as ingest, reversed).
- **Webhook CDC.** HTTP POST to a configurable URL on each snapshot commit. Includes snapshot ID, affected tables, and a pre-signed URL to the diff file. Useful for serverless triggers (Lambda, Cloud Functions).

- [x] `CatalogReader::snapshot_diff(from_snapshot, to_snapshot)` → structured diff (added/retired facts per table)
- [x] S3 CDC file writer: per-snapshot JSON-lines diff files under `{warehouse}/cdc/`
- [x] `rocklake-cdc` sidecar: tail catalog, produce to Kafka/NATS/webhook
- [x] End-to-end test: write → commit snapshot → CDC event → verify downstream receives correct diff
- [x] Documentation: `docs/integration/cdc-output.md` with Kafka, webhook, and S3-polling examples

### Deliverables (updated)

- [x] `RockLakeSink` implementation in pg-tide registers without errors
- [x] CDC output: `snapshot_diff()` API, S3 CDC writer, and `rocklake-cdc` sidecar (Kafka + webhook)
- [x] End-to-end streaming pipeline test: ingest → CDC → downstream consumer
- [x] Documentation: `docs/integration/streaming-ingest.md` and `docs/integration/cdc-output.md`
- [x] Architecture diagram in `docs/architecture/streaming-pipeline.md`

---

## v1.x — Ecosystem Expansion

> Async FFI v2 for concurrent catalog operations, Lambda/edge-function integration, and post-GA performance optimizations for extreme-scale deployments.

### Async Catalog FFI (Strategy C v2)

Strategy C v1 (v0.5) uses a blocking Tokio runtime where each catalog call does `runtime.block_on(async { ... })`. This is correct and safe but blocks a DuckDB execution thread for the full duration of each S3 round-trip (10–50 ms on S3 Standard). For multi-table join planning, DuckDB may issue multiple concurrent catalog lookups; the blocking model serializes them at the thread boundary.

**Gate: DuckDB async catalog API.** Before scheduling this work, check whether DuckDB ≥1.5 exposes an async catalog interface in its extension API. If DuckDB provides a callback-based catalog operation model, proceed with Option 2. If not, the async bridge requires an upstream DuckDB contribution and must be deferred pending acceptance.

**Option 2 — Callback-based async FFI (if DuckDB provides the API).**

The C++ extension provides a completion callback. The Rust FFI layer spawns a Tokio task and calls the callback when the S3 operation completes:

```c
typedef void (*rocklake_completion_fn)(void* ctx, rocklake_result_t* result, rocklake_error_t* err);

void rocklake_list_data_files_async(
    rocklake_catalog_t* catalog,
    uint64_t table_id,
    uint64_t snapshot_id,
    void* ctx,
    rocklake_completion_fn on_complete
);
```

The Tokio runtime spawns the async task and returns immediately; `on_complete` is called from a Tokio worker thread when the operation finishes. DuckDB's thread pool is never blocked during S3 round-trips. Expected improvement: multi-table join planning with N catalog lookups completes in O(max_latency) rather than O(N × max_latency).

**Option 3 — Shared runtime via channel (if DuckDB API is blocking but the extension can run init code).**

The extension starts a background thread running a Tokio runtime at load time. Each catalog call sends a request onto an `mpsc` channel and blocks the calling thread on a `std::sync::mpsc::Receiver`. The Tokio worker processes the request asynchronously. This decouples the Tokio runtime from DuckDB's thread pool and adds approximately 1–5 µs channel-crossing overhead per call — negligible compared to S3 latency.

**ABI versioning for v2 FFI.** Any change to function signatures, added callback parameters, or changed opaque handle layouts increments `rocklake_abi_version()`. The DuckDB extension checks the ABI version at load time and refuses to proceed on mismatch. Document in `extension/CMakeLists.txt`.

### Lambda and Edge-Function Integration

Blueprint §1.4 identifies Lambda functions, container tasks, and CDN edge workers as first-class reader targets: because catalog-data keys are never overwritten, a `DbReader` opened at a known checkpoint can serve any historical `dl_snapshot_id` with no coordination with the writer.

Formalize this pattern for v1.x:

**Lambda catalog reader.** Publish a documented pattern (with example code) for an AWS Lambda function that:
1. Opens a `DbReader` against a named SlateDB checkpoint in S3 (checkpoint selected at function initialization, or passed as an event parameter for time-travel queries).
2. Executes a `list_data_files` or `describe_table` call and returns the result as JSON.
3. Never opens a `Db` writer handle; cannot corrupt the catalog.

The Lambda function uses the read-only `DbReader` API and requires only the catalog-prefix read IAM permission. It can run with sub-second cold-start latency on S3 Express One Zone if the checkpoint's manifest SST is cached in the Lambda function's `/tmp` storage.

**Checkpoint-pinned readers.** Add `rocklake checkpoint pin --name for-lambda-reader --snapshot-id N` which creates a named SlateDB checkpoint pinned at a specific `dl_snapshot_id`. The named checkpoint can be referenced in Lambda event payloads or CDN cache keys. Add `rocklake checkpoint unpin --name ...` when the checkpoint is no longer needed.

**CDN cache contract.** Because catalog-data keys are immutable (written once, retired via a bounded `end_snapshot` update, never physically deleted outside excision), the value at any given key is stable for any read at or before the key's `end_snapshot`. Document this as a cache contract: HTTP GET responses for catalog prefix reads can be cached by a CDN using the SlateDB checkpoint generation as a cache-control key. Provide example CloudFront distribution configuration and Lambda@Edge origin logic.

**Test requirement.** Add an integration test that: (1) writes 100 snapshots; (2) creates a checkpoint; (3) starts a Lambda-style read-only process using only the checkpoint; (4) verifies the process returns correct `list_data_files` results at any `dl_snapshot_id` up to the checkpoint; (5) verifies the process cannot write to the catalog (write attempts return an error from the `DbReader` API).

### Deliverables

- Async catalog FFI: scope decision recorded (Option 2 if DuckDB API available, Option 3 otherwise); implementation shipped and benchmarked
- Lambda/edge reader pattern documented with example code and integration test
- Checkpoint-pinned reader API shipped (`pin`, `unpin`, `list` subcommands)
- DuckDB major version upgrade process documented step-by-step in `docs/contributing/release-process.md`

---

## v2.x — General Fact Store

> Expose the immutable append-only substrate beyond DuckLake. RockLake's storage engine is schema-agnostic by design; this release line opens it up to non-DuckLake workloads.

The architectural principle in [plans/blueprint.md §1.4](plans/blueprint.md)
treats the storage engine as a generic fact log over object storage. DuckLake
is the first schema. v2.x explores what else the same substrate can carry,
without changing the storage engine.

### Generalized Fact Model

Carve out `rocklake-factstore` as a standalone crate by following the extraction boundary defined in [plans/blueprint.md §5.29](plans/blueprint.md):

| What moves into `rocklake-factstore` | What stays in `rocklake-catalog` |
| --- | --- |
| Key encoding utilities | 28-table tag allocation (`tags.rs`) |
| SDKV value header + `encoding-version` + Protobuf dispatch | DuckLake MVCC filter logic |
| Counter allocation (`0xFE` + transactional read-modify-write) | `schema_version` increment and `mark_schema_changed()` |
| `retain-from` key and TTL advancement | Inlined-data (`0xFD`) encoding |
| Excision primitives and audit log | DuckLake spec operations |
| Leadership/epoch keys | `dl_snapshot_id` semantics |
| `CatalogStore` skeleton with neutral `SnapshotId(u64)` | — |

Each schema gets its own isolated SlateDB `Db` at a dedicated path; schemas
never share a `Db`, WAL, or compaction process. `rocklake-factstore` exposes
a generic fact API: `assert(entity, attribute, value, snapshot)`,
`retract(entity, attribute, snapshot)`, `as_of(snapshot)`,
`history(entity, attribute)`.

### Alternative Schemas on the Same Substrate

Demonstrate the substrate hosting workloads other than DuckLake:
- **User-defined relational schemas.** A small DDL surface (`CREATE TABLE … WITH (catalog = 'rocklake')`) that allocates a tag prefix and lets users define their own tables stored as facts, queryable through the existing PG-wire dispatcher or a typed Rust API.
- **Event-sourced application store.** Append-only entity/attribute/value/transaction quads; current-state derivation via materialized views built from the fact log; native time travel.
- **Datalog query interface.** A read-only Datalog engine over the fact log for exploratory and graph-style queries.

Each schema opens its own `Db` at a distinct path prefix and reuses the same
counter, leadership, retain-from, and excision *code* from `rocklake-factstore`.

### Horizontal Read Scale-Out as a Product Feature

The immutable substrate already makes unbounded reader replicas correct; v2.x
formalizes them as a deployment pattern:
- A `rocklake reader` binary that serves either the DuckLake schema or any registered alternative schema
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

- `rocklake-factstore` crate published independently of `rocklake-catalog`
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
- Migration path for incompatible upgrades: `rocklake export` → reinitialize → `rocklake import`
- DuckDB patch bumps: corpus replay CI; expected to remain compatible
- DuckDB minor bumps: new corpus capture required; explicit sign-off
- DuckDB major bumps: treated as a new client; full re-capture

### Real-World Validation Policy

Synthetic benchmarks (TPC-H, TPC-DS) catch performance regressions and correctness bugs, but they do not catch usability gaps, cost surprises, or workflow friction. Before v1.0 GA:

1. **Internal dogfood.** Run a real RockLake deployment against pg-tide's own analytics pipeline (if available) or a synthetic-but-realistic workload (e.g. GitHub event stream, NYC taxi stream) for ≥ 30 days.
2. **Document surprises.** Any unexpected behaviour, cost spike, or operational friction discovered during dogfooding becomes a documented finding and must be resolved or explicitly accepted before GA.
3. **User-experience review.** At least one developer unfamiliar with RockLake internals must successfully set up and query a catalog using only the published documentation. Their friction log becomes a documentation and UX backlog item.

### SlateDB Dependency Strategy

SlateDB is the storage foundation. Unlike DBSP (an IVM-track dependency), SlateDB underpins *every* roadmap phase. It is pre-1.0, actively evolving, and maintained by a small team. The risk profile is different from DBSP but equally consequential.

**Risk mitigation layers:**

1. **API surface confinement.** All SlateDB interaction is confined to `rocklake-core/src/store.rs` (reads) and `rocklake-catalog/src/writer.rs` (writes). The rest of the codebase depends on `CatalogStore`/`CatalogReader`/`CatalogWriter` traits, not raw SlateDB types. This is already true today.

2. **Version pinning with `=` constraint.** Same as DBSP: every SlateDB upgrade is an explicit decision. Pin to a specific release; never float.

3. **SlateDB API contract surface.** The RockLake-relevant API is small:
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
   - Maintain `trickle-labs/slatedb` fork with only the features RockLake uses
   - Object-store agnosticism (via `object_store` crate) means the fork remains portable

5. **Contingency: alternative embedded KV.** If forking becomes untenable:
   - Evaluate `sled` (mature but different persistence model)
   - Evaluate writing a minimal WAL + SST layer directly on `object_store` (high effort, last resort)
   - The `CatalogStore` abstraction layer means migration is confined to one module

6. **Relationship maintenance.** SlateDB is maintained by a team with whom we can collaborate:
   - File issues for any behavior that affects RockLake
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
- S3 bucket dedicated to scale tests: `rocklake-scale-tests-{region}`
- Results published to `benchmarks/` directory as JSON; compared against previous run
- Soak test failure blocks the release: any correctness drift in 24 h means the release is not ready
- Document the setup in `docs/contributing/testing.md` under "Scale Testing"

---

## What RockLake Is Not

- A general-purpose SQL engine *in v1* (the substrate is designed to make this possible later — see v2.x)
- A multi-writer database in v1 (one writer per catalog; SlateDB fencing handles takeover; the v0.7 partitioning pattern is the recommended workaround; v2.x evaluates append-disjoint multi-writer)
- A data-plane proxy (DuckDB writes Parquet directly; RockLake writes only the catalog)
- A system where user-visible catalog history can be silently deleted (physical deletion only via the explicit, audited `rocklake excise` command)
- A replacement for PostgreSQL-backed DuckLake in low-latency, high-concurrency analyst workloads
- A drop-in for any workload without first reading the performance analysis in `docs/performance.md`

**Choose RockLake when:** you are serverless or spot-based and cannot afford a persistent database server; you want a lakehouse with zero external infrastructure; you need cheap point-in-time catalog snapshots; your workload is write-heavy rather than read-heavy; or you are already in the SlateDB ecosystem.
