# Key Design Rationale

This page explains the reasoning behind specific choices in Rocklake's binary key encoding. The key layout is documented in [Architecture: Key Layout](../architecture/key-layout.md) — that page describes _what_ the encoding is. This page explains _why_ each detail was chosen over alternatives. Key encoding is one of those areas where small decisions have outsized consequences: a wrong choice in the first byte affects every scan, every write, and every debugging session for the lifetime of the system.

## Design Principles

Before examining individual decisions, the overarching principles that guided key design:

1. **Scans are the primary access pattern.** Most catalog operations are prefix scans ("list all tables in schema X," "list all columns in table Y"). The key encoding must make these scans efficient and unambiguous.

2. **Byte comparison must equal semantic comparison.** SlateDB sorts keys lexicographically (byte-by-byte comparison). The key encoding must ensure that this byte ordering produces the desired semantic ordering (entities of the same type together, IDs in ascending order).

3. **Fixed-width fields for constant-time parsing.** Given a key's tag byte, you should be able to extract any field at a known byte offset without scanning variable-length content.

4. **Debuggability matters.** When examining hex dumps of keys (during debugging, corruption analysis, or testing), the encoding should be interpretable without specialized tools.

## Why Tag-First?

The tag byte is the first byte of every key. This is the most consequential encoding decision.

### What the Tag Does

The tag identifies which DuckLake table the row belongs to:

| Tag | Table | Key Structure |
|-----|-------|---------------|
| 0x01 | ducklake_databases | tag \| database_id |
| 0x04 | ducklake_schemas | tag \| schema_id \| begin_snapshot |
| 0x05 | ducklake_tables | tag \| table_id \| begin_snapshot |
| 0x06 | ducklake_columns | tag \| table_id \| column_id \| begin_snapshot |
| 0x0A | ducklake_data_files | tag \| table_id \| file_id \| begin_snapshot |
| 0xFE | counters | tag \| counter_name |
| 0xFF | system | tag \| key_name |

### Why Not Hierarchical Keys?

An alternative approach (used by systems like CockroachDB) is hierarchical string keys:

```
# Hierarchical approach (rejected):
/schemas/1/tables/42/columns/7/version/500

# Tag-first approach (chosen):
0x06 | 0x0000000000000042 | 0x0000000000000007 | 0x00000000000001F4
```

The hierarchical approach has several drawbacks:

- **Variable-length keys:** Key length varies with entity depth and ID size. Parsing requires scanning for delimiters.
- **Delimiter handling:** If entity names can contain the delimiter character (`/`), escaping is required.
- **String comparison inefficiency:** Comparing `"/schemas/1/"` vs. `"/schemas/10/"` requires character-level comparison, not byte-level.
- **Wasted bytes:** The literal text `"schemas"`, `"tables"`, `"columns"` consumes bytes in every key without adding information (the position in the hierarchy already implies the type).

The tag-first approach:

- **Fixed-length keys:** Given the tag, key length is known and parsing requires no scanning.
- **No delimiters:** Every field starts at a known byte offset.
- **Byte comparison works:** No special handling for numeric vs. string comparison.
- **Compact:** A single byte replaces the entire table name.

### Why the Tag Acts as a Namespace

Because the tag is the first byte, all rows of the same type form a contiguous range in SlateDB's sorted key space:

```
[0x04 ...] [0x04 ...] [0x04 ...] ← All schema rows
[0x05 ...] [0x05 ...] [0x05 ...] ← All table rows
[0x06 ...] [0x06 ...] [0x06 ...] ← All column rows
```

A prefix scan with `prefix = [0x06]` retrieves all column rows without ever touching schema or table rows. This is O(relevant_rows), not O(all_rows). SlateDB's seek operation jumps directly to the first key with the matching prefix — there is no full-table scan.

If the tag were a suffix or embedded in the value, you would need to scan all keys and filter by type — vastly less efficient.

## Why Big-Endian?

All multi-byte integers in keys are stored in big-endian (network byte order): the most significant byte comes first.

### The Problem Big-Endian Solves

SlateDB (and virtually all key-value stores) sorts keys by lexicographic byte comparison. This means keys are compared byte-by-byte from left to right, with the first differing byte determining the order.

Consider two table IDs: 1 and 256.

In **big-endian** (chosen):

```
ID 1:   0x00 0x00 0x00 0x00 0x00 0x00 0x00 0x01
ID 256: 0x00 0x00 0x00 0x00 0x00 0x00 0x01 0x00
```

Lexicographic comparison: first 6 bytes are equal, byte 7 differs (0x00 < 0x01), so ID 1 sorts before ID 256. **Correct.**

In **little-endian** (rejected):

```
ID 1:   0x01 0x00 0x00 0x00 0x00 0x00 0x00 0x00
ID 256: 0x00 0x01 0x00 0x00 0x00 0x00 0x00 0x00
```

Lexicographic comparison: byte 0 differs (0x01 > 0x00), so ID 1 sorts AFTER ID 256. **Wrong.**

Big-endian ensures that lexicographic byte comparison produces the same ordering as numeric comparison. This means SlateDB's natural sort order delivers keys in ascending ID order — exactly what you want when iterating scan results.

### Why Not Use Variable-Length Integers?

Variable-length integers (varints, as used in protobuf) are more compact but break sort order:

```
# Varint encoding of different values:
1:     0x01           (1 byte)
128:   0x80 0x01      (2 bytes)
1000:  0xE8 0x07      (2 bytes)
```

The varint `128` (0x80 0x01) sorts after `1000` (0xE8 0x07) in byte comparison because 0x80 < 0xE8 for the first byte, but numerically 128 < 1000. Varints do not preserve numeric ordering under byte comparison.

To use varints in keys, you would need a comparison-preserving varint encoding (like SQLite4's varint). These exist but add complexity and are not standard. Fixed-width big-endian is simpler and universally understood.

## Why Fixed-Width u64?

All ID fields (snapshot_id, schema_id, table_id, column_id, file_id) use 8-byte u64 (unsigned 64-bit integer) even though most catalogs will never use values above a few million.

### The Benefits

**Simplicity:** Every key can be parsed with fixed byte offsets. Given tag 0x06 (columns), the structure is always:

```
Byte 0:     tag (0x06)
Bytes 1-8:  table_id (u64 big-endian)
Bytes 9-16: column_id (u64 big-endian)
Bytes 17-24: begin_snapshot_id (u64 big-endian)
Total: 25 bytes (always)
```

No variable-length integer decoding, no length prefixes, no special handling for large values. The parser is trivial: `table_id = u64::from_be_bytes(key[1..9])`.

**Uniformity:** All keys of the same tag have exactly the same length. This simplifies:

- Size estimation (predict storage requirements without scanning)
- Debugging tools (hex dump alignment is consistent)
- Test assertions (expected key length is a constant)
- Documentation (key format can be specified as a fixed structure)

**Future-proofing:** A u64 counter at one increment per millisecond will not overflow for 584 million years. At one increment per nanosecond, it lasts 584 years. There is no practical concern about running out of IDs, even for the most active catalogs imaginable.

### The Cost

The cost is 4 bytes of waste per ID field compared to a 4-byte u32 (which handles values up to 4.3 billion — more than sufficient). For a key with 3 ID fields, this is 12 bytes of overhead per key.

**Quantifying the total cost:**

| Catalog Size | Keys | Extra Bytes (vs. u32) | Total Overhead |
|-------------|------|----------------------|----------------|
| Small (1,000 keys) | 1,000 | 12 KB | Negligible |
| Medium (100,000 keys) | 100,000 | 1.2 MB | Negligible |
| Large (10,000,000 keys) | 10M | 120 MB | 2-5% of total |

For any realistic catalog size, the overhead is negligible relative to overall storage. The simplicity benefit far outweighs the space cost.

## Why begin_snapshot in Versioned Keys?

For versioned tables (schema, table, column, view, macro), the `begin_snapshot_id` is the last component of the key:

```
tag | entity_id | begin_snapshot_id
```

This means multiple versions of the same logical entity have different keys and coexist as separate key-value pairs in SlateDB.

### The Alternative: Single Key, Multiple Versions in Value

An alternative design stores one key per logical entity with a list of versions in the value:

```
Key: tag | entity_id
Value: [{version: 1, data: ...}, {version: 2, data: ...}, ...]
```

### Why This Was Rejected

1. **Read-modify-write for every update.** Adding a new version requires reading the current value (GET), appending the new version to the list, and writing back (PUT). This is not atomic without transactions — if the process crashes between GET and PUT, the value may be corrupted or stale.

2. **Value size grows unboundedly.** An entity with 1,000 historical versions would have a value containing 1,000 serialized rows. This defeats SlateDB's block-based I/O model (values should be small enough to fit in one block).

3. **Prefix scans must parse values.** To find the visible version at a given snapshot, you must decode the entire value and search through the version list. With the chosen approach, the MVCC filter operates at the key level — you can skip irrelevant versions without even reading their values.

4. **GC is complex.** Removing old versions requires rewriting the value (read-modify-write again). With separate keys, GC simply deletes the old key — a single tombstone write.

### Why begin_snapshot Sorts Correctly

With `begin_snapshot_id` as the last key component and big-endian encoding, versions of the same entity sort in ascending snapshot order:

```
0x05 | table_id=5 | snapshot=100   (version created at snapshot 100)
0x05 | table_id=5 | snapshot=200   (version created at snapshot 200)
0x05 | table_id=5 | snapshot=300   (version created at snapshot 300)
```

A prefix scan with `prefix = [0x05, ...table_id=5...]` returns all versions in chronological order. Finding the visible version at snapshot 250 means scanning forward from the prefix start and taking the last entry where `begin_snapshot <= 250` and (`end_snapshot` is NULL or `end_snapshot > 250`).

## Why Counters in a Separate Tag?

Counter values (next_snapshot_id, next_catalog_id, next_file_id) live under tag `0xFE` rather than being embedded in the system key space (`0xFF`).

### Organizational Clarity

Counters are updated on every write transaction — they are the hottest keys in the catalog. System keys (format version, writer epoch, retain_from) are updated rarely (during GC, failover, or upgrades). Keeping them in separate tags makes monitoring and debugging clearer:

- "How many keys are under 0xFE?" → "How many counter reads/writes are happening?"
- "How many keys are under 0xFF?" → "How many system operations are happening?"

### Performance Isolation

Because counters and system keys have different access patterns (counters are hot, system keys are cold), separating them into different key prefixes ensures they land in different SST blocks after compaction. This means a scan of system keys never reads counter blocks, and vice versa.

## Why System Keys Use String Suffixes?

Most keys use numeric (u64) components, but system keys use human-readable string suffixes:

```
0xFF | "writer-epoch"      → epoch value
0xFF | "retain-from"       → snapshot ID
0xFF | "format-version"    → version number
0xFF | "hot-key"           → latest hot key data
```

### Why Not Numeric IDs?

System keys are:

1. **Few in number** (fewer than 20 distinct keys)
2. **Accessed by exact lookup** (GET with a known key, never by prefix scan)
3. **Never iterated over in performance-critical paths**

For these keys, human-readable strings provide massive debugging benefits at zero performance cost:

- A hex dump shows `FF 77 72 69 74 65 72 2D 65 70 6F 63 68` — you can immediately read "writer-epoch" in the ASCII representation
- Error messages can include the key name literally: "system key 'retain-from' is missing"
- No mapping table needed between numeric IDs and their semantic meaning

The performance argument for numeric IDs (faster comparison, fixed width) does not apply because system keys are accessed by exact GET — they are never compared to each other in a sort context.

## Tag Allocation Strategy

Tags are allocated in groups to leave room for future expansion:

| Range | Purpose | Current Usage |
|-------|---------|---------------|
| 0x01–0x0F | Core DuckLake tables | 8 of 15 used |
| 0x10–0x1F | Extended DuckLake tables | 3 of 16 used |
| 0x20–0x7F | Reserved for future catalog tables | Unused |
| 0x80–0xEF | Reserved for indexes/secondary structures | Unused |
| 0xF0–0xFD | Special-purpose (inlined data, etc.) | 2 of 14 used |
| 0xFE | Counters | 1 tag, multiple keys |
| 0xFF | System | 1 tag, multiple keys |

This allocation leaves ample room for growth without requiring re-encoding of existing data.

## Further Reading

- **[Architecture: Key Layout](../architecture/key-layout.md)** — Complete key format specification
- **[Architecture: Value Encoding](../architecture/value-encoding.md)** — How values complement keys
- **[Protobuf Encoding](protobuf-encoding.md)** — Value format decisions
- **[Why SlateDB?](why-slatedb.md)** — Why sorted keys matter for the storage engine
