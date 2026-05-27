# DuckDB Wire Corpus — Phase 0

> Captured behavioral expectations for DuckDB ↔ PostgreSQL-wire DuckLake interaction.
> Version: DuckDB 1.5.2 (baseline)

## Capture Methodology

DuckDB's `ducklake` extension communicates with a PostgreSQL-compatible backend
using the PostgreSQL wire protocol. The corpus below documents the complete set
of protocol interactions observed during the DuckLake tutorial operations.

## Startup Handshake

DuckDB issues the following probe queries immediately after connection:

1. **Server version probe:** `SELECT version()`
   - Expected response: any PostgreSQL-compatible version string
   - DuckDB parses major version but does not enforce minimum

2. **Current schema:** `SELECT current_schema()`
   - Expected: `'public'`

3. **Type catalog:** `SELECT oid, typname FROM pg_catalog.pg_type WHERE typname IN ('bool', 'int2', 'int4', 'int8', 'float4', 'float8', 'text', 'varchar', 'timestamp', 'timestamptz', 'uuid', 'json', 'jsonb')`
   - Expected: standard PostgreSQL OIDs for each type

4. **Settings:** `SET client_encoding = 'UTF8'`, `SET DateStyle = 'ISO'`
   - Expected: `SET` confirmation

5. **Transaction isolation:** `SHOW transaction_isolation`
   - Expected: `'read committed'` (DuckDB does not enforce)

## Protocol Observations

| Aspect | Observation |
|--------|-------------|
| Query protocol | Simple query protocol for DDL/DML; extended protocol for prepared statements |
| Transaction wrapping | DuckLake uses explicit `BEGIN`/`COMMIT` for multi-statement catalog mutations |
| Parameter encoding | Text format for all parameters (format code 0) |
| Result format | Text format for all results (format code 0) |
| ID allocation | DuckDB reads `next_catalog_id`/`next_file_id` from `ducklake_metadata` and allocates locally |
| `data_path` format | Relative paths in SQLite-backed mode; absolute in PostgreSQL-backed mode |

## SQL Statement Shapes

### Read Shapes

```sql
-- Current snapshot
SELECT max(snapshot_id) FROM ducklake_snapshot;

-- Schema listing
SELECT schema_id, schema_name FROM ducklake_schema
WHERE begin_snapshot <= $1 AND (end_snapshot IS NULL OR $1 < end_snapshot);

-- Table listing
SELECT table_id, table_name, schema_id FROM ducklake_table
WHERE schema_id = $1 AND begin_snapshot <= $2 AND (end_snapshot IS NULL OR $2 < end_snapshot);

-- Column listing
SELECT column_id, column_name, data_type, column_index FROM ducklake_column
WHERE table_id = $1 AND begin_snapshot <= $2 AND (end_snapshot IS NULL OR $2 < end_snapshot)
ORDER BY column_index;

-- Data files
SELECT df.data_file_id, df.file_path, df.file_format, df.record_count,
       del.delete_file_id, del.file_path as delete_path
FROM ducklake_data_file df
LEFT JOIN ducklake_delete_file del ON df.data_file_id = del.data_file_id
WHERE df.table_id = $1;

-- File column stats for pruning
SELECT data_file_id FROM ducklake_file_column_stats
WHERE table_id = $1 AND column_id = $2 AND min_value > $3;
```

### Write Shapes

```sql
-- Snapshot creation
INSERT INTO ducklake_snapshot (snapshot_id, snapshot_time, schema_version)
VALUES ($1, $2, $3);

-- Schema creation
INSERT INTO ducklake_schema (schema_id, schema_name, begin_snapshot)
VALUES ($1, $2, $3);

-- Table creation
INSERT INTO ducklake_table (table_id, table_name, schema_id, table_uuid, begin_snapshot)
VALUES ($1, $2, $3, $4, $5);

-- Column creation
INSERT INTO ducklake_column (column_id, column_name, data_type, column_index, table_id, begin_snapshot)
VALUES ($1, $2, $3, $4, $5, $6);

-- Data file registration
INSERT INTO ducklake_data_file (data_file_id, table_id, file_path, file_format, record_count, file_size_bytes, begin_snapshot)
VALUES ($1, $2, $3, $4, $5, $6, $7);

-- End-snapshot marking
UPDATE ducklake_table SET end_snapshot = $1 WHERE table_id = $2 AND end_snapshot IS NULL;
UPDATE ducklake_column SET end_snapshot = $1 WHERE table_id = $2 AND end_snapshot IS NULL;

-- Counter updates
UPDATE ducklake_metadata SET value = $1 WHERE scope = 'global' AND key = 'next_catalog_id';
```

## Transaction Patterns

```
BEGIN;
-- Read current counters
SELECT value FROM ducklake_metadata WHERE scope = 'global' AND key = 'next_catalog_id';
-- Allocate IDs and perform mutations
INSERT INTO ducklake_table ...;
INSERT INTO ducklake_column ...;
INSERT INTO ducklake_snapshot ...;
-- Update counters
UPDATE ducklake_metadata SET value = $1 WHERE scope = 'global' AND key = 'next_catalog_id';
COMMIT;
```

## Extended Query Protocol Sequences

DuckDB uses extended protocol (`Parse`/`Bind`/`Describe`/`Execute`/`Sync`) for:
- Parameterized INSERT statements
- SELECT with bind parameters

Simple query protocol used for:
- `SET` commands
- `SHOW` commands
- DDL for inlined tables

## pg_catalog Probes

Beyond type lookup, DuckDB queries:
- `pg_catalog.pg_namespace` — for schema OID mapping
- No other `pg_catalog` tables observed in baseline capture
