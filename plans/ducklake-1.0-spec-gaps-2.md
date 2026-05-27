# DuckLake 1.0 / DuckDB Compatibility Review

Date: 2026-05-26

Reviewed sources:

- DuckDB source tree: `../duckdb`
- DuckLake source tree: `../ducklake`
- Rocklake workspace: `crates/rocklake-core`, `crates/rocklake-catalog`, `crates/rocklake-sql`, `crates/rocklake-pgwire`

Primary upstream file used for current DuckLake request behavior:

- `../ducklake/src/storage/ducklake_metadata_manager.cpp`

Key upstream areas inspected:

- DuckLake metadata table declarations around the `ducklake_table_stats`, `ducklake_table_column_stats`, `ducklake_snapshot`, `ducklake_snapshot_changes`, and dynamic inlined data table definitions.
- `GetSnapshotAndStatsAndChanges`, which sends a single `UNION ALL` query combining latest snapshot, snapshot changes, table stats, and table column stats.
- `UpdateGlobalTableStats`, which can write stats with `INSERT` or `UPDATE` depending on DuckLake's cached view of metadata state.
- `WriteNewInlinedData`, which writes rows into dynamic `ducklake_inlined_data_<table_id>_<schema_version>` tables.
- `DuckLakeTransaction::GetNewDataFiles`, which uses global table stats, especially `record_count` and `next_row_id`, to assign row IDs and drive read planning.

## Executive Summary

The current DuckDB DuckLake happy path now works against Rocklake for the real inlined-data lifecycle that exposed the regressions in this review:

1. Attach DuckLake through the PostgreSQL metadata manager.
2. Create schema and table.
3. Insert initial rows.
4. Delete a row.
5. Append a row after DuckLake reuses a stale emitted row ID.
6. Update an existing row through DuckLake's insert-plus-retire inlined-row pattern.
7. Read the table raw, ordered, and filtered.
8. Restart Rocklake and repeat the raw, ordered, filtered, stats, and direct dynamic inlined table reads.

The major bugs found during this review were fixed in code rather than only documented:

- DuckLake's combined snapshot/stats/changelog `UNION ALL` conflict query is classified and answered with the expected 15-column shape.
- `ducklake_table_stats` responses preserve DuckLake's requested projection order and expose v1.0 `next_row_id` semantics.
- Incremental DuckLake stats inserts are merged into global stats instead of replacing earlier stats.
- Table column stats are widened across incremental batches, including numeric-aware min/max merging.
- Inlined row deletes and updates now adjust global record counts.
- Dynamic inlined row updates are classified and buffered.
- Stale ordinary append row IDs are remapped to the next free key instead of overwriting live rows.
- Same-transaction inlined row replacements still preserve DuckLake's update behavior.
- Extended-query descriptions can now describe dynamic `ducklake_inlined_data_*` tables, including casted projections used through DuckDB `postgres_query`.

The broader conclusion is more cautious: Rocklake is now much closer to DuckDB/DuckLake compatibility for current inlined-data workflows, but full "works perfectly together" compatibility still requires completing the SQL catalog facade for all DuckLake v1.0 tables, especially data files, delete files, advanced metadata, and conformance testing for larger non-inlined data paths.

## Compatibility Status After This Review

### Works in real DuckDB/DuckLake validation

- DuckLake attach through Rocklake's PgWire endpoint.
- Creating a schema and a simple table.
- Inlined initial insert.
- Inlined delete.
- Inlined append after stale row ID emission.
- Inlined update represented as replacement insert plus old-row retirement.
- Fresh reads:
  - `SELECT rowid, * FROM s.t`
  - `SELECT rowid, * FROM s.t ORDER BY id`
  - `SELECT rowid, * FROM s.t WHERE id = 1`
  - `SELECT rowid, * FROM s.t WHERE id = 3`
  - `SELECT rowid, * FROM s.t WHERE id = 4`
- Restart reads for the same statements.
- Direct stats inspection through DuckDB's postgres scanner:
  - `SELECT table_id, record_count, next_row_id, file_size_bytes FROM public.ducklake_table_stats`
- Direct dynamic inlined table query through DuckDB's postgres scanner:
  - `SELECT row_id, CAST(id AS INTEGER) AS id FROM public.ducklake_inlined_data_3_3 ORDER BY row_id`

Validated final visible rows:

| rowid | id | name |
|---:|---:|---|
| 0 | 1 | one |
| 2 | 3 | THREE |
| 3 | 4 | four |

Validated final persisted table stats:

| table_id | record_count | next_row_id | file_size_bytes |
|---:|---:|---:|---:|
| 3 | 3 | 5 | 0 |

### Still partial or not yet proven

- Large data-file based DuckLake tables, where metadata rows in `ducklake_data_file`, `ducklake_delete_file`, and `ducklake_file_column_stats` must drive scans.
- Full DuckLake v1.0 SQL catalog facade for every metadata table and every column.
- Complex stats types beyond integer/float/string min/max comparisons.
- Full DROP TABLE cascading metadata retirement.
- Views, macros, tags, sort info, column tags, partition info, partition column mapping, file partition values, file sort orders, encryption, and snapshots from imported DuckLake catalogs.
- Multi-client concurrency, writer fencing, and conflict/retry behavior under real DuckDB workloads.
- Complete CI coverage against a pinned DuckDB + DuckLake binary matrix.

## Upstream Request Matrix

This matrix describes request families observed or inferred from current DuckDB/DuckLake code paths and how Rocklake now responds.

| Request family | Upstream purpose | Current Rocklake status | Notes |
|---|---|---|---|
| Startup/auth/session parameters | Allow DuckDB postgres scanner and DuckLake metadata manager to open a PostgreSQL connection | Handled for current tests | The server advertises enough PostgreSQL compatibility for DuckDB to connect. |
| `SELECT version()` / version checks | Client compatibility probing | Handled | Includes RDS-style version check variant. |
| `DISCARD ALL`, `DISCARD PLANS`, `DISCARD SEQUENCES`, `DISCARD TEMP` | DuckDB postgres scanner session cleanup | Handled | Regression tests cover execution tags and no-error behavior. |
| `to_regclass(...)` | DuckDB extension introspection | Handled | Returns null where expected. |
| `pg_database_size(...)` | Scanner metadata request | Handled | Returns integer. |
| `information_schema` existence probes | Scanner metadata request | Handled | Covered by regression tests. |
| `pg_catalog.pg_namespace`, `pg_class`, `pg_attribute`, `pg_type` scans | DuckDB relation discovery | Handled for current corpus | Empty result sets still need correct schemas. |
| Binary `COPY` from DuckDB | Bootstrap/import metadata transfer | Partially handled | Current copy parser handles known DuckLake bootstrap tables; needs full table coverage. |
| Binary `COPY TO STDOUT` for catalog reads | DuckDB postgres scanner table reads | Partially handled | Projection and RowDescription correctness are critical. Inlined dynamic reads now validate. |
| Latest snapshot reads | DuckLake metadata attach and conflict checks | Handled for current shapes | Includes latest tuple and max snapshot variants. |
| Combined snapshot/stats/changes `UNION ALL` | DuckLake conflict detection and cached global stats refresh | Handled now | New 15-column response builder answers the exact current upstream shape. |
| `ducklake_schema` reads/writes | Schema catalog | Mostly handled for current create/read paths | Exact full v1.0 facade should still be audited. |
| `ducklake_table` reads/writes | Table catalog | Mostly handled for current create/read paths | DROP/rename and complete end-snapshot behavior need stronger coverage. |
| `ducklake_column` reads/writes | Column catalog | Mostly handled for current primitive columns | Nested columns/default expressions need more coverage. |
| `ducklake_schema_versions` | DuckLake schema version metadata | Handled for current reads/writes | Regression coverage exists through classifier/executor paths. |
| `ducklake_inlined_data_tables` | Maps logical tables to dynamic inlined row tables | Handled | Current tests and real validation use this path. |
| Dynamic `ducklake_inlined_data_<table_id>_<schema_version>` inserts | Store small data inline in metadata DB | Handled and validated | Payload stores only user columns after `row_id`, `begin_snapshot`, `end_snapshot`. |
| Dynamic inlined row `UPDATE ... SET end_snapshot` | Retire inlined rows for DELETE/UPDATE | Handled now | Classified as `UpdateInlinedRowEndSnapshot` and buffered as `DeleteInlinedRows`. |
| Dynamic inlined row `SELECT` | DuckLake reads inline data through postgres scanner | Handled for current shape | Extended describe now supports dynamic table fields and casted projections. |
| `ducklake_table_stats` insert/update/select | Global row counts, next row IDs, pruning, read planning | Handled for current shape | Stats are accumulated and deletes decrement record count. Internal naming still deserves cleanup. |
| `ducklake_table_column_stats` insert/update/select | Global column min/max/null/NaN stats | Handled for current inlined path | Merge now widens stats and compares numeric strings numerically when possible. |
| `ducklake_file_column_stats` read/write | Per-file stats for pruning | Partially handled | Current empty/projection response bugs were fixed earlier, but full data-file semantics need more tests. |
| `ducklake_data_file` read/write | External object/data-file metadata | Partial | Required for larger/non-inlined tables. Needs exact v1.0 semantics and restart tests. |
| `ducklake_delete_file` read/write | Merge-on-read deletes over data files | Partial | Not proven with real data-file DELETE/UPDATE workloads. |
| `ducklake_snapshot_changes` | Commit metadata and conflict summaries | Partial | Current conflict query receives a compatible response, but persisted spec-complete `changes_made` is still not complete. |
| Views, macros, macro impls, macro parameters | SQL object metadata | Partial | Classifier/reader/writer support exists in places; needs end-to-end DuckDB validation. |
| Tags and column tags | DuckLake labels/properties | Partial | Reads/writes need conformance tests. |
| Sort info and file sort orders | Ordering metadata | Partial | Needs exact table facade and scanner tests. |
| Partition info, partition columns, file partition values | Partitioned table metadata | Not complete | Required for full DuckLake v1.0. |
| Encryption key metadata | Encrypted data file support | Not complete | Can be deferred unless encrypted DuckLake tables are in scope. |

## Bugs Found And Fixed

### 1. Combined snapshot/stats/changelog query returned the wrong behavior

DuckLake issues a conflict/read-state query that unions latest snapshot metadata with global table stats and table column stats. Without a dedicated path, Rocklake either failed to classify it correctly or returned the wrong schema.

Fix:

- Added `StatementKind::SelectSnapshotStatsAndChanges` classification for the characteristic `UNION ALL` query shape.
- Added `make_snapshot_stats_changes_response` with the expected 15 columns:
  - `snapshot_id`
  - `schema_version`
  - `next_catalog_id`
  - `next_file_id`
  - `changes`
  - `table_id`
  - `column_id`
  - `record_count`
  - `next_row_id`
  - `file_size_bytes`
  - `contains_null`
  - `contains_nan`
  - `min_value`
  - `max_value`
  - `extra_stats`

Coverage:

- `classify_ducklake_snapshot_stats_changes_union`
- `ducklake_snapshot_stats_changes_union_has_expected_shape`

### 2. `ducklake_table_stats` column order and shape were v1.0-incompatible

DuckLake v1.0 expects:

`table_id, record_count, next_row_id, file_size_bytes`

DuckDB can request these columns in a different projection order, for example:

`table_id, record_count, file_size_bytes, next_row_id`

Fix:

- Added projection-aware table stats response building.
- Preserved requested field order.
- Exposed `next_row_id` from the stored stats row.

Coverage:

- `ducklake_table_stats_preserves_requested_projection_order`
- Real DuckDB `postgres_query` over `ducklake_table_stats` after restart.

### 3. Incremental stats inserts replaced global stats

DuckLake can emit repeated `INSERT INTO ducklake_table_stats` statements for incremental batches. Rocklake was replacing the global row, so after delete/append/update the table stats could become stale or too narrow. The most visible failure was restart behavior: raw reads were correct, but filtered and ordered reads could return wrong rows because DuckLake used persisted global stats for pruning and planning.

Fix:

- `update_table_stats` now reads existing stats and accumulates:
  - `record_count`
  - internal `file_count`
  - `file_size_bytes`
- `next_row_id` advances by the incoming batch `record_count`.
- `adjust_table_record_count` applies positive or negative deltas without changing `next_row_id`.

Coverage:

- `table_stats_merge_incremental_inlined_batches`
- Fresh and restart real DuckDB lifecycle reads.
- Persisted stats inspection after restart.

### 4. Table column stats were overwritten instead of widened

Global table column stats must represent the live table, not only the last incoming batch.

Fix:

- `upsert_table_column_stats` now merges with existing stats:
  - `contains_null` uses boolean OR.
  - `contains_nan` uses boolean OR when present.
  - `min_value` keeps the global minimum.
  - `max_value` keeps the global maximum.
  - `extra_stats` keeps the incoming value when present, otherwise existing.
- Numeric-looking min/max values are compared numerically when both values parse as integers or finite floats; otherwise comparison falls back to string order.

Coverage:

- `table_stats_merge_incremental_inlined_batches`, including a regression case for `10` vs `2`.
- Real restart validation shows `id` stats persisted as `min_value = 1`, `max_value = 4`.

### 5. Inlined deletes did not adjust global row count

DuckLake's inlined DELETE is represented as an update of `end_snapshot` on rows in the dynamic inlined table. Rocklake was retiring rows but leaving table stats too high or too stale.

Fix:

- `BufferedOp::DeleteInlinedRows` now subtracts deleted row count from table stats through `adjust_table_record_count`.
- Same-transaction update replacement still subtracts the retired row while the replacement insert stats add the replacement row, preserving net row count.

Coverage:

- Fresh and restart lifecycle after `DELETE id = 2` and `UPDATE id = 3` validates final `record_count = 3`.

### 6. Stale inlined append row IDs could overwrite live rows

DuckLake sometimes emitted an ordinary append row with `row_id = 0` after prior rows already existed. Rocklake previously trusted the incoming key and overwrote the live row, causing missing data.

Fix:

- Added `inlined_insert_key_exists` to check staged and existing inlined keys.
- During commit, ordinary inlined inserts are remapped to the next free row ID if the incoming ID is already reserved or persisted.
- Inserts that are also retired in the same logical DuckLake update preserve their incoming row ID to retain update replacement semantics.

Coverage:

- `inlined_append_with_stale_row_id_is_remapped_to_free_key`
- Real lifecycle preserves row IDs `0`, `2`, and remapped append `3`.

### 7. Dynamic inlined row updates were not classified

DuckLake represents row retirement with an `UPDATE ducklake_inlined_data_<table_id>_<schema_version> SET end_snapshot = ... WHERE row_id IN (...)`. This was not treated as an inlined-row delete/update.

Fix:

- Added classifier fast path for `UPDATE` against `ducklake_inlined_data_` tables.
- Executor parses retired row IDs and buffers `DeleteInlinedRows`.

Coverage:

- `classify_dynamic_inlined_row_update`
- Real `DELETE` and `UPDATE` lifecycle.

### 8. Casted dynamic inlined projections could fail field description

DuckDB's `postgres_query` path needs a valid RowDescription before it reads rows. A casted dynamic inlined query such as `SELECT row_id, CAST(id AS INTEGER) AS id ...` could fail with no fields returned.

Fix:

- Handler projection parsing now recurses through `Expr::Cast` and `Expr::Nested`.
- Extended-query describe now looks up the dynamic inlined table's logical columns from the catalog and returns field metadata for `row_id`, `begin_snapshot`, `end_snapshot`, and user columns.

Coverage:

- Real DuckDB direct validation:
  - `SELECT * FROM postgres_query('pg', 'SELECT row_id, CAST(id AS INTEGER) AS id FROM public.ducklake_inlined_data_3_3 ORDER BY row_id')`

## Changed Rocklake Areas

### SQL classifier

Files:

- `crates/rocklake-sql/src/classifier/mod.rs`
- `crates/rocklake-sql/src/classifier/ast.rs`
- `crates/rocklake-sql/src/classifier/table_selects.rs`

Current important behavior:

- Detects DuckLake's combined snapshot/stats/changes query before AST classification.
- Detects dynamic inlined row `UPDATE` statements.
- Classifies DuckLake v1.0 stats, schema version, dynamic inlined rows, and metadata table reads used by current DuckDB paths.

### Catalog writer and reader

Files:

- `crates/rocklake-catalog/src/writer/mod.rs`
- `crates/rocklake-catalog/src/writer/stats.rs`
- `crates/rocklake-catalog/src/reader.rs`

Current important behavior:

- Supports checking inlined row key existence before commit.
- Accumulates table stats and advances `next_row_id`.
- Adjusts table record counts for deletes.
- Merges table column stats across incremental batches.
- Reads all table stats and all table column stats for DuckLake conflict/cache refresh queries.

### PgWire executor

Files:

- `crates/rocklake-pgwire/src/executor/mod.rs`
- `crates/rocklake-pgwire/src/executor/catalog.rs`
- `crates/rocklake-pgwire/src/session.rs`

Current important behavior:

- Buffers inlined row deletes.
- Remaps stale inlined append IDs.
- Preserves update replacement behavior.
- Responds to combined snapshot/stats/changes query.
- Responds to table stats queries with projection-sensitive column order.
- Responds to joined table/table-column stats queries.
- Responds to dynamic inlined row reads with correct schema and values.

### PgWire handler

File:

- `crates/rocklake-pgwire/src/handler.rs`

Current important behavior:

- Returns RowDescription for more DuckLake metadata requests.
- Resolves casted projection names.
- Describes dynamic inlined row tables from catalog schema during extended query describe.

## Validation Performed

### Focused Rust tests

Command:

`cargo test -p rocklake-sql --test v0274_classifier_tests`

Result:

- 25 passed.

Command:

`cargo test -p rocklake-pgwire --test v0274_postgres_scanner_tests`

Result:

- 14 passed.

Command:

`cargo test -p rocklake-pgwire --test duckdb_binary_tests`

Result:

- 2 passed.
- 1 ignored: `duckdb_attach_full_lifecycle`, which is intentionally ignored because it requires external DuckLake setup and can be slow/flaky in CI.

### Build

Command:

`cargo build -p rocklake-pgwire`

Result:

- Succeeded.

### Fresh real DuckDB/DuckLake lifecycle

Server:

`./target/debug/rocklake serve --catalog /tmp/rocklake-ducklake-review-catalog --bind 127.0.0.1:15434`

DuckDB attach:

`LOAD ducklake; ATTACH 'ducklake:postgres:host=127.0.0.1 port=15434 dbname=rocklake' AS my_lake (DATA_PATH '/tmp/rocklake-ducklake-review-data');`

Workload:

- `CREATE SCHEMA s`
- `CREATE TABLE s.t(id INTEGER, name VARCHAR)`
- `INSERT INTO s.t VALUES (1, 'one'), (2, 'two'), (3, 'three')`
- `DELETE FROM s.t WHERE id = 2`
- `INSERT INTO s.t VALUES (4, 'four')`
- `UPDATE s.t SET name = 'THREE' WHERE id = 3`
- Raw, ordered, and filtered reads.
- Direct dynamic inlined table query via `postgres_query`.

Result:

- All reads returned expected rows.
- Dynamic inlined direct query returned `row_id/id` pairs `0/1`, `2/3`, `3/4`.
- Arbitrary aliasing of dynamic inlined projections, such as renaming `row_id` to a different output name during binary COPY, is not claimed as supported by this validation and should be covered by the broader COPY/projection work below.

### Restart real DuckDB/DuckLake lifecycle

Procedure:

1. Stop Rocklake.
2. Restart Rocklake against the same catalog directory.
3. Reattach DuckLake with the same data path.
4. Repeat raw, ordered, filtered, stats, and direct dynamic inlined table reads.

Result:

- All reads returned expected rows after restart.
- Stats persisted as `record_count = 3`, `next_row_id = 5`, `file_size_bytes = 0`.
- Direct dynamic inlined table query still returned `row_id/id` pairs `0/1`, `2/3`, `3/4`.
- The direct dynamic inlined validation used source-matching output names; arbitrary projection aliases remain part of the extended RowDescription/COPY coverage gap.

### Trace cleanup

Checked for temporary trace/debug output in Rust source:

`rg -n "TRACE-|\[TRACE|eprintln!\(\"\[TRACE" crates`

Result:

- No matches.

## Remaining Gaps To Reach Full "Perfect Together" Compatibility

These are not the bugs that blocked the validated inlined-data lifecycle. They are the remaining work needed before Rocklake can claim broad DuckDB/DuckLake compatibility across the full v1.0 spec and larger workloads.

### P0. Complete exact SQL catalog facade for all DuckLake v1.0 tables

DuckLake treats the metadata database as a SQL catalog with specific tables, columns, types, and field order. Rocklake stores catalog facts as key/value rows and exposes selected virtual result sets through PgWire. That architecture can work, but every public SQL response must match DuckLake exactly.

Required work:

- Create a table-by-table schema contract for all DuckLake v1.0 tables.
- For every `ducklake_*` table, add:
  - exact RowDescription fields,
  - exact projection handling,
  - exact value encoding,
  - empty result schemas,
  - read filters used by DuckLake,
  - insert/update/delete classification.
- Add golden tests from DuckLake's actual generated SQL.

High-risk tables still needing deeper conformance:

- `ducklake_data_file`
- `ducklake_delete_file`
- `ducklake_file_column_stats`
- `ducklake_partition_info`
- `ducklake_partition_column`
- `ducklake_file_partition_value`
- `ducklake_file_order`
- `ducklake_file_column_mapping`
- `ducklake_encryption_key`
- `ducklake_snapshot_changes`
- `ducklake_view`
- `ducklake_macro`
- `ducklake_macro_impl`
- `ducklake_macro_parameters`
- `ducklake_tag`
- `ducklake_column_tag`
- `ducklake_sort_info`

### P0. Finish data-file based table support

The validated workload used inlined data. DuckLake will use file-backed metadata for larger data and normal lakehouse operation. File-backed correctness depends on exact data-file and delete-file semantics.

Required work:

- Store and expose all v1.0 `ducklake_data_file` fields:
  - `data_file_id`
  - `table_id`
  - `begin_snapshot`
  - `end_snapshot`
  - `file_order`
  - `path`
  - `path_is_relative`
  - `file_format`
  - `record_count`
  - `file_size_bytes`
  - `footer_size`
  - `row_id_start`
  - `partition_id`
  - `encryption_key`
  - `mapping_id`
  - `partial_max`
- Store and expose all v1.0 `ducklake_delete_file` fields:
  - `delete_file_id`
  - `table_id`
  - `begin_snapshot`
  - `end_snapshot`
  - `data_file_id`
  - `path`
  - `path_is_relative`
  - `format`
  - `delete_count`
  - `file_size_bytes`
  - `footer_size`
  - `encryption_key`
  - `partial_max`
- Implement MVCC visibility over data files and delete files by requested snapshot.
- Ensure `row_id_start` is assigned from pre-increment `next_row_id`.
- Ensure reads are ordered by DuckLake's file ordering rules.
- Add real DuckDB tests that force file-backed storage rather than inlined rows.

### P0. Clean up stats model semantics

The public response now exposes DuckLake v1.0 shape, and `next_row_id` works for the validated path. Internally, however, there is still legacy naming such as `file_count` in the table stats flow. That field is no longer a DuckLake v1.0 public column and can confuse future maintenance.

Required work:

- Rename or wrap internal table stats fields so the code distinguishes clearly between:
  - public DuckLake `next_row_id`,
  - internal file counts if Rocklake wants to keep them,
  - public `file_size_bytes`.
- Update `InsertTableStats` parsing so the third v1.0 literal is named and treated as `next_row_id` or ignored deliberately if Rocklake computes `next_row_id` itself.
- Add migration/facade handling for old stats rows if persisted catalogs need backward compatibility.

### P0. Make snapshot changes spec-complete

Current conflict/query behavior is compatible enough for the validated path, but the full `ducklake_snapshot_changes` table still needs spec-complete persistence.

Required work:

- Persist `changes_made` strings as DuckLake expects.
- Persist `author`, `commit_message`, and `commit_extra_info` in the correct table.
- Keep current structured change rows only as internal extensions if needed.
- Test conflict checks across concurrent or sequential writers.

### P0. Prove transaction atomicity and writer conflict behavior

DuckLake commit batches can contain multiple metadata statements whose net meaning depends on their combination. The inlined update case already demonstrated this: an inserted replacement and old-row retirement must be interpreted together.

Required work:

- Treat one DuckLake metadata commit as one atomic operation even when the SQL arrives as several statements.
- Ensure replacement detection, stale row ID remapping, stats changes, snapshot changes, and catalog counters are evaluated against the full logical batch.
- Add tests with interleaved writers and retry/conflict behavior.
- Validate writer fencing and transaction rollback semantics.

### P1. Complete extended-query and COPY RowDescription coverage

DuckDB is strict about field descriptions, even for empty result sets. The current inlined dynamic table and current metadata paths are improved, but every virtual table must be described consistently in simple query, extended query, and COPY modes.

Required work:

- Centralize schema definitions so executor responses and handler descriptions cannot drift.
- Add tests that call DuckDB `postgres_query` for every relevant metadata table and selected cast/projection variants.
- Add COPY-to-stdout tests for projection order and binary encoding.
- Add explicit coverage for arbitrary output aliases in dynamic inlined table projections before declaring alias support.

### P1. Expand type-aware stats handling

Current stats merging compares integers and finite floats numerically and falls back to string order. This is enough to fix the observed numeric regression, but DuckLake stats can cover more logical types.

Required work:

- Merge stats using column logical type when available.
- Add support for dates, timestamps, decimals, unsigned integers, booleans, UUIDs, and binary/text distinctions.
- Preserve exact encoded min/max values expected by DuckLake.
- Add DuckDB validation for pruning with values such as `10`, `2`, negative numbers, dates, timestamps, and strings.

### P1. Complete DROP/ALTER metadata retirement

DROP and ALTER operations need cascading `end_snapshot` updates across all affected metadata tables.

Required work:

- DROP TABLE should retire table, columns, column tags, data files, delete files, partitions, tags, sort info, and related rows as DuckLake specifies.
- ALTER TABLE should update schemas, columns, schema versions, tags, and dependent metadata consistently.
- Add time-travel tests before and after DROP/ALTER.

### P1. Validate views, macros, tags, and sort metadata end to end

Rocklake has partial classifier/storage support for several advanced metadata tables, but real DuckDB/DuckLake validation is still needed.

Required work:

- Add real DuckDB tests for views and macros.
- Add tag and column tag tests.
- Add sort order tests.
- Confirm RowDescription, insert/update semantics, and restart persistence.

### P2. Build a durable compatibility corpus

The current work added focused tests for the bugs found. The next step is a durable corpus that can catch upstream DuckLake SQL drift.

Required work:

- Capture DuckLake metadata SQL from real attach/create/insert/delete/update/drop/view/macro/partition workflows.
- Store normalized SQL fixtures by DuckDB/DuckLake version.
- Test classification and response shape for each statement.
- Run a nightly or optional CI job against local DuckDB and DuckLake binaries.

## Implementation Roadmap

### Phase 1: Stabilize current inlined-data compatibility

Status: mostly done by this review.

Remaining tasks:

- Move the real DuckDB/DuckLake lifecycle out of manual validation and into an opt-in integration test that can be run locally.
- Add a restart variant of the lifecycle test.
- Add a direct `postgres_query` dynamic inlined table test.
- Add more stats merge cases for negative numbers, floats, and strings.

### Phase 2: Centralize DuckLake table schemas

Goal: eliminate drift between classifier, executor response builders, handler descriptions, and COPY metadata.

Tasks:

- Add a `DuckLakeTableSchema` registry for all v1.0 metadata tables.
- Generate or share FieldInfo definitions from the registry.
- Use the registry for empty responses, projected responses, extended describe, and COPY output.
- Add projection-order golden tests for every table.

### Phase 3: Finish data-file/delete-file metadata

Goal: make non-inlined DuckLake tables correct.

Tasks:

- Expand core row structs and catalog keys for full v1.0 data-file/delete-file fields.
- Add migration/facade compatibility for existing rows.
- Implement MVCC visibility and ordering.
- Wire per-file stats into reads and pruning.
- Run real DuckDB tests that exceed inlined storage and generate Parquet files.

### Phase 4: Complete transaction and conflict semantics

Goal: make DuckLake commits atomic and robust under realistic writer behavior.

Tasks:

- Group all statements belonging to one logical DuckLake commit.
- Apply snapshot, stats, row retirement, row insertion, and changes atomically.
- Validate rollback and retry behavior.
- Add concurrent writer tests with conflicting commits.

### Phase 5: Full metadata feature coverage

Goal: cover the rest of DuckLake v1.0.

Tasks:

- Views and macros.
- Tags and column tags.
- Sort info and file ordering.
- Partitions and partition values.
- Encryption key metadata.
- Imported existing DuckLake catalogs.

### Phase 6: Compatibility CI

Goal: prevent regressions as DuckDB/DuckLake evolve.

Tasks:

- Pin known-good DuckDB and DuckLake versions.
- Add optional jobs for latest upstream DuckDB/DuckLake.
- Preserve SQL corpus failures as actionable diffs.
- Include fresh, restart, and concurrent scenarios.

## Acceptance Criteria For "DuckDB And Rocklake Work Perfectly Together"

Rocklake should not claim full compatibility until all of the following are true:

- DuckDB can attach to a fresh Rocklake catalog with DuckLake and create/drop/read schemas and tables without custom flags.
- Inlined and file-backed tables both work.
- INSERT, DELETE, UPDATE, ALTER, DROP, view, macro, tag, partition, and sort metadata operations work.
- Fresh reads, restart reads, time-travel reads, ordered reads, filtered reads, and projection reads are correct.
- DuckDB `postgres_query` can inspect every DuckLake metadata table and dynamic inlined table without RowDescription failures.
- All DuckLake v1.0 metadata tables have exact SQL schemas and projection behavior.
- Table stats, table column stats, file stats, data-file metadata, and delete-file metadata remain correct after incremental commits and restarts.
- Conflict checks and snapshot changes behave correctly with multiple writers.
- The compatibility suite runs against pinned DuckDB/DuckLake versions and catches SQL/request drift.

## Final Recommendation

The immediate bugs that blocked the reviewed DuckDB/DuckLake inlined-data lifecycle have been fixed and validated. The next best engineering move is not another one-off classifier branch; it is a schema-driven DuckLake SQL facade shared by the PgWire handler, executor, COPY code, and tests. Once that facade exists, the remaining data-file/delete-file and advanced metadata work becomes much more mechanical, and Rocklake can move from "compatible with this current path" toward a defensible full DuckLake v1.0 compatibility claim.