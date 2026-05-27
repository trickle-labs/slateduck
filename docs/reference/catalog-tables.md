# Catalog Tables Reference

This page provides the authoritative specification of every table type stored in the Rocklake catalog. Each table type corresponds to a tag byte in the key encoding and a protobuf message type for its values. Understanding this schema is essential for contributors implementing new catalog operations, operators debugging storage contents, and anyone who needs to understand exactly what data Rocklake persists.

The DuckLake protocol defines 28 logical tables (SQL tables in the PostgreSQL-backed reference implementation). Rocklake stores these as key-value pairs in SlateDB, with each table type identified by a unique tag byte prefix. This page documents every table type, its fields, its key structure, and its relationships to other tables.

## Schema Conventions

Throughout this page:

- **Tag** is the first byte of the key, identifying the table type
- **Versioned** tables use begin_snapshot/end_snapshot for MVCC (multiple versions of the same logical row can coexist)
- **Unversioned** tables are point-in-time records (belong to a specific snapshot but are not updated)
- **Key structure** shows how the key is encoded (all multi-byte integers are big-endian for correct byte-level sort order)
- **u64** is an unsigned 64-bit integer
- **Option\<T\>** means the field may be absent (protobuf optional)

## DuckLake Protocol Tables

These tables implement the DuckLake catalog protocol. Their schema matches what DuckDB's `ducklake` extension expects when communicating with a catalog backend.

### ducklake_catalog (Tag 0x01)

The root catalog entry. Typically one row per catalog instance. This is the first entry written when a new catalog is initialized.

**Key structure:** `[0x01][catalog_id: u64][snapshot_id: u64]`

**Versioned:** Yes (begin_snapshot / end_snapshot)

| Field | Type | Description |
|-------|------|-------------|
| catalog_id | u64 | Unique catalog identifier (typically 1) |
| catalog_name | string | Human-readable catalog name |
| catalog_version | u64 | DuckLake schema version this catalog conforms to |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version (None if current) |

**Usage:** Read during initialization to verify format compatibility. Updated when catalog metadata changes (name, version).

**Relationship:** Parent of all schemas. The catalog_id is the top-level scope.

---

### ducklake_snapshot (Tag 0x02)

Records each catalog snapshot (atomic commit point). Every write transaction that commits creates exactly one snapshot row. Snapshots are never updated or deleted — they are an append-only audit log of all changes.

**Key structure:** `[0x02][snapshot_id: u64]`

**Versioned:** No (immutable once written)

| Field | Type | Description |
|-------|------|-------------|
| snapshot_id | u64 | Unique snapshot identifier (monotonically increasing) |
| timestamp | i64 | Unix timestamp in microseconds when the snapshot was created |
| author | string | Process or user that created this snapshot |
| message | string | Optional human-readable description of what changed |
| changes_count | u32 | Number of key-value mutations in this snapshot |

**Usage:** Time-travel queries use snapshots to select a point-in-time view. The `ducklake_snapshot` table answers "what snapshots exist?" and "when were they created?"

**Relationship:** Referenced by begin_snapshot and end_snapshot fields in all versioned tables.

---

### ducklake_schema (Tag 0x04)

Schema definitions (namespaces for tables). Schemas group related tables together — analogous to PostgreSQL schemas or database namespaces.

**Key structure:** `[0x04][schema_id: u64][!snapshot_id: u64]`

(The `!` prefix on snapshot_id indicates descending sort — higher snapshots sort first, so the latest version appears first in a prefix scan.)

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| schema_id | u64 | Unique schema identifier |
| schema_name | string | Schema name (e.g., "main", "analytics", "staging") |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version (None if current) |

**Usage:** Listed during DuckDB's `SHOW SCHEMAS` or `information_schema.schemata` queries. Referenced by table definitions.

**Relationship:** Parent of tables, views, and macros. Child of catalog.

**MVCC behavior:** When a schema is renamed, a new row is written with the same schema_id but a new name and new begin_snapshot. The old row gets end_snapshot set. Both coexist until GC removes the superseded version.

---

### ducklake_table (Tag 0x05)

Table definitions. A table is a named collection of columns that may have associated data files.

**Key structure:** `[0x05][schema_id: u64][table_id: u64][!snapshot_id: u64]`

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| table_id | u64 | Unique table identifier |
| schema_id | u64 | Schema this table belongs to |
| table_name | string | Table name |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version |

**Usage:** Listed during DuckDB's `SHOW TABLES` or `information_schema.tables` queries. Referenced by columns and data files.

**Relationship:** Child of schema. Parent of columns, data files, delete files.

**Key encoding note:** Tables are sorted within their parent schema (schema_id is part of the key prefix). This means listing all tables in a schema is a single prefix scan on `[0x05][schema_id]`.

---

### ducklake_column (Tag 0x06)

Column definitions. Each column belongs to a table and has a position, type, and nullability.

**Key structure:** `[0x06][table_id: u64][column_id: u64][!snapshot_id: u64]`

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| column_id | u64 | Unique column identifier |
| table_id | u64 | Table this column belongs to |
| column_name | string | Column name |
| data_type | string | DuckDB type name (e.g., "BIGINT", "VARCHAR", "TIMESTAMP") |
| column_index | u32 | Position in the table (0-based) |
| is_nullable | bool | Whether the column allows NULL values |
| default_value | Option\<string\> | Default expression (SQL literal or function) |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version |

**Usage:** Read when DuckDB queries table structure (column names, types, positions). Updated on ALTER TABLE ADD/DROP/RENAME COLUMN.

**Relationship:** Child of table. Referenced by file_column_stats (for pruning).

---

### ducklake_data_file (Tag 0x07)

Data file registrations. Each row represents one Parquet (or other format) file that contains data for a table. Files are registered when DuckDB completes a `COPY` or `INSERT` operation.

**Key structure:** `[0x07][table_id: u64][file_id: u64]`

**Versioned:** No (belongs to a specific snapshot, never updated)

| Field | Type | Description |
|-------|------|-------------|
| file_id | u64 | Unique file identifier |
| table_id | u64 | Table this file contains data for |
| snapshot_id | u64 | Snapshot that registered this file |
| file_path | string | Full path in object storage (e.g., `s3://bucket/data/part-001.parquet`) |
| file_size_bytes | u64 | Size of the data file in bytes |
| row_count | u64 | Number of rows in the file |
| file_format | string | File format (typically "parquet") |

**Usage:** Read during query planning — DuckDB fetches the list of data files to determine what to scan. File pruning uses snapshot_id to filter files visible at a given point in time.

**Relationship:** Child of table. May have associated file_column_stats and delete_files.

---

### ducklake_delete_file (Tag 0x08)

Delete file registrations. Delete files record which rows in a data file have been logically deleted. This implements row-level deletes without rewriting the original data file.

**Key structure:** `[0x08][table_id: u64][file_id: u64]`

**Versioned:** No

| Field | Type | Description |
|-------|------|-------------|
| file_id | u64 | Unique file identifier for this delete file |
| table_id | u64 | Table this delete applies to |
| snapshot_id | u64 | Snapshot that registered this delete |
| file_path | string | Path to the delete file in object storage |
| data_file_id | u64 | The data file whose rows are being deleted |

**Usage:** Applied during query execution — after reading a data file, the engine checks for associated delete files and filters out deleted rows.

**Relationship:** References a specific data_file. Child of table.

---

### ducklake_file_column_stats (Tag 0x09)

Per-column statistics for data files. These enable partition pruning — DuckDB can skip entire files if the query's filter predicate falls outside the file's min/max range.

**Key structure:** `[0x09][file_id: u64][column_id: u64]`

**Versioned:** No

| Field | Type | Description |
|-------|------|-------------|
| file_id | u64 | Data file these stats describe |
| column_id | u64 | Column these stats describe |
| min_value | Option\<bytes\> | Minimum value (type-aware binary encoding) |
| max_value | Option\<bytes\> | Maximum value (type-aware binary encoding) |
| null_count | u64 | Number of NULL values in this column for this file |
| has_null | bool | Whether any NULLs exist |
| distinct_count | Option\<u64\> | Estimated distinct values (if available) |

**Usage:** Read during query planning for predicate pushdown. If a WHERE clause references a column and the file's min/max range does not intersect the predicate, the entire file is skipped.

**Relationship:** Child of data_file and column.

**Encoding note:** min_value and max_value use type-aware binary encoding (see [Type-Aware Stats](../internals/type-aware-stats.md)) so that byte-level comparison gives correct results for the column's data type.

---

### ducklake_table_stats (Tag 0x0A)

Aggregate statistics for tables. Summary-level information that DuckDB uses for query planning without needing to read individual file stats.

**Key structure:** `[0x0A][table_id: u64][snapshot_id: u64]`

**Versioned:** No (snapshot-specific)

| Field | Type | Description |
|-------|------|-------------|
| table_id | u64 | Table these stats describe |
| snapshot_id | u64 | Snapshot these stats apply to |
| total_row_count | u64 | Total rows across all data files |
| total_file_count | u32 | Number of data files |
| total_size_bytes | u64 | Total data size |

**Usage:** DuckDB's optimizer uses row count and file count for cost estimation.

---

### ducklake_view (Tag 0x0B)

View definitions. A view is a named SQL query that can be referenced like a table.

**Key structure:** `[0x0B][schema_id: u64][view_id: u64][!snapshot_id: u64]`

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| view_id | u64 | Unique view identifier |
| schema_id | u64 | Schema this view belongs to |
| view_name | string | View name |
| sql | string | View definition SQL (the SELECT statement) |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version |

**Usage:** When DuckDB queries a view, it retrieves the SQL definition and executes it.

---

### ducklake_macro (Tag 0x0C)

Macro definitions (user-defined functions written in SQL).

**Key structure:** `[0x0C][schema_id: u64][macro_id: u64][!snapshot_id: u64]`

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| macro_id | u64 | Unique macro identifier |
| schema_id | u64 | Schema this macro belongs to |
| macro_name | string | Macro name |
| macro_definition | string | Macro SQL body |
| parameters | string | Comma-separated parameter list |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version |

---

### ducklake_table_macro (Tag 0x0D)

Table-returning macros (functions that return a table).

**Key structure:** `[0x0D][schema_id: u64][macro_id: u64][!snapshot_id: u64]`

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| macro_id | u64 | Unique macro identifier |
| schema_id | u64 | Schema this macro belongs to |
| macro_name | string | Macro name |
| macro_definition | string | Macro SQL body |
| parameters | string | Comma-separated parameter list |
| column_definitions | string | Return column definitions |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version |

---

### ducklake_index (Tag 0x0E)

Index definitions. DuckLake supports index metadata (for query optimization hints) though the actual index structures are managed by DuckDB.

**Key structure:** `[0x0E][table_id: u64][index_id: u64][!snapshot_id: u64]`

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| index_id | u64 | Unique index identifier |
| table_id | u64 | Table this index belongs to |
| index_name | string | Index name |
| index_type | string | Index type (e.g., "ART", "MINMAX") |
| column_ids | string | Comma-separated column IDs |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version |

---

### ducklake_type (Tag 0x0F)

Custom type definitions (user-defined types, enums).

**Key structure:** `[0x0F][schema_id: u64][type_id: u64][!snapshot_id: u64]`

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| type_id | u64 | Unique type identifier |
| schema_id | u64 | Schema this type belongs to |
| type_name | string | Type name |
| type_definition | string | Type body (enum values, struct definition) |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version |

---

### ducklake_sequence (Tag 0x10)

Sequence definitions (auto-incrementing generators).

**Key structure:** `[0x10][schema_id: u64][sequence_id: u64][!snapshot_id: u64]`

**Versioned:** Yes

| Field | Type | Description |
|-------|------|-------------|
| sequence_id | u64 | Unique sequence identifier |
| schema_id | u64 | Schema this sequence belongs to |
| sequence_name | string | Sequence name |
| start_value | i64 | Starting value |
| increment | i64 | Increment step |
| current_value | i64 | Current value |
| begin_snapshot | u64 | Snapshot that created this version |
| end_snapshot | Option\<u64\> | Snapshot that superseded this version |

---

## System Tables

System tables store operational state that is not part of the DuckLake protocol. They use reserved high-value tag bytes to avoid collision with protocol tables.

### counter (Tag 0xFE)

ID allocation counters. These track the next available ID for each entity type. The writer increments these atomically during transactions.

**Key structure:** `[0xFE][counter_name: string]`

**Versioned:** No (overwritten in place)

| Key Suffix | Type | Description |
|------------|------|-------------|
| `next_snapshot_id` | u64 | Next snapshot ID to allocate |
| `next_schema_id` | u64 | Next schema ID to allocate |
| `next_table_id` | u64 | Next table ID to allocate |
| `next_column_id` | u64 | Next column ID to allocate |
| `next_file_id` | u64 | Next file/delete-file ID to allocate |
| `next_view_id` | u64 | Next view ID to allocate |
| `next_macro_id` | u64 | Next macro ID to allocate |

**Usage:** Read and incremented during each write transaction. Multiple IDs may be allocated in a single transaction (e.g., creating a table allocates a table ID and multiple column IDs).

**Crash safety:** If the process crashes after incrementing a counter but before committing the WriteBatch, the counter is NOT incremented (the increment was only in the uncommitted batch). On restart, the same IDs are re-allocated. Gaps in ID sequences are harmless — nothing depends on contiguity.

---

### system (Tag 0xFF)

System configuration and operational state. These keys store critical metadata about the catalog's health and configuration.

**Key structure:** `[0xFF][key_name: string]`

**Versioned:** No (overwritten in place)

| Key Name | Type | Description |
|----------|------|-------------|
| `catalog-format-version` | u32 | Format version of this catalog (for upgrade detection) |
| `writer-epoch` | u64 | Current writer epoch (for fencing) |
| `retain-from` | u64 | GC retention horizon (snapshots below this are inaccessible) |
| `hot-key` | bytes | Cached frequently-read metadata (serialized composite) |
| `ducklake-version` | string | DuckLake protocol version this catalog supports |

**Usage:**
- `catalog-format-version`: Read on startup to verify the binary can handle this catalog's format
- `writer-epoch`: Read by the writer to detect fencing; incremented when a new writer starts
- `retain-from`: Read by readers to reject time-travel queries for snapshots below this horizon
- `hot-key`: Read on every operation (cached in memory to avoid storage round-trip)

---

## Key Encoding Summary

All keys follow the pattern: `[tag: u8][...components: big-endian integers]`

Big-endian encoding ensures that byte-level lexicographic sort matches numeric sort order. This is critical because SlateDB (the underlying store) provides only byte-level ordering.

The `!` prefix (descending sort) is achieved by XOR-ing the integer with `u64::MAX` before encoding. This flips the sort order so that the highest value sorts first.

| Table | Tag | Key Components | Sort Order |
|-------|-----|----------------|------------|
| catalog | 0x01 | catalog_id, snapshot_id | catalog ASC, snapshot DESC |
| snapshot | 0x02 | snapshot_id | snapshot ASC |
| schema | 0x04 | schema_id, snapshot_id | schema ASC, snapshot DESC |
| table | 0x05 | schema_id, table_id, snapshot_id | schema ASC, table ASC, snapshot DESC |
| column | 0x06 | table_id, column_id, snapshot_id | table ASC, column ASC, snapshot DESC |
| data_file | 0x07 | table_id, file_id | table ASC, file ASC |
| delete_file | 0x08 | table_id, file_id | table ASC, file ASC |
| file_stats | 0x09 | file_id, column_id | file ASC, column ASC |
| table_stats | 0x0A | table_id, snapshot_id | table ASC, snapshot ASC |
| view | 0x0B | schema_id, view_id, snapshot_id | schema ASC, view ASC, snapshot DESC |
| macro | 0x0C | schema_id, macro_id, snapshot_id | schema ASC, macro ASC, snapshot DESC |
| counter | 0xFE | counter_name | name ASC |
| system | 0xFF | key_name | name ASC |

## Further Reading

- **[Architecture: Key Layout](../architecture/key-layout.md)** — Detailed explanation of key encoding rationale
- **[Internals: Tag Allocation](../internals/tag-allocation.md)** — How tag bytes are assigned
- **[Internals: Type-Aware Stats](../internals/type-aware-stats.md)** — How min/max values are encoded
- **[Concepts: Key-Value Mapping](../architecture/key-layout.md)** — How SQL tables become key-value pairs
