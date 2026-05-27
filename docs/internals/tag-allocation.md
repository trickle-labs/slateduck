# Tag Allocation

Every key in Rocklake begins with a single byte — the tag — that identifies which catalog table the key belongs to. This one byte is the most important byte in the entire key: it determines which prefix scan range the key lives in, which decoder to use for the value, and which operational category (DuckLake protocol, internal, or system) the key belongs to. This page documents the complete tag allocation scheme, the rationale behind the grouping strategy, and the rules for allocating new tags.

## The Tag Space

A tag is one byte: values 0x00 through 0xFF, providing 256 possible table types. Of these, approximately 30 are currently allocated, leaving substantial room for growth. The unallocated space is not random — it is organized into reserved ranges with specific intended purposes, ensuring that future expansion does not require reorganization of existing allocations.

The tag is always the first byte of the key. This position is not arbitrary — it ensures that all rows of the same table type form a contiguous range in SlateDB's sorted key space. A prefix scan with a single-byte prefix retrieves exactly the rows of one table type, never accidentally crossing into another type.

## Allocation Ranges

Tags are divided into four major ranges, each serving a distinct purpose:

### DuckLake Protocol Tables (0x01–0x1F)

These tags correspond to the catalog tables defined by DuckDB's DuckLake extension. They store the metadata that DuckDB expects to find when it connects to a DuckLake catalog. The structure, semantics, and content of these tables are defined by the DuckLake protocol specification — Rocklake implements them but does not invent them.

| Tag | Table Name | Key Structure | Description |
|-----|-----------|---------------|-------------|
| 0x01 | ducklake_catalog | tag \| database_id | Root catalog entry (database metadata) |
| 0x02 | ducklake_snapshot | tag \| snapshot_id | Snapshot metadata (ID, timestamp, author) |
| 0x03 | ducklake_table_snapshot | tag \| table_id \| snapshot_id | Table-to-snapshot associations |
| 0x04 | ducklake_schema | tag \| schema_id \| begin_snapshot | Schema definitions (versioned) |
| 0x05 | ducklake_table | tag \| table_id \| begin_snapshot | Table definitions (versioned) |
| 0x06 | ducklake_column | tag \| table_id \| column_id \| begin_snapshot | Column definitions (versioned) |
| 0x07 | ducklake_data_file | tag \| table_id \| file_id \| begin_snapshot | Registered data file metadata |
| 0x08 | ducklake_delete_file | tag \| table_id \| file_id \| begin_snapshot | Registered delete file metadata |
| 0x09 | ducklake_file_column_stats | tag \| file_id \| column_id | Per-column statistics for data files |
| 0x0A | ducklake_table_stats | tag \| table_id \| snapshot_id | Table-level aggregate statistics |
| 0x0B | ducklake_view | tag \| view_id \| begin_snapshot | View definitions (versioned) |
| 0x0C | ducklake_macro | tag \| macro_id \| begin_snapshot | Macro/function definitions (versioned) |
| 0x0D | ducklake_tag | tag \| tag_id | Metadata tags/labels for catalog objects |
| 0x0E–0x1F | (reserved) | — | Future DuckLake protocol tables |

**Why this range starts at 0x01 (not 0x00):** Tag 0x00 is reserved as a sentinel value (never used in actual keys). This allows 0x00-prefixed byte sequences to be used for control purposes without conflicting with real data.

**Why tags 0x0E–0x1F are reserved:** The DuckLake protocol may expand in future versions (new table types for partitions, indexes, constraints, etc.). Reserving space in the same range ensures new protocol tables sort adjacent to existing ones, maintaining scan locality.

### Extended Tables (0x20–0x7F)

This range is reserved for future DuckLake protocol extensions or Rocklake-specific catalog extensions that are semantically "data" rather than "infrastructure." None are currently allocated. The range provides 96 additional tag values — far more than any foreseeable protocol expansion.

**Potential future uses:**

- Partition metadata tables (if DuckLake adds partition support)
- Index definition tables (if DuckLake adds secondary indexes)
- Constraint tables (if DuckLake adds check constraints or foreign keys)
- Statistics history tables (if DuckLake tracks statistics over time)

### Internal Tables (0x80–0xFD)

These tags are for Rocklake's internal use — data structures that exist to support Rocklake's operation but are not part of the DuckLake protocol. DuckDB does not know about these tables and never queries them directly.

| Tag | Table Name | Key Structure | Description |
|-----|-----------|---------------|-------------|
| 0x80 | secondary_index | tag \| index_type \| key_data | Hot-path secondary indexes |
| 0x81 | audit_entry | tag \| snapshot_id \| sequence | Audit log entries |
| 0xFD | inlined_insert | tag \| table_id \| file_id \| begin_snapshot | Inlined small data files |
| 0x82–0xFC | (reserved) | — | Future internal tables |

**Why internal tags start at 0x80:** The 0x80 boundary (bit 7 set) provides a visual distinction in hex dumps. Any tag with the high bit set is internal; any tag with the high bit clear is a protocol table. This makes debugging easier — you can immediately tell whether a key is "DuckLake data" or "Rocklake internals" by glancing at the first byte.

**Why 0xFD for inlined inserts:** Inlined data is conceptually "almost system" — it is stored for performance optimization rather than protocol compliance. Placing it at 0xFD (adjacent to the system range) reflects this positioning.

### System Space (0xFE–0xFF)

The highest two tag values are reserved for system-level keys that manage Rocklake's own state:

| Tag | Table Name | Key Structure | Description |
|-----|-----------|---------------|-------------|
| 0xFE | counter | tag \| counter_name (string) | ID allocation counters |
| 0xFF | system | tag \| key_name (string) | System configuration and state |

**Counter keys (0xFE):**

| Full Key | Purpose |
|----------|---------|
| 0xFE \| "next-snapshot-id" | Next snapshot ID to allocate |
| 0xFE \| "next-schema-id" | Next schema ID to allocate |
| 0xFE \| "next-table-id" | Next table ID to allocate |
| 0xFE \| "next-column-id" | Next column ID to allocate |
| 0xFE \| "next-file-id" | Next file ID to allocate |
| 0xFE \| "next-view-id" | Next view ID to allocate |

**System keys (0xFF):**

| Full Key | Purpose |
|----------|---------|
| 0xFF \| "writer-epoch" | Current writer epoch (for fencing) |
| 0xFF \| "retain-from" | Oldest snapshot that may be queried |
| 0xFF \| "catalog-format-version" | Catalog format version |
| 0xFF \| "hot-key" | Cached high-frequency read data |

**Why 0xFE and 0xFF sort last:** Because system keys should be encountered last during a full keyspace scan. A forward scan naturally visits all data first, then counters, then system keys. This matches the access pattern — you almost never need to scan system keys together with data tables.

**Why counters and system keys are separate tags:** Despite both being "system-level," counters and system keys have fundamentally different access patterns. Counters are updated on every write transaction (they are the hottest keys in the catalog). System keys are updated rarely (during GC, failover, or format upgrades). Separating them ensures they land in different SST blocks after compaction, preventing cold system keys from evicting hot counters from cache.

## Tag Properties

Each tag has associated properties that affect how keys and values under that tag are processed:

### Versioned vs. Unversioned

| Property | Versioned Tags | Unversioned Tags |
|----------|---------------|-----------------|
| Key contains begin_snapshot | Yes | No |
| Multiple versions per entity | Yes | No |
| MVCC filter applies | Yes | No |
| GC can remove old versions | Yes | No |
| Examples | 0x04, 0x05, 0x06, 0x07, 0x0B, 0x0C | 0x01, 0x02, 0x09, 0xFE, 0xFF |

Versioned tags store entities that can change over time (schemas, tables, columns). Unversioned tags store entities that are either immutable after creation (snapshots, statistics) or have special update semantics (counters, system keys).

### Key Length

Given a tag, the key length is deterministic:

| Tag | Key Length | Components |
|-----|-----------|-----------|
| 0x01 | 9 bytes | tag (1) + database_id (8) |
| 0x04 | 17 bytes | tag (1) + schema_id (8) + begin_snapshot (8) |
| 0x05 | 17 bytes | tag (1) + table_id (8) + begin_snapshot (8) |
| 0x06 | 25 bytes | tag (1) + table_id (8) + column_id (8) + begin_snapshot (8) |
| 0x07 | 25 bytes | tag (1) + table_id (8) + file_id (8) + begin_snapshot (8) |
| 0x09 | 17 bytes | tag (1) + file_id (8) + column_id (8) |
| 0xFE | variable | tag (1) + counter_name (string) |
| 0xFF | variable | tag (1) + key_name (string) |

All data table keys (0x01–0x0D) have fixed-length keys. System keys (0xFE, 0xFF) have variable-length keys (because they use string suffixes). This means a key parser can determine the exact length from the first byte for any data table key — no scanning for delimiters needed.

## Allocation Rules for New Tags

When adding a new tag (for a new table type), these rules apply:

1. **Choose the correct range.** DuckLake protocol tables go in 0x01–0x1F. Internal tables go in 0x80–0xFD. Never allocate system tags (0xFE–0xFF).

2. **Allocate sequentially within the range.** Do not leave gaps unless there is a specific grouping reason. The next DuckLake table gets the next unallocated tag in 0x01–0x1F.

3. **Document immediately.** Every allocated tag must be documented in this page and in the source code (the `tags.rs` module).

4. **No tag reuse.** Once a tag is allocated, it is never reassigned to a different table type, even if the original table is deprecated. Reuse would create ambiguity when reading old catalog data.

5. **Backward compatibility.** Adding a new tag is always backward-compatible (old binaries simply do not recognize the tag and ignore those keys). Changing an existing tag's semantics requires a format version bump.

## Hex Dump Reading Guide

When examining hex dumps of SlateDB keys (during debugging or corruption analysis), the first byte immediately tells you what you are looking at:

```
01 00 00 00 00 00 00 00 01    → ducklake_catalog, database_id=1
05 00 00 00 00 00 00 00 2A    → ducklake_table, table_id=42, begin_snapshot=...
   00 00 00 00 00 00 00 C8       (begin_snapshot=200)
06 00 00 00 00 00 00 00 05    → ducklake_column, table_id=5, column_id=...
   00 00 00 00 00 00 00 03       (column_id=3)
   00 00 00 00 00 00 00 64       (begin_snapshot=100)
FE 6E 65 78 74 2D 73 6E 61   → counter, "next-sna..." (next-snapshot-id)
FF 77 72 69 74 65 72 2D 65   → system, "writer-e..." (writer-epoch)
```

The ability to read keys visually from hex dumps is intentional — it makes debugging faster and reduces the need for specialized decoding tools.

## Further Reading

- **[Architecture: Key Layout](../architecture/key-layout.md)** — Complete key format specification
- **[Design Decisions: Key Design Rationale](../design-decisions/key-design-rationale.md)** — Why these specific encoding choices
- **[MVCC Filter](mvcc-filter.md)** — How versioned tags interact with visibility filtering
- **[Inlined Data](inlined-data.md)** — The 0xFD tag in detail
