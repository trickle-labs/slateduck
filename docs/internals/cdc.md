# CDC Internals — Real Parquet Row Scanning (v0.27.1)

This document describes the implementation of the Change Data Capture (CDC) row-level scanning path added in v0.27.1.

## Overview

`table_changes()` is a SQL table function that exposes row-level CDC from DuckLake snapshots. Before v0.27.1, the CDC pipeline correctly classified inserts, updates, and deletes based on the catalog diff, but the column payload (`columns_json`) was always an empty object `"{}"` because `extract_rows_from_file()` discarded the file path and returned synthetic rows.

As of v0.27.1, `extract_rows_from_parquet()` opens the actual Parquet file from the object store, deserialises every record batch into column-name → JSON-value mappings, and populates `columns_json` with the real row data.

## `extract_rows_from_parquet()`

```rust
pub async fn extract_rows_from_parquet(
    object_store: &Arc<dyn ObjectStore>,
    file_path: &str,
    base_rowid: u64,
    expected_record_count: Option<u64>,
    batch_size: usize,
) -> Result<Vec<ParquetRowData>, TableChangesError>
```

**Parameters:**

| Parameter | Description |
|-----------|-------------|
| `object_store` | The same `Arc<dyn ObjectStore>` that backs the catalog. File paths are resolved against this store. |
| `file_path` | The path to the Parquet file, as stored in `DataFileRow.path`. |
| `base_rowid` | The starting row ID for the first row in this file. Row IDs are assigned sequentially within a file. |
| `expected_record_count` | The `record_count` value from catalog metadata. Used for mismatch detection (N-04). Pass `None` to skip verification. |
| `batch_size` | Number of rows per record batch. Use `DEFAULT_CDC_BATCH_SIZE` (50,000) for standard use. |

**Returns:** One `ParquetRowData` per row with `rowid` and `columns_json` (a JSON object string, e.g. `{"id":1,"name":"alice"}`).

**Implementation notes:**

- Bytes are fetched from the object store with a single `get()` call and held in memory while reading. This is a deliberate trade-off: it avoids the `object_store` version conflict that would arise from using `ParquetObjectReader` (which requires `object_store 0.11`, but the workspace uses `0.12`).
- The synchronous `ParquetRecordBatchReaderBuilder` is used on the in-memory `Bytes`. For most DuckLake data files (< 512 MB), this is preferable to the async streaming path because it has less protocol overhead.
- If a file is too large to fit in memory, consider chunking at the catalog registration level (e.g. multiple smaller Parquet files instead of one large file).

## Batch Size — `DEFAULT_CDC_BATCH_SIZE`

```rust
pub const DEFAULT_CDC_BATCH_SIZE: usize = 50_000;
```

Controls the maximum number of rows per Arrow record batch requested from the Parquet reader. 50,000 rows is the default; lower values reduce peak memory at the cost of more iterations.

Pass a custom value when calling `extract_rows_from_parquet()` directly if your workload has specific memory constraints.

## Record-Count Mismatch Warning (N-04)

After scanning all batches, the function compares the actual scanned row count against `expected_record_count`:

```
actual != expected  →  emit tracing::warn! + increment CDC_RECORD_COUNT_MISMATCHES
```

The warn log includes:
- `file_path` — the Parquet file path that triggered the mismatch
- `expected_record_count` — the value stored in catalog metadata
- `actual_record_count` — the real scanned count
- `counter` — the Prometheus counter name (`slateduck_cdc_record_count_mismatch_total`)

**Why this can happen:** A writer that crashed after writing part of a Parquet file, but before updating the catalog's `record_count`, will leave a file with fewer rows than expected. The scanned count is always used — the catalog metadata is informational. This is the correct recovery behaviour (CDC consumers receive the partial write, not fabricated rows).

### Prometheus counter

```
# HELP slateduck_cdc_record_count_mismatch_total Times a Parquet file's scanned row count differed from catalog metadata (N-04).
# TYPE slateduck_cdc_record_count_mismatch_total counter
slateduck_cdc_record_count_mismatch_total 0
```

The counter value is read from the `CDC_RECORD_COUNT_MISMATCHES` global atomic in `slateduck-sql` and synced to `CatalogMetrics` by a background task in `slateduck-pgwire/src/main.rs` every 5 seconds.

## Object Store Error Handling

If the object store returns any error (including `NotFound` for a missing data file path), `extract_rows_from_parquet()` returns `TableChangesError::Storage(message)` with SQLSTATE `58030` (external object store error). This propagates through `execute_table_changes()` in the PG-Wire executor as a `SlateDuckError::SqlState { code: "58030", .. }`.

The caller receives a proper PostgreSQL error response with `SQLSTATE 58030` rather than a panic or opaque internal error.

## Type Mapping — Arrow → JSON

| Arrow type | JSON representation |
|------------|---------------------|
| `Boolean` | `true` / `false` |
| `Int8`/`Int16`/`Int32`/`Int64` | JSON number |
| `UInt8`/`UInt16`/`UInt32`/`UInt64` | JSON number |
| `Float32`/`Float64` | JSON number; `NaN`/`Inf` → `null` |
| `Utf8`/`LargeUtf8` | JSON string |
| `Binary` | hex-encoded JSON string (e.g. `"deadbeef"`) |
| `Date32`/`Date64` | JSON integer (days / milliseconds since epoch) |
| `Null` | `null` |
| All other types | `"<DataType>"` string (type name) |

## Integration with `execute_table_changes()`

The PG-Wire executor (`crates/slateduck-pgwire/src/executor/catalog.rs`) orchestrates the full CDC pipeline:

1. Acquire the catalog lock and read the snapshot diff (`snapshot_diff(start, end)`).
2. Release the catalog lock **before** any async I/O to avoid holding the mutex during Parquet scans.
3. For each file in `diff.added_data_files`: call `extract_rows_from_parquet()` and accumulate `added_rows`.
4. For each file in `diff.retired_data_files`: call `extract_rows_from_parquet()` and accumulate `removed_rows`.
5. Call `compute_table_changes()` to classify inserts, deletes, and updates.
6. Return the change records as a `QueryResponse` with columns `rowid`, `change_type`, `columns_json`.

Row IDs within each file list are allocated sequentially, starting at 0 for `added_data_files` and 0 for `retired_data_files` independently, with each file's `base_rowid` incremented by the actual scanned row count of the previous file (not the catalog `record_count`).
