# MVCC and Snapshot Isolation

Multi-Version Concurrency Control is the mechanism that makes Rocklake's time travel, crash safety, and reader independence work at the implementation level. If immutability is the architectural principle (we never delete committed facts), then MVCC is the technique that makes immutability useful — it is the system that answers the question "given that many versions of each entity coexist in storage, which version should this particular reader see?"

DuckLake's versioning model uses `begin_snapshot` and `end_snapshot` columns on catalog rows to express the snapshot interval during which a given version of a row is visible. Rocklake maps this model onto SlateDB's key-value layout in a way that makes the most common query patterns efficient. This page explains how the mapping works, what the visibility filter looks like, how different types of catalog entities are versioned, and how garbage collection interacts with the MVCC model.

## The Visibility Rule

Every versioned row in the catalog carries two fields that define its temporal bounds:

- **`begin_snapshot`** — The snapshot ID at which this version became visible. Set when the row is first written as part of a catalog transaction.
- **`end_snapshot`** — The snapshot ID at which this version was superseded. Set when a newer version of the same logical entity is created. NULL if the version is still current (has not been superseded).

A row is visible at a target snapshot S if and only if:

```
begin_snapshot <= S AND (end_snapshot IS NULL OR S < end_snapshot)
```

This rule is elegant in its simplicity: a row is visible from the moment it was created until (but not including) the moment it was superseded. A reader targeting any snapshot sees exactly one version of each logical entity, or no version at all if the entity did not exist at that snapshot. There is no ambiguity, no ordering dependency between rows, and no need for external coordination to determine visibility.

The visibility filter is applied at read time — when Rocklake scans a key range in SlateDB, it reads all versions of all entities within the range and then filters by the target snapshot. This is a simple integer comparison (at most two comparisons per row), which is extremely fast relative to the object-store I/O that dominates overall operation latency.

## How Versioned Keys Work

The key insight of Rocklake's MVCC implementation is that different versions of the same logical entity occupy different keys in SlateDB. This is different from traditional MVCC implementations (like PostgreSQL's) where different versions of the same row occupy the same heap location or different locations within the same table.

In Rocklake, a versioned key includes `begin_snapshot` as part of the key suffix:

```
[tag][identifying_fields...][begin_snapshot]
```

For example, the key for a table row is:

```
[0x05][schema_id: 4 bytes][table_id: 4 bytes][begin_snapshot: 8 bytes]
```

When a table is created at snapshot 5 and renamed at snapshot 12, two distinct keys exist:

| Key bytes | begin_snapshot | end_snapshot (in value) | table_name (in value) |
|-----------|---------------|------------------------|----------------------|
| `[0x05][00000001][00000042][0000000000000005]` | 5 | 12 | orders |
| `[0x05][00000001][00000042][000000000000000C]` | 12 | NULL | customer_orders |

These are distinct key-value entries in SlateDB — they do not overwrite each other, they coexist permanently. A prefix scan on `[0x05][00000001][00000042]` returns both entries, and the MVCC filter selects the appropriate one based on the target snapshot.

This design has several important consequences:

**Old versions are never overwritten.** Because each version has a unique key (different `begin_snapshot`), writing a new version cannot corrupt or affect an old version. This is the mechanical implementation of the immutability guarantee.

**Range scans return all versions.** A prefix scan returns every version of every entity within the range. For entities with many historical versions, this means the scan returns more data than a single reader needs. However, the MVCC filter is applied in memory after the scan returns, and the per-row filtering cost is negligible. In practice, most catalog entities have very few versions (typically 1–5).

**Version cleanup requires excision.** Because old versions are distinct keys, SlateDB's normal compaction process cannot remove them (compaction only removes tombstoned keys, and old versions are not tombstoned — they are valid key-value entries with `end_snapshot` set). Physical removal of old versions requires the explicit excision process.

## Example: A Table Through Its Lifecycle

Let's trace a table through creation, modification, and the resulting MVCC state:

**Snapshot 5: Table created.** DuckDB issues `CREATE TABLE analytics.events (...)`. Rocklake writes:
- A table row with key `[0x05][schema_id][table_id][5]` and value `{name: "events", end_snapshot: NULL}`
- Five column rows, each with keys `[0x06][table_id][column_id][5]`

**Snapshot 12: Table renamed.** DuckDB issues `ALTER TABLE analytics.events RENAME TO user_events`. Rocklake atomically:
- Sets `end_snapshot = 12` in the value of the existing table row at key `[...][5]`
- Writes a new table row at key `[0x05][schema_id][table_id][12]` with value `{name: "user_events", end_snapshot: NULL}`

**Snapshot 15: Column added.** DuckDB issues `ALTER TABLE analytics.user_events ADD COLUMN region VARCHAR`. Rocklake writes:
- A new column row at key `[0x06][table_id][new_column_id][15]`
- (The table row is not modified — column additions do not change the table name)

Now a reader can query at any snapshot:

- **At snapshot 7:** Sees table "events" with 5 columns. The rename has not happened yet (begin_snapshot 12 > 7), and the new column does not exist (begin_snapshot 15 > 7).
- **At snapshot 13:** Sees table "user_events" with 5 columns. The rename is visible (12 <= 13 and end_snapshot is NULL), but the new column is not yet visible (15 > 13).
- **At snapshot 20:** Sees table "user_events" with 6 columns. Both the rename and the column addition are visible.

Each of these queries runs against the same underlying key-value data — the difference is solely in the MVCC filter parameters.

## Categories of Versioned Tables

Not all DuckLake catalog tables use the full `begin_snapshot`/`end_snapshot` MVCC model. Rocklake classifies tables into three categories based on their versioning behavior:

### Fully Versioned Tables

These tables participate in the standard MVCC model. Each logical entity can have multiple historical versions, each visible at a different snapshot range. Examples include:

- `ducklake_schema` — Schemas can be renamed or dropped
- `ducklake_table` — Tables can be renamed, have properties changed
- `ducklake_column` — Columns can be renamed, retyped, added, dropped
- `ducklake_view` — Views can be redefined
- `ducklake_macro` — Macros can be redefined

For these tables, the key includes `begin_snapshot` and the value includes `end_snapshot`.

### Append-Only Tables

These tables only gain new entries — existing entries are never superseded. Once written, a row is visible at all snapshots from its `begin_snapshot` onward (until the retention horizon). Examples include:

- `ducklake_data_file` — A Parquet file, once registered, belongs to the table permanently (its visibility is bounded only by the table's own lifecycle)
- `ducklake_data_file_column_statistics` — Statistics for a file are immutable once written
- `ducklake_delete_file` — Delete markers are append-only
- `ducklake_snapshot` — Snapshot records are never modified

For append-only tables, the visibility rule simplifies to `begin_snapshot <= S` — no `end_snapshot` check is needed because there is none.

### Singleton Tables

A small number of tables represent single-valued state that is overwritten rather than versioned:

- `ducklake_metadata` — Configuration metadata (catalog description, settings) where only the current value matters

For singleton tables, the key does not include `begin_snapshot`, and updates overwrite the previous value. These tables do not participate in time travel — they always return the current state regardless of the target snapshot.

## The Distinction Between `dl_snapshot_id` and `kv_snapshot`

This distinction is subtle but important for anyone reading the Rocklake source code or diagnosing MVCC-related issues:

**`dl_snapshot_id`** is the DuckLake-level catalog version — the monotonically increasing integer that identifies a catalog snapshot. It is the value stored in `begin_snapshot` and `end_snapshot` columns, and it is what DuckDB specifies when doing time travel (`ATTACH ... SNAPSHOT '15'`). It is managed by Rocklake's counter allocation system and advances with every catalog mutation.

**`kv_snapshot` / `kv_read_view`** is SlateDB's internal read view — a point-in-time reference to the key-value store's state that determines which WAL entries and SST files are visible to a reader. This is the storage-layer concept, analogous to PostgreSQL's transaction snapshot that determines which heap tuples are visible.

These are different things at different layers of the stack. A `dl_snapshot_id` is a parameter to the MVCC visibility filter. A `kv_snapshot` is a parameter to SlateDB's reader that determines which bytes are readable. In normal operation, a reader opens a `kv_snapshot` (which sees all committed key-value entries) and then applies the `dl_snapshot_id` visibility filter to narrow down which of those entries are relevant.

Confusing the two leads to bugs that are extremely hard to diagnose. A reader that uses a stale `kv_snapshot` might not see recently committed entries, even when filtering at the latest `dl_snapshot_id`. Conversely, a reader with a fresh `kv_snapshot` but an old `dl_snapshot_id` correctly sees the historical state — this is time travel working as intended.

## MVCC and Garbage Collection

The `retain-from` system value controls the garbage collection horizon. When `retain-from` is advanced to snapshot N, it means "queries at snapshots before N are no longer permitted." This does not delete any bytes — it only constrains the set of valid target snapshots for the MVCC filter.

A superseded row (one with `end_snapshot` set) becomes GC-eligible when `end_snapshot <= retain-from`. At that point, no valid reader can ever see the row: readers at snapshots before `retain-from` are rejected, and readers at snapshots >= `retain-from` will not see the row because `S >= retain-from > end_snapshot` means `S >= end_snapshot`, violating the `S < end_snapshot` visibility condition.

The actual physical removal of GC-eligible rows requires the excision process, which is a separate operational step. GC only advances the logical visibility boundary; excision performs the physical deletion.

## Performance Characteristics

The MVCC filter adds minimal overhead to read operations:

- **Per-row cost:** Two integer comparisons (begin_snapshot <= target, end_snapshot == NULL or target < end_snapshot). This is a few nanoseconds per row — negligible relative to object-store I/O.
- **Scan amplification:** Prefix scans return all versions of all entities within the range, including historical versions that will be filtered out. For entities with few versions (the common case), amplification is minimal. For entities with many versions (a table that has been renamed 100 times), the scan returns more data than needed, but this is rare in practice.
- **Memory overhead:** All versions within a scanned range are loaded into memory before filtering. For typical catalog sizes (thousands of entities, each with 1–5 versions), this is a few megabytes at most.

The dominant cost in any Rocklake read operation is the object-store I/O to fetch SST blocks — typically 20–50 ms per block on S3 Standard. The MVCC filtering that happens after the data is in memory is orders of magnitude faster than the I/O and does not meaningfully affect operation latency.

## Further Reading

- **[Time Travel](snapshots.md)** — The user-facing consequence of MVCC: querying at any historical snapshot
- **[Catalog Immutability](immutability.md)** — The principle that makes MVCC possible without overwriting old versions
- **[Architecture: MVCC Implementation](../architecture/mvcc-implementation.md)** — The Rust implementation details
- **[Internals: MVCC Filter](../internals/mvcc-filter.md)** — The exact filter logic and edge cases
- **[Reference: Catalog Tables](../reference/catalog-tables.md)** — Which tables are versioned, append-only, or singleton
