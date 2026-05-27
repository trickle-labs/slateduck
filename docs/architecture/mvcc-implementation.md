# MVCC Implementation

This page describes the storage-level implementation of multi-version concurrency control in Rocklake. While the [Concepts: MVCC & Snapshots](../concepts/mvcc.md) page explains the theory — what MVCC means, why it matters, and how readers and writers coexist — this page focuses on the engineering: how version information is encoded in keys and values, how the visibility filter works at the code level, how the three different MVCC behaviors (versioned, append-only, mutable singleton) are implemented, and how garbage collection identifies rows that are safe to remove.

Understanding this page requires familiarity with the [Key Layout](key-layout.md) and [Value Encoding](value-encoding.md) — specifically, how `begin_snapshot` appears as both a key suffix (for versioned entities) and a value field, and how `end_snapshot` is stored exclusively in the value.

## The Three MVCC Behaviors

Not all catalog tables use the same versioning strategy. Rocklake defines three distinct MVCC behaviors, assigned per table type in the tag registry. The behavior determines how rows are written, how they are superseded, and how visibility filtering works during reads.

### Behavior 1: Versioned Entities

Versioned entities are catalog objects that can be modified over time — their metadata can change without destroying history. Each modification creates a new version (a new key-value pair) while the previous version is marked as superseded (its value is updated with an `end_snapshot`).

**Tables using this behavior:**

- `ducklake_schema` (tag `0x04`)
- `ducklake_table` (tag `0x05`)
- `ducklake_column` (tag `0x06`)
- `ducklake_view` (tag `0x07`)
- `ducklake_macro` (tag `0x08`)

**Key structure:** `tag | parent_ids | entity_id | begin_snapshot`

The `begin_snapshot` is the last component of the key. This means all versions of the same entity (same parent_ids and entity_id) are lexicographically contiguous — they differ only in the trailing `begin_snapshot` field.

**Visibility rule:**

```
begin_snapshot <= target_snapshot AND (end_snapshot IS NULL OR target_snapshot < end_snapshot)
```

A row is visible at snapshot N if it was created at or before N and has not yet been superseded at N.

**Example: Table renamed at snapshot 10:**

```
Key:   0x05 | schema_id=1 | table_id=42 | begin_snapshot=5
Value: { name: "orders", end_snapshot: Some(10), ... }

Key:   0x05 | schema_id=1 | table_id=42 | begin_snapshot=10  
Value: { name: "customer_orders", end_snapshot: None, ... }
```

At snapshot 7, the reader sees "orders" (begin=5 ≤ 7, end=10 > 7).
At snapshot 12, the reader sees "customer_orders" (begin=10 ≤ 12, end=None).
At snapshot 4, the reader sees nothing (begin=5 > 4 for both versions).

**Supersession process:**

When a versioned entity is modified (renamed, altered, dropped), Rocklake:

1. Reads the current version's key-value pair
2. Updates the current version's value with `end_snapshot = new_snapshot_id` (this is a SlateDB PUT to the existing key with a new value)
3. Writes a new key-value pair with `begin_snapshot = new_snapshot_id` and the updated metadata

Both operations (updating the old value and writing the new key) are included in the same `WriteBatch`, so they are atomic. No reader can see a state where the old version's `end_snapshot` is set but the new version does not yet exist.

### Behavior 2: Append-Only Entities

Append-only entities are catalog objects that are created once and never modified. They do not have multiple versions — once written, their key-value pair is immutable for the lifetime of the catalog (until excision removes them). They have a `begin_snapshot` (recording when they became visible) but no `end_snapshot` concept.

**Tables using this behavior:**

- `ducklake_snapshot` (tag `0x02`)
- `ducklake_snapshot_changes` (tag `0x03`)
- `ducklake_data_file` (tag `0x0B`)
- `ducklake_delete_file` (tag `0x0C`)
- `ducklake_files_scheduled_for_deletion` (tag `0x0D`)
- `ducklake_macro_impl` (tag `0x09`)
- `ducklake_macro_parameters` (tag `0x0A`)
- `ducklake_inlined_data_tables` (tag `0x0E`)

**Key structure:** `tag | identifying_fields` (no `begin_snapshot` in key)

For append-only entities, `begin_snapshot` is stored in the value rather than the key. This is a space optimization: since append-only entities never have multiple versions, there is no need to use `begin_snapshot` as a key component for version ordering.

**Visibility rule:**

```
begin_snapshot <= target_snapshot
```

An append-only row is visible at snapshot N if it was created at or before N. Once visible, it remains visible forever (at all subsequent snapshots) because it is never superseded.

**Why data files are append-only:**

Data files (Parquet files registered with the catalog) are never "modified." A file is registered once and remains registered. If data needs to be deleted from a file, a separate `ducklake_delete_file` entry is created — the original data file registration is never updated. This makes data files naturally append-only.

### Behavior 3: Mutable Singletons

Mutable singletons are catalog objects with exactly one current value, updated in place. They have no version history and are always read at the latest state. This behavior exists for operational metadata that changes frequently and where historical values have no meaning.

**Tables using this behavior:**

- `ducklake_table_stats` (tag `0x11`)
- `ducklake_metadata` (tag `0x01`, for certain scope/key combinations)
- Counters (tag `0xFE`)
- System keys (tag `0xFF`)

**Key structure:** `tag | identifying_fields` (no version component)

**Visibility rule:** Always visible. The current value is the only value.

**Implementation:** A write to a mutable singleton is a simple SlateDB PUT to a fixed key. The new value replaces the old value (conceptually — in the LSM-tree, the new write shadows the old one until compaction merges them).

Mutable singletons are the exception to Rocklake's immutability principle. They exist because some metadata (like "how many data files does table X have?" or "what is the current writer epoch?") changes frequently and where only the current value matters. Keeping historical versions of these values would waste storage for no benefit.

## The Core Visibility Filter

The heart of MVCC is the `is_visible` function — two integer comparisons that determine whether a row is visible at a given snapshot:

```rust
/// Returns true if a row with the given version bounds is visible at dl_snapshot_id.
pub fn is_visible(
    begin_snapshot: u64,
    end_snapshot: Option<u64>,
    dl_snapshot_id: u64,
) -> bool {
    begin_snapshot <= dl_snapshot_id
        && end_snapshot.map_or(true, |end| dl_snapshot_id < end)
}
```

This function is called for every row returned by every prefix scan. It sits in the absolute hot path of the read operation — a prefix scan returning 1,000 rows calls `is_visible` 1,000 times. The function is deliberately minimal (two integer comparisons, one Option check) because even a few nanoseconds of overhead per call would be noticeable at scale.

### Why `dl_snapshot_id < end` (strict less-than)?

The strict less-than in the end_snapshot check means that a row is invisible at the exact snapshot where it was superseded. This is correct because supersession and creation of the replacement happen in the same transaction (same snapshot). At the supersession snapshot, the new version (with `begin_snapshot = supersession_snapshot`) should be visible instead of the old version.

If the check were `<=` instead of `<`, both the old and new version would be visible at the supersession snapshot, which would violate the invariant that each entity has exactly one visible version at any given snapshot.

## Latest Visible Version Resolution

For versioned entities, a reader often needs "the current version" of an entity at a given snapshot. Since all versions of an entity are contiguous in key space (same prefix, different `begin_snapshot` suffix), the reader scans the prefix and selects the correct version:

```rust
/// Given all versions of an entity, return the one visible at dl_snapshot_id.
/// If multiple versions are visible (should not happen with correct MVCC invariants),
/// returns the one with the highest begin_snapshot.
pub fn latest_visible_version<T>(
    versions: impl Iterator<Item = (u64, Option<u64>, T)>,
    dl_snapshot_id: u64,
) -> Option<T> {
    versions
        .filter(|(begin, end, _)| is_visible(*begin, *end, dl_snapshot_id))
        .max_by_key(|(begin, _, _)| *begin)
        .map(|(_, _, row)| row)
}
```

The `max_by_key` on `begin_snapshot` is a safety measure: with correct MVCC invariants, there should be exactly zero or one visible version at any snapshot. But if an invariant is violated (perhaps due to a bug or corruption), selecting the newest visible version is the safest behavior.

### Performance of Version Resolution

For the common case (entity has 1–3 versions), version resolution is trivial — iterate through 1–3 entries and pick the visible one. For pathological cases (an entity altered 100 times), the reader must examine all 100 entries. This is still fast (100 protobuf decodes + 100 visibility checks takes under 100 microseconds) but would be noticeable for extremely hot entities.

In practice, the pathological case is rare. Most catalog entities are created once and never modified. The entities most likely to have many versions are columns (frequently renamed or type-annotated) and tables (frequently renamed). Even for these, more than 10 versions is exceptional.

## GC Eligibility Determination

A superseded row becomes eligible for physical deletion (excision) when no valid reader can ever need it:

```rust
/// Returns true if a superseded row can be safely excised.
/// A row is excisable when its end_snapshot is at or before the retention horizon,
/// meaning no reader is allowed to query a snapshot where this row would be visible.
pub fn is_gc_eligible(
    end_snapshot: Option<u64>,
    retain_from: u64,
) -> bool {
    match end_snapshot {
        None => false,  // Active rows (no end_snapshot) are never GC-eligible
        Some(end) => end <= retain_from,
    }
}
```

The logic: if `retain_from = 50`, then no reader is allowed to query at snapshot < 50. A row with `end_snapshot = 45` was superseded at snapshot 45, meaning it is only visible at snapshots < 45. Since no reader can query at snapshots < 50 (which includes < 45), no reader will ever need this row. It can be safely removed.

Rows without `end_snapshot` (still-active rows) are never GC-eligible regardless of the retention horizon. They might be visible at the current snapshot.

## Inlined Data MVCC

Inlined data (small row data stored directly in the catalog under tag `0xFD`) uses a variant MVCC model:

**Inlined inserts** have both `begin_snapshot` and optional `end_snapshot`:

- `begin_snapshot` records when the row was inserted
- `end_snapshot` records when the row was logically deleted (by a subsequent UPDATE or DELETE that creates a delete marker)
- Visibility uses the standard `is_visible` rule

**Inlined deletes** have only `begin_snapshot`:

- They record that a specific row (in a data file or in inlined data) has been logically deleted
- Visibility check: `begin_snapshot <= target` (same as append-only)
- They are never superseded — a delete is permanent from its creation snapshot onward

## Secondary Index and Snapshot-Scoped Lookups

The secondary index (tag `0xFC`) provides an alternative access pattern for data files. Its key includes `snapshot_id`:

```
Key: 0xFC | snapshot_id | table_id | data_file_id
Value: (empty or minimal metadata)
```

This enables queries like "what files were registered at snapshot 42 for table T?" — useful for change tracking and incremental processing — without scanning all files for the table and filtering by begin_snapshot.

The secondary index is maintained as part of the commit batch: whenever a data file is registered, a corresponding secondary index entry is added to the same WriteBatch. It is not MVCC-filtered (it is read as-is), and it is excised when the corresponding data file entry is excised.

## MVCC and SlateDB Compaction

SlateDB's compaction process merges SST files to reduce read amplification. It does not understand MVCC semantics — it treats all key-value pairs as opaque bytes. This means:

- Superseded MVCC versions (rows with `end_snapshot` set) remain in storage after compaction
- Compaction does not remove "old" versions because it cannot distinguish MVCC-superseded rows from active ones
- Physical deletion of superseded rows only happens through Rocklake's explicit excision process

SlateDB's own tombstones (markers for deleted keys) are handled by SlateDB compaction. But during normal Rocklake operation, keys are never deleted — supersession updates the value at an existing key (setting `end_snapshot`) but does not remove any key. Physical key deletion happens only during excision, which uses SlateDB's delete API.

## Invariants and Safety Checks

The MVCC implementation maintains several invariants that the `verify` tool checks:

1. **No orphaned versions.** Every versioned entity with `end_snapshot` set must have a successor version (with `begin_snapshot = end_snapshot`) — unless the entity was dropped (in which case, no successor exists, and the `end_snapshot` represents the drop).

2. **Monotonic version ordering.** For any entity, `begin_snapshot` values across its versions must form a strictly increasing sequence.

3. **No overlapping visibility.** At any valid snapshot, at most one version of an entity should pass the visibility filter.

4. **Consistent counters.** The `next_snapshot_id` counter must be greater than the maximum `begin_snapshot` or `end_snapshot` found in any row. Similarly for other ID counters.

5. **Retention boundary respect.** No row with `begin_snapshot > retain_from` should have `end_snapshot <= retain_from` (this would mean a row was created after the retention boundary but superseded before it, which violates temporal causality).

Violations of these invariants indicate bugs or corruption. The `verify` tool reports them with specific key references, and the `repair` tool can fix certain classes of violations (like stale counters) conservatively.

## Performance Characteristics

| Operation | Complexity | Notes |
|-----------|-----------|-------|
| `is_visible` check | O(1) | Two integer comparisons |
| Latest version resolution | O(V) | V = number of versions of the entity (typically 1–3) |
| Prefix scan with MVCC filter | O(N) | N = total rows in prefix (including superseded) |
| GC eligibility check | O(1) | One integer comparison |
| Secondary index lookup | O(1) | Point query by snapshot + table + file |

The MVCC overhead per prefix scan is linear in the number of historical (superseded) rows. For catalogs with long retention (many historical versions), this overhead grows. This is mitigated by:

- Garbage collection (advancing `retain_from` reduces the visible range, and excision physically removes old rows)
- Secondary indexes (providing alternative access paths that skip historical versions)
- Hot key optimization (caching frequently-needed current state to avoid scanning)

## Further Reading

- **[Concepts: MVCC & Snapshots](../concepts/mvcc.md)** — The theoretical foundation
- **[Key Layout](key-layout.md)** — How version information is encoded in keys
- **[Value Encoding](value-encoding.md)** — How version information is encoded in values
- **[Transaction Model](transaction-model.md)** — How commits create new versions atomically
- **[Internals: MVCC Filter](../internals/mvcc-filter.md)** — Performance analysis of the filter under various workloads
