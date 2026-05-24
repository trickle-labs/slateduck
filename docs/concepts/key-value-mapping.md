# Key-Value Mapping

SlateDuck stores relational catalog concepts — schemas, tables, columns, data files, snapshots — in a key-value store where keys are opaque byte sequences with careful internal structure. Understanding this mapping helps you reason about scan performance, predict which operations are fast, understand the output of debugging tools, and grasp why certain query patterns are possible (or impossible) with SlateDuck's architecture. If you have ever wondered why SlateDuck can serve a "list all files for this table" query in single-digit milliseconds but cannot efficiently answer "find all tables with a column named 'email'"  — the key structure explains exactly why.

## The Design Challenge

A relational catalog has natural hierarchies and relationships: schemas contain tables, tables contain columns, tables reference data files, data files have column statistics. To store this hierarchy in a flat key-value namespace without sacrificing performance, SlateDuck encodes relationships directly into the key bytes using a tag-prefixed, big-endian encoding scheme that preserves lexicographic ordering. The result is a key layout where the most common access patterns in the DuckLake protocol — "give me all columns for table X" or "give me all files for table Y" — translate to single prefix scans that touch only the relevant keys, never the entire key space.

Every design decision in the key layout flows from one constraint: the most common query pattern for each entity type must be servable with a single prefix scan, not a full table scan. This constraint is what makes the catalog fast, and it is why certain keys look the way they do.

## Key Structure

Every key in SlateDuck's catalog follows a simple pattern:

```
[tag: 1 byte] [composite key fields: variable length]
```

The first byte is the **tag**, which identifies which catalog table the entry belongs to. After the tag, the remaining bytes encode the composite key fields for that table, with multi-byte integers stored in **big-endian** format.

Big-endian encoding is critical and worth pausing on. In a key-value store like SlateDB, keys are sorted lexicographically — byte by byte, from left to right. If you store integer IDs in little-endian format (least-significant byte first, as x86 processors natively use), the numeric ordering does not match the byte ordering. For example, the integer 256 in little-endian is `0x00 0x01`, but 255 is `0xFF 0x00`. Lexicographically, `0xFF` sorts after `0x00`, so 255 would sort after 256 — wrong numeric order. With big-endian encoding, 255 is `0x00 0xFF` and 256 is `0x01 0x00`, and `0x00` sorts before `0x01`, so 255 sorts before 256. Numeric and lexicographic ordering agree.

This matters because prefix scans in SlateDB use lexicographic ordering. "Give me all entries with the prefix `[table_tag][schema_id=1][table_id=42]`" works correctly only if entries with the same prefix sort together, which requires big-endian encoding for numeric fields.

## The Tag Registry

SlateDuck allocates tags from a fixed, pre-planned registry. The full 28-entry DuckLake catalog table space is allocated even for tables not yet implemented, which prevents tag collision if new tables are added in the future:

| Tag Range | Purpose |
|-----------|---------|
| `0x01` | `ducklake_metadata` — schema-level key-value metadata |
| `0x02` | `ducklake_table_metadata` — table-level key-value metadata |
| `0x03` | `ducklake_snapshot` — catalog snapshots |
| `0x04` | `ducklake_schema` — schema definitions |
| `0x05` | `ducklake_table` — table definitions |
| `0x06` | `ducklake_column` — column definitions |
| `0x07` | `ducklake_view` — view definitions |
| `0x08` | `ducklake_macro` — macro definitions |
| `0x09` | `ducklake_table_stats` — table-level statistics |
| `0x0A` | `ducklake_tag` — table and column tags |
| `0x0B` | `ducklake_data_file` — data file registrations |
| `0x0C` | `ducklake_data_file_column_statistics` — per-file column statistics |
| `0x0D` | `ducklake_delete_file` — deletion vectors |
| `0x0E`–`0x1C` | Remaining DuckLake catalog tables (reserved for future use) |
| `0xFC` | Secondary index for snapshot-scoped file lookups |
| `0xFD` | Inlined data (small inserts/deletes stored in the catalog key space) |
| `0xFE` | Counters — auto-incrementing ID generators |
| `0xFF` | System keys — writer epoch, retention settings, format version, audit log |

The tag provides perfect namespace isolation at the byte level. A prefix scan for `0x05` (all tables) will never accidentally include entries from `0x06` (columns) or `0x0B` (data files). The namespaces are completely disjoint.

The high-value range `0xFD`–`0xFF` uses tags that sort after all DuckLake catalog entries, which is intentional: when you scan the full key space from beginning to end (as a debugging tool might), you see catalog entries first, then inlined data, then counters, then system keys — a sensible reading order.

## Key Layouts for Each Entity

### Schemas (`0x04`)

```
[0x04] [schema_id: 8 bytes BE] [begin_snapshot: 8 bytes BE]
```

**Access pattern:** "Give me all schemas visible at snapshot N" is a prefix scan on `[0x04]` with MVCC filtering. Since schemas are few, this is typically a handful of entries.

**Example:** Schema ID 1, created at snapshot 3:
```
0x04 | 0x0000000000000001 | 0x0000000000000003
```

### Tables (`0x05`)

```
[0x05] [schema_id: 8 bytes BE] [table_id: 8 bytes BE] [begin_snapshot: 8 bytes BE]
```

**Access pattern:** "Give me all tables in schema 2" is a prefix scan on `[0x05][0x0000000000000002]`. Only entries belonging to schema 2 fall within this prefix.

**Example:** Table ID 42 in schema ID 1, created at snapshot 7:
```
0x05 | 0x0000000000000001 | 0x000000000000002A | 0x0000000000000007
```

Notice that `schema_id` comes before `table_id` in the key. This is the "most common access pattern first" principle: the query "list tables in schema X" needs a prefix scan by `schema_id`, so `schema_id` must come immediately after the tag. If `table_id` came first, there would be no way to efficiently list tables for a given schema.

### Columns (`0x06`)

```
[0x06] [table_id: 8 bytes BE] [column_id: 8 bytes BE] [begin_snapshot: 8 bytes BE]
```

**Access pattern:** "Give me all columns for table 42" is a prefix scan on `[0x06][0x000000000000002A]`.

**Example:** Column ID 3 in table 42, created at snapshot 7:
```
0x06 | 0x000000000000002A | 0x0000000000000003 | 0x0000000000000007
```

Note that `schema_id` does not appear in the column key — the column key uses `table_id` directly. This is because column lookups are always scoped by table, never by schema. If you want all columns in schema 2, you first look up all tables in schema 2 (prefix scan on `0x05`), then for each table you look up its columns (prefix scan on `0x06`). The two-pass approach is efficient because each individual scan is fast.

### Data Files (`0x0B`)

```
[0x0B] [table_id: 8 bytes BE] [file_id: 8 bytes BE]
```

**Access pattern:** "Give me all data files for table 42" is a prefix scan on `[0x0B][0x000000000000002A]`.

**Example:** File ID 100 for table 42:
```
0x0B | 0x000000000000002A | 0x0000000000000064
```

Note that data file keys do not include `begin_snapshot` as a suffix. This is a difference from schema/table/column keys. Data files use a different versioning mechanism: the `begin_snapshot` and `end_snapshot` values are stored in the *value* (the Protobuf payload) rather than the key. This is because data file updates (setting `end_snapshot`) are less frequent than the MVCC filtering needs, and the secondary index at tag `0xFC` provides efficient snapshot-scoped file lookups when needed.

### Snapshots (`0x03`)

```
[0x03] [snapshot_id: 8 bytes BE]
```

**Access pattern:** "Give me the most recent snapshot" is a reverse scan from the maximum key in the `0x03` prefix — the highest snapshot ID sorts last.

**Example:** Snapshot ID 15:
```
0x03 | 0x000000000000000F
```

### Counters (`0xFE`)

```
[0xFE] [counter_name: variable bytes]
```

SlateDuck maintains three global counters: `next_snapshot_id`, `next_catalog_id`, and `next_file_id`. Plus one per-table counter: `column_id_{table_id}`. Each is stored as a key in the `0xFE` namespace with a string name suffix.

**Example:** The `next_snapshot_id` counter:
```
0xFE | b"next_snapshot_id"
```

Counter values are Protobuf-encoded `CounterValue` messages containing the current counter value.

### System Keys (`0xFF`)

```
[0xFF] [key_name: variable bytes]
```

System keys store global catalog state that does not fit the MVCC versioning model:

- `0xFF | "writer-epoch"` — The current writer epoch, used for fencing stale writers
- `0xFF | "retain-from"` — The oldest snapshot ID that remains query-accessible
- `0xFF | "excised"` — Audit trail of physical deletion operations
- `0xFF | "catalog-format-version"` — The format version of this catalog (currently v1)

**Example:** The `retain-from` key:
```
0xFF | b"retain-from"
```

## MVCC and Key Uniqueness

For versioned tables (schemas, tables, columns, views, macros), the `begin_snapshot` is part of the key. This means multiple versions of the same logical entity have different keys and coexist in storage simultaneously:

```
# Table 42 before rename:
0x05 | schema=1 | table=42 | begin_snapshot=7   → value: {name="events", end_snapshot=12}

# Table 42 after rename at snapshot 12:
0x05 | schema=1 | table=42 | begin_snapshot=12  → value: {name="user_events", end_snapshot=NULL}
```

When a reader is operating at snapshot 10, it scans all entries with the prefix `[0x05][schema=1]` and applies the visibility filter: a row is visible if `begin_snapshot <= 10 AND (end_snapshot IS NULL OR 10 < end_snapshot)`. The first row has `begin_snapshot=7 <= 10` and `end_snapshot=12 > 10`, so it is visible. The second row has `begin_snapshot=12 > 10`, so it is not visible. The reader sees `events`.

At snapshot 15, the first row has `end_snapshot=12 <= 15`, so it is no longer visible (it was superseded before snapshot 15). The second row has `begin_snapshot=12 <= 15` and `end_snapshot=NULL`, so it is visible. The reader sees `user_events`.

Both rows exist permanently in storage — the rename did not overwrite anything. This is the physical implementation of catalog immutability: history is preserved because historical versions are distinct keys.

## The Secondary Index (`0xFC`)

The data file access pattern has a subtlety: for large tables with many data files registered across many snapshots, a plain prefix scan on `[0x0B][table_id]` returns all data files ever registered, including those that have been superseded (with `end_snapshot` set). MVCC filtering then needs to discard the superseded entries.

For a table with 100,000 data file registrations across 500 schema-change snapshots, this could return many irrelevant entries before filtering. The secondary index at tag `0xFC` provides a different access path: it is keyed by `[table_id][snapshot_range]` and allows efficiently finding only the files visible at a specific snapshot without scanning all historical entries.

This is the classic database trade-off: the secondary index takes extra storage and requires maintenance on every write, but it makes the most common read operation — "files visible at the current snapshot" — faster for large, active tables.

## Value Encoding

Values are wrapped in a lightweight envelope format that provides corruption detection and forward compatibility:

```
[encoding_version: 1 byte] [magic: b"SDKV" 4 bytes] [protobuf payload: variable]
```

The `SDKV` magic serves as a corruption detector: if a read returns bytes where the first four bytes after the version byte do not match `SDKV`, the decoder refuses to proceed and returns an error. This catches cases where a key lookup returns the wrong entry (a logic bug) or where storage has been partially corrupted.

The `encoding_version` byte enables forward compatibility: if a future version of SlateDuck introduces a different encoding for some entry type, it can use a different version byte. Old readers encountering an unknown version byte return an error rather than silently misinterpreting the data. New readers can handle both old and new versions by checking the version byte first.

The payload is a Protobuf-encoded message whose schema is determined by the tag. Tag `0x05` values decode as `TableRow`, tag `0x06` as `ColumnRow`, tag `0x0B` as `DataFileRow`, and so on. Each row type is defined in the `slateduck-core/src/rows.rs` Protobuf definitions.

## Performance Implications

The key structure directly determines which operations are fast and which are slow:

**Fast — single prefix scan:** Listing tables in a schema, columns in a table, data files for a table. These are all O(results), never O(total entries).

**Fast — point lookup:** Looking up a specific entity by its full key. SlateDB's bloom filters make point lookups efficient even when the table has millions of entries.

**Moderate — two-pass lookup:** Describing a table requires two prefix scans — one for the `TableRow` and one for all `ColumnRow` entries. This is O(columns), which is typically small.

**Slow — cross-tag aggregation:** Finding all tables across all schemas that have a column named `email` would require scanning the entire `0x06` namespace and filtering in memory. SlateDuck does not expose this as a query (it is not in the bounded SQL set), but understanding why helps explain what the bounded set excludes.

**Slow — unkeyed filter:** Finding all data files with more than 1 million rows would require scanning all data file entries and filtering on the row count value. Again, not supported in the bounded SQL set, and the key structure explains why.

The key design is optimized specifically for the DuckLake protocol's access patterns. For that protocol, it is excellent. For general SQL access patterns, it would be limiting — but general SQL access patterns are not what SlateDuck is designed to serve.

## Further Reading

- **[Key Layout Architecture](../architecture/key-layout.md)** — The complete reference with byte-level diagrams for all 28 table types
- **[Value Encoding Architecture](../architecture/value-encoding.md)** — The SDKV header and Protobuf encoding in detail
- **[MVCC and Snapshot Isolation](mvcc.md)** — How the begin/end snapshot model works at the concept level
- **[Design Decision: Key Design Rationale](../design-decisions/key-design-rationale.md)** — Why each key is shaped the way it is, with alternatives considered

## The Design Challenge

A relational catalog has natural hierarchies: schemas contain tables, tables contain columns, tables reference data files. To store this hierarchy in a flat key-value namespace, SlateDuck encodes the relationships directly into the key bytes using a tag-prefixed, big-endian encoding scheme that preserves lexicographic ordering.

## Key Structure

Every key in SlateDuck's catalog follows this pattern:

```
[tag: 1 byte] [composite key fields: variable length]
```

The first byte is the **tag**, which identifies which catalog table the entry belongs to. For example, tag `0x04` is `ducklake_schema`, tag `0x05` is `ducklake_table`, tag `0x06` is `ducklake_column`, and tag `0x0B` is `ducklake_data_file`.

After the tag, the remaining bytes encode the composite key fields for that table, with multi-byte integers stored in **big-endian** format. Big-endian encoding is critical because it ensures that lexicographic byte ordering matches numeric ordering. When you scan keys with a prefix, you get results in ascending ID order.

## Examples

A table row for table ID 42 in schema ID 1, created at snapshot 7:

```
key: 0x05 | 0x0000000000000001 | 0x000000000000002A | 0x0000000000000007
      tag      schema_id (1)        table_id (42)       begin_snapshot (7)
```

A column row for column ID 3 in table ID 42, created at snapshot 7:

```
key: 0x06 | 0x000000000000002A | 0x0000000000000003 | 0x0000000000000007
      tag      table_id (42)        column_id (3)       begin_snapshot (7)
```

A data file row for file ID 100 in table ID 42:

```
key: 0x0B | 0x000000000000002A | 0x0000000000000064
      tag      table_id (42)        data_file_id (100)
```

## Why This Encoding?

The encoding is designed to make the most common access patterns efficient:

**Listing all tables in a schema** is a prefix scan on `0x05 | schema_id`. SlateDB seeks to the first key with that prefix and scans forward until the prefix changes. This is O(tables_in_schema), not O(total_tables).

**Listing all columns for a table** is a prefix scan on `0x06 | table_id`. Again, O(columns_in_table).

**Listing all data files for a table** is a prefix scan on `0x0B | table_id`. This is the most common read operation in the catalog (DuckDB needs the file list to execute any query) and it is optimally efficient.

**Point lookups** for a specific entity (given all key fields) are single GET operations in SlateDB. SlateDB's LSM-tree with bloom filters makes point lookups very fast.

## The Tag Registry

SlateDuck allocates tags from a fixed registry of 28 DuckLake catalog tables plus internal system tables:

| Tag Range | Purpose |
|-----------|---------|
| `0x01` - `0x1C` | DuckLake catalog tables (metadata, snapshots, schemas, tables, columns, views, macros, data files, delete files, statistics, partitions, sort info, tags, etc.) |
| `0xFC` | Secondary index (performance optimization for snapshot-scoped file lookups) |
| `0xFD` | Inlined data (small row inserts/deletes stored directly in the catalog) |
| `0xFE` | Counters (auto-incrementing ID generators for snapshots, catalog IDs, file IDs) |
| `0xFF` | System keys (writer epoch, retention settings, format version, audit log, checkpoints) |

The tag is the first byte of every key, which means a prefix scan for `0x05` (all tables) will never accidentally include entries from `0x06` (columns) or `0x0B` (data files). The tag provides perfect namespace isolation at the byte level.

## MVCC and Key Uniqueness

For versioned tables (schemas, tables, columns, views, macros), the `begin_snapshot` is part of the key. This means multiple versions of the same logical entity have different keys:

```
0x05 | schema_id=1 | table_id=42 | begin_snapshot=7   (original version)
0x05 | schema_id=1 | table_id=42 | begin_snapshot=15  (renamed version)
```

When reading at a specific snapshot, the MVCC filter examines both rows and returns only the one whose visibility bounds include the target snapshot. The original version has `end_snapshot=15` (set when the rename occurred) and the new version has `end_snapshot=NULL` (still active). A reader at snapshot 10 sees the original; a reader at snapshot 20 sees the renamed version.

## Value Encoding

Values are wrapped in a lightweight envelope format:

```
[encoding_version: 1 byte] [magic: "SDKV" 4 bytes] [protobuf payload: variable]
```

The magic bytes and version allow detection of corruption and future format evolution. The payload is a protobuf-encoded message whose schema depends on the tag (e.g., tag `0x05` values decode as `TableRow`, tag `0x06` as `ColumnRow`).

## Implications for Performance

The key-value mapping directly determines performance characteristics:

- **Operations that follow natural key prefixes are fast:** listing tables in a schema, columns in a table, files for a table. These are all prefix scans that touch only relevant keys.
- **Operations that span multiple tags are multiple scans:** describing a table (need TableRow from `0x05` and ColumnRows from `0x06`) requires two prefix scans.
- **Cross-cutting queries are expensive:** finding all tables across all schemas that have a column named "email" would require scanning all columns (`0x06` prefix) and filtering in memory. SlateDuck does not need this operation, but it illustrates the trade-off.
