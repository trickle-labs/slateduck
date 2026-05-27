# Schema Version

Rocklake stores a format version in every catalog to ensure that binaries never accidentally read or write incompatible data. This is the simplest and most critical safety mechanism in the system: if a Rocklake binary encounters a catalog whose format version does not match its expectations, it refuses to operate. No corruption, no silent data loss, no mysterious errors hours later. An immediate, clear rejection.

This page documents how format versions are stored, how compatibility is checked, what constitutes a breaking change versus a compatible change, and how future migrations will work when the format eventually evolves.

## Why Format Versioning Matters

Catalog data is long-lived. A Rocklake catalog created today will still be accessed in two years, five years, possibly longer. Over that time, the Rocklake binary will be updated many times. Each update may change:

- How keys are encoded (new fields, different ordering)
- How values are serialized (protobuf schema changes)
- What tags mean (new table types, changed semantics)
- What system keys exist and how they are interpreted

Without version tracking, an old binary could read new-format data and misinterpret it. A new binary could read old-format data and fail in unpredictable ways. The format version number eliminates this ambiguity: every binary knows exactly which format it expects, and every catalog declares which format it is in.

## Current Format Version

The current catalog format version is **1**. This version has been stable since Rocklake's initial release and encompasses the following contracts:

| Component | Format Version 1 Specification |
|-----------|-------------------------------|
| Tag allocation | Tags 0x01–0x0D for DuckLake tables, 0x80–0x81 + 0xFD for internal, 0xFE–0xFF for system |
| Key encoding | Tag (1 byte) + big-endian u64 components (fixed-width) |
| Value envelope | 1 byte version + 4 bytes magic ("SDKV") + protobuf payload |
| Protobuf fields | Specific field numbering per row type (see proto definitions) |
| System key names | "writer-epoch", "retain-from", "catalog-format-version", "hot-key" |
| Counter key names | "next-snapshot-id", "next-schema-id", etc. |
| Snapshot semantics | Monotonically increasing, begin/end for MVCC |

Any change to these specifications that would cause a binary expecting format 1 to misinterpret data requires a version bump to format 2.

## How the Version Is Stored

The format version is a system key:

```
Key:   0xFF | "catalog-format-version"
Value: SDKV envelope containing the integer 1 (as protobuf-encoded u64)
```

This key is stored in the same keyspace as all other catalog data. It is written once during catalog initialization and updated only during format migrations (which have not yet occurred).

### Startup Check

When a Rocklake binary starts, it performs this sequence:

1. Open the SlateDB manifest at the configured storage path
2. Read the system key `0xFF | "catalog-format-version"`
3. Compare the stored version against the binary's compiled-in expected version
4. If they match: proceed normally
5. If they do not match: emit error `FormatVersionMismatch` (SQLSTATE 0A000) and refuse to operate

This check happens before any catalog operations are served. No client can connect to a Rocklake instance that has detected a format mismatch.

### New Catalog Initialization

When creating a new catalog (first write to an empty storage path), Rocklake writes the format version as part of the initialization batch:

```
Write batch (atomic):
  - 0xFF | "catalog-format-version" → 1
  - 0xFF | "writer-epoch" → 1
  - 0xFE | "next-snapshot-id" → 1
  - 0xFE | "next-schema-id" → 1
  - ... (other counter initializations)
```

All initialization writes are in one atomic batch. A catalog either exists fully initialized (with a format version) or does not exist at all.

## What Requires a Version Bump

Changes that break backward compatibility — meaning a format-1 binary cannot correctly read data written by the new format — require incrementing the format version.

### Breaking Changes (Require Version Bump)

| Change Type | Example | Why It Breaks |
|-------------|---------|---------------|
| Tag reassignment | Changing 0x05 from "tables" to "partitions" | Old binary would misinterpret partition rows as table rows |
| Key encoding change | Switching from big-endian to varint for IDs | Old binary would parse key fields incorrectly |
| Value envelope change | Changing the magic bytes from "SDKV" to something else | Old binary would reject all values as corrupt |
| Protobuf field renumbering | Changing field 1 from `table_id` to `schema_id` | Old binary would read schema_id as table_id |
| System key semantic change | Changing "retain-from" to mean something different | Old binary would apply incorrect GC logic |

### Compatible Changes (No Version Bump Required)

| Change Type | Example | Why It Is Safe |
|-------------|---------|---------------|
| Adding new tags | Allocating 0x0E for a new table type | Old binary ignores unknown tags (skips those keys) |
| Adding protobuf fields | Adding field 20 to the table row type | Old binary ignores unknown fields (protobuf forward-compatible) |
| Adding system keys | Adding 0xFF \| "new-feature-flag" | Old binary ignores unknown system keys |
| Adding counter keys | Adding 0xFE \| "next-partition-id" | Old binary ignores unknown counters |
| Logic changes | Changing how MVCC filter handles edge cases | No format change — same data, different interpretation |
| Performance changes | Changing compaction strategy | No format change — SST file format is SlateDB's concern |

The key principle: **additive changes are forward-compatible; structural changes are not.**

Protobuf's wire format is specifically designed for forward and backward compatibility. Adding new fields (with new field numbers) is always safe. Removing fields is safe (the old field just becomes "unknown" to new code). Only renumbering or retyping existing fields is breaking.

## Forward Compatibility

Rocklake is designed to be maximally forward-compatible:

### Adding New Table Types

When a new DuckLake protocol version adds a new table type (e.g., "ducklake_partitions"), Rocklake:

1. Allocates a new tag (e.g., 0x0E)
2. Implements the new row type with new protobuf message
3. Adds handling in the catalog reader and writer

An older Rocklake binary encountering keys with tag 0x0E will:
- Skip them during scans (unknown tag, not part of any known prefix)
- Not serve them to DuckDB (DuckDB of the old version does not know about partitions either)
- Not corrupt them (reads are non-destructive)

This means you can upgrade the catalog format (by writing new tag types) without upgrading all reader binaries simultaneously. Old readers simply do not see the new data — they are not harmed by it.

### Adding New Fields to Existing Rows

When Rocklake adds information to an existing row type (e.g., adding a "created_by" field to table rows):

1. Add a new protobuf field with the next available field number
2. New binaries write the field; old binaries do not
3. New binaries reading old rows find the field absent (use default value)
4. Old binaries reading new rows ignore the unknown field

No format version change is needed. Both old and new binaries can read and write the same catalog concurrently (though only one can be the writer — the single-writer model simplifies this enormously).

## Migration Strategy

When a format version bump is eventually necessary (expected to be rare — possibly never for most deployments), the migration will follow one of two strategies:

### Strategy A: In-Place Migration

The new binary detects the old format version, transforms all data in place, and updates the version key:

```
1. Read format version (= 1)
2. Read all keys/values
3. Transform to new format (re-encode keys, add new fields, etc.)
4. Write all transformed data as one large batch
5. Update format version key to 2
6. Delete old-format keys (or leave for SlateDB compaction to clean)
```

**Advantages:** Simple, self-contained, no external tools needed.
**Disadvantages:** Requires holding entire catalog in memory during transformation. Potentially large write batch.

### Strategy B: Export/Import Migration

A migration tool exports the catalog state, creates a fresh format-2 catalog, and imports:

```
1. Run: rocklake export --format v1 --output catalog-v1.ndjson
2. Run: rocklake import --format v2 --input catalog-v1.ndjson --catalog new-path
3. Switch to new catalog path
4. (Optionally) archive old catalog
```

**Advantages:** No in-place modification risk. Can validate new catalog before switching.
**Disadvantages:** Requires operator intervention. Temporary storage for export file.

### Rollback

If a migration fails or the new format has issues:

- **In-place migration:** Restore from S3 versioning (object storage keeps previous versions of all files for configured retention period)
- **Export/import:** The old catalog is untouched — simply switch back to the old path

## Testing Format Compatibility

The test suite includes format compatibility tests:

1. **Snapshot tests:** Serialized catalog data from known states is stored in the test fixtures. Each test verifies that the current binary correctly reads these fixtures.
2. **Round-trip tests:** Write data, read it back, verify it matches.
3. **Unknown field tests:** Inject protobuf data with unknown fields, verify the binary handles them gracefully.
4. **Version mismatch tests:** Create a catalog with version 99, verify the binary refuses to open it with the correct error.

## Practical Implications for Operators

### Upgrading Rocklake

When upgrading Rocklake to a new version:

1. Check the release notes for format version changes (extremely rare)
2. If no format change: simply replace the binary and restart
3. If format change: follow the migration guide in the release notes

### Downgrading Rocklake

Downgrading to an older binary is safe as long as:

- The catalog format version has not been bumped (most upgrades)
- No new-tag data has been written that the old binary depends on

If the catalog contains data from new tags that the old binary does not understand, the old binary will skip those entries but otherwise operate normally.

### Multiple Binaries Accessing Same Catalog

Because only one writer exists at a time, format compatibility is primarily about readers. Multiple reader binaries of different versions can access the same catalog simultaneously, as long as all of them support the catalog's format version.

## Further Reading

- **[Architecture: Value Encoding](../architecture/value-encoding.md)** — The SDKV envelope format
- **[Tag Allocation](tag-allocation.md)** — How tags are assigned and reserved
- **[Crash Safety](crash-safety.md)** — How format version writes are atomic
- **[Operations: Upgrades](../operations/upgrades.md)** — Binary upgrade procedures
