# Inlined Data

For very small data files — those below a configurable size threshold — SlateDuck stores the file contents directly in the catalog rather than as separate objects in storage. This optimization eliminates the overhead of a separate object storage GET for data that is literally smaller than the HTTP headers of the request to fetch it. This page documents the motivation, implementation, trade-offs, and configuration of data inlining.

Data inlining is an optimization, not a fundamental architectural feature. Disabling it changes nothing about correctness — only about performance for workloads that produce many tiny files. But for those workloads (single-row inserts, real-time event tables, slowly-changing dimension updates), the performance improvement is substantial: one fewer network round-trip per tiny file.

## Motivation

Consider what happens when DuckDB queries a table that has had many single-row INSERT statements:

Without inlining:
1. DuckDB asks SlateDuck for the table's data files
2. SlateDuck returns a list of 500 file paths: `s3://bucket/data/file-001.parquet`, `file-002.parquet`, ..., `file-500.parquet`
3. DuckDB fetches each file from S3 — 500 GET requests
4. Each file is 1–2 KB (a Parquet file with a single row)
5. Total: 500 × 30–100ms = 15–50 seconds just for file fetches
6. The actual data (500 rows) would fit in a single 50 KB file

The problem is economic: each S3 GET has a fixed overhead (TCP connection, TLS handshake, HTTP request/response, S3 authentication) regardless of whether you are fetching 1 KB or 100 MB. For a 1 KB file, the overhead vastly exceeds the useful work.

With inlining:
1. DuckDB asks SlateDuck for the table's data files
2. SlateDuck returns a mix: some external file references AND some inline data (the tiny files' contents embedded in the catalog response)
3. DuckDB reads the inline data directly from the catalog response — zero additional round-trips
4. Only the larger files require separate S3 GETs

**The savings:** For 500 tiny files that are inlined, you eliminate 500 × 30–100ms = 15–50 seconds of network overhead. The inline data adds only a few KB to the catalog response (which was already being fetched).

## How It Works

### Write Path

When DuckDB registers a data file through the DuckLake protocol, SlateDuck checks the file size reported in the registration metadata:

1. DuckDB sends: `INSERT INTO ducklake_data_file (table_id, file_path, file_size, ...) VALUES (5, 's3://bucket/tiny.parquet', 1500, ...)`
2. SlateDuck checks: `file_size < inline_threshold`
3. If below threshold: SlateDuck fetches the file contents from S3 and stores them as an `inlined_insert` entry (tag 0xFD) in the catalog
4. If above threshold: SlateDuck stores only the file metadata (path, size, stats) — standard behavior
5. In both cases, the `ducklake_data_file` metadata entry is also created (the inline entry is supplementary)

The key format for inlined data:

```
Tag: 0xFD
Key: 0xFD | table_id (u64) | file_id (u64) | begin_snapshot (u64)
Value: SDKV envelope containing raw file bytes (typically Parquet format)
```

### Read Path

When DuckDB requests data files for a table, SlateDuck's response includes:

1. Standard file metadata (all `ducklake_data_file` entries for the table)
2. For any file that has a corresponding inlined entry, the inline flag is set and the raw bytes are included in the response

DuckDB handles this transparently through the DuckLake protocol. When it sees an inlined file, it reads the Parquet bytes directly from the catalog response instead of issuing a separate GET to object storage.

### Storage Layout

Inlined data keys sort after all standard DuckLake tables (tag 0xFD is in the internal range) but before counters and system keys:

```
... [0x07 data_file entries] ... [0x09 file_column_stats] ...
... [0xFD inlined data] ...
... [0xFE counters] [0xFF system keys]
```

This separation means a scan for data file metadata (tag 0x07) never accidentally encounters inlined data bytes. The inline entries are only accessed when explicitly requested.

## Size Threshold

The inline threshold determines which files are inlined and which are left as external references:

| Threshold | Effect | Catalog Growth |
|-----------|--------|---------------|
| 0 bytes | Inlining disabled | None |
| 1 KB | Only trivially small files | Minimal |
| 4 KB (default) | Single-row inserts, small batches | Moderate |
| 16 KB | Small-to-medium Parquet files | Significant |
| 64 KB | Most Parquet files with < 100 rows | Large |

### Default: 4 KB

The default threshold of 4 KB was chosen based on:

- A single-row Parquet file is typically 1–3 KB (file header + row group header + column data + footer)
- A 5-row Parquet file is typically 2–5 KB
- An S3 GET request has ~30ms overhead regardless of size
- A 4 KB inline entry adds ~4 KB to the catalog response (negligible compared to typical response sizes)

At 4 KB, you capture essentially all "single-row insert" patterns without bloating the catalog with larger files that are more efficiently served directly from S3.

### Configuration

```bash
# Set inline threshold (bytes)
SLATEDUCK_INLINE_THRESHOLD_BYTES=4096 slateduck serve --catalog s3://bucket/catalog/

# Disable inlining entirely
SLATEDUCK_INLINE_THRESHOLD_BYTES=0 slateduck serve --catalog s3://bucket/catalog/

# Aggressive inlining (for workloads with many small files)
SLATEDUCK_INLINE_THRESHOLD_BYTES=16384 slateduck serve --catalog s3://bucket/catalog/
```

## Trade-offs

### Advantages

**Eliminates network round-trips for tiny files.** The primary benefit. For tables with hundreds of tiny files (common after many individual inserts), this reduces query latency from "N × S3 GET latency" to "0 additional round-trips."

**Reduces total object count in storage.** Fewer tiny objects in S3 means simpler bucket management, faster `aws s3 ls` operations, and potentially lower costs (some storage providers charge per object).

**Improves query predictability.** Without inlining, query latency depends on how many tiny files a table has. With inlining, the catalog response contains all the data — latency is predictable regardless of file count.

**Simplifies data lifecycle.** Inlined data is garbage-collected with its catalog entry. No orphaned tiny files in S3 to clean up separately.

### Disadvantages

**Increases catalog size.** Every inlined file adds its full content to the catalog. A table with 10,000 inlined files at 3 KB each adds 30 MB to the catalog. This increases SlateDB's storage requirements and potentially affects cache efficiency.

**Heavier catalog scans.** When scanning data file metadata, the response includes inline data bytes. If you are only interested in file statistics (for partition pruning) but the response also contains inline data, you are transferring more bytes than needed.

**Complicates garbage collection.** Inlined data entries must be cleaned up when the parent data file is garbage collected. This adds a coordination step to GC (check for corresponding inline entries when removing file metadata).

**Not useful for large files.** Files above the threshold are not inlined (by design). The optimization only helps workloads that produce many tiny files.

### When to Disable Inlining

Consider disabling inlining (`SLATEDUCK_INLINE_THRESHOLD_BYTES=0`) when:

- Your ETL pipeline always produces large Parquet files (> 10 MB) — inlining never triggers anyway
- You are optimizing for minimal catalog size (small cache, cost-sensitive storage)
- You are debugging catalog behavior and want to simplify the data path

### When to Increase the Threshold

Consider increasing the threshold when:

- Your workload produces many files in the 4–16 KB range (small batch inserts)
- Query latency is dominated by S3 GETs for these small files
- Catalog size is not a concern (you have ample cache and storage budget)

## Interaction with Compaction

When DuckDB compacts small files into larger files (merging many 1 KB files into one 50 MB file), the compaction process:

1. Registers the new large file in the catalog
2. Deregisters the old small files (marks them with end_snapshot)
3. After GC, the old file metadata AND inline entries are removed

This means inlined data naturally goes away when files are compacted — the optimization is self-cleaning over time.

## Interaction with Statistics

Inlined files still have `ducklake_file_column_stats` entries (min/max values, null counts). DuckDB uses these statistics for partition pruning regardless of whether the file is inlined or external. The statistics are stored separately from the inline data (different tag: 0x09 vs. 0xFD).

## Implementation Details

### Fetch-on-Write

When SlateDuck decides to inline a file, it must fetch the file contents from object storage during the write transaction. This adds one S3 GET to the write path:

```
Write path without inlining: 1 WAL PUT
Write path with inlining:    1 S3 GET (fetch file) + 1 WAL PUT
```

The additional GET adds 30–100ms to write latency for inlined files. This is acceptable because:
- Write operations already take 50–150ms (dominated by the WAL PUT)
- The inline benefit (saved GETs during reads) is realized many times per file (every query that scans the table)
- The ratio of reads to writes for catalog data is typically 100:1 or higher

### Atomic Guarantee

The inline entry is written in the same write batch as the file metadata entry. Both are committed atomically. There is no state where a file has metadata but no inline data (or vice versa). If the transaction fails, neither entry exists.

### Size Validation

SlateDuck validates that the actual file size matches the declared size in the metadata. If the file's actual size exceeds the threshold (even though the declared size was below), the file is not inlined and only the metadata entry is created. This prevents pathological cases where incorrect metadata causes unexpectedly large inline entries.

## Metrics

Monitor these metrics to understand inlining behavior:

| Metric | Meaning | Healthy Range |
|--------|---------|---------------|
| inline_files_total | Total inlined files in catalog | Depends on workload |
| inline_bytes_total | Total bytes stored as inline data | < 10% of catalog size |
| inline_ratio | Fraction of files that are inlined | 0–0.5 (workload-dependent) |
| inline_threshold | Current threshold setting | As configured |

## Further Reading

- **[Tag Allocation](tag-allocation.md)** — Tag 0xFD allocation and key format
- **[Performance: Tuning](../performance/tuning.md)** — Overall performance optimization
- **[Operations: Garbage Collection](../operations/garbage-collection.md)** — How inline entries are cleaned up
- **[Architecture: Value Encoding](../architecture/value-encoding.md)** — SDKV envelope format for inline data
