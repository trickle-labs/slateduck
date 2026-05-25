# Key Layout

Every piece of catalog metadata in SlateDuck is stored as a key-value pair in SlateDB's LSM-tree. The key encoding is the foundation upon which everything else is built: efficient prefix scans, lexicographic ordering that matches logical ordering, hierarchical relationships without secondary indexes, and MVCC version ranges embedded directly in the key structure. Understanding the key layout is essential for contributors working on the catalog store and valuable for operators who want to reason about scan performance and storage characteristics.

This page documents the complete key schema, explains the encoding principles behind it, walks through the common access patterns it optimizes, and discusses the trade-offs that shaped its design.

## Design Principles

The key layout follows five invariant principles:

### 1. Tag-First Ordering

The first byte of every key is a "tag" — a single byte that identifies which catalog table (or internal entity) the key belongs to. Tags range from `0x01` to `0x1C` for DuckLake catalog tables and `0xFC` to `0xFF` for internal tables. This means that all keys for the same logical table are lexicographically adjacent in the LSM-tree, regardless of their other key fields. A prefix scan with a single-byte prefix efficiently retrieves all entries for a given catalog table.

### 2. Big-Endian Integer Encoding

Multi-byte integers (u64, u32) are encoded in big-endian (network byte order). This is critical because SlateDB stores keys as raw byte sequences and sorts them lexicographically. Big-endian encoding ensures that the lexicographic byte ordering matches the numeric ordering: `encode(1) < encode(2) < encode(1000)`. If we used little-endian encoding, `1000` would sort before `2` because its least-significant byte happens to be smaller.

### 3. Fixed-Width Fields

Most key components are fixed-width u64 values (8 bytes). This makes key parsing trivial — you know exactly where each field starts and ends without scanning for delimiters or reading length prefixes. Fixed-width encoding also means that all keys for a given table type have the same length, which simplifies debugging and makes storage estimates predictable.

### 4. Hierarchical Parent-First Ordering

When a key encodes a parent-child relationship (for example, a column within a table), the parent ID comes before the child ID in the key. This means all children of a given parent are lexicographically contiguous: all columns for table 42 share the prefix `0x06 | 0x000000000000002A` (tag for columns, followed by big-endian 42). A prefix scan on this byte sequence returns exactly the columns for table 42 and nothing else.

### 5. Version as Trailing Field

For versioned entities (schemas, tables, columns, views, macros), the `begin_snapshot` is the last field in the key. This means all versions of the same entity are adjacent and ordered chronologically: version at snapshot 5 sorts before version at snapshot 10. To find the "current" version of an entity, you scan its prefix and take the last entry where `begin_snapshot <= target_snapshot` and `end_snapshot` is NULL or greater than `target_snapshot`.

## Complete Key Schema

### DuckLake Catalog Tables (Tags 0x01 – 0x1C)

| Tag | Table Name | Key Fields | Total Key Size |
|-----|-----------|------------|----------------|
| `0x01` | ducklake_metadata | `scope_enum(u8) \| scope_id(u64) \| key_len(u16) \| key_bytes` | 12 + key_len |
| `0x02` | ducklake_snapshot | `snapshot_id(u64)` | 9 |
| `0x03` | ducklake_snapshot_changes | `snapshot_id(u64)` | 9 |
| `0x04` | ducklake_schema | `schema_id(u64) \| begin_snapshot(u64)` | 17 |
| `0x05` | ducklake_table | `schema_id(u64) \| table_id(u64) \| begin_snapshot(u64)` | 25 |
| `0x06` | ducklake_column | `table_id(u64) \| column_id(u64) \| begin_snapshot(u64)` | 25 |
| `0x07` | ducklake_view | `schema_id(u64) \| view_id(u64) \| begin_snapshot(u64)` | 25 |
| `0x08` | ducklake_macro | `schema_id(u64) \| macro_id(u64) \| begin_snapshot(u64)` | 25 |
| `0x09` | ducklake_macro_impl | `macro_id(u64) \| impl_id(u64)` | 17 |
| `0x0A` | ducklake_macro_parameters | `macro_id(u64) \| impl_id(u64) \| column_id(u64)` | 25 |
| `0x0B` | ducklake_data_file | `table_id(u64) \| data_file_id(u64)` | 17 |
| `0x0C` | ducklake_delete_file | `data_file_id(u64) \| delete_file_id(u64)` | 17 |
| `0x0D` | ducklake_files_scheduled_for_deletion | `schedule_start(u64) \| data_file_id(u64)` | 17 |
| `0x0E` | ducklake_inlined_data_tables | `table_id(u64) \| schema_version(u64)` | 17 |
| `0x0F` | ducklake_column_mapping | `table_id(u64) \| column_id(u64)` | 17 |
| `0x10` | ducklake_name_mapping | `table_id(u64) \| column_id(u64)` | 17 |
| `0x11` | ducklake_table_stats | `table_id(u64)` | 9 |
| `0x12` | ducklake_file_column_stats | `table_id(u64) \| column_id(u64) \| data_file_id(u64)` | 25 |
| `0x13` | ducklake_file_variant_stats | `table_id(u64) \| column_id(u64) \| data_file_id(u64)` | 25 |
| `0x14` – `0x1C` | partition_info, partition_columns, file_partition_values, sort_info, sort_expressions, tags, column_tags, schema_versions | (various) | (various) |

### Internal Tables (Tags 0xFC – 0xFF)

| Tag | Purpose | Key Fields | Total Key Size |
|-----|---------|-----------|----------------|
| `0xFC` | Secondary index | `snapshot_id(u64) \| table_id(u64) \| data_file_id(u64)` | 25 |
| `0xFD` | Inlined data | `subtype(u8) \| table_id(u64) \| (schema_version or data_file_id)(u64) \| row_id(u64)` | 26 |
| `0xFE` | Counters | `counter_id(u8)` | 2 |
| `0xFF` | System keys | `suffix_bytes` (variable, e.g. "writer-epoch", "retain-from") | 1 + suffix_len |

## Detailed Key Anatomy

Let's examine several key types in detail to make the encoding concrete.

### Schema Key: `0x04 | schema_id | begin_snapshot`

```
Byte offset:  0    1         9         17
             ┌────┬─────────┬─────────┐
             │0x04│schema_id│begin_snap│
             └────┴─────────┴─────────┘
              tag   8 bytes   8 bytes
```

A schema with ID 3 created at snapshot 7 has the key: `04 00000000 00000003 00000000 00000007`.

If the schema is renamed at snapshot 15, the old row gets `end_snapshot = 15` in its value (the key is unchanged), and a new row is written with key `04 00000000 00000003 00000000 0000000F`. Both rows share the same `schema_id`, so scanning prefix `04 00000000 00000003` returns all versions of schema 3.

### Table Key: `0x05 | schema_id | table_id | begin_snapshot`

```
Byte offset:  0    1         9         17        25
             ┌────┬─────────┬─────────┬─────────┐
             │0x05│schema_id│table_id │begin_snap│
             └────┴─────────┴─────────┴─────────┘
              tag   8 bytes   8 bytes   8 bytes
```

Tables are keyed by their parent schema first. This means scanning prefix `05 00000000 00000001` returns all tables (all versions) in schema 1. The tables are further ordered by table_id, and within the same table, versions are ordered by begin_snapshot.

### Data File Key: `0x0B | table_id | data_file_id`

```
Byte offset:  0    1         9         17
             ┌────┬─────────┬─────────┐
             │0x0B│table_id │file_id  │
             └────┴─────────┴─────────┘
              tag   8 bytes   8 bytes
```

Data files do not have `begin_snapshot` in the key because they are append-only (never versioned). Once a data file is registered, its key-value pair is immutable. The file's visibility is determined by `begin_snapshot` stored in the value — this is checked during MVCC filtering.

Why not put `begin_snapshot` in the key for data files? Because data files are never superseded (they do not have multiple versions). Putting the snapshot in the key would waste 8 bytes per entry and provide no scan benefit.

## Prefix Scan Patterns

The key layout is optimized for these common access patterns:

### List All Tables in a Schema

**Scan prefix:** `0x05 | schema_id`

Returns all table versions (including historical) for that schema, ordered by table_id then begin_snapshot. The executor applies MVCC filtering in memory to find the currently-visible version of each table.

### List All Columns for a Table

**Scan prefix:** `0x06 | table_id`

Returns all column versions for that table, ordered by column_id then begin_snapshot. This is the most frequent scan pattern — it is called every time DuckDB resolves a table's column list.

### List All Data Files for a Table

**Scan prefix:** `0x0B | table_id`

Returns all data files registered for a table, ordered by data_file_id. The MVCC filter (checking `begin_snapshot` in the value) removes files registered after the reader's snapshot.

### List Column Statistics for Query Planning

**Scan prefix:** `0x12 | table_id`

Returns column-level min/max statistics for all columns across all files in a table. DuckDB uses these for predicate pushdown during query planning — it can skip entire files whose column ranges do not intersect the query's WHERE clause.

### List All Snapshots

**Scan prefix:** `0x02`

Returns all snapshot records in chronological order (because snapshot_id increases monotonically). Used by `ducklake_snapshots()` to show catalog history.

## MVCC and Key Ordering

For versioned entities (schemas, tables, columns, views, macros), the key includes `begin_snapshot` as the trailing field. This creates an important property: **all versions of the same entity are contiguous in key space**.

Consider a table that has been renamed three times:

| Key | Meaning |
|-----|---------|
| `05 | schema=1 | table=42 | snap=5` | Created with name "orders" |
| `05 | schema=1 | table=42 | snap=10` | Renamed to "transactions" |
| `05 | schema=1 | table=42 | snap=20` | Renamed to "order_history" |

To find the table's name at snapshot 15, scan prefix `05 | schema=1 | table=42`, iterate through the results, and find the version where `begin_snapshot <= 15 AND (end_snapshot IS NULL OR end_snapshot > 15)`. In this case, it is the second entry (begin=10, end=20), so the table was called "transactions" at snapshot 15.

This scan is efficient because it touches at most N entries, where N is the number of versions of that specific entity (typically 1–5 for most entities, since renames and alterations are rare).

## Counter Keys

Counter keys are minimal: `0xFE | counter_id(u8)`. The value is a big-endian u64 representing the next available ID.

| Counter ID | Meaning | Initial Value |
|-----------|---------|---------------|
| `0x01` | next_snapshot_id | 1 |
| `0x02` | next_catalog_id | 1 |
| `0x03` | next_file_id | 1 |
| `0x04+` | next_column_id per table | 1 |

Counters are updated atomically in the same `WriteBatch` as the rows they reference. This ensures that an allocated ID is always paired with its corresponding row. If the process crashes after allocating an ID but before writing the row, both the counter update and the row write are lost together (neither is visible to subsequent readers).

The counter for `next_catalog_id` is a shared counter used for schema IDs, table IDs, view IDs, and macro IDs. These entities share a single namespace to simplify ID allocation — any catalog entity gets the next available number regardless of its type.

## System Keys

System keys use tag `0xFF` followed by a variable-length ASCII suffix:

| Key | Purpose | Value Type |
|-----|---------|-----------|
| `0xFF \| "writer-epoch"` | Current writer epoch for fencing | u64 |
| `0xFF \| "retain-from"` | GC retention horizon (0 = infinite) | u64 (snapshot_id) |
| `0xFF \| "catalog-format-version"` | Schema format version | u32 |
| `0xFF \| "hot-key"` | Packed current state for cold start | protobuf |
| `0xFF \| "pin:" \| snapshot_id` | Pinned snapshot marker | JSON metadata |

System keys are sparse — typically fewer than 20 entries in any catalog. They are accessed by exact-key lookup, not prefix scan. The variable-length suffix is a pragmatic choice: system keys are rarely scanned, so the cost of variable-length parsing is negligible, and the human-readable suffixes make debugging easier (you can see "writer-epoch" in hex dumps rather than trying to decode a numeric ID).

## Storage Efficiency

The key layout is designed for compactness. Let's calculate the storage overhead for a typical catalog:

| Entity | Count | Key Size | Value Size (avg) | Total |
|--------|-------|----------|------------------|-------|
| Schemas | 5 | 17 bytes | 50 bytes | 335 B |
| Tables | 50 | 25 bytes | 80 bytes | 5.25 KB |
| Columns | 500 | 25 bytes | 100 bytes | 62.5 KB |
| Data files | 10,000 | 17 bytes | 200 bytes | 2.17 MB |
| Column stats | 50,000 | 25 bytes | 60 bytes | 4.25 MB |
| Snapshots | 1,000 | 9 bytes | 50 bytes | 59 KB |

A catalog with 50 tables, 500 columns, and 10,000 data files occupies approximately 6.5 MB including all historical versions. The dominant storage cost is column statistics (which are optional and can be disabled for reduced storage at the cost of query planning efficiency).

## Design Trade-Offs

### Why Not Composite String Keys?

Some key-value systems use string keys like `/schema/1/table/42/column/7`. SlateDuck uses binary encoding instead because:

- Binary keys are shorter (25 bytes vs. 30+ bytes for the equivalent string)
- Binary comparison is faster (memcmp vs. string parsing)
- No ambiguity in delimiter handling (what if an entity name contains `/`?)
- Fixed-width encoding enables pointer arithmetic without parsing

### Why Not Separate MVCC from Entity Keys?

An alternative design would store `begin_snapshot` and `end_snapshot` only in the value, not in the key. SlateDuck includes `begin_snapshot` in the key for versioned entities because:

- It makes all versions of an entity contiguous (important for efficient version resolution)
- It enables writing a new version without modifying the old one (the old key is untouched; only its value gets `end_snapshot` updated)
- It prevents key collisions (two versions of the same entity at different snapshots have different keys)

### Why Not Include `end_snapshot` in the Key?

Only `begin_snapshot` is in the key, not `end_snapshot`. This is because `end_snapshot` is mutable — when an entity is superseded, its existing entry's value is updated with the `end_snapshot`. If `end_snapshot` were part of the key, superseding an entity would require deleting the old key and writing a new one (which is more expensive in an LSM-tree than updating a value in place through a new write at the same key).

## Further Reading

- **[Value Encoding](value-encoding.md)** — What goes in the value side of each key-value pair
- **[MVCC Implementation](mvcc-implementation.md)** — How version filtering uses these keys
- **[SQL Dispatcher](sql-dispatcher.md)** — How SQL queries map to prefix scan patterns
- **[Internals: Tag Allocation](../internals/tag-allocation.md)** — How new tag values are assigned as the format evolves
- **[IVM Plane](ivm-plane.md)** — Architecture of the v0.11 IVM extensions

## v0.11 IVM Tag Extensions

Tag bytes `0x1D`–`0x20` are additive IVM extensions introduced in v0.11.
They follow the same key-encoding conventions as earlier tags.

| Tag    | Table                            | Key shape                                                         | MVCC             |
|--------|----------------------------------|-------------------------------------------------------------------|------------------|
| `0x1D` | `slateduck_matview`              | `tag \| matview_id(u64 BE) \| begin_snapshot(u64 BE)`            | Versioned        |
| `0x1E` | `slateduck_matview_dep`          | `tag \| matview_id(u64 BE) \| base_table_id(u64 BE)`             | AppendOnly       |
| `0x1F` | `slateduck_matview_checkpoint`   | `tag \| matview_id(u64 BE) \| shard_id(u32 BE) \| seq(u64 BE)`   | AppendOnly       |
| `0x20` | `slateduck_matview_shard`        | `tag \| matview_id(u64 BE) \| shard_id(u32 BE)`                  | MutableSingleton |

Tag bytes `0x21`–`0x2F` are reserved for future IVM-related tables.

### Tag Extensibility Guarantees

- **Unknown tags are skipped** during prefix scans. The first byte of every key is checked against `ALL_TAGS`; unknown values produce a `KeyError::UnknownTag` on direct access but are silently skipped during iteration.
- **Older binaries** that do not know about IVM tags will never encounter `0x1D`–`0x20` keys during normal DuckLake catalog operations because those operations use tag-specific prefix scans (e.g. `prefix_for_tag(TAG_TABLE)` returns `[0x04]`).
- **Forward compatibility**: a v0.10 reader scanning all keys and encountering `0x1D` bytes should skip them. This is enforced by the `is_known_tag()` check in `extract_tag()`.
