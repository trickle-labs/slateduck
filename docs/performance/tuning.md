# Performance Tuning

This page covers the configuration options and operational practices that improve SlateDuck's performance for specific workloads. The guidance is ordered by impact — the first item provides the largest performance improvement for the least effort, and subsequent items provide diminishing returns. For most deployments, applying the first two or three recommendations is sufficient. Going further is for environments where every millisecond matters.

Performance tuning in SlateDuck is different from tuning a traditional database. There are no query plans to optimize, no indexes to create, no buffer pool sizes to calculate. The primary knobs are: where data lives (storage backend), how much is cached (block cache size), how clean the data is (garbage collection), and how work is organized (write batching). These are operational decisions, not code changes.

## Storage Backend Selection

The single highest-impact performance decision is your choice of object storage tier. This decision affects every read and write operation and cannot be overcome by any other optimization. Choosing the right backend is worth 5–10x in latency for cold operations and 5–10x in write throughput.

| Backend | Read Latency (p50) | Write Latency (p50) | Monthly Cost (100GB) | Best For |
|---------|-------------------|--------------------|--------------------|----------|
| Local SSD | < 1ms | < 1ms | Hardware cost | Development, testing, edge |
| S3 Express One Zone | 3–10ms | 3–10ms | ~$300 | Latency-sensitive production |
| S3 Standard | 20–100ms | 50–150ms | ~$23 | Cost-optimized production |
| GCS Standard | 10–50ms | 30–80ms | ~$26 | GCP deployments |
| Azure Blob (Hot) | 15–60ms | 30–100ms | ~$21 | Azure deployments |
| MinIO (local network) | 1–5ms | 1–5ms | Self-hosted | Air-gapped, sovereignty |

### Choosing Your Backend

**Development and testing:** Use local filesystem. There is no reason to introduce network latency during development. Tests that run against local filesystem complete in seconds; the same tests against S3 Standard take minutes.

```bash
# Local development
slateduck serve --catalog ./local-catalog/
```

**Cost-optimized production:** Use S3 Standard (or GCS/Azure equivalent). The 50–150ms write latency and 20–100ms cold-read latency is acceptable for most catalog workloads. The cost is minimal. This is the correct default for new deployments.

```bash
# Production with S3 Standard
slateduck serve --catalog s3://your-bucket/catalogs/production/
```

**Latency-sensitive production:** Use S3 Express One Zone. This reduces all storage-bound operations by 5–10x. The cost is approximately 10x higher than S3 Standard, but for a metadata catalog (typically under 1GB), the absolute cost difference is small ($23/month vs. $300/month for 100GB — and most catalogs are 10–100MB, making the cost difference trivial).

```bash
# Low-latency production with S3 Express
slateduck serve --catalog s3express://your-bucket--use1-az4--x-s3/catalogs/production/
```

**Air-gapped or sovereignty-constrained:** Use MinIO (S3-compatible) deployed within your controlled environment. Latency depends on network distance but is typically 1–5ms for same-datacenter deployments.

### Migration Between Backends

You can migrate a catalog between storage backends by copying the SlateDB directory tree. The catalog format is backend-agnostic — it is just files and directories.

```bash
# Copy from S3 Standard to S3 Express
aws s3 sync s3://standard-bucket/catalog/ s3express://express-bucket--use1-az4--x-s3/catalog/
```

After migration, restart SlateDuck pointing to the new location. No catalog modification is needed.

## Cache Sizing

SlateDB's block cache keeps recently-accessed SST file blocks in memory. A larger cache means more operations are served directly from memory without any object storage I/O. For catalogs that fit entirely in cache, steady-state performance is effectively "in-memory database" speed (sub-millisecond reads).

### How the Cache Works

SlateDB stores data in Sorted String Table (SST) files. Each SST file is divided into blocks (typically 4–16KB). When a read operation needs data from an SST block that is not in cache, it fetches the block from object storage and stores it in the cache for future use. The cache uses an LRU (Least Recently Used) eviction policy — when the cache is full, the least recently accessed block is evicted.

### Sizing Guidelines

The optimal cache size depends on your catalog's total size and access patterns:

| Catalog Size (Rows) | Approximate Storage Size | Recommended Cache |
|---------------------|------------------------|-------------------|
| 1,000 rows | 1–5MB | 16MB (fits entirely) |
| 10,000 rows | 10–50MB | 64MB (fits entirely) |
| 100,000 rows | 100–500MB | 256MB (working set) |
| 1,000,000 rows | 1–5GB | 1GB (working set) |

**Rule of thumb:** If your catalog fits in cache with 50% headroom, set the cache to that size. If not, size the cache to hold your "working set" — the tables, schemas, and data files accessed in a typical 5-minute window.

### Configuration

```bash
# Set cache size to 256MB
SLATEDUCK_CACHE_SIZE_MB=256 slateduck serve --catalog s3://bucket/catalog/

# Verify cache effectiveness via metrics
# Look at cache_hit_ratio — should be > 0.9 in steady state
```

### Cache Cold Start

When SlateDuck starts, the cache is empty. The first operations (typically 10–50 depending on catalog size and access pattern) will be slow because they trigger block fetches. After warm-up, the cache contains the active working set and subsequent operations are fast.

**Strategies to reduce cold-start impact:**

1. **Sticky scheduling:** Deploy SlateDuck on the same node across restarts (preserves OS page cache, though not SlateDB's block cache)
2. **Warm-up queries:** After startup, issue a few representative queries to prime the cache before directing production traffic
3. **Large cache:** A cache that holds the entire catalog means warm-up completes after one full scan of the catalog (a few seconds)

## Garbage Collection

Garbage collection (GC) is the operational practice with the largest impact on scan performance. When versions accumulate (historical schema changes, dropped tables, file deregistrations), prefix scans read more data than necessary because they encounter invisible rows that the MVCC filter discards.

### The Scan Amplification Problem

Consider a table with 50 columns that has been altered 10 times (adding columns, modifying types). Without GC:

- Total column rows in storage: 50 original + (10 alterations × ~5 affected columns) = ~100 rows
- Visible column rows at current snapshot: 55 (50 original + 5 added)
- Scan amplification: 100 / 55 = 1.8x

This seems modest, but for tables with heavy churn:

- Table altered 100 times: amplification could reach 10x or higher
- Table with 10,000 files, 500 files rotated: amplification reaches 1.05x (manageable)
- Schema with 50 tables, 20 dropped: amplification for schema scan is 2.5x

### Measuring Amplification

Use the inspect tool to measure current amplification:

```bash
slateduck inspect --catalog s3://bucket/catalog/

# Output includes:
# Total rows: 45,000
# Live entities: 12,000
# Amplification factor: 3.75x
# Recommendation: GC recommended (factor > 3.0)
```

### When to Run GC

| Amplification Factor | Action | Priority |
|---------------------|--------|----------|
| < 2x | No action needed | — |
| 2–3x | Schedule GC at convenience | Low |
| 3–5x | Run GC within a day | Medium |
| 5–10x | Run GC now | High |
| > 10x | Run GC immediately, investigate cause | Critical |

### GC + Excision for Maximum Effect

GC marks old versions as reclaimable by advancing `retain_from`. Excision (compaction) physically removes those rows from SST files. For maximum scan performance improvement, run both:

```bash
# Step 1: Mark old versions as reclaimable
slateduck gc --catalog s3://bucket/catalog/ --retain-snapshots 10

# Step 2: Physically remove dead rows (reduce SST file sizes)
slateduck excise --catalog s3://bucket/catalog/
```

After excision, scan amplification drops to 1.0x (only visible rows remain in storage).

## Write Batching

SlateDuck's write performance is determined by the number of object storage PUTs, not the number of rows written. A single PUT can carry thousands of key-value pairs. The optimization strategy is to group as many writes as possible into each transaction.

### How Batching Works

Within a transaction (between `BEGIN` and `COMMIT`), all writes are accumulated in memory. At `COMMIT`, they are written to object storage as a single atomic batch. The cost of that PUT is approximately fixed regardless of batch size (up to ~1MB payload).

| Approach | Transactions | S3 PUTs | Total Time (S3 Standard) |
|----------|-------------|---------|--------------------------|
| 100 separate INSERTs (autocommit) | 100 | 100 | ~8 seconds |
| 100 INSERTs in 1 transaction | 1 | 1 | ~85ms |
| 1,000 INSERTs in 1 transaction | 1 | 1 | ~130ms |

The difference is dramatic: 100 autocommitted writes take 100x longer than the same 100 writes batched into one transaction.

### DuckDB's Natural Batching

DuckDB's `ducklake` extension already batches naturally. When you execute:

```sql
-- In DuckDB:
INSERT INTO my_table SELECT * FROM read_parquet('s3://data/*.parquet');
```

DuckDB writes all the resulting Parquet files, then registers them all in the catalog in a single transaction. You do not need to manually batch — DuckDB does it for you.

Manual batching is only relevant when writing to SlateDuck directly (via `psql` or custom clients) outside of DuckDB's extension.

## Network Optimization

Network latency is the dominant factor for cache-hot operations (where storage I/O is avoided). Reducing network distance between DuckDB and SlateDuck provides consistent latency reduction on every operation.

### Co-Location Strategies

| Strategy | Network Latency | Effort |
|----------|----------------|--------|
| Same machine (localhost) | 0.05–0.2ms | Low (deploy together) |
| Same Kubernetes pod (sidecar) | 0.1–0.3ms | Low (pod spec change) |
| Same availability zone | 0.5–2ms | Low (AZ selection) |
| Same region, different AZ | 2–5ms | Default for HA |
| Cross-region | 20–100ms | Avoid if possible |

**Recommendation:** Deploy SlateDuck in the same availability zone as your DuckDB instances. If DuckDB runs in multiple AZs, deploy SlateDuck readers in each AZ with a single writer in one AZ.

### VPC Endpoints

When SlateDuck accesses S3, the traffic can go through the public internet or through a VPC endpoint. VPC endpoints provide:

- Lower latency (private network path, no internet gateway)
- Higher bandwidth (not subject to internet routing congestion)
- No data transfer charges (free within the same region)

```bash
# Ensure VPC endpoint for S3 is configured in your VPC
# SlateDuck uses it automatically (no configuration needed)
# Verify with: aws s3 ls --debug 2>&1 | grep endpoint
```

### Native Extension (Strategy C)

For maximum performance, eliminate the network entirely by using SlateDuck as a native DuckDB extension. The extension embeds the catalog logic directly in DuckDB's process — there is no TCP connection, no protocol serialization, no network latency.

Performance with the native extension:

| Operation | Network (same AZ) | Native Extension | Improvement |
|-----------|-------------------|-----------------|-------------|
| Point read (cache hot) | 1.5ms | 0.3ms | 5x |
| Scan 50 keys (cache hot) | 5ms | 1.2ms | 4x |
| Write (S3 Standard) | 83ms | 81ms | 1.02x (negligible) |

The native extension provides significant improvement for reads (eliminating 1–2ms of network round-trip) but negligible improvement for writes (dominated by S3 PUT latency, not network).

## Compaction Tuning

SlateDB periodically compacts SST files — merging small files into larger ones and removing tombstones (deleted key-value pairs). Compaction affects performance in two ways:

1. **Fewer SST files = fewer potential block fetches per scan** (positive)
2. **Compaction uses I/O bandwidth, potentially competing with reads** (negative during compaction)

### Default Behavior

SlateDB's compaction runs in the background with conservative settings. For most SlateDuck deployments, the defaults are appropriate.

### When to Tune Compaction

Tune compaction settings if:

- **Write-heavy workload:** If you register thousands of files per hour, compaction may fall behind, resulting in many small SST files and degraded scan performance. Increase compaction frequency.
- **Cost-sensitive deployment:** Compaction generates PUT requests to object storage (rewriting files). If you are optimizing for storage cost, reduce compaction frequency at the expense of read performance.

## Monitoring for Performance

The key metrics to watch for performance health:

| Metric | Healthy Range | Action if Outside |
|--------|-------------|-------------------|
| Cache hit ratio | > 0.9 | Increase cache size |
| Scan amplification | < 3x | Run GC |
| Write batch size (avg) | > 10 keys | Review batching strategy |
| p99 read latency | < 100ms (S3) | Check S3 health, cache |
| p99 write latency | < 200ms (S3) | Check S3 health |

## Performance Checklist

For a new deployment, walk through this checklist:

- [ ] Storage backend appropriate for latency requirements?
- [ ] Cache sized to hold working set (or full catalog if small)?
- [ ] DuckDB and SlateDuck in same availability zone?
- [ ] VPC endpoint configured for object storage access?
- [ ] GC scheduled to run periodically (daily or weekly)?
- [ ] Monitoring alerts on cache hit ratio and latency percentiles?

## Further Reading

- **[Latency Model](latency-model.md)** — Understanding the request path
- **[Benchmarks](benchmarks.md)** — Quantified impact of these optimizations
- **[Operations: Garbage Collection](../operations/garbage-collection.md)** — GC operational procedures
- **[Operations: Monitoring](../operations/monitoring.md)** — Setting up performance monitoring
