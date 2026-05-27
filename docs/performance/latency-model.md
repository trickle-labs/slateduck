# Latency Model

Understanding Rocklake's latency requires understanding the complete request path — every layer that a catalog operation passes through, what each layer contributes to total latency, and which layers dominate under different conditions. This page provides a detailed breakdown that will help you predict performance for your deployment configuration and identify optimization opportunities.

The key insight is that Rocklake's latency is not one number. It is a sum of components, and the dominant component changes depending on cache state, storage backend, network topology, and catalog size. A cache-hot read in a co-located deployment takes under 2ms. A cache-cold read from S3 Standard across availability zones takes 80ms. Both are "Rocklake latency" — but they represent fundamentally different scenarios.

## The Request Path

A catalog read operation passes through these layers, in order:

```
DuckDB ─→ Network ─→ PG-Wire Protocol ─→ SQL Classifier ─→ Catalog Logic ─→ SlateDB ─→ Object Storage
                                                                                ↑
                                                                          Block Cache
```

Each layer adds time. Some layers are constant (fractions of a millisecond regardless of conditions), some are variable (depend on cache state and network), and some are dominant (contribute 90%+ of total latency under certain conditions).

### Layer 1: Network Transport

The first layer is the network between DuckDB and Rocklake. DuckDB connects to Rocklake via TCP using the PostgreSQL wire protocol.

| Configuration | Typical Latency | Variance |
|--------------|----------------|----------|
| localhost (same machine) | 0.05–0.2ms | Very low |
| Same availability zone | 0.5–2ms | Low |
| Cross availability zone | 1–5ms | Moderate |
| Cross region | 20–100ms | High |
| Native extension (in-process) | 0 | Zero |

**When this dominates:** Cross-region deployments. If DuckDB is in `us-east-1` and Rocklake is in `eu-west-1`, every catalog operation pays 60–100ms of network latency regardless of whether the data is cached.

**Optimization:** Co-locate DuckDB and Rocklake in the same availability zone. For maximum performance, use the native extension (Strategy C) which eliminates network transport entirely by embedding Rocklake in DuckDB's process.

### Layer 2: PostgreSQL Wire Protocol

Rocklake speaks the PostgreSQL wire protocol (pgwire). Incoming bytes are parsed into protocol messages: Query, Parse, Bind, Execute, Describe, Sync. Outgoing results are serialized into RowDescription, DataRow, CommandComplete, ReadyForQuery messages.

| Operation | Typical Latency |
|-----------|----------------|
| Parse incoming message | 0.01–0.05ms |
| Serialize response | 0.01–0.1ms |
| Total protocol overhead | 0.02–0.15ms |

**When this dominates:** Never. Protocol parsing is memory-only computation with zero I/O. It contributes negligible latency.

**Note:** The protocol overhead per message is constant regardless of payload size. A response with 5 rows and a response with 500 rows have the same parsing overhead (the serialization time scales linearly with row count, but even 500 rows serialize in under 1ms).

### Layer 3: SQL Classification

Rocklake does not use a general SQL parser. It uses a pattern matcher (the SQL classifier) that recognizes the ~50 known DuckLake statement patterns and dispatches to the appropriate handler. This is implemented as a series of string comparisons and regex matches.

| Operation | Typical Latency |
|-----------|----------------|
| Statement classification | 0.005–0.02ms |

**When this dominates:** Never. Classification is a pure CPU operation on a short string (typical SQL statements are under 500 bytes).

### Layer 4: Catalog Logic

The catalog layer translates the classified SQL operation into key-value operations. For a `SELECT * FROM ducklake_columns WHERE table_id = 42` query, the catalog logic determines the key prefix (`tag=0x06, table_id=42`), constructs the scan range, applies MVCC filtering to the results, and formats the response.

| Operation | Typical Latency |
|-----------|----------------|
| Key construction | < 0.01ms |
| MVCC filter (per row) | < 0.001ms |
| Result formatting | 0.01–0.1ms |
| Total catalog logic | 0.02–0.2ms |

**When this dominates:** Never in isolation. However, MVCC filtering becomes relevant when scan amplification is high. If a prefix scan returns 1,000 raw rows but only 100 are visible at the current snapshot, the catalog logic processes 10x more data than strictly necessary. The MVCC filtering itself is cheap (a comparison of snapshot IDs), but the I/O to read those 1,000 rows may not be.

### Layer 5: SlateDB Read Path

SlateDB is an LSM-tree key-value store. The read path for a single key GET:

1. Check the memtable (in-memory write buffer) — microseconds
2. Check the block cache (in-memory SST block cache) — microseconds
3. Check bloom filters (if applicable) — microseconds
4. Fetch the relevant SST block from object storage — milliseconds to hundreds of milliseconds

For a prefix scan:

1. Create an iterator that merges memtable + cached SST blocks + uncached SST blocks
2. Advance the iterator, fetching SST blocks on demand
3. Each block fetch is one object storage GET

| Scenario | Typical Latency |
|----------|----------------|
| Key in memtable | < 0.01ms |
| Key in block cache | 0.1–0.5ms |
| Key requires 1 SST block fetch (S3 Standard) | 20–100ms |
| Key requires 1 SST block fetch (S3 Express) | 3–10ms |
| Key requires 1 SST block fetch (GCS) | 10–50ms |
| Key requires 1 SST block fetch (local SSD) | 0.1–1ms |

**When this dominates:** Whenever the block cache does not contain the required data. For a freshly started Rocklake instance (cold cache), every operation pays the full object storage round-trip. After warm-up, the frequently accessed blocks stay in cache and this layer contributes minimal latency.

### Layer 6: Object Storage

The final layer — where bytes are fetched from or written to S3, GCS, Azure Blob Storage, or local filesystem.

| Provider | GET Latency (p50) | GET Latency (p99) | PUT Latency (p50) | PUT Latency (p99) |
|----------|-------------------|-------------------|-------------------|-------------------|
| S3 Standard | 30ms | 100ms | 80ms | 200ms |
| S3 Express One Zone | 4ms | 12ms | 5ms | 15ms |
| GCS Standard | 20ms | 60ms | 40ms | 100ms |
| Azure Blob | 25ms | 80ms | 50ms | 120ms |
| MinIO (local) | 2ms | 8ms | 3ms | 10ms |
| Local filesystem (SSD) | 0.1ms | 0.5ms | 0.2ms | 1ms |

**When this dominates:** On every cache miss (reads) and every write operation. For writes, this layer always dominates because writes go directly to storage (they cannot be served from cache).

## Dominant Factors by Scenario

### Steady-State Production (Warm Cache)

In a running production system where the catalog is accessed regularly, the block cache contains all frequently-read SST blocks. The dominant latency factor is **network transport** (Layer 1).

Typical end-to-end: **1–5ms** (same AZ deployment)

### Cold Start / First Access

Immediately after Rocklake starts (or after accessing a rarely-used catalog section), the block cache is empty. The dominant factor is **object storage GET latency** (Layer 6).

Typical end-to-end: **30–100ms** (S3 Standard)

### Write Operations

Write latency is always dominated by **object storage PUT latency** (Layer 6) because writes go to the WAL which is flushed to object storage. Cache state is irrelevant.

Typical end-to-end: **50–150ms** (S3 Standard), **3–10ms** (S3 Express)

### Large Scans (Many Keys)

Operations that scan many keys (listing all files for a table with 10,000 files) may require multiple SST block fetches. The dominant factor is **number of block fetches × per-fetch latency**.

If 10,000 keys span 5 SST blocks, and 3 are cached, the scan requires 2 block fetches. On S3 Standard, that adds 60–200ms. On S3 Express, 6–20ms.

## Scan Amplification

Scan amplification occurs when the prefix scan reads rows that are not visible at the current snapshot. This happens when:

1. **Historical versions accumulate.** A table that has been altered 20 times has 20 versions in the key space. A scan for the current version reads all 20 and discards 19.
2. **Deleted entities persist.** A dropped table's rows still exist (with `end_snapshot` set) until GC removes them.
3. **GC has not run.** Without garbage collection, the ratio of total rows to visible rows grows over time.

**Quantifying the impact:**

| Scenario | Total Rows Scanned | Visible Rows | Amplification Factor |
|----------|-------------------|-------------|---------------------|
| Fresh catalog, no modifications | 100 | 100 | 1.0x |
| After 5 schema migrations | 100 | 20 | 5.0x |
| After 50 schema migrations, no GC | 500 | 10 | 50x |
| After GC (retain_from = current) | 10 | 10 | 1.0x |

At 1.0x amplification, scan performance is optimal. At 50x amplification, you are reading 50 rows for every visible row — 50x the I/O and 50x the cache pressure. The fix is running garbage collection followed by excision (compaction) to physically remove the dead rows.

**Rule of thumb:** If the amplification factor exceeds 3x, schedule GC. If it exceeds 10x, GC is overdue.

## Hot Key Optimization

Rocklake implements a "hot key" optimization for the most frequently accessed system key. This key contains high-level catalog metadata that DuckDB reads on every connection (or every query, depending on caching configuration).

The hot key is:

- Cached in Rocklake's memory (not in SlateDB's block cache — in application memory)
- Refreshed on every write transaction (the writer updates the hot key as part of the commit)
- Served with zero I/O — it is a memory read, contributing < 0.01ms to response time

This optimization eliminates the most common cache miss scenario: DuckDB connecting for the first time and reading the catalog header.

## Write Latency Breakdown

Write operations have a different latency profile than reads:

```
SQL Parse → Catalog Logic → Write Batch Construction → SlateDB Commit → WAL Flush → Response
                                                                            ↓
                                                                   Object Storage PUT
```

| Phase | Typical Latency | Notes |
|-------|----------------|-------|
| SQL parse + classify | 0.01ms | Constant |
| Catalog logic (key/value construction) | 0.1–1ms | Scales with batch size |
| Write batch assembly | 0.01–0.1ms | Memory-only |
| SlateDB commit (memtable insert) | 0.01ms | Memory-only |
| WAL flush to object storage | 50–150ms (S3) | **Dominates** |
| Response serialization | 0.01ms | Constant |

The critical insight: **write latency is independent of batch size** (up to the point where the batch is large enough to require multiple S3 PUTs, which is hundreds of MB). A transaction registering 1 file and a transaction registering 1,000 files have essentially the same latency — one object storage PUT.

This is why write batching is Rocklake's primary write optimization. Instead of "make writes faster," the strategy is "do more work per write."

## End-to-End Examples

### Example 1: DuckDB Lists Tables (Warm Cache)

```
Operation: SELECT * FROM ducklake_tables WHERE schema_id = 1
Deployment: DuckDB and Rocklake in same AZ (us-east-1a)
Cache state: Warm (tables block cached)
```

| Layer | Time |
|-------|------|
| Network (TCP in same AZ) | 0.8ms |
| PG-wire parse | 0.03ms |
| SQL classification | 0.01ms |
| Catalog logic (construct prefix, MVCC filter) | 0.1ms |
| SlateDB prefix scan (block cache hit, 15 keys) | 0.3ms |
| Response serialization (5 visible rows) | 0.05ms |
| Network (response) | 0.7ms |
| **Total** | **~2ms** |

### Example 2: ETL Registers 100 Files (S3 Standard)

```
Operation: BEGIN; INSERT INTO ducklake_data_files ...; (×100); COMMIT;
Deployment: Rocklake on EC2 in us-east-1, S3 Standard
```

| Layer | Time |
|-------|------|
| Network (×102 messages: BEGIN, 100 INSERTs, COMMIT) | 2ms |
| SQL classification (×102) | 0.02ms |
| Catalog logic (construct 100 key-value pairs) | 0.5ms |
| Write batch assembly (100 pairs + snapshot) | 0.1ms |
| SlateDB WAL flush (1 S3 PUT, ~50KB) | 85ms |
| Response serialization | 0.05ms |
| **Total** | **~88ms** |

### Example 3: First Query After Cold Start (S3 Standard)

```
Operation: SELECT * FROM ducklake_columns WHERE table_id = 5
Deployment: Fresh start, empty block cache
```

| Layer | Time |
|-------|------|
| Network | 1ms |
| PG-wire + SQL classification | 0.05ms |
| Catalog logic | 0.1ms |
| SlateDB scan: 1 SST block fetch from S3 | 55ms |
| MVCC filter (30 raw rows → 12 visible columns) | 0.01ms |
| Response serialization (12 rows) | 0.05ms |
| Network (response) | 1ms |
| **Total** | **~57ms** |

After this first query, the SST block is cached. A repeat of the same query takes ~2ms.

## Latency Reduction Strategies

In order of impact:

| Strategy | Reduction | Effort |
|----------|-----------|--------|
| Switch to S3 Express One Zone | 5–10x on cold reads, 5–10x on writes | Low (config change) |
| Increase block cache size | Eliminates cold reads (if catalog fits) | Low (config change) |
| Co-locate in same AZ | 2–5x on network layer | Medium (deployment change) |
| Use native extension (Strategy C) | Eliminates network entirely | Medium (build change) |
| Run GC regularly | Reduces scan amplification 2–50x | Low (operational practice) |
| Use local MinIO for dev | 10–50x on all I/O | Medium (infrastructure) |

The strategies are additive. Applying all of them transforms latency from "80ms cold reads" to "sub-millisecond for everything."

## Further Reading

- **[Benchmarks](benchmarks.md)** — Measured numbers on standardized hardware
- **[Tuning](tuning.md)** — How to apply these optimizations in practice
- **[Architecture: Key Layout](../architecture/key-layout.md)** — Why key structure affects scan performance
