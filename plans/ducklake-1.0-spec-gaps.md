# DuckLake 1.0 Specification Gap Assessment

Date: 2026-05-25

Spec source reviewed: `../ducklake-web/docs/stable/specification/`

Implementation areas reviewed:

- `crates/slateduck-core/src/rows.rs`
- `crates/slateduck-core/src/tags.rs`
- `crates/slateduck-core/src/keys.rs`
- `crates/slateduck-core/src/types.rs`
- `crates/slateduck-catalog/src/reader.rs`
- `crates/slateduck-catalog/src/writer/mod.rs`
- `crates/slateduck-catalog/src/writer/snapshot.rs`
- `crates/slateduck-catalog/src/writer/stats.rs`
- `crates/slateduck-pgwire/src/executor/mod.rs`
- `crates/slateduck-pgwire/src/executor/catalog.rs`
- `crates/slateduck-pgwire/src/executor/meta.rs`
- `crates/slateduck-sql/src/classifier/`

## Executive Summary

SlateDuck has allocated tags and protobuf row types for all 28 DuckLake v1.0 catalog tables, and the core MVCC idea is present for schemas, tables, columns, data files, tags, partitions, sort info, and schema versions. That is a strong foundation.

However, SlateDuck does not yet conform to the DuckLake v1.0 specification at the SQL catalog boundary. Many internal row shapes differ from the spec table columns, several PgWire query responses expose non-spec columns, and some SQL statement classes are accepted but ignored or return empty result sets. The biggest interoperability risks are snapshots, snapshot changes, data files, delete files, table stats/row IDs, and the absence of a complete SQL-compatible catalog facade.

Short version:

- Internal table/tag coverage: high. All 28 DuckLake table tags are allocated and marked live.
- Field-level schema compatibility: partial to low. Several central tables are missing required columns or use different column names/semantics.
- Query/write behavior compatibility: partial. Basic schema/table/column/data-file flows exist, but many spec reads return empty and several writes ignore spec fields.
- External DuckDB DuckLake extension compatibility: likely blocked without a stricter SQL facade or export/import adapter.

## Compatibility Verdict

SlateDuck is currently better described as DuckLake-inspired internal catalog storage than a complete DuckLake v1.0 catalog implementation.

The main reason is architectural: the DuckLake spec defines a SQL catalog database with 28 SQL tables. SlateDuck stores catalog facts as SlateDB key/value entries with protobuf values, then exposes selected operations through PgWire classification and custom response builders. That can still be made compatible, but the PgWire/virtual-table layer must project exact DuckLake table schemas and semantics. Today it only projects a subset, and some projected result sets use SlateDuck-specific field names.

## Severity Legend

- P0: blocks DuckLake v1.0 interoperability, correctness, time travel, or external DuckDB compatibility.
- P1: important spec parity gap; may not block a narrow happy path but limits supported features.
- P2: cleanup, fidelity, or advanced-feature gap.

## Highest Priority Gaps

### P0. Provide an exact DuckLake SQL catalog facade

The spec requires SQL tables such as `ducklake_snapshot`, `ducklake_data_file`, and `ducklake_delete_file` with exact columns. SlateDuck internally stores protobuf rows, and PgWire currently returns custom schemas for several tables.

Examples:

- `ducklake_snapshot` responses expose `author` and `message`, but the spec requires `next_catalog_id` and `next_file_id`.
- `ducklake_table` responses expose `data_path`, but the spec requires `table_uuid`, `path`, and `path_is_relative`.
- `ducklake_column` responses expose `data_type`, `column_index`, and `is_nullable`, but the spec requires `column_type`, `column_order`, and `nulls_allowed` plus default/nested columns.
- `SelectSnapshot`, `SelectTableStats`, `SelectMetadata`, `SelectViews`, `SelectMacros`, `SelectDeleteFiles`, and inlined reads currently return empty result sets in `crates/slateduck-pgwire/src/executor/mod.rs`.

Roadmap item:

- Add a DuckLake v1.0 SQL facade with exact column names, column order, nullability behavior, and value semantics for all 28 tables. This can be implemented as PgWire virtual tables over the KV/protobuf storage, but the public SQL shape must match the spec.

### P0. Fix snapshot and snapshot change schema

Spec:

- `ducklake_snapshot(snapshot_id, snapshot_time, schema_version, next_catalog_id, next_file_id)`
- `ducklake_snapshot_changes(snapshot_id, changes_made, author, commit_message, commit_extra_info)`

Current SlateDuck:

- `SnapshotRow` has `snapshot_id`, `schema_version`, `snapshot_time`, `author`, `message`.
- `next_catalog_id` and `next_file_id` are kept in `TAG_COUNTERS` instead of denormalized into each snapshot row.
- `SnapshotChangesRow` models individual events with `change_type`, `change_info`, `schema_id`, and `table_id`.
- PgWire accepts `InsertSnapshotChanges` but `execute_commit` treats it as informational and does not store it separately.

Impact:

- External readers cannot reconstruct `next_catalog_id` and `next_file_id` from snapshots as required by the spec.
- Conflict resolution cannot use the documented `changes_made` list.
- Commit metadata is stored in the wrong table.

Roadmap item:

- Move `author`/`message` semantics into `ducklake_snapshot_changes` as `author` and `commit_message`.
- Add `next_catalog_id` and `next_file_id` to the snapshot row/facade at commit time.
- Persist a spec-compatible `changes_made` string per snapshot, using the documented values such as `created_schema:<schema_name>`, `inserted_into_table:<table_id>`, and `dropped_table:<table_id>`.
- Keep structured change events only as an internal extension if needed.

### P0. Implement spec-complete data file semantics

Spec `ducklake_data_file` columns:

`data_file_id`, `table_id`, `begin_snapshot`, `end_snapshot`, `file_order`, `path`, `path_is_relative`, `file_format`, `record_count`, `file_size_bytes`, `footer_size`, `row_id_start`, `partition_id`, `encryption_key`, `mapping_id`, `partial_max`

Current `DataFileRow`:

`data_file_id`, `table_id`, `path`, `file_format`, `row_count`, `file_size_bytes`, `snapshot_id`, `footer_size`, `encryption_key`, `begin_snapshot`, `end_snapshot`

Missing or divergent:

- Missing `file_order`.
- Missing `path_is_relative`.
- `record_count` is named `row_count` internally and in PgWire responses.
- `footer_size` is stored as `Option<String>` rather than numeric `BIGINT` semantics.
- Missing `row_id_start`.
- Missing `partition_id`.
- Missing `mapping_id`.
- Missing `partial_max`.
- Keeps legacy `snapshot_id` alongside `begin_snapshot`.
- `CatalogReader::list_data_files` uses `TAG_DATA_FILE_BY_SNAPSHOT` and does not filter out rows whose `end_snapshot` is visible as retired at the requested snapshot.
- `list_data_files` does not order by spec `file_order` because no `file_order` is stored.

Impact:

- Time-travel reads can include logically retired data files.
- DELETE/UPDATE/compaction behavior cannot be spec-correct without row IDs and file ordering.
- Partitioned files, mapped Parquet fields, partial files, and relative paths are not representable.

Roadmap item:

- Introduce a spec-compatible data-file model, probably with a catalog-format migration.
- Persist `begin_snapshot`, `end_snapshot`, `file_order`, `path_is_relative`, numeric `footer_size`, `row_id_start`, `partition_id`, `mapping_id`, and `partial_max`.
- Update `list_data_files` to apply MVCC visibility and order by `file_order`.
- Keep secondary indexes only as internal acceleration structures, not as a substitute for spec semantics.

### P0. Implement spec-complete delete files

Spec `ducklake_delete_file` columns:

`delete_file_id`, `table_id`, `begin_snapshot`, `end_snapshot`, `data_file_id`, `path`, `path_is_relative`, `format`, `delete_count`, `file_size_bytes`, `footer_size`, `encryption_key`, `partial_max`

Current `DeleteFileRow`:

`delete_file_id`, `data_file_id`, `path`, `row_count`, `file_size_bytes`, `snapshot_id`

Missing or divergent:

- Missing `table_id`.
- Missing `begin_snapshot` and `end_snapshot`; delete files have no spec MVCC window.
- Missing `path_is_relative`.
- Missing `format`; therefore no explicit `parquet` vs `puffin` support.
- `delete_count` is represented as `row_count`.
- Missing `footer_size`.
- Missing `encryption_key`.
- Missing `partial_max`.
- PgWire `SelectDeleteFiles` currently returns an empty result set.

Impact:

- Merge-on-read DELETE cannot be faithfully implemented for DuckLake readers.
- Time travel cannot decide which delete files are visible for a snapshot.
- UPDATE, which is DELETE plus INSERT in DuckLake, cannot be spec-correct.

Roadmap item:

- Add full `DeleteFileRow` fields and key/index support.
- Add `list_delete_files(table_id, snapshot_id)` with spec MVCC visibility.
- Return delete files in the SQL facade and join them correctly with data files.
- Support `format = 'parquet'`; track `puffin` as a later advanced feature if desired.

### P0. Implement row ID tracking and table stats correctly

Spec `ducklake_table_stats` columns:

`table_id`, `record_count`, `next_row_id`, `file_size_bytes`

Current `TableStatsRow`:

`table_id`, `row_count`, `file_count`, `total_size_bytes`

Other current behavior:

- A per-table rowid counter exists (`COUNTER_NEXT_ROWID_PREFIX`) and `slateduck.next_rowid_range` exists, but the DuckLake `next_row_id` column is not present in `TableStatsRow`.
- PgWire `UpdateTableStats` calls `update_table_stats(table_id, 0, 0, 0)`, ignoring the row-count delta.
- PgWire `SelectTableStats` returns an empty result set.
- Data files do not store `row_id_start`.

Impact:

- Inserts cannot publish the row ID ranges expected by DuckLake metadata.
- DELETE/UPDATE and CDC semantics are weakened because row identity is not anchored in spec columns.

Roadmap item:

- Replace or facade-map `row_count` to `record_count` and `total_size_bytes` to `file_size_bytes`.
- Add `next_row_id` to table stats and update it atomically with data-file registration.
- Add `row_id_start` to data files using the pre-increment `next_row_id`.
- Keep `file_count` as an internal/extension statistic only.

### P0. Make DROP TABLE retire all related metadata

Spec DROP TABLE updates `end_snapshot` for:

- `ducklake_table`
- `ducklake_partition_info`
- `ducklake_column`
- `ducklake_column_tag`
- `ducklake_data_file`
- `ducklake_delete_file`
- `ducklake_tag`

Current `CatalogWriter::drop_table` only marks the table row ended. PgWire `UpdateEndSnapshot` only handles `ducklake_table` and `ducklake_column`; other tables are accepted or ignored.

Impact:

- Dropped tables can leave visible columns, tags, data files, delete files, and partition metadata.
- Time-travel and catalog listing behavior diverges from the spec.

Roadmap item:

- Implement cascading metadata retirement for DROP TABLE.
- Add conformance tests that drop a table and verify every related spec table has `end_snapshot` set at the drop snapshot.

## Table-by-Table Coverage

| Spec table | Current status | Main gaps | Priority |
|---|---|---|---|
| `ducklake_metadata` | Partial internal support | `MetadataScope` is encoded in keys, but `MetadataRow` lacks `scope`/`scope_id`; PgWire `InsertMetadata` is accepted but ignored; `SelectMetadata` returns empty. | P1 |
| `ducklake_snapshot` | Divergent | Missing `next_catalog_id`, `next_file_id`; incorrectly includes `author`/`message`; PgWire shape is non-spec. | P0 |
| `ducklake_snapshot_changes` | Divergent/stub | Uses event fields instead of `changes_made`; missing `author`, `commit_message`, `commit_extra_info`; PgWire accepts then ignores. | P0 |
| `ducklake_schema` | Partial | Missing `schema_uuid`, `path`, `path_is_relative`; PgWire returns only four columns. | P1 |
| `ducklake_table` | Partial | Missing `table_uuid`, `path_is_relative`; uses `data_path` instead of spec `path`; PgWire shape is non-spec. | P1 |
| `ducklake_view` | Partial/stubbed SQL | Missing `view_uuid`, `dialect`, `column_aliases`; PgWire insert is accepted but execute path does not call `create_view`; `SelectViews` returns empty. | P1 |
| `ducklake_column` | Partial | Missing `initial_default`, `parent_column`, `default_value_type`, `default_value_dialect`; uses `data_type`, `column_index`, `is_nullable` naming; no nested-column representation. | P1 |
| `ducklake_macro` | Partial/stubbed SQL | `MacroRow` adds `macro_type`, which belongs in `ducklake_macro_impl.type`; PgWire insert accepted but execute path does not call `create_macro`. | P1 |
| `ducklake_macro_impl` | Partial/no-op SQL | Missing `dialect` and `type`; uses `definition` instead of `sql`; PgWire insert is accepted as no-op. | P1 |
| `ducklake_macro_parameters` | Partial/no-op SQL | Missing `default_value_type`; PgWire insert is accepted as no-op. | P1 |
| `ducklake_data_file` | Partial, correctness risk | Missing `file_order`, `path_is_relative`, `row_id_start`, `partition_id`, `mapping_id`, `partial_max`; non-spec names; visibility filtering incomplete. | P0 |
| `ducklake_delete_file` | Major gap | Missing most spec fields and MVCC; select returns empty. | P0 |
| `ducklake_files_scheduled_for_deletion` | Partial | Missing `path_is_relative`; adds non-spec `file_type`; timestamp stored as integer seconds rather than SQL `TIMESTAMPTZ` semantics. | P2 |
| `ducklake_inlined_data_tables` | Divergent/stubbed SQL | Spec requires `table_name`; SlateDuck stores `sql`; PgWire insert accepted but execute path ignores it. | P1 |
| `ducklake_column_mapping` | Divergent | Spec table is `mapping_id`, `table_id`, `type`; SlateDuck stores `file_column_name` and `column_id`, which belongs conceptually under name mapping. | P1 |
| `ducklake_name_mapping` | Divergent | Missing `target_field_id`, `parent_column`, `is_partition`; adds `source_name_hash`. | P1 |
| `ducklake_table_stats` | Divergent | Missing `next_row_id`; spec names `record_count` and `file_size_bytes`; SlateDuck adds `file_count`; PgWire select empty and update ignores delta. | P0 |
| `ducklake_table_column_stats` | Partial | Missing `contains_nan` and `extra_stats`; uses `has_null` instead of spec `contains_null`. | P1 |
| `ducklake_file_column_stats` | Partial | Missing `column_size_bytes`, `value_count`, `null_count`, `extra_stats`; uses `has_null` boolean instead of `null_count`. | P1 |
| `ducklake_file_variant_stats` | Skeletal | Missing `shredded_type`, `column_size_bytes`, `value_count`, `null_count`, `contains_nan`, `extra_stats`; adds internal `variant_path_hash`. | P1 |
| `ducklake_partition_info` | Mostly covered | Row fields match the spec; need ensure SQL facade exposes it and DROP TABLE retires it. | P1 |
| `ducklake_partition_column` | Partial | Missing `table_id`. | P1 |
| `ducklake_file_partition_value` | Partial | Uses `value` instead of `partition_value`; otherwise close. | P2 |
| `ducklake_sort_info` | Mostly covered | Row fields match the spec; need SQL facade and lifecycle coverage. | P2 |
| `ducklake_sort_expression` | Divergent | Missing `table_id`, `expression`, `dialect`; uses `column_id`, `ascending`, `nulls_first` booleans instead of spec string fields. | P1 |
| `ducklake_tag` | Semantically close | Uses `tag_key`/`tag_value` internally instead of SQL `key`/`value`; need exact facade and lifecycle tests. | P2 |
| `ducklake_column_tag` | Semantically close | Uses `tag_key`/`tag_value` internally instead of SQL `key`/`value`; DROP TABLE must retire these rows. | P2 |
| `ducklake_schema_versions` | Mostly covered | Row fields are present; confirm SQL facade column order and write/update coverage. | P2 |

## Query and Operation Coverage

### Reading

Covered or partly covered:

- Current/latest snapshot lookup exists internally.
- Schema listing uses MVCC visibility.
- Table listing uses MVCC visibility.
- Column listing uses MVCC visibility and orders by `column_index`.
- Data-file listing exists and uses a secondary index for files added before a snapshot.
- Basic min/max file pruning exists.

Gaps:

- Data-file listing does not filter `end_snapshot`, so retired files can remain visible.
- Data-file listing cannot order by `file_order`.
- Delete-file listing returns empty through PgWire.
- Snapshot row selects expose non-spec fields.
- Table stats, metadata, views, macros, and inlined-data selects return empty through PgWire.
- File pruning passes `DuckLakeType::Varchar` in PgWire, so type-aware comparisons are not actually driven from `ducklake_column.column_type` there.
- Many of the 28 tables have no PgWire/SQL projection at all.

Roadmap item:

- Add a read conformance suite that runs the SQL examples from `specification/queries.md` against SlateDuck and compares result columns and row semantics.

### Writing

Covered or partly covered:

- Snapshot creation is atomic with staged catalog writes and counters.
- Create/drop schema and table exist internally.
- Add/drop column exists internally.
- Data-file registration exists internally.
- Delete-file registration exists internally but is under-modeled.
- Basic stats writes exist.

Gaps:

- Snapshot changes are not persisted in spec form.
- PgWire metadata, inlined table, view, macro, macro implementation, and macro parameter inserts are accepted but mostly ignored or no-op.
- Table stats updates are not applying deltas correctly in PgWire.
- Insert data file ignores many spec parameters.
- Insert delete file ignores many spec parameters.
- DROP TABLE does not retire related metadata.
- Stats writes are direct `db.put` operations outside `create_snapshot`; this may be recoverable internally, but it does not match the spec's transaction model for catalog mutations.

Roadmap item:

- Treat every DuckLake spec write as a transactionally staged catalog mutation unless the spec explicitly permits recomputation outside the snapshot boundary.

## Data Type Coverage

Spec primitive types include:

`boolean`, `int8`, `int16`, `int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64`, `int128`, `uint128`, `float32`, `float64`, `decimal(P,S)`, `time`, `timetz`, `date`, `timestamp`, `timestamptz`, `timestamp_s`, `timestamp_ms`, `timestamp_ns`, `interval`, `varchar`, `blob`, `json`, `uuid`.

Spec nested/semi-structured/spatial types include:

- `list`
- `struct`
- `map`
- `variant`
- geometry primitives through Parquet geometry type

Current `DuckLakeType` supports broad comparison categories: signed/unsigned integers, decimal, float, timestamp with timezone flag, date, time with timezone flag, interval, varchar, blob, boolean, uuid, unknown.

Gaps:

- No explicit modeling of timestamp precision (`timestamp_s`, `timestamp_ms`, `timestamp_ns`).
- No explicit `json` type.
- No explicit nested type model; `ducklake_column.parent_column` is missing.
- No explicit `variant` type model beyond skeletal variant stats rows.
- No geometry statistics support (`extra_stats` bounding boxes/types are absent).
- Stats encoding coverage is incomplete for `extra_stats`, nested types, variant shredding, and geometry.

Roadmap item:

- Add a type parser for DuckLake type strings and use it consistently for catalog validation, stats encoding, and pruning.
- Add nested column rows using `parent_column` before claiming nested-type support.
- Add `extra_stats` storage and JSON validation for variant and geometry stats.

## Compatibility Risks for External DuckDB/DuckLake Clients

External DuckLake clients expect to query the catalog tables directly. With the current SlateDuck PgWire layer, the likely failure modes are:

- Column-not-found errors because facade responses do not expose spec columns such as `next_catalog_id`, `path_is_relative`, `record_count`, `row_id_start`, `next_row_id`, or `changes_made`.
- Incorrect time-travel reads because retired data/delete files are not consistently filtered by `begin_snapshot`/`end_snapshot`.
- Incorrect DELETE/UPDATE results because delete files are not MVCC-versioned and PgWire select returns empty.
- Incorrect row lineage/CDC because `row_id_start` and `next_row_id` are missing from spec tables.
- Conflict resolution gaps because `snapshot_changes.changes_made` is absent.
- Silent data loss or behavior gaps because PgWire accepts several INSERT statements as successful but does not persist them.

## Recommended Roadmap

### Phase 0: Conformance Harness

- Add a machine-readable DuckLake v1.0 schema manifest derived from `../ducklake-web/docs/stable/specification/tables/overview.md`.
- Add tests that assert the SQL facade exposes all 28 tables with exact column names and compatible types.
- Add golden tests for the SQL examples in `specification/queries.md`.
- Add tests that verify unsupported DuckLake statements fail explicitly instead of returning success for no-op writes.

### Phase 1: Interop-Critical Catalog Rows

- Make `ducklake_snapshot` and `ducklake_snapshot_changes` spec-compatible.
- Make `ducklake_data_file` spec-compatible and visibility-correct.
- Make `ducklake_delete_file` spec-compatible and visible through SQL.
- Make `ducklake_table_stats` spec-compatible, including `next_row_id`.
- Make DROP TABLE retire all dependent metadata.
- Update PgWire response builders to expose exact spec columns for the critical read path: snapshots, schemas, tables, columns, data files, delete files, file stats, table stats, and metadata.

### Phase 2: Schema and Metadata Parity

- Add UUID fields for schemas, tables, and views.
- Add `path` and `path_is_relative` to schemas/tables/files/delete files.
- Add scoped metadata rows and SQL visibility for `scope`/`scope_id`.
- Add full column defaults and nested column support.
- Add full view and macro persistence through PgWire.
- Add column mapping/name mapping spec parity.

### Phase 3: Stats, Partitioning, Sorting, and Advanced Types

- Add full file/table column stats fields, including `null_count`, `value_count`, `column_size_bytes`, `contains_nan`, and `extra_stats`.
- Add full variant stats and `extra_stats` support.
- Add geometry stats support.
- Add `partition_column.table_id` and ensure partition metadata lifecycle is complete.
- Replace boolean sort representation with spec `expression`, `dialect`, `sort_direction`, and `null_order` fields.
- Add partial-file support via `partial_max`.

### Phase 4: External Compatibility Validation

- Run a real DuckDB DuckLake extension client against the SlateDuck PgWire/catalog facade.
- Verify create schema/table, insert, select, delete, update, drop, time travel, file pruning, and conflict-resolution flows.
- Add an import/export or migration path for catalogs created with earlier SlateDuck protobuf shapes.

## Suggested Definition of Done

SlateDuck can claim DuckLake v1.0 catalog compatibility when:

- All 28 spec tables are visible through SQL with exact columns and compatible types.
- Every field in the spec schema is either persisted internally or losslessly synthesized in the SQL facade.
- DuckLake query examples from `specification/queries.md` pass against SlateDuck.
- Create/insert/delete/update/drop operations produce rows matching spec semantics.
- Time travel uses `begin_snapshot` and `end_snapshot` consistently for data files, delete files, schemas, tables, columns, tags, partitions, views, macros, and related metadata.
- Snapshot rows include `next_catalog_id` and `next_file_id`.
- Snapshot changes include `changes_made`, `author`, `commit_message`, and `commit_extra_info`.
- Data files include `file_order`, `row_id_start`, and path/mapping/partition/partial-file fields.
- Delete files include full MVCC windows and are returned to readers.
- Row ID allocation is represented through `ducklake_table_stats.next_row_id` and `ducklake_data_file.row_id_start`.
- No supported DuckLake SQL write is accepted as a no-op unless documented as intentionally unsupported and returned with an explicit error.

## Bottom Line

The foundation is useful: SlateDuck already has all DuckLake tag namespaces, protobuf rows for all nominal tables, MVCC primitives, atomic snapshot commits, and selected reader/writer paths. The gap is at the spec boundary. Closing it should focus less on allocating more tags and more on making the public SQL table model exact, preserving all spec fields, and making data/delete-file visibility and row IDs correct.
