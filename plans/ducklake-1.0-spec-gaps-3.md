# DuckLake 1.0 / DuckDB Perfect Interoperability Specification & Spec-Gap Report

---

## 1. Executive Summary & Purpose
This document provides a comprehensive, rigorous, and complete specification of the remaining gaps between SlateDuck and the DuckLake v1.0 specification.

### 1.1 Strict Scope and Targets
This gap assessment and roadmap strictly target the following software and catalog versions:
- **Client/Engine:** **DuckDB v1.5.3** (from the checked-out `v1.5.3` branch in `../duckdb`).
- **Lakehouse Catalog Spec:** **DuckLake 1.0 Specification** (Catalog Version 7 / `V1_0` as defined in `../ducklake`).
- **Strict Version Constraint:** SlateDuck **strictly targets only the 1.0 specification** of DuckLake. Any subsequent development branches (such as DuckLake v1.1 / Catalog Version 8 / `V1_1_DEV_1` or higher) are explicitly marked **out of scope**. This boundary limits scope creep and ensures a stable, robust compatibility baseline.

Our primary objective is to define the exact technical specifications and changes required in SlateDuck to achieve **100% perfect interoperability** with DuckDB v1.5.3 and DuckLake v1.0 across all operational regimes—including inlined data, data-file based storage, transaction-isolation guarantees, metadata replication, and complex analytical operations.

Through deep audits of the upstream components, we have identified the underlying protocol handshakes, system catalog queries, and metadata table specifications that dictate how DuckLake clients query, modify, and manage databases. When the specifications detailed in this report are implemented, SlateDuck and DuckDB v1.5.3 will work together perfectly without any hacks, workarounds, or silent failures.

---

## 2. DuckDB / DuckLake Connection & Inquiry Audit
### 2.1 The Connection & Connection Pool Reset Handshake
When an external client executes standard attach operations such as:
`LOAD ducklake; ATTACH 'ducklake:postgres:host=127.0.0.1 port=15434 dbname=slateduck' AS my_lake (DATA_PATH '/path/to/data');`
Two separate extensions inside DuckDB v1.5.3 interact with SlateDuck via the PostgreSQL PG-Wire protocol:
1. **Postgres Scanner Extension (`duckdb-postgres`):** Responsible for establishing the underlying PostgreSQL TCP connection, probing server versions, checking secret registry tables, and performing a complete **System Catalog Scan** to map OIDs.
2. **DuckLake Extension (`duckdb-ducklake`):** Once connected, it initiates a transaction and queries the DuckLake metadata catalog tables to read snapshots, schemas, tables, columns, and files.

Additionally, when connection pools return connections or clean up sessions, standard PostgreSQL session reset commands like `DISCARD ALL` are issued.

### 2.2 System Catalog Query Translation & Routing
DuckLake v1.0 executes its metadata calls within the DuckDB session using specialized CALL commands:
`CALL postgres_query('pg', 'SELECT ...')` or `CALL postgres_execute('pg', 'INSERT ...')`
These CALL statements are executed **locally inside DuckDB v1.5.3** by the `postgres_scanner` extension. The extension translates the embedded query and forwards standard PostgreSQL PG-Wire protocol frames directly to SlateDuck's socket. Thus, SlateDuck never sees `CALL postgres_query` or `CALL postgres_execute`. It only sees:
- Standard SELECT queries (like `SELECT * FROM main.ducklake_snapshot`)
- Standard INSERT statements (like `INSERT INTO main.ducklake_column VALUES (...)`)
- Standard COPY TO STDOUT / COPY FROM STDIN protocol frames.

### 2.3 The Multi-Statement Catalog Scan (C1 Critical Barrier)
During Phase 1, DuckDB sends a single multi-statement string over `PQsendQuery` containing a sequence of system catalog queries wrapped in a transaction:
```sql
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ;
SELECT oid, nspname FROM pg_namespace ORDER BY oid;
SELECT ... FROM pg_class JOIN pg_namespace ... JOIN pg_attribute ... JOIN pg_type ... WHERE relkind IN ('r', 'v', 'm', 'f', 'p') UNION ALL ... ORDER BY namespace_id, relname, attnum;
SELECT ... FROM pg_enum ...;
SELECT ... FROM pg_type composites ...;
SELECT ... FROM pg_indexes ...;
ROLLBACK;
```
SlateDuck intercepting this specific `StatementKind::PgCatalogScan` batch and returning the 5 mock catalog result sets (plus a ROLLBACK response tag) is **critically required** to prevent DuckDB from crashing with index-out-of-bounds errors.

---

## 3. The 28-Table DuckLake v1.0 Schema & Mapping Matrix
This section audits all 28 tables defined in DuckLake v1.0 (`ducklake_metadata_manager.cpp`), mapping them to SlateDuck's PgWire schema registry and identifying specific field, order, and type mismatches.

| Spec Table Name | Upstream C++ Schema Declaration | SlateDuck Schema Registry Status & Gaps | Priority |
| :--- | :--- | :--- | :--- |
| `ducklake_metadata` | `(key VARCHAR NOT NULL, value VARCHAR NOT NULL, scope VARCHAR, scope_id BIGINT)` | ⚠️ **Divergent:** Registry column names are `metadata_key` and `metadata_value`. Must be renamed to `key` and `value` to match upstream queries exactly. | P0 |
| `ducklake_snapshot` | `(snapshot_id BIGINT PRIMARY KEY, snapshot_time TIMESTAMPTZ, schema_version BIGINT, next_catalog_id BIGINT, next_file_id BIGINT)` | ✅ **Matched:** Internal fields match. Column order has slight variance but fields are correct. | P0 |
| `ducklake_snapshot_changes` | `(snapshot_id BIGINT PRIMARY KEY, changes_made VARCHAR, author VARCHAR, commit_message VARCHAR, commit_extra_info VARCHAR)` | ✅ **Matched:** Perfect alignment. | P0 |
| `ducklake_schema` | `(schema_id BIGINT PRIMARY KEY, schema_uuid UUID, begin_snapshot BIGINT, end_snapshot BIGINT, schema_name VARCHAR, path VARCHAR, path_is_relative BOOLEAN)` | ⚠️ **Matched Columns, Order Variance:** Column order in registry differs from spec declaration. Order must be aligned. | P1 |
| `ducklake_table` | `(table_id BIGINT, table_uuid UUID, begin_snapshot BIGINT, end_snapshot BIGINT, schema_id BIGINT, table_name VARCHAR, path VARCHAR, path_is_relative BOOLEAN)` | ⚠️ **Matched Columns, Order Variance:** Adjust column order in registry. | P1 |
| `ducklake_column` | `(column_id BIGINT, begin_snapshot BIGINT, end_snapshot BIGINT, table_id BIGINT, column_order BIGINT, column_name VARCHAR, column_type VARCHAR, initial_default VARCHAR, default_value VARCHAR, nulls_allowed BOOLEAN, parent_column BIGINT, default_value_type VARCHAR, default_value_dialect VARCHAR)` | ⚠️ **Matched Columns, Order Variance:** Adjust column order in registry. | P1 |
| `ducklake_data_file` | `(data_file_id BIGINT PRIMARY KEY, table_id BIGINT, begin_snapshot BIGINT, end_snapshot BIGINT, file_order BIGINT, path VARCHAR, path_is_relative BOOLEAN, file_format VARCHAR, record_count BIGINT, file_size_bytes BIGINT, footer_size BIGINT, row_id_start BIGINT, partition_id BIGINT, encryption_key VARCHAR, mapping_id BIGINT, partial_max BIGINT)` | ❌ **Major Gap:** Missing 5 critical spec fields in registry: `footer_size`, `partition_id`, `encryption_key`, `mapping_id`, `partial_max`. | P0 |
| `ducklake_delete_file` | `(delete_file_id BIGINT PRIMARY KEY, table_id BIGINT, begin_snapshot BIGINT, end_snapshot BIGINT, data_file_id BIGINT, path VARCHAR, path_is_relative BOOLEAN, format VARCHAR, delete_count BIGINT, file_size_bytes BIGINT, footer_size BIGINT, encryption_key VARCHAR, partial_max BIGINT)` | ❌ **Major Gap:** Missing 6 critical spec fields in registry: `data_file_id`, `path_is_relative`, `format`, `footer_size`, `encryption_key`, `partial_max`. | P0 |
| `ducklake_table_stats` | `(table_id BIGINT, record_count BIGINT, next_row_id BIGINT, file_size_bytes BIGINT)` | ✅ **Matched:** Perfectly aligned. | P0 |
| `ducklake_table_column_stats` | `(table_id BIGINT, column_id BIGINT, contains_null BOOLEAN, contains_nan BOOLEAN, min_value VARCHAR, max_value VARCHAR, extra_stats VARCHAR)` | ✅ **Matched:** Perfectly aligned. | P0 |
| `ducklake_file_column_stats` | `(data_file_id BIGINT, table_id BIGINT, column_id BIGINT, column_size_bytes BIGINT, value_count BIGINT, null_count BIGINT, min_value VARCHAR, max_value VARCHAR, contains_nan BOOLEAN, extra_stats VARCHAR)` | ✅ **Matched:** Perfectly aligned. | P0 |
| `ducklake_file_variant_stats` | `(data_file_id BIGINT, table_id BIGINT, column_id BIGINT, variant_path VARCHAR, shredded_type VARCHAR, column_size_bytes BIGINT, value_count BIGINT, null_count BIGINT, min_value VARCHAR, max_value VARCHAR, contains_nan BOOLEAN, extra_stats VARCHAR)` | ❌ **Missing Schema:** Not defined in `schema_registry.rs`. Not listed in `fields_for_table`. | P1 |
| `ducklake_view` | `(view_id BIGINT, view_uuid UUID, begin_snapshot BIGINT, end_snapshot BIGINT, schema_id BIGINT, view_name VARCHAR, dialect VARCHAR, sql VARCHAR, column_aliases VARCHAR)` | ⚠️ **Divergent Field Name:** Registry uses `view_definition` instead of spec `sql`. Must be renamed. | P1 |
| `ducklake_macro` | `(schema_id BIGINT, macro_id BIGINT, macro_name VARCHAR, begin_snapshot BIGINT, end_snapshot BIGINT)` | ⚠️ **Extra Column:** Registry has `macro_uuid` which does not exist in the spec. | P1 |
| `ducklake_macro_impl` | `(macro_id BIGINT, impl_id BIGINT, dialect VARCHAR, sql VARCHAR, type VARCHAR)` | ✅ **Matched:** Perfectly aligned. | P1 |
| `ducklake_macro_parameters` | `(macro_id BIGINT, impl_id BIGINT, column_id BIGINT, parameter_name VARCHAR, parameter_type VARCHAR, default_value VARCHAR, default_value_type VARCHAR)` | ✅ **Matched:** Perfectly aligned. | P1 |
| `ducklake_tag` | `(object_id BIGINT, begin_snapshot BIGINT, end_snapshot BIGINT, key VARCHAR, value VARCHAR)` | ❌ **Major Gap:** Registry defines columns `tag_id`, `object_id`, `tag_name`, `tag_value`. Must be mapped to `key`, `value` and `tag_id` removed. | P1 |
| `ducklake_column_tag` | `(table_id BIGINT, column_id BIGINT, begin_snapshot BIGINT, end_snapshot BIGINT, key VARCHAR, value VARCHAR)` | ❌ **Major Gap:** Registry uses `tag_id`, `tag_name`, `tag_value` and is missing `table_id`. | P1 |
| `ducklake_partition_info` | `(partition_id BIGINT, table_id BIGINT, begin_snapshot BIGINT, end_snapshot BIGINT)` | ✅ **Matched:** Perfectly aligned. | P1 |
| `ducklake_partition_column` | `(partition_id BIGINT, table_id BIGINT, partition_key_index BIGINT, column_id BIGINT, transform VARCHAR)` | ❌ **Major Gap:** Registry defines `partition_index`, `column_id`, `transform`, `transform_param`, missing `table_id` and has non-spec column names. | P1 |
| `ducklake_file_partition_value` | `(data_file_id BIGINT, table_id BIGINT, partition_key_index BIGINT, partition_value VARCHAR)` | ✅ **Matched:** registry matches. (Avoid confusion with extra `ducklake_partition_value` schema). | P2 |
| `ducklake_sort_info` | `(sort_id BIGINT, table_id BIGINT, begin_snapshot BIGINT, end_snapshot BIGINT)` | ✅ **Matched:** Perfectly aligned. | P2 |
| `ducklake_sort_expression` | `(sort_id BIGINT, table_id BIGINT, sort_key_index BIGINT, expression VARCHAR, dialect VARCHAR, sort_direction VARCHAR, null_order VARCHAR)` | ❌ **Major Gap:** Registry misses `table_id`, `expression`, `dialect`, and maps column names to non-spec variants. | P1 |
| `ducklake_files_scheduled_for_deletion` | `(data_file_id BIGINT, path VARCHAR, path_is_relative BOOLEAN, schedule_start TIMESTAMPTZ)` | ❌ **Major Gap:** Registry is missing `data_file_id` and has different names/types. | P2 |
| `ducklake_inlined_data_tables` | `(table_id BIGINT, table_name VARCHAR, schema_version BIGINT)` | ✅ **Matched:** Perfectly aligned. | P2 |
| `ducklake_column_mapping` | `(mapping_id BIGINT, table_id BIGINT, type VARCHAR)` | ❌ **Missing Schema:** Not defined in `schema_registry.rs`. | P1 |
| `ducklake_name_mapping` | `(mapping_id BIGINT, column_id BIGINT, source_name VARCHAR, target_field_id BIGINT, parent_column BIGINT, is_partition BOOLEAN)` | ❌ **Missing Schema:** Not defined in `schema_registry.rs`. | P1 |
| `ducklake_schema_versions` | `(begin_snapshot BIGINT, schema_version BIGINT, table_id BIGINT)` | ✅ **Matched:** Present. | P2 |

---

## 4. Implementation Roadmap
To close these gaps systematically, we propose a four-phase technical execution plan strictly constrained to DuckLake v1.0 and DuckDB v1.5.3:

### Phase 1: Perfect SQL Schema Facade & Schema Registry Refactoring
**Goal:** Eliminate all catalog column name, type, and order drifts.
1. **Schema Registry Update:** Modify `crates/slateduck-pgwire/src/schema_registry.rs` to ensure all 28 tables match their exact spec SQL schemas in both column order and name. Add missing schemas for `ducklake_file_variant_stats`, `ducklake_column_mapping`, and `ducklake_name_mapping`.
2. **Describe Field Mapping:** Update `crates/slateduck-pgwire/src/handler.rs`'s `describe_fields_for_sql` to map queries using the corrected schemas. Ensure dynamic projection and CAST operations are properly described.
3. **Response Building:** Align the QueryResponse encoders in `crates/slateduck-pgwire/src/executor/catalog.rs` to serialize the exact column sequences.

### Phase 2: Complete Data-File & Delete-File Conformance
**Goal:** Support large-scale, file-backed (Parquet) lakehouses.
1. **MVCC Isolation for External Files:** Extend the catalog readers (`crates/slateduck-catalog/src/reader.rs`) to ensure `list_data_files` and `list_delete_files` filter on `begin_snapshot <= snapshot_id` and `(end_snapshot IS NULL OR end_snapshot > snapshot_id)`.
2. **File Order Sorting:** Persist the `file_order` attribute during Parquet file registration, and ensure that `list_data_files` results are sorted ascending by `file_order` to maintain the exact layout expected by DuckLake's query planner.
3. **Numeric and Extra Stats:** Store and expose `footer_size` (as `BIGINT`), `partition_id`, `encryption_key`, `mapping_id`, and `partial_max` in data files and delete files.

### Phase 3: Multi-Statement Atomicity & Writer Concurrency
**Goal:** Secure multi-user environments with robust transactional guarantees.
1. **Atomic Statement Grouping:** Update `execute_commit` in `crates/slateduck-pgwire/src/executor/catalog.rs` to group all INSERT/UPDATE statements from a single logical commit transaction and apply them to the KV catalog atomically.
2. **Stats Delta Consolidation:** Ensure that a transaction with both `INSERT` and `DELETE` buffered operations consolidates the stats delta before updating `ducklake_table_stats`, maintaining precise record counts.
3. **Writer Fencing and ROLLBACK:** Fully enforce the repeatable-read isolation barrier in the catalog writer to reject stale snapshot commits with SQLSTATE `40001` (serialization failure), driving DuckLake's retry loop.

### Phase 4: Advanced Catalog Objects (Views, Macros, Partitions, Mappings)
**Goal:** Complete DDL capabilities for analytical SQL.
1. **Cascading Drop:** When a table is dropped, cascaded mark `end_snapshot = current_snapshot` across `ducklake_table`, `ducklake_column`, `ducklake_column_tag`, `ducklake_tag`, `ducklake_data_file`, `ducklake_delete_file`, `ducklake_partition_info`, and `ducklake_sort_info`.
2. **Macro and View Persistence:** Implement direct storage handlers for `ducklake_view`, `ducklake_macro`, `ducklake_macro_impl`, and `ducklake_macro_parameters` instead of ignoring their writes.
3. **Partition & Sort Mappings:** Update catalog serialization to save partition transforms and sort directions (`sort_direction`, `null_order`) exactly.

---

## 5. Definition of Done (Interoperability Acceptance Checklist)
SlateDuck is fully "compatible" and ready for 1.0 when:
- [ ] Every catalog table (all 28) can be fully described (`DescribeStatement` / `DescribePortal`) with column count, names, and OIDs identical to the upstream DuckLake specifications.
- [ ] DuckDB's postgres_scanner can connect and successfully parse the multi-statement schema discovery transaction `StatementKind::PgCatalogScan` without warnings or failures.
- [ ] Direct select queries against `ducklake_metadata` return `key` and `value` columns (not `metadata_key`/`metadata_value`).
- [ ] Inlined and Parquet file-backed (non-inlined) workflows—including multi-row appends, deletes, updates, and cascading drops—succeed with correct stats, MVCC visibility, and row ID mappings under the **1.0 specification of DuckLake** (using DuckDB v1.5.3).
- [ ] A nightly CI job runs SlateDuck against a matrix strictly consisting of **DuckDB v1.5.3** and **DuckLake 1.0 Spec (Catalog Version 7)** to automatically flag protocol or schema drift, rejecting any out-of-scope v1.1 / version 8 commits.
