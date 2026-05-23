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
| **v1.0 — General Availability** | TPC-H @ SF10 benchmarks, GA polish, full operational story | Planning |
| **v1.x — Ecosystem Expansion** | Streaming ingest, additional DuckLake clients (Spark/Trino), zone-map index, async catalog FFI | Planning |
| **v2.x — General Fact Store** | Non-DuckLake schemas on the same immutable substrate; alternative query interfaces | Exploration |

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

## v1.0 — General Availability

> Full TPC-H @ SF10 benchmarks, GA polish, and full operational story.

### Full Benchmark Suite

TPC-H @ SF10 comparison: SlateDuck vs. DuckLake-on-PostgreSQL (RDS same AZ) vs. DuckLake-on-SQLite:
- `list_data_files` at 10⁴, 10⁵, 10⁶ files
- `create_snapshot` at 1, 10, 100 file additions
- Cold-start read latency from a fresh process
- Concurrent reader throughput
- p50/p95/p99 for all operations on S3 Standard and S3 Express One Zone

**Performance acceptance gate:** If common S3 Express planning operations exceed 3× PostgreSQL p99 latency after v0.7 optimization, document the gap clearly and defer production-readiness claim; correctness milestones may still ship as alpha/beta.

### Success Criteria

1. Full DuckLake tutorial runs end-to-end from DuckDB through SlateDuck with catalog in S3; no PostgreSQL or SQLite database required
2. Concurrent reads from a second DuckDB process see consistent, snapshot-isolated catalog views
3. `kill -9` on the writer mid-commit leaves the catalog readable and consistent; new writer fences and takes over
4. Benchmarks published: p50/p95/p99 catalog latency vs. PostgreSQL-backed DuckLake on RDS and SQLite-backed DuckLake
5. All 28 DuckLake v1.0 catalog tables implemented, tag-allocated, fixture-covered, and explicitly status-tracked
6. Phase 0 validation gates pass on LocalFS, MinIO, S3 Standard, and S3 Express
7. Writer failover completes within 30 seconds on S3 Standard, 10 seconds on S3 Express
8. IAM separation tested; expected failures return correct SQLSTATEs
9. All implementation-readiness artifacts from Phase 0 are checked in and referenced from CI

### Deliverables

- v1.0 release tag, changelog, migration guide
- `slateduck serve` and `slateduck` CLI with all maintenance commands production-ready
- Native DuckDB extension available via community extension repository
- Benchmark report published
- `docs/compatibility.md` with explicit version matrix

---

## v1.x — Ecosystem Expansion

> Streaming ingest, additional DuckLake clients, large-scale pruning, and async catalog FFI.

### Streaming Ingest via pg-tide-relay

[pg-tide](https://github.com/trickle-labs/pg-tide) v0.34.0 registers DuckLake (and `SlateDuckSink`) as a valid reverse pipeline sink. This enables:
- **Kafka → SlateDuck** and **NATS → SlateDuck** patterns with no persistent database other than the SlateDB-backed catalog
- Any external source (Kafka, NATS, Redis, SQS, webhook) writes directly to a DuckLake or SlateDuck catalog without routing through a PostgreSQL inbox
- `SlateDuckSink` connects directly to the PG-wire sidecar, giving a zero-infrastructure path from a PostgreSQL transaction to a queryable data lake in S3

The pg-tide-relay SQL corpus is already bounded by the patterns validated in v0.6 and v1.0 (no JOINs, CTEs, subqueries, or DDL).

### Additional DuckLake Clients

Using the established wire-corpus onboarding process (pg-tide-relay corpus established in v0.6):
- Spark-DuckLake
- Trino-DuckLake
- Any future catalog client that issues spec-compliant queries

Each client brings its own captured corpus; the bounded dispatcher grows only to the extent of category-a and category-b statement families.

### Coarse Zone-Map Index for Large-Scale Pruning

For tables with >1 million data files and 100+ columns:
- Add a coarse zone-map / interval index: `(table_id, column_id, stats_bucket, data_file_id)`
- Groups typed min/max ranges for approximate pruning before reading full stats rows
- Reduces the 67 MB per-column pruning scan to a much smaller pre-filtered candidate set
- This is a v1.x optimization target; correctness must be verified against exact min/max stats

### Async Catalog FFI (Strategy C v2)

If DuckDB exposes an async catalog extension API (check in Phase 0):
- Replace the blocking Tokio runtime bridge with a callback-based async FFI
- Each catalog call spawns a Tokio task; DuckDB's thread pool is not blocked during S3 round-trips
- Expected to improve multi-table join planning latency by eliminating serialization at thread boundaries

### Deliverables

- pg-tide-relay `SlateDuckSink` passing full acceptance test suite
- At least one additional client (Spark or Trino) with full corpus coverage in CI

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
