# Value Encoding

Every value stored in Rocklake's catalog is wrapped in a lightweight envelope format that provides corruption detection, forward compatibility, and efficient serialization. The key tells Rocklake where something is in the catalog hierarchy; the value tells Rocklake what that thing actually contains. Keys are fixed-width binary sequences optimized for lexicographic ordering and prefix scans; values are variable-length payloads optimized for compact storage, fast deserialization, and graceful evolution over time.

The actual row data within each value is encoded using Protocol Buffers (protobuf). This gives Rocklake the ability to evolve its internal schema without breaking existing catalogs, add new fields without migrating old data, and achieve compact binary representation that keeps catalog storage costs negligible even at scale.

This page documents the envelope format, explains why protobuf was chosen over alternatives, catalogs all the row types and their fields, and discusses the corruption detection mechanisms that protect catalog integrity.

## The Envelope Format

All values in Rocklake follow a fixed envelope structure:

```
┌─────────────────────┬──────────────┬─────────────────────────────────────┐
│ encoding_version: 1B│ magic: 4B    │ payload: variable length            │
│ (currently 0x01)    │ "SDKV"       │ (protobuf message or raw bytes)     │
└─────────────────────┴──────────────┴─────────────────────────────────────┘
 Byte 0                Bytes 1-4      Bytes 5 onward
```

The envelope adds exactly 5 bytes of overhead to every value. For a typical catalog row (50–200 bytes of protobuf payload), this is 2.5–10% overhead — a small price for the safety guarantees it provides.

### Encoding Version (1 byte)

The first byte is the encoding version, currently `0x01`. This enables future format changes without breaking existing readers. If Rocklake encounters a value with an encoding version it does not recognize (for example, `0x02` on a Rocklake binary that only knows about version `0x01`), it fails immediately with `UnsupportedVersion` rather than silently misinterpreting the data.

This is a forward-compatibility mechanism: a catalog written by a newer version of Rocklake is not accidentally readable by an older version. The error is unambiguous and actionable ("upgrade your Rocklake binary").

### Magic Bytes (4 bytes)

Bytes 1–4 contain the ASCII string `SDKV` (Rocklake Key-Value). These serve as a corruption canary: if the magic bytes are not present at the expected offset, the value has been corrupted — perhaps by a bit-flip in storage, truncation during a failed write, or contamination from unrelated data being written to the same key.

The magic check provides early detection before attempting protobuf deserialization, which might produce confusing errors or (worse) silently decode corrupted bytes into a structurally-valid but semantically-wrong message. By checking the magic first, Rocklake distinguishes between "this value is corrupted" (InvalidMagic) and "this value has a valid envelope but the payload is malformed" (DecodeError).

### Payload (variable length)

The remainder of the value is the actual data, whose format depends on the key's tag byte. For most tags (0x01–0x1C), the payload is a protobuf message. For counters (tag 0xFE), the payload is a raw big-endian u64. For system keys (tag 0xFF), the payload may be protobuf, raw integers, or JSON depending on the specific system key.

## Why Protocol Buffers?

Rocklake chose protobuf over alternatives (JSON, MessagePack, FlatBuffers, Cap'n Proto, CBOR, custom binary formats) for several reasons:

### Compact Binary Encoding

Protobuf uses variable-length integer encoding (varint), field-number-based tagging, and no padding or alignment requirements. A typical `TableRow` is 50–100 bytes. A `ColumnRow` is 30–80 bytes. A `DataFileRow` is 80–200 bytes. For a catalog with 100,000 data files, the total protobuf payload is approximately 10–20 MB — small enough to fit comfortably in a handful of SST files.

Compare this to JSON: the same `DataFileRow` would be 400–600 bytes due to field names, string quoting, and Base64 encoding of binary values. That is 3–5x larger, which directly translates to more SST files, more object storage PUTs during compaction, and more bytes transferred during reads.

### Forward and Backward Compatibility

Protobuf's field numbering and optional fields allow the row schema to evolve without migration:

- **Adding a new field:** Old catalogs do not have the field. New readers handle its absence gracefully (using a default value or treating it as None). Old readers encountering the new field ignore it (protobuf's unknown field handling).
- **Deprecating a field:** New writers stop including it. Old readers that expect it handle its absence (because all protobuf fields are implicitly optional at the wire level). New readers ignore it completely.
- **Changing a field type:** This is the one operation that requires careful handling. Rocklake avoids it — instead, a new field with the new type is added, and the old field is deprecated.

This matters because Rocklake catalogs are long-lived. A catalog created in version 0.3 might be read by version 1.2 years later. The protobuf encoding guarantees that the catalog remains readable without any migration step.

### Fast Serialization and Deserialization

Protobuf encoding and decoding is O(n) in the size of the data with minimal overhead. There is no parsing in the traditional sense — no tokenization, no lookahead, no backtracking. The decoder reads a field tag, determines the wire type, reads the appropriate number of bytes, and moves to the next field.

For Rocklake's hot path — scanning hundreds of data file rows to answer a DuckDB query about which Parquet files to scan — deserialization speed matters. A prefix scan might return 1,000 key-value pairs, each requiring protobuf decoding. At ~100 nanoseconds per decode (typical for small messages), this adds 100 microseconds to the scan — negligible compared to the network round-trip to object storage.

### Language-Neutral Schema

While Rocklake is written in Rust and uses `prost` for protobuf code generation, the `.proto` schema files are language-neutral. This means external tools written in Python, Go, or Java could read catalog data directly from SlateDB's SST files by using the same protobuf definitions. This is valuable for debugging tools, migration utilities, and monitoring infrastructure that may not want to embed the full Rocklake binary.

## Row Types Catalog

Each key tag corresponds to a protobuf message type. Here is the complete mapping:

### Versioned Entities (have begin_snapshot/end_snapshot)

| Tag | Message Type | Key Fields | Versioned |
|-----|-------------|------------|-----------|
| `0x04` | `SchemaRow` | schema_id, begin_snapshot | Yes |
| `0x05` | `TableRow` | schema_id, table_id, begin_snapshot | Yes |
| `0x06` | `ColumnRow` | table_id, column_id, begin_snapshot | Yes |
| `0x07` | `ViewRow` | schema_id, view_id, begin_snapshot | Yes |
| `0x08` | `MacroRow` | schema_id, macro_id, begin_snapshot | Yes |

For these entities, the value contains `end_snapshot` (null if the version is still current). The `begin_snapshot` appears in both the key (for ordering) and the value (for MVCC filtering without re-parsing the key).

### Append-Only Entities (have begin_snapshot only)

| Tag | Message Type | Key Fields | Versioned |
|-----|-------------|------------|-----------|
| `0x02` | `SnapshotRow` | snapshot_id | No (immutable once written) |
| `0x03` | `SnapshotChangesRow` | snapshot_id | No |
| `0x0B` | `DataFileRow` | table_id, data_file_id | No (begin_snapshot in value) |
| `0x0C` | `DeleteFileRow` | data_file_id, delete_file_id | No |

Append-only entities are never superseded — once written, their key-value pair is immutable for the lifetime of the catalog (until excision). The `begin_snapshot` in the value marks when they became visible; they remain visible forever after.

### Mutable Singletons

| Tag | Message Type | Key Fields | Behavior |
|-----|-------------|------------|----------|
| `0x11` | `TableStatsRow` | table_id | Overwritten on each update |
| `0xFE` | Raw u64 | counter_id | Overwritten on each increment |
| `0xFF` | Various | suffix string | Overwritten on each update |

Mutable singletons are the exception to the immutability principle. They are always read at the latest version (no MVCC filtering) and are overwritten in place (new write to the same key).

### Metadata and Mappings

| Tag | Message Type | Key Fields | Behavior |
|-----|-------------|------------|----------|
| `0x01` | `MetadataRow` | scope, scope_id, key | Versioned (scope-qualified) |
| `0x0F` | `ColumnMappingRow` | table_id, column_id | Mutable singleton |
| `0x10` | `NameMappingRow` | table_id, column_id | Mutable singleton |
| `0x12` | `FileColumnStatsRow` | table_id, column_id, file_id | Append-only |
| `0x13` | `FileVariantStatsRow` | table_id, column_id, file_id | Append-only |

## Special Value Types

Not all values use protobuf. Some use simpler encodings where protobuf would be unnecessary overhead:

### Counter Values (Tag 0xFE)

```
┌────────┬──────┬──────────────────┐
│ 0x01   │ SDKV │ u64_big_endian   │
└────────┴──────┴──────────────────┘
```

Counters store a single 64-bit integer: the next available ID. The value is always exactly 13 bytes (1 + 4 + 8). Big-endian encoding is used for consistency with key encoding, though it is not strictly necessary for values.

### Hot Key Value (Tag 0xFF, suffix "hot-key")

The hot key is a special protobuf message (`HotKeyValue`) that packs frequently-accessed state for cold-start optimization:

- Current snapshot ID
- Number of schemas, tables, columns
- Total file count and total storage size
- Writer epoch
- Retention horizon

On cold start, reading this single key-value pair gives Rocklake enough information to respond to basic queries (like "what is the current snapshot?") without scanning the entire catalog. The hot key is updated as part of every commit.

## Size Limits and Constraints

Rocklake enforces a maximum value size of 64 MiB. This limit exists because:

- SlateDB WAL segments have practical size constraints
- SST blocks are sized for efficient random access (typically 4–64 KB)
- Extremely large values degrade compaction performance

In practice, catalog values are tiny — under 1 KB for virtually all entries. The only exception is inlined data rows (tag 0xFD), which store small table data directly in the catalog for tables with fewer than a configurable threshold of rows. Even these are typically under 1 MB.

If a write would produce a value exceeding 64 MiB, it fails with `ValueTooLarge` (SQLSTATE `54001`). This should never happen in normal operation.

## Corruption Detection Layers

The envelope format provides three independent layers of corruption detection:

### Layer 1: Magic Byte Check

If bytes 1–4 are not `SDKV`, the value is corrupt. This catches:

- Random bit-flips (the probability of a random 4-byte sequence equaling "SDKV" is 1 in 4 billion)
- Zero-fills (common storage failure mode where a region is overwritten with zeros)
- Cross-contamination (data from an unrelated system accidentally written to the same location)

### Layer 2: Version Check

If the encoding version is not 0x01 (the only currently-defined version), one of two things happened:

- A newer Rocklake version wrote this value (forward incompatibility — the reader must be upgraded)
- The version byte is corrupted (bit-flip on the first byte specifically)

Rocklake distinguishes these cases by checking whether the version is "reasonably close" to known versions. If it is far from any known version (e.g., 0xAB), corruption is assumed.

### Layer 3: Protobuf Structural Validation

Protobuf decoding has built-in structural checks: field tags must use valid wire types, length-delimited fields must not exceed the remaining bytes, and nested messages must be well-formed. If the magic and version pass but protobuf decoding fails, the payload is corrupt (or was written by an incompatible schema version).

### What Happens on Corruption

When corruption is detected:

- During normal operation: the corrupted entry is logged, and an error is returned to the client
- During `rocklake verify`: the corrupted entry is reported with its key, corruption type, and byte offset
- During `rocklake repair`: the corrupted entry may be reconstructed from redundant information (if available) or marked as unrecoverable

## Further Reading

- **[Key Layout](key-layout.md)** — The key side of the key-value contract
- **[MVCC Implementation](mvcc-implementation.md)** — How version fields in values interact with visibility filtering
- **[Design Decisions: Protobuf Encoding](../design-decisions/protobuf-encoding.md)** — The rationale for choosing protobuf over alternatives
- **[Internals: Schema Version](../internals/schema-version.md)** — How the protobuf schema evolves between Rocklake releases
