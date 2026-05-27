# CDC Design: `table_changes()` Execution Pipeline

## Overview

The `table_changes(table_ref, start_snapshot, end_snapshot)` function provides
row-level Change Data Capture (CDC) for DuckLake tables managed by Rocklake.
It returns the set of row-level mutations that occurred between two catalog
snapshots: inserts, deletes, and (when detectable) update pre/post-image pairs.

## Execution Pipeline

```
┌─────────────────┐
│ SQL: SELECT *   │
│ FROM table_     │
│ changes(t,s,e)  │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ GC Boundary     │  → SQLSTATE 55000 if start_snapshot < retain_from
│ Check           │
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ SnapshotDiff    │  → Catalog scan: (from_snapshot, to_snapshot]
│ Computation     │     Returns added_data_files + retired_data_files
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Parquet Scan    │  → Read row data from each file in the diff
│ (per file)      │     Extract rowid + column values per row
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Change          │  → Match rowids between added/retired sets
│ Correlation     │     Same rowid in both → UPDATE (pre + post image)
│                 │     Only in added → INSERT
│                 │     Only in retired → DELETE
└────────┬────────┘
         │
         ▼
┌─────────────────┐
│ Result Set      │  → Columns: rowid, change_type, columns_json
│ Emission        │
└─────────────────┘
```

## SnapshotDiff Multi-Snapshot Windows (v0.19)

Prior to v0.19, `snapshot_diff()` only returned files where
`snapshot_id == to_snapshot`. The v0.19 implementation scans the full
`(from_snapshot, to_snapshot]` interval using the new `begin_snapshot` and
`end_snapshot` fields on `DataFileRow`:

- **Added files**: `begin_snapshot > from AND begin_snapshot <= to`
- **Retired files**: `end_snapshot > from AND end_snapshot <= to`
- **Backward compatibility**: Pre-v0.19 rows without `begin_snapshot` fall
  back to `snapshot_id` for the begin window check.

This means `table_changes(42, 45)` correctly includes changes at snapshots
43, 44, and 45.

## Row ID Extraction

Each `DataFileRow` contains a `row_count` field. Row IDs are assigned
sequentially within a file starting from a base offset derived from the
table's rowid counter at the time of file registration.

In a full Parquet integration, rowids are extracted from the Parquet file's
row group metadata (`__sd_rowid` column). The current implementation uses
sequential assignment based on the file's `row_count`.

## Change Type Correlation

Given the sets of rows from added files (A) and retired files (R):

1. **Updates**: `rowid ∈ A ∩ R` → emit `update_preimage` (from R) and
   `update_postimage` (from A)
2. **Inserts**: `rowid ∈ A \ R` → emit `insert`
3. **Deletes**: `rowid ∈ R \ A` → emit `delete`

## Property: Stream Reconstruction

The CDC implementation satisfies the following property:

> Applying the emitted change stream to the start-snapshot state exactly
> reconstructs the end-snapshot state.

This is verified by the `apply_changes()` function and the
`test_apply_changes_reconstructs_end_state` test.

## Error Handling

| Condition | SQLSTATE | Error |
|-----------|----------|-------|
| `start_snapshot` below retain-from | `55000` | Snapshot too old |
| Table not found | `42P01` | Table not found |
| Parquet read failure | `XX000` | Internal error |
