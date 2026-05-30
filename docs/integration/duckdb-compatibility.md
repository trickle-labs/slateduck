# DuckDB Compatibility

RockLake aims to be a transparent, drop-in replacement for DuckLake's built-in PostgreSQL and SQLite catalog backends. From DuckDB's perspective, talking to RockLake should be indistinguishable from talking to a PostgreSQL instance running the DuckLake schema. This page documents the complete SQL compatibility matrix, version support, known differences, and the testing methodology that ensures compatibility across DuckDB releases.

## Compatibility Philosophy

DuckDB's `ducklake` extension emits a specific, bounded set of SQL statements to manage catalog metadata. RockLake does not implement general-purpose SQL — it implements exactly the SQL patterns that DuckDB's ducklake extension produces. This is the "bounded SQL" design: rather than building a full SQL engine, RockLake recognizes and handles only the specific statement patterns it will encounter.

This approach has important implications:

- If DuckDB's ducklake extension emits a new SQL pattern (in a new version), RockLake must be updated to recognize it
- Arbitrary SQL from custom clients will be rejected unless it matches a known pattern
- The compatibility surface is finite and testable

## Compatibility Matrix

### Schema Operations

| Operation | SQL Pattern | Status | Since |
|-----------|------------|--------|-------|
| Create schema | `INSERT INTO ducklake_schemas(...) VALUES(...)` | ✅ Full | 0.5.0 |
| Drop schema | `UPDATE ducklake_schemas SET end_snapshot_id=... WHERE schema_id=...` | ✅ Full | 0.5.0 |
| Rename schema | `UPDATE ducklake_schemas SET schema_name=... WHERE schema_id=...` | ✅ Full | 0.5.0 |
| List schemas | `SELECT ... FROM ducklake_schemas WHERE ...` | ✅ Full | 0.5.0 |

### Table Operations

| Operation | Status | Notes |
|-----------|--------|-------|
| CREATE TABLE | ✅ Full | All DuckDB column types supported |
| DROP TABLE | ✅ Full | Marks table and columns as ended |
| ALTER TABLE ADD COLUMN | ✅ Full | Appends column metadata |
| ALTER TABLE DROP COLUMN | ✅ Full | Marks column as ended |
| ALTER TABLE RENAME COLUMN | ✅ Full | Updates column name in-place |
| ALTER TABLE RENAME | ✅ Full | Updates table name |
| ALTER TABLE SET SCHEMA | ✅ Full | Moves table to different schema |
| CREATE TABLE ... AS | ✅ Full | DuckDB handles data, RockLake handles metadata |

### Column Type Support

RockLake stores column types as strings matching DuckDB's type system:

| Type Category | Examples | Status |
|--------------|---------|--------|
| Numeric | INTEGER, BIGINT, DOUBLE, DECIMAL(p,s), HUGEINT | ✅ |
| String | VARCHAR, TEXT, BLOB | ✅ |
| Temporal | DATE, TIME, TIMESTAMP, TIMESTAMP WITH TIME ZONE, INTERVAL | ✅ |
| Boolean | BOOLEAN | ✅ |
| Nested | STRUCT, LIST, MAP, UNION | ✅ |
| Special | JSON, UUID, ENUM | ✅ |
| Array | VARCHAR[], INTEGER[], etc. | ✅ |

### Data File Operations

| Operation | Status | Notes |
|-----------|--------|-------|
| Register data file | ✅ Full | Stores path, format, size, row count |
| Deregister data file | ✅ Full | Marks file as deleted (end_snapshot_id) |
| List data files | ✅ Full | Returns files visible at snapshot |
| Register delete file | ✅ Full | For row-level deletes |
| Column statistics (min/max/null_count) | ✅ Full | For partition pruning |

### Transaction Operations

| Operation | Status | Notes |
|-----------|--------|-------|
| BEGIN TRANSACTION | ✅ Full | Allocates new snapshot ID |
| COMMIT | ✅ Full | Atomically persists all changes |
| ROLLBACK | ✅ Full | Discards pending changes |
| Snapshot isolation | ✅ Full | Readers see consistent point-in-time view |
| Time travel (AT SNAPSHOT) | ✅ Full | Read historical catalog state |
| Time travel (AT TIMESTAMP) | ✅ Full | Resolves timestamp to snapshot ID |

### View and Macro Operations

| Operation | Status | Notes |
|-----------|--------|-------|
| CREATE VIEW | ✅ Full | Stores view definition text |
| DROP VIEW | ✅ Full | Marks view as ended |
| CREATE MACRO | ✅ Full | Stores macro definition |
| DROP MACRO | ✅ Full | Marks macro as ended |
| ALTER VIEW | ✅ Full | Updates view definition |

### Sequence Operations

| Operation | Status | Notes |
|-----------|--------|-------|
| CREATE SEQUENCE | ✅ Full | Stores sequence metadata |
| DROP SEQUENCE | ✅ Full | Marks sequence as ended |
| NEXTVAL | ⚠️ Partial | Handled but counter semantics differ slightly |

### Inlined Insert Operations

| Operation | Status | Notes |
|-----------|--------|-------|
| Small INSERT (< threshold) | ✅ Full | Data stored directly in catalog |
| Flush inlined to Parquet | ✅ Full | Converts inlined rows to data file |

## DuckDB Version Compatibility

### Supported Versions

| DuckDB Version | ducklake Version | RockLake Compatibility | Notes |
|----------------|-----------------|------------------------|-------|
| 1.5.x | 1.0 | ✅ Full | Minimum required version |
| < 1.5.2 | N/A | ❌ Not supported | DuckLake 1.0 requires DuckDB 1.5.2+ |

### Version Detection

RockLake detects the client's DuckDB version from the startup message parameters and adjusts its behavior accordingly. Older versions may emit slightly different SQL patterns for the same operations, and RockLake handles these variations transparently.

### Wire Corpus Testing

Compatibility is verified through a **wire corpus** — a collection of recorded PostgreSQL wire protocol messages captured from each supported DuckDB version. The corpus records the exact SQL that DuckDB emits for every catalog operation, and the test suite replays these messages against RockLake to verify correct handling.

```
tests/golden/wire-corpus/
├── duckdb-1.5.2/
│   ├── create-schema.pgwire
│   ├── create-table.pgwire
│   ├── insert-file.pgwire
│   └── ...
├── duckdb-1.5.3/
│   ├── create-schema.pgwire
│   ├── create-table.pgwire
│   └── ...
```

When a new DuckDB version is released, the corpus is extended with recordings from that version. If the new version emits new SQL patterns, RockLake's bounded SQL dispatcher is updated to recognize them.

## Known Differences

### Transaction Isolation Level

| Feature | DuckLake on PostgreSQL | RockLake |
|---------|----------------------|-----------|
| Isolation level | SERIALIZABLE (PostgreSQL) | Snapshot Isolation |
| Write conflicts | Detected at commit | Prevented by single-writer |
| Phantom reads | Prevented | Prevented |
| Read-write skew | Detected | Not applicable (single writer) |

**Practical impact:** None for typical workloads. The single-writer model means write conflicts cannot occur — there is only one writer. Readers always see a consistent snapshot.

### Error Messages

RockLake returns standard SQLSTATE error codes identical to what PostgreSQL would return. However, the human-readable error messages differ:

```
-- PostgreSQL:
ERROR: relation "ducklake_schemas" does not exist

-- RockLake:
ERROR: Table not found in catalog: ducklake_schemas
```

Well-behaved clients (including DuckDB) use SQLSTATE codes for error handling, not message text. If your custom tooling parses error messages as strings, it may need adjustment.

### Session Variables

RockLake acknowledges but ignores most PostgreSQL session variables:

| Variable | Behavior |
|----------|----------|
| `client_encoding` | Acknowledged (always UTF-8) |
| `DateStyle` | Acknowledged (no effect) |
| `TimeZone` | Acknowledged (no effect) |
| `standard_conforming_strings` | Acknowledged (always on) |
| `search_path` | Acknowledged (no effect — schema is explicit in queries) |

### System Catalog Queries

DuckDB's ducklake extension does not query PostgreSQL system catalogs (`pg_catalog`, `information_schema`). If custom clients send such queries, RockLake returns empty results or an error depending on the specific table.

### COPY Protocol

The PostgreSQL COPY protocol (bulk data transfer) is not supported. DuckDB does not use it for ducklake operations — all data transfers happen through standard query/response messages.

## Bounded SQL: What's Recognized

RockLake's SQL dispatcher classifies incoming statements into categories. Here are examples of recognized patterns:

### Catalog Reads (SELECT)

```sql
-- List schemas
SELECT schema_id, schema_name FROM ducklake_schemas WHERE database_id = $1 AND (end_snapshot_id IS NULL OR end_snapshot_id > $2)

-- List tables in schema
SELECT table_id, table_name, table_uuid FROM ducklake_tables WHERE schema_id = $1 AND (end_snapshot_id IS NULL OR end_snapshot_id > $2)

-- List columns for table
SELECT column_id, column_name, data_type, ordinal_position, is_nullable FROM ducklake_columns WHERE table_id = $1 AND (end_snapshot_id IS NULL OR end_snapshot_id > $2)

-- List data files
SELECT file_id, file_path, file_format, row_count, file_size_bytes FROM ducklake_data_files WHERE table_id = $1 AND (end_snapshot_id IS NULL OR end_snapshot_id > $2)
```

### Catalog Writes (INSERT/UPDATE)

```sql
-- Create schema
INSERT INTO ducklake_schemas (schema_name, database_id, created_snapshot_id) VALUES ($1, $2, $3) RETURNING schema_id

-- Create table
INSERT INTO ducklake_tables (table_name, schema_id, table_uuid, created_snapshot_id) VALUES ($1, $2, $3, $4) RETURNING table_id

-- Drop table (soft delete)
UPDATE ducklake_tables SET end_snapshot_id = $1 WHERE table_id = $2
```

### Unrecognized SQL

If RockLake receives SQL it does not recognize, it returns:

```
ERROR: Unsupported SQL statement
SQLSTATE: 42601
DETAIL: The bounded SQL dispatcher does not recognize this statement pattern.
HINT: RockLake only handles catalog SQL emitted by DuckDB's ducklake extension.
```

## Testing Your Setup

After connecting DuckDB to RockLake, verify compatibility:

```sql
-- Create a test schema and table
CREATE SCHEMA test_compat;
CREATE TABLE test_compat.verify (
    id BIGINT,
    name VARCHAR,
    amount DECIMAL(18,4),
    created_at TIMESTAMP,
    tags VARCHAR[],
    metadata JSON
);

-- Verify all types were stored correctly
DESCRIBE test_compat.verify;

-- Insert and verify data round-trip
INSERT INTO test_compat.verify VALUES (1, 'test', 99.99, '2024-01-01', ['a','b'], '{"key": "value"}');
SELECT * FROM test_compat.verify;

-- Test time travel
CREATE TABLE test_compat.versioned (id INTEGER, value VARCHAR);
INSERT INTO test_compat.versioned VALUES (1, 'first');
INSERT INTO test_compat.versioned VALUES (2, 'second');
-- Query at older snapshot should show fewer rows

-- Clean up
DROP SCHEMA test_compat CASCADE;
```

## Further Reading

- **[DuckDB](duckdb.md)** — Connection setup and usage guide
- **[Architecture: SQL Dispatcher](../architecture/sql-dispatcher.md)** — How bounded SQL classification works
- **[Architecture: PG Wire Protocol](../architecture/pg-wire-protocol.md)** — Wire protocol implementation details
- **[Internals: Wire Corpus](../internals/wire-corpus.md)** — Testing methodology
