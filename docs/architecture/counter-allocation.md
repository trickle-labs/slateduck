# Counter Allocation

Every entity in the DuckLake catalog — snapshots, schemas, tables, views, columns, data files, delete files — carries a unique integer identifier. These identifiers must be assigned without gaps, without duplicates, and without coordination between concurrent processes. They must also be durable: if Rocklake crashes mid-transaction, no ID that was allocated but not persisted should ever be reused.

This page explains how Rocklake solves the ID allocation problem using a combination of in-memory caching and transactional SlateDB writes.

## Why ID Allocation Is Hard

In a classical relational database, ID allocation is handled by the engine itself: `SERIAL` columns or `IDENTITY` sequences are managed transactionally with full ACID guarantees. If a transaction that allocated an ID rolls back, the ID is typically "lost" (sequences don't roll back), but there is no risk of reuse because the engine owns the sequence state.

In a distributed or semi-distributed system like Rocklake, you cannot rely on a central sequence server. The catalog lives in object storage (S3, GCS, Azure). There is no background process keeping a sequence value in memory across requests. And because MVCC uses `begin_snapshot` and `end_snapshot` columns to version every catalog row, snapshot IDs must be monotonically increasing — a snapshot ID that appears out of order would break the visibility filter.

Rocklake's solution is elegant: it takes advantage of the **single-writer guarantee**. Because only one Rocklake process can hold the writer epoch at any time, there is no concurrent contention for ID allocation. The writer process owns the counter state in memory and flushes it to SlateDB as part of each transaction that consumes new IDs.

## Counter Domains

Rocklake maintains four distinct counter domains, each identified by a different sub-key in the `0xFE` counter namespace:

| Domain | Sub-key byte | Scope | What uses it |
|--------|-------------|-------|--------------|
| `SnapshotId` | `0x01` | Global | Every snapshot write in `ducklake_snapshot` |
| `CatalogId` | `0x02` | Global | Schemas, tables, views, macros |
| `FileId` | `0x03` | Global | Data files, delete files |
| `ColumnId(table_id)` | `0x04` + big-endian `table_id` | Per-table | Columns in `ducklake_column` |

The first three counters are global — their keys are fixed two-byte sequences in the `0xFE` namespace. The column counter is per-table: each table `t` has a distinct counter key `[0xFE, 0x04, <t as big-endian u64>]`. This design allows column IDs to be independent across tables (each table starts at column 0) while remaining globally unique within the table.

### Why Four Domains?

Each domain corresponds to a different DuckLake entity type that requires independent monotonic numbering:

**SnapshotId** advances on every write transaction. Each `CREATE TABLE`, `INSERT`, `DROP`, or `ALTER` creates a new snapshot. Because the snapshot ID is used for MVCC visibility (`begin_snapshot` and `end_snapshot` columns), it must be monotonically increasing with no gaps or reuse.

**CatalogId** is shared by schemas, tables, views, and macros — any catalog object that is not a file. A single counter serves all of them because these objects are sparse (most catalogs have hundreds to low thousands), and the IDs are only used for stable referencing (foreign key relationships in DuckLake tables). 

**FileId** is separate from CatalogId because data files are dense — a heavily used table might register tens of thousands of data files over its lifetime, vastly outnumbering catalog objects. Keeping file IDs separate from catalog IDs prevents an active data pipeline from burning through catalog-range IDs and potentially causing confusion if catalog IDs were ever inspected.

**ColumnId(table_id)** is per-table because DuckLake's column change-tracking relies on column IDs being stable across schema changes. When a column is dropped and a new column is added to the same table, the new column must get a higher ID than any column that table has ever had. A global column counter would work but would waste IDs for tables that rarely change; a per-table counter is more space-efficient and easier to reason about.

## The CounterCache

The `CounterCache` struct in `rocklake-core/src/counters.rs` holds all three global counters in memory:

```rust
pub struct CounterCache {
    next_snapshot_id: u64,
    next_catalog_id: u64,
    next_file_id: u64,
}
```

When Rocklake starts up, it reads the persisted counter values from SlateDB and initializes `CounterCache` with them:

```rust
impl CounterCache {
    pub fn new(next_snapshot_id: u64, next_catalog_id: u64, next_file_id: u64) -> Self {
        Self { next_snapshot_id, next_catalog_id, next_file_id }
    }
}
```

Allocation is a simple increment of the cached value:

```rust
pub fn alloc_snapshot_id(&mut self) -> u64 {
    let id = self.next_snapshot_id;
    self.next_snapshot_id += 1;
    id
}
```

This returns the current value (which becomes the allocated ID) and increments the cache so the next call returns a different value. Because Rocklake is single-writer, no locking is needed around this operation at the Rust level. The borrow checker's `&mut self` requirement ensures that allocations are serialized.

Column counter values are not cached in `CounterCache`. Per-table column counters are loaded from SlateDB at the start of each DDL transaction that creates columns, incremented for each new column, and written back within the same transaction.

## Transactional Durability Protocol

The counter cache is only half the story. In-memory values are fast to increment, but if Rocklake crashes, the in-memory value is lost. On restart, Rocklake must be able to read the counters back from SlateDB — and the value it reads must be at least as large as any ID that was ever handed to a catalog row.

The protocol is: **counter increments are written to SlateDB in the same `DbTransaction` that creates the rows consuming those IDs.**

Concretely, when processing a `CREATE TABLE` SQL statement:

1. Allocate a `CatalogId` from `CounterCache` (in memory — fast, no I/O)
2. Allocate a `SnapshotId` from `CounterCache` (in memory — fast)
3. Construct the catalog rows (`ducklake_schema` update, new `ducklake_table` row, new `ducklake_column` rows)
4. Open a SlateDB `DbTransaction`
5. Write all catalog rows (keyed by entity type + ID)
6. Write the updated counter values (the new `next_snapshot_id` and `next_catalog_id`)
7. Commit the transaction

If the process crashes between steps 1–3 and step 7, the transaction never commits. The counter values written to SlateDB remain at their pre-allocation values. On restart, `CounterCache` is initialized from SlateDB — the crashed allocation is invisible. The IDs allocated in memory were never used, but they are also never in SlateDB, so no reuse is possible.

If the process crashes between step 7 (committed) and any subsequent in-memory state update, the transaction is durable. On restart, Rocklake reads the committed counter values and initializes `CounterCache` correctly. The allocated IDs are in both SlateDB (in the catalog rows) and in the counter values, so everything is consistent.

This is the key invariant: **the persisted counter value is always ≥ any ID present in a catalog row**. The monotonic sequence is preserved across crashes.

## Startup Counter Loading

At startup, Rocklake reads the counter values from SlateDB:

```rust
// Pseudocode for startup counter loading
let next_snapshot_id = db.get(CounterDomain::SnapshotId.key())
    .map(|bytes| decode_counter_value(&bytes))
    .unwrap_or(0);

let next_catalog_id = db.get(CounterDomain::CatalogId.key())
    .map(|bytes| decode_counter_value(&bytes))
    .unwrap_or(0);

let next_file_id = db.get(CounterDomain::FileId.key())
    .map(|bytes| decode_counter_value(&bytes))
    .unwrap_or(0);

let cache = CounterCache::new(next_snapshot_id, next_catalog_id, next_file_id);
```

If a counter key is absent (a fresh catalog with no writes), the counter starts at 0. IDs are allocated starting at 0 and increase from there. There is no "reserved" range or initial offset.

## Key Encoding

Counter keys live in the `0xFE` namespace. The exact byte layout is:

| Counter | Key bytes | Length |
|---------|-----------|--------|
| `SnapshotId` | `[0xFE, 0x01]` | 2 bytes |
| `CatalogId` | `[0xFE, 0x02]` | 2 bytes |
| `FileId` | `[0xFE, 0x03]` | 2 bytes |
| `ColumnId(t)` | `[0xFE, 0x04, <t as 8-byte big-endian>]` | 10 bytes |

Counter values are encoded as big-endian u64 (8 bytes) wrapped in the standard SDKV value envelope (1-byte format version + 1-byte type tag + 8-byte payload = 10 bytes total).

The `0xFE` prefix is chosen to be the penultimate byte value (just below `0xFF` used for system metadata keys), placing all counters at the very end of the key space in SlateDB's sorted order. This is a minor optimization: SST files that contain only counters are isolated in the key space and unlikely to be co-located with data files in the same SST shard, reducing read amplification for counter-heavy workloads.

## Overflow Handling

All IDs are u64 values, giving a theoretical maximum of 2^64 − 1 ≈ 1.8 × 10^19 per counter. At a rate of 1 million operations per second:

- **SnapshotId**: would overflow after ~585,000 years
- **CatalogId**: would overflow after ~585,000 years (at 1M schema/table operations/second, which is far beyond realistic usage)
- **FileId**: would overflow after ~585,000 years (at 1M file registrations/second)

Overflow is not a practical concern. The counter allocation code does not implement overflow detection — there is no wrapping, saturating, or error-on-overflow behavior. In the astronomically unlikely event of overflow, u64 arithmetic would wrap to 0, which would cause ID reuse. If you are somehow running Rocklake at internet scale across centuries of continuous operation and are concerned about this, please file an issue.

## Inspecting Counters

You can inspect the current counter values using the `inspect` command:

```bash
rocklake inspect snapshot --latest --catalog s3://my-bucket/catalog/
```

The output includes the latest snapshot ID. To see raw counter keys directly, use the `--key` flag:

```bash
rocklake inspect snapshot --latest --catalog s3://my-bucket/catalog/ --format json
```

For lower-level inspection, the `verify catalog` command checks counter consistency: it scans all catalog rows and verifies that no row has an ID ≥ the corresponding counter value (which would mean the counter has fallen behind the actual data).

## Interaction with the Single-Writer Model

The counter allocation design is deeply tied to Rocklake's single-writer architecture. The in-memory `CounterCache` is only safe because exactly one process holds the writer epoch at any time.

If two Rocklake processes somehow both believed themselves to be the current writer and both had a `CounterCache`, they could both allocate the same ID — one process allocates `snapshot_id = 100`, the other also allocates `snapshot_id = 100`, and both write conflicting rows to SlateDB at that ID. SlateDB's last-write-wins semantics would silently corrupt the catalog.

The writer epoch prevents this. When a Rocklake process starts, it reads the current epoch from a system key and increments it. The new epoch is written to SlateDB before any catalog mutations begin. If a previous writer is still running, its subsequent writes will fail when it discovers its epoch has been superseded. This ensures that at any point in time, only the process that incremented the epoch most recently can commit transactions — and therefore only one `CounterCache` is active at any time.

For more on the single-writer constraint and epoch fencing, see [Single Writer, Many Readers](../concepts/single-writer-many-readers.md).

## Testing Counter Consistency

The `verify catalog` command checks counter consistency as part of its scan. It examines every entity key in the catalog and verifies that its embedded ID is strictly less than the corresponding counter value. If any entity has an ID ≥ the counter, the counter has fallen behind — this is a bug, because it means a future allocation could assign an ID that already exists in a catalog row.

```bash
rocklake verify catalog --catalog s3://my-bucket/catalog/
```

If `verify catalog` reports counter desync, use `repair --apply` to advance the counter to the correct value:

```bash
rocklake repair --dry-run --catalog s3://my-bucket/catalog/
# Review the proposed repairs, then:
rocklake repair --apply --catalog s3://my-bucket/catalog/
```

Counter desync should never happen in normal operation — it indicates either a bug in ID allocation code or a manual catalog edit that bypassed the normal write path. Either way, `repair` can fix it safely by scanning the full catalog, finding the maximum observed ID in each domain, and writing corrected counter values in a single atomic transaction.

## Further Reading

- **[Key-Value Mapping](../concepts/key-value-mapping.md)** — Complete key layout for all entity types, including the `0xFE` counter namespace
- **[MVCC Implementation](mvcc-implementation.md)** — How `begin_snapshot` and `end_snapshot` use snapshot IDs for versioning
- **[Single Writer, Many Readers](../concepts/single-writer-many-readers.md)** — The architectural constraint that makes in-memory counter caching safe
- **[Transaction Model](transaction-model.md)** — How SlateDB transactions ensure counter atomicity