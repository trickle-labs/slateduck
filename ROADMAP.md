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
| **v0.9.4 — GA Ready** | Concurrent reads, zone-map (conditional), Spark/Trino clients, DataFusion scan/pg-wire, virtual catalog SQL, test coverage, CI gates, docs complete, versioning policy, release automation | Done |
| **v1.0 — General Availability** | TPC-H @ SF10/SF100 benchmarks, S3 Express acceptance gate, GA sign-off | Planning |
| **v0.10.0 — Streaming Ingest** | pg-tide-relay integration, Kafka/NATS support, exactly-once delivery semantics, metadata key namespacing | Planning |
| **v1.x — Ecosystem Expansion** | Async FFI v2, Lambda integration, additional performance optimizations | Planning |
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

### Deliverables

- v1.0 release tag and `CHANGELOG.md` entry
- Benchmark report `benchmarks/v1.0-tpch-sf10.json` published in the repository and linked from `docs/performance/`
- Final S3 Express acceptance decision documented in `docs/performance/s3-express-validation.md`

---

## v0.10.0 — Streaming Ingest

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

- [ ] `SlateDuckSink` implementation in pg-tide registers without errors
- [ ] End-to-end Kafka → SlateDuck → DuckDB query test passes with ≥100k records
- [ ] NATS → SlateDuck → DuckDB query test passes with ≥100k records
- [ ] Application metadata key namespace enforced: `{app}.{instance}.{key}` pattern validated in tests
- [ ] Exactly-once delivery: process death between Parquet write and metadata commit is survivable; offset is not advanced on retry
- [ ] Consumer offset tracking test: offset advances monotonically across 10 consecutive ingest batches
- [ ] Performance test: Kafka ingest throughput ≥ 10k records/sec to S3 with catalog commit latency ≤ 50ms p95
- [ ] Documentation: `docs/integration/streaming-ingest.md` with Kafka and NATS examples, offset recovery procedure, and failure mode handling

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

---

## What SlateDuck Is Not

- A general-purpose SQL engine *in v1* (the substrate is designed to make this possible later — see v2.x)
- A multi-writer database in v1 (one writer per catalog; SlateDB fencing handles takeover; the v0.7 partitioning pattern is the recommended workaround; v2.x evaluates append-disjoint multi-writer)
- A data-plane proxy (DuckDB writes Parquet directly; SlateDuck writes only the catalog)
- A system where user-visible catalog history can be silently deleted (physical deletion only via the explicit, audited `slateduck excise` command)
- A replacement for PostgreSQL-backed DuckLake in low-latency, high-concurrency analyst workloads
- A drop-in for any workload without first reading the performance analysis in `docs/performance.md`

**Choose SlateDuck when:** you are serverless or spot-based and cannot afford a persistent database server; you want a lakehouse with zero external infrastructure; you need cheap point-in-time catalog snapshots; your workload is write-heavy rather than read-heavy; or you are already in the SlateDB ecosystem.
