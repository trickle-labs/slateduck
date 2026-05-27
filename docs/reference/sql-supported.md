# Supported SQL Reference

This page provides an exhaustive, authoritative list of every SQL statement pattern that Rocklake's bounded SQL classifier recognizes and handles. These are the exact patterns emitted by DuckDB's `ducklake` extension when it communicates with a catalog backend over the PostgreSQL wire protocol.

Rocklake does not implement a general-purpose SQL parser. It implements pattern matching against a known, finite set of SQL statements. If a statement does not match any pattern in this list, Rocklake returns SQLSTATE 42601 (syntax_error). This is by design — Rocklake implements the DuckLake protocol, not a general SQL engine.

The SQL examples shown below are the canonical forms. DuckDB may vary whitespace, quoting, and parameter ordering between versions. The classifier handles these variations through flexible pattern matching rather than exact string comparison.

## Catalog Operations

### Initialize Catalog

Creates the initial catalog metadata. Sent once when DuckDB first attaches to a Rocklake instance.

```sql
-- Create the catalog entry
INSERT INTO ducklake_catalog (catalog_name, catalog_version)
VALUES ('my_catalog', 1)
```

**Maps to:** `CatalogWriter::initialize_catalog(name, version)`

**Produces:** One catalog row, one snapshot row, counter initializations.

### Get Catalog Info

Retrieves catalog metadata (name, version, current snapshot).

```sql
SELECT catalog_id, catalog_name, catalog_version
FROM ducklake_catalog
WHERE begin_snapshot <= ? AND (end_snapshot IS NULL OR end_snapshot > ?)
```

**Maps to:** `CatalogReader::get_catalog_info(snapshot_id)`

---

## Schema Operations

### Create Schema

Creates a new schema (namespace) in the catalog.

```sql
INSERT INTO ducklake_schemas (schema_name)
VALUES ('analytics')
```

**Maps to:** `CatalogWriter::create_schema(name)`

**Produces:** One schema row with begin_snapshot = current, end_snapshot = None.

### Drop Schema

Marks a schema as ended (superseded). Does not physically delete the row.

```sql
UPDATE ducklake_schemas
SET end_snapshot = ?
WHERE schema_id = ? AND end_snapshot IS NULL
```

**Maps to:** `CatalogWriter::drop_schema(schema_id)`

**Produces:** Updates end_snapshot on the current schema row.

### Rename Schema

Creates a new version of the schema row with the new name, ends the old version.

```sql
-- End the old version
UPDATE ducklake_schemas
SET end_snapshot = ?
WHERE schema_id = ? AND end_snapshot IS NULL

-- Create new version with new name
INSERT INTO ducklake_schemas (schema_id, schema_name, begin_snapshot)
VALUES (?, 'new_name', ?)
```

**Maps to:** `CatalogWriter::rename_schema(schema_id, new_name)`

**Produces:** Old row gets end_snapshot; new row created with same schema_id.

### List Schemas

Retrieves all schemas visible at a given snapshot.

```sql
SELECT schema_id, schema_name
FROM ducklake_schemas
WHERE begin_snapshot <= ? AND (end_snapshot IS NULL OR end_snapshot > ?)
```

**Maps to:** `CatalogReader::list_schemas(snapshot_id)`

**Returns:** All schema rows where `begin_snapshot <= requested` AND (`end_snapshot IS NULL` OR `end_snapshot > requested`).

---

## Table Operations

### Create Table

Creates a new table in a schema, typically accompanied by column definitions.

```sql
INSERT INTO ducklake_tables (table_name, schema_id)
VALUES ('events', 1)
```

**Maps to:** `CatalogWriter::create_table(schema_id, name)`

**Produces:** One table row. Usually followed immediately by column INSERT statements.

### Drop Table

Marks a table as ended.

```sql
UPDATE ducklake_tables
SET end_snapshot = ?
WHERE table_id = ? AND end_snapshot IS NULL
```

**Maps to:** `CatalogWriter::drop_table(table_id)`

**Also ends:** All columns belonging to this table (their end_snapshot is also set).

### Rename Table

```sql
UPDATE ducklake_tables
SET end_snapshot = ?
WHERE table_id = ? AND end_snapshot IS NULL

INSERT INTO ducklake_tables (table_id, schema_id, table_name, begin_snapshot)
VALUES (?, ?, 'new_name', ?)
```

**Maps to:** `CatalogWriter::rename_table(table_id, new_name)`

### Move Table to Schema

Changes the schema a table belongs to (ALTER TABLE SET SCHEMA).

```sql
UPDATE ducklake_tables
SET end_snapshot = ?
WHERE table_id = ? AND end_snapshot IS NULL

INSERT INTO ducklake_tables (table_id, schema_id, table_name, begin_snapshot)
VALUES (?, new_schema_id, ?, ?)
```

**Maps to:** `CatalogWriter::move_table(table_id, new_schema_id)`

### List Tables

```sql
SELECT table_id, table_name
FROM ducklake_tables
WHERE schema_id = ?
  AND begin_snapshot <= ?
  AND (end_snapshot IS NULL OR end_snapshot > ?)
```

**Maps to:** `CatalogReader::list_tables(schema_id, snapshot_id)`

---

## Column Operations

### Add Column

```sql
INSERT INTO ducklake_columns
  (table_id, column_name, data_type, column_index, is_nullable, default_value)
VALUES (?, ?, ?, ?, ?, ?)
```

**Maps to:** `CatalogWriter::add_column(table_id, column_def)`

### Drop Column

```sql
UPDATE ducklake_columns
SET end_snapshot = ?
WHERE column_id = ? AND end_snapshot IS NULL
```

**Maps to:** `CatalogWriter::drop_column(column_id)`

### Rename Column

```sql
UPDATE ducklake_columns
SET end_snapshot = ?
WHERE column_id = ? AND end_snapshot IS NULL

INSERT INTO ducklake_columns
  (column_id, table_id, column_name, data_type, column_index, is_nullable, begin_snapshot)
VALUES (?, ?, 'new_name', ?, ?, ?, ?)
```

**Maps to:** `CatalogWriter::rename_column(column_id, new_name)`

### Change Column Type

```sql
UPDATE ducklake_columns
SET end_snapshot = ?
WHERE column_id = ? AND end_snapshot IS NULL

INSERT INTO ducklake_columns
  (column_id, table_id, column_name, data_type, column_index, is_nullable, begin_snapshot)
VALUES (?, ?, ?, 'NEW_TYPE', ?, ?, ?)
```

**Maps to:** `CatalogWriter::alter_column_type(column_id, new_type)`

### Set Column Default

```sql
UPDATE ducklake_columns
SET end_snapshot = ?
WHERE column_id = ? AND end_snapshot IS NULL

INSERT INTO ducklake_columns
  (column_id, table_id, column_name, data_type, column_index, is_nullable, default_value, begin_snapshot)
VALUES (?, ?, ?, ?, ?, ?, 'new_default', ?)
```

**Maps to:** `CatalogWriter::set_column_default(column_id, default_expr)`

### List Columns

```sql
SELECT column_id, column_name, data_type, column_index, is_nullable, default_value
FROM ducklake_columns
WHERE table_id = ?
  AND begin_snapshot <= ?
  AND (end_snapshot IS NULL OR end_snapshot > ?)
ORDER BY column_index
```

**Maps to:** `CatalogReader::list_columns(table_id, snapshot_id)`

---

## Data File Operations

### Register Data File

Records that a new data file has been created for a table (after COPY or INSERT).

```sql
INSERT INTO ducklake_data_files
  (table_id, file_path, file_size_bytes, row_count, file_format, snapshot_id)
VALUES (?, ?, ?, ?, 'parquet', ?)
```

**Maps to:** `CatalogWriter::register_data_file(table_id, file_info)`

### Register Delete File

Records that rows in a data file have been logically deleted.

```sql
INSERT INTO ducklake_delete_files
  (table_id, file_path, data_file_id, snapshot_id)
VALUES (?, ?, ?, ?)
```

**Maps to:** `CatalogWriter::register_delete_file(table_id, delete_info)`

### Register Column Statistics

Records min/max/null statistics for a column in a data file.

```sql
INSERT INTO ducklake_file_column_stats
  (file_id, column_id, min_value, max_value, null_count, has_null)
VALUES (?, ?, ?, ?, ?, ?)
```

**Maps to:** `CatalogWriter::register_column_stats(file_id, column_id, stats)`

### List Data Files

```sql
SELECT file_id, file_path, file_size_bytes, row_count
FROM ducklake_data_files
WHERE table_id = ? AND snapshot_id <= ?
```

**Maps to:** `CatalogReader::list_data_files(table_id, snapshot_id)`

### Get Column Statistics

```sql
SELECT min_value, max_value, null_count, has_null
FROM ducklake_file_column_stats
WHERE file_id = ? AND column_id = ?
```

**Maps to:** `CatalogReader::get_column_stats(file_id, column_id)`

---

## View Operations

### Create View

```sql
INSERT INTO ducklake_views (schema_id, view_name, sql)
VALUES (?, ?, ?)
```

**Maps to:** `CatalogWriter::create_view(schema_id, name, sql)`

### Drop View

```sql
UPDATE ducklake_views
SET end_snapshot = ?
WHERE view_id = ? AND end_snapshot IS NULL
```

**Maps to:** `CatalogWriter::drop_view(view_id)`

### List Views

```sql
SELECT view_id, view_name, sql
FROM ducklake_views
WHERE schema_id = ?
  AND begin_snapshot <= ?
  AND (end_snapshot IS NULL OR end_snapshot > ?)
```

**Maps to:** `CatalogReader::list_views(schema_id, snapshot_id)`

---

## Macro Operations

### Create Macro

```sql
INSERT INTO ducklake_macros (schema_id, macro_name, macro_definition, parameters)
VALUES (?, ?, ?, ?)
```

**Maps to:** `CatalogWriter::create_macro(schema_id, name, definition, params)`

### Drop Macro

```sql
UPDATE ducklake_macros
SET end_snapshot = ?
WHERE macro_id = ? AND end_snapshot IS NULL
```

**Maps to:** `CatalogWriter::drop_macro(macro_id)`

---

## Transaction Operations

### Begin Transaction

Allocates a new snapshot ID and enters transaction state.

```sql
BEGIN TRANSACTION
```

**Maps to:** `Session::begin_transaction()`

**Behavior:** Increments the snapshot counter. Subsequent operations in this session operate within the transaction.

### Commit

Finalizes the current transaction, making all changes visible.

```sql
COMMIT
```

**Maps to:** `Session::commit()`

**Behavior:** Writes the accumulated WriteBatch to SlateDB as a single atomic WAL segment. Creates the snapshot row.

### Rollback

Discards the current transaction without writing anything.

```sql
ROLLBACK
```

**Maps to:** `Session::rollback()`

**Behavior:** Discards the in-memory WriteBatch. No storage writes occur. The allocated snapshot ID is wasted (gap in sequence is harmless).

---

## Snapshot Operations

### Get Current Snapshot

```sql
SELECT MAX(snapshot_id) FROM ducklake_snapshots
```

**Maps to:** `CatalogReader::current_snapshot()`

### List Snapshots

```sql
SELECT snapshot_id, timestamp, author, message
FROM ducklake_snapshots
WHERE snapshot_id >= ?
ORDER BY snapshot_id DESC
```

**Maps to:** `CatalogReader::list_snapshots(from_snapshot)`

---

## Session Operations

These statements are accepted for PostgreSQL wire protocol compatibility but have limited or no effect on catalog state:

```sql
-- Connection initialization (accepted, minimal effect)
SET client_encoding TO 'UTF8'
SET DateStyle TO 'ISO'
SET search_path TO 'main'
SET timezone TO 'UTC'

-- Version query (returns Rocklake version info)
SELECT version()

-- Protocol queries (return compatible responses)
SHOW server_version
SHOW server_encoding
```

**Maps to:** `Session::handle_set()` or `Session::handle_show()`

**Behavior:** These are recorded in session state but do not modify the catalog. They exist solely to satisfy DuckDB's connection initialization handshake.

---

## Unsupported Statements

Any SQL statement that does not match the patterns above is rejected with SQLSTATE 42601:

```
ERROR:  syntax_error
DETAIL: Statement not recognized by bounded SQL classifier
HINT:   Rocklake only accepts DuckLake protocol statements
```

Common statements that are NOT supported:

- `SELECT * FROM table` (Rocklake is not a query engine)
- `CREATE INDEX` (index metadata is registered, but arbitrary CREATE INDEX syntax varies)
- `ALTER TABLE ... ADD CONSTRAINT` (constraints are not tracked)
- `GRANT` / `REVOKE` (no authorization model)
- Arbitrary DML (`INSERT INTO`, `UPDATE`, `DELETE` on user tables)

## Further Reading

- **[Design Decisions: Bounded SQL](../design-decisions/bounded-sql.md)** — Why SQL support is limited
- **[Error Codes](error-codes.md)** — SQLSTATE codes for rejected statements
- **[Integration: DuckDB Compatibility](../integration/duckdb-compatibility.md)** — What DuckDB operations work
- Rocklake's classifier handles these variations through pattern matching rather than exact string comparison
- Statements not matching any known pattern are rejected with SQLSTATE 42601 (Syntax Error)
