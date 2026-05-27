# Access-Pattern and Key-Layout Analysis — Phase 0

> Derived from wire corpus analysis. Confirms or revises proposed key shapes
> before any encoder is written.

## Key Findings

### 1. ID Allocation Strategy

**Finding:** DuckDB reads `next_catalog_id` and `next_file_id` from
`ducklake_metadata` and allocates IDs locally within a transaction. It does
NOT rely on database-generated sequences.

**Implication:** Rocklake's counter-allocation model (read counter →
increment → write new value + consuming row atomically in one transaction)
is the correct approach.

### 2. `data_path` Handling

**Finding:** In PostgreSQL-backed mode, `data_path` stored in
`ducklake_data_file.file_path` is an absolute URI (e.g.,
`s3://bucket/data/warehouse/table_uuid/file.parquet`). In SQLite-backed mode,
paths are relative to the database file location.

**Decision:** Rocklake will store absolute object-store URIs. The
`CatalogPath` struct will handle `data_path_mode` (`Absolute` vs
`RelativeToDataPrefix`) for compatibility with DuckDB's expectations.

### 3. Transaction Wrapping

**Finding:** DuckLake wraps all catalog mutations in explicit
`BEGIN`/`COMMIT`. Multi-statement operations (e.g., `CREATE TABLE` which
inserts into `ducklake_table`, `ducklake_column`, `ducklake_snapshot`) are
always atomic.

**Decision:** The sidecar's `PendingCatalogTxn` accumulator between `BEGIN`
and `COMMIT` is the correct model.

### 4. Extended Query Protocol Usage

**Finding:** DuckDB uses extended protocol (`Parse`/`Bind`/`Execute`/`Sync`)
for all parameterized statements. Simple query protocol is used only for
`SET`, `SHOW`, and DDL statements.

**Decision:** The `pgwire` sidecar must fully implement extended query
protocol. Simple-query-only is insufficient.

### 5. pg_catalog Probes

**Finding:** DuckDB probes `pg_catalog.pg_type` for type OID resolution and
`pg_catalog.pg_namespace` for schema OID mapping. No other `pg_catalog`
tables are accessed.

**Decision:** Implement synthetic responses for `pg_type` and
`pg_namespace` only. Return empty result sets for any other `pg_catalog`
query.

## Confirmed Key Shapes

| Table | Dominant Query Pattern | Confirmed Key Shape |
|-------|----------------------|---------------------|
| `ducklake_metadata` | Point lookup by (scope, key) | `01 \| scope_enum \| scope_id \| key_bytes` |
| `ducklake_snapshot` | `max(snapshot_id)` / latest | `02 \| snapshot_id` (big-endian, enables max via reverse scan) |
| `ducklake_schema` | Range scan filtered by MVCC | `04 \| schema_id` |
| `ducklake_table` | Range scan by schema_id + MVCC | `05 \| schema_id \| table_id \| begin_snapshot` |
| `ducklake_column` | Range scan by table_id + MVCC | `06 \| table_id \| column_id \| begin_snapshot` |
| `ducklake_data_file` | Range scan by table_id | `0B \| table_id \| data_file_id` |
| `ducklake_delete_file` | Lookup by data_file_id | `0C \| data_file_id \| delete_file_id` |
| `ducklake_file_column_stats` | Scan by (table_id, column_id) for pruning | `13 \| table_id \| column_id \| data_file_id` |

## Revised Decisions

No revisions needed. All proposed key shapes from the design document are
confirmed by the wire corpus analysis.

## Query Frequency Estimates

| Operation | Relative Frequency | Latency Sensitivity |
|-----------|-------------------|---------------------|
| `get_current_snapshot` | Very High (every query) | Critical (< 5ms target) |
| `list_tables` | High | Moderate |
| `describe_table` | High | Moderate |
| `list_data_files` | High | Moderate (scales with file count) |
| `prune_files` | Medium | Important for large tables |
| `create_snapshot` | Low (write path) | Less sensitive |
| `register_data_file` | Low (write path) | Less sensitive |
