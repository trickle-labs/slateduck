# Benchmarks

Performance claims without reproducible measurements are marketing. This page documents Rocklake's benchmarking methodology, presents baseline results from controlled environments, explains how to reproduce benchmarks in your own infrastructure, and provides guidance on interpreting results. Every number on this page comes from automated benchmark suites that you can run yourself.

The goal of benchmarking is not to produce impressive numbers for a slide deck. The goal is to characterize the system's behavior so that operators can predict performance for their specific workload and deployment configuration. Accordingly, this page includes both best-case numbers (warm cache, co-located, S3 Express) and realistic numbers (mixed cache state, S3 Standard, typical production conditions).

## Methodology

### Framework

Benchmarks use the `criterion` framework for Rust, which provides:

- **Statistical rigor:** Each measurement is repeated until the confidence interval narrows to 5% or 100 iterations complete (whichever comes first)
- **Warm-up iterations:** The first N iterations are discarded to allow cache warming and JIT stabilization
- **Outlier detection:** Measurements that fall outside 3 standard deviations are flagged and excluded from percentile calculations
- **Regression detection:** Results are compared against previous runs to detect performance regressions

### Test Environment

Baseline results on this page were produced on:

| Parameter | Value |
|-----------|-------|
| Instance type | AWS EC2 c5.xlarge (4 vCPUs, 8GB RAM) |
| Region | us-east-1 |
| Operating system | Amazon Linux 2023 |
| Storage backend | S3 Standard (unless noted otherwise) |
| Network | Same-AZ deployment (DuckDB and Rocklake on same instance) |
| Rocklake version | 0.7.x |
| SlateDB cache size | 64MB (default) |
| Catalog size | 100 tables, 50 columns each, 10,000 total data files |

### Catalog Setup

Each benchmark run creates a fresh catalog with deterministic content:

- 1 database, 5 schemas
- 20 tables per schema (100 total)
- 50 columns per table (5,000 total column rows)
- 100 data files per table (10,000 total file registration rows)
- 10 historical snapshots (some tables have been altered, creating version history)

This represents a moderately-sized analytics catalog — larger than a startup's first data warehouse, smaller than a Fortune 500 enterprise catalog.

## Baseline Results: Point Operations

Point operations access a single entity by its key (or a small number of keys).

### Cache-Hot Point Reads

After the benchmark warms up (first 100 iterations), all relevant SST blocks are in the block cache. These numbers represent steady-state performance for frequently-accessed data.

| Operation | p50 | p95 | p99 | Throughput |
|-----------|-----|-----|-----|-----------|
| Read schema by ID | 0.8ms | 1.2ms | 2.1ms | 1,200 ops/s |
| Read table metadata by ID | 1.1ms | 1.8ms | 3.2ms | 900 ops/s |
| Read single column | 0.9ms | 1.5ms | 2.8ms | 1,100 ops/s |
| Read data file by ID | 1.0ms | 1.6ms | 2.9ms | 1,000 ops/s |
| Read system key (hot key) | 0.4ms | 0.7ms | 1.1ms | 2,500 ops/s |
| Check catalog version | 0.3ms | 0.5ms | 0.8ms | 3,300 ops/s |

The hot key (system metadata) is served from application-level cache, bypassing SlateDB entirely. This makes it roughly 2–3x faster than regular cache-hit reads.

### Cache-Cold Point Reads

These numbers represent the worst case: every read requires fetching an SST block from S3 Standard. This occurs on first access after startup or when accessing rarely-used catalog sections.

| Operation | p50 | p95 | p99 | Throughput |
|-----------|-----|-----|-----|-----------|
| Read schema by ID | 48ms | 72ms | 95ms | 20 ops/s |
| Read table metadata by ID | 52ms | 78ms | 110ms | 19 ops/s |
| Read single column | 45ms | 68ms | 92ms | 22 ops/s |
| Read data file by ID | 50ms | 75ms | 105ms | 20 ops/s |

The variance in cold reads comes from S3's own latency variability, not from Rocklake's processing. Note that each cold read also populates the cache, so subsequent reads of nearby keys (same SST block) will be cache-hot.

## Baseline Results: Scan Operations

Scan operations read a range of keys with a common prefix (all columns of a table, all files for a table, all tables in a schema).

### Cache-Hot Scans

| Operation | Rows Returned | p50 | p95 | p99 | Throughput |
|-----------|--------------|-----|-----|-----|-----------|
| List schemas (5 schemas) | 5 | 1.5ms | 2.5ms | 3.8ms | 650 ops/s |
| List tables in schema (20 tables) | 20 | 2.8ms | 4.2ms | 6.5ms | 350 ops/s |
| List columns (50 columns) | 50 | 4.8ms | 7.2ms | 12ms | 200 ops/s |
| List data files (100 files) | 100 | 6.2ms | 9.5ms | 15ms | 160 ops/s |
| List data files (1,000 files) | 1,000 | 18ms | 28ms | 42ms | 55 ops/s |
| List all tables (100 tables) | 100 | 7.1ms | 11ms | 18ms | 140 ops/s |

Scan latency scales linearly with the number of rows returned. The cost is approximately 0.05–0.1ms per row for cache-hot data. This is dominated by MVCC filtering and response serialization, not by I/O.

### Cache-Cold Scans

| Operation | Rows Returned | Blocks Fetched | p50 | p95 | p99 |
|-----------|--------------|---------------|-----|-----|-----|
| List columns (50 columns) | 50 | 1 | 52ms | 78ms | 110ms |
| List data files (100 files) | 100 | 1-2 | 55ms | 95ms | 140ms |
| List data files (1,000 files) | 1,000 | 3-5 | 150ms | 250ms | 380ms |

For large scans, the number of SST block fetches determines latency. Each block contains approximately 200–500 key-value pairs (depending on value size). A scan of 1,000 keys requires 3–5 block fetches, each adding ~50ms.

## Baseline Results: Write Operations

All write operations go through the WAL, which requires one PUT to object storage regardless of batch size.

| Operation | Keys Written | p50 | p95 | p99 | Throughput |
|-----------|-------------|-----|-----|-----|-----------|
| Create table (5 columns) | 7 | 82ms | 120ms | 180ms | 12 ops/s |
| Create table (50 columns) | 52 | 85ms | 125ms | 190ms | 11 ops/s |
| Register 1 data file | 3 | 75ms | 110ms | 160ms | 13 ops/s |
| Register 10 data files (batch) | 22 | 78ms | 115ms | 170ms | 12 ops/s |
| Register 100 data files (batch) | 202 | 95ms | 140ms | 210ms | 10 ops/s |
| Register 1,000 data files (batch) | 2,002 | 130ms | 195ms | 310ms | 7 ops/s |
| ALTER TABLE add column | 3 | 76ms | 112ms | 165ms | 13 ops/s |
| DROP TABLE (soft delete) | varies | 80ms | 118ms | 175ms | 12 ops/s |

### Observations on Write Performance

**Batch size has minimal impact up to ~100 operations.** The difference between registering 1 file (75ms) and 100 files (95ms) is only 20ms — because both result in one S3 PUT. The extra 20ms is the time to serialize a larger write batch.

**At 1,000 operations per batch, serialization becomes visible.** Constructing and serializing 2,002 key-value pairs adds ~50ms compared to a single-file registration. The S3 PUT itself also takes slightly longer because the payload is larger (~200KB vs. ~2KB).

**Write throughput is limited by S3 PUT latency.** Sequential writes (one transaction at a time) achieve 10–13 ops/s. This means approximately 1,000–1,300 files can be registered per second (at 100 files per batch). For most catalog workloads, this is more than sufficient.

## S3 Express One Zone Results

For comparison, the same benchmarks on S3 Express One Zone storage:

| Operation | S3 Standard p50 | S3 Express p50 | Improvement |
|-----------|----------------|---------------|-------------|
| Point read (cold) | 48ms | 5ms | 9.6x |
| Scan 50 keys (cold) | 52ms | 7ms | 7.4x |
| Write (single) | 75ms | 8ms | 9.4x |
| Write (batch 100) | 95ms | 12ms | 7.9x |

S3 Express provides approximately 8–10x improvement on all storage-bound operations. Cache-hot operations see no improvement (they do not access storage).

## Throughput Under Load

Sequential throughput (one client, one operation at a time):

| Workload | Operations/sec | Notes |
|----------|---------------|-------|
| Pure reads (cache hot) | 800–1,200 | Limited by TCP round-trip |
| Pure reads (cache cold, S3 Standard) | 15–25 | Limited by S3 GET latency |
| Pure writes (S3 Standard) | 10–15 | Limited by S3 PUT latency |
| Mixed read/write (90/10 split, warm) | 600–900 | Writes block briefly |

Concurrent read throughput (multiple readers, no writer):

| Readers | Total Read Throughput | Per-Reader Throughput |
|---------|----------------------|---------------------|
| 1 | 1,000 ops/s | 1,000 ops/s |
| 5 | 4,500 ops/s | 900 ops/s |
| 10 | 8,000 ops/s | 800 ops/s |
| 50 | 25,000 ops/s | 500 ops/s |

Read throughput scales well with concurrent readers because each reader has its own connection and accesses the immutable SST files independently. The slight per-reader throughput decrease at high concurrency is due to CPU scheduling and lock contention in the block cache.

## Running Benchmarks

### Prerequisites

```bash
# Rust toolchain (stable)
rustup update stable

# Criterion installed (comes with the workspace)
# No additional installation needed
```

### Running the Standard Suite

```bash
cd crates/rocklake-catalog
cargo bench --bench catalog_bench
```

This runs the full benchmark suite and stores results in `target/criterion/`. Subsequent runs compare against previous results and report regressions.

### Storage Configuration

```bash
# Local filesystem (fastest, for development — no network latency)
BENCHMARK_STORAGE=./bench-catalog cargo bench

# S3 Standard (realistic production latency)
AWS_REGION=us-east-1 BENCHMARK_STORAGE=s3://your-bench-bucket/catalog/ cargo bench

# S3 Express One Zone
AWS_REGION=us-east-1 BENCHMARK_STORAGE=s3express://your-express-bucket--use1-az4--x-s3/catalog/ cargo bench

# GCS
BENCHMARK_STORAGE=gs://your-bench-bucket/catalog/ cargo bench
```

### Running Specific Benchmarks

```bash
# Only point read benchmarks
cargo bench --bench catalog_bench -- "point_read"

# Only write benchmarks
cargo bench --bench catalog_bench -- "write"

# Only scan benchmarks with specific size
cargo bench --bench catalog_bench -- "scan/1000"
```

### Generating HTML Reports

Criterion generates HTML reports with interactive charts:

```bash
cargo bench --bench catalog_bench
# Reports at: target/criterion/report/index.html
```

## Interpreting Results

### Your Results Will Differ

The numbers on this page are from a controlled environment. Your results will differ based on:

**Network distance.** If DuckDB runs on a separate machine (not localhost), add the network round-trip time to every operation. For same-AZ, add 1–2ms. For cross-region, add 50–100ms.

**S3 latency variability.** S3 latency varies with time of day, region, and load. During peak hours, p99 latency can be 2–3x the off-peak values shown here.

**Catalog size.** The benchmark catalog has 100 tables. A catalog with 10,000 tables has more SST blocks, making cold-start scans proportionally slower. Cache-hot operations are unaffected by catalog size (as long as the working set fits in cache).

**Cache warm-up time.** The first 10–50 operations after startup will be slower (cold cache). If your workload has strong locality (same tables accessed repeatedly), the cache warms in seconds. If access is highly random, warm-up takes longer.

### What Good Looks Like

For a DuckLake catalog serving an analytics workload:

- **p50 read latency under 5ms** indicates good cache utilization
- **p99 read latency under 50ms** indicates occasional cache misses (normal)
- **Write latency under 150ms** on S3 Standard is expected and acceptable
- **Read throughput above 500 ops/s** for a single reader indicates healthy operation

### Red Flags

- **p50 read latency consistently above 30ms** suggests the cache is too small or the working set is too large
- **Write latency above 300ms** suggests S3 is experiencing unusual latency (check AWS status)
- **Scan latency growing over time** suggests GC has not run and scan amplification is increasing

## Benchmark History

We maintain a history of benchmark results to track performance over time:

- **[Phase 2 Baseline](https://github.com/trickle-labs/rocklake/blob/main/benchmarks/phase-2-baseline.json)** — Initial benchmark results from the Phase 2 milestone
- **[v0.7 Performance Report](https://github.com/trickle-labs/rocklake/blob/main/benchmarks/v07-performance-report.json)** — Current release benchmark data

## Further Reading

- **[Latency Model](latency-model.md)** — Understanding where time is spent
- **[Tuning](tuning.md)** — How to apply these insights to improve your deployment
- **[Operations: Monitoring](../operations/monitoring.md)** — Tracking performance in production
