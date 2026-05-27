# Performance

Performance in a metadata catalog is a different beast than performance in a query engine or application database. You are not optimizing for millions of rows per second or sub-microsecond response times. You are optimizing for consistent, predictable latency on operations that happen a few hundred times per minute — schema lookups, file registrations, snapshot creation. The numbers need to be good enough that catalog operations never become the bottleneck in your analytics pipeline. Beyond that threshold, additional performance gains yield diminishing returns.

Rocklake's performance profile is shaped by a fundamental architectural reality: the catalog state lives in object storage, and object storage has latency characteristics that are different from local disk or networked databases. A GET from S3 Standard takes 20–100ms. A PUT takes 50–150ms. These numbers define the floor for cold operations. Everything Rocklake does to achieve better performance — caching, batching, hot key optimization — is about avoiding those round-trips or amortizing their cost.

This section provides honest, reproducible performance data for Rocklake's catalog operations. The emphasis is on honesty over marketing. Where Rocklake is slower than alternatives, this documentation says so clearly. Where Rocklake provides advantages that offset raw latency, those advantages are quantified. The goal is to give you the information needed to make a correct decision for your specific workload.

## Understanding the Numbers

Before diving into specific pages, some context about what "performance" means for a DuckLake catalog:

**Catalog operations are not on the critical path of data queries.** When DuckDB executes `SELECT * FROM sales WHERE region = 'APAC'`, it needs the schema and file list exactly once — at query planning time. The actual query execution (reading Parquet files, evaluating predicates, producing results) is entirely independent of the catalog. A catalog lookup that takes 5ms vs. 50ms adds 45ms to a query that might take 2 seconds to execute. That is a 2% difference.

**Reads vastly outnumber writes.** A typical catalog sees 100–1000 reads per write. Most reads hit the cache after initial warm-up. Write latency is dominated by the storage backend PUT, which is a fixed cost independent of transaction size (within reason).

**Latency variance matters more than median latency.** A p50 of 2ms with a p99 of 200ms is worse than a p50 of 10ms with a p99 of 15ms, because the variance causes unpredictable behavior. Rocklake's architecture (deterministic key layout, bounded scan amplification, explicit caching) is designed to minimize variance.

## Pages in This Section

<div class="grid cards" markdown>

-   **[Latency Model](latency-model.md)**

    ---

    Where time is spent in catalog operations — the full request path from DuckDB through Rocklake to object storage and back. Understand the layers, identify dominant factors, and learn which layers can be optimized.

-   **[Benchmarks](benchmarks.md)**

    ---

    Reproducible benchmark methodology and baseline results. Point operations, scan operations, and write operations measured on standardized hardware with statistical rigor. Instructions for running benchmarks in your environment.

-   **[Tuning](tuning.md)**

    ---

    Configuration options and operational practices that improve performance for specific workloads. Storage backend selection, cache sizing, garbage collection impact, write batching, and network optimization.

-   **[vs. Alternatives](vs-alternatives.md)**

    ---

    Honest comparison against PostgreSQL, SQLite, and MySQL as DuckLake catalog backends. Raw latency numbers, operational cost analysis, and a decision framework for choosing the right backend.

-   **[When to Use Rocklake](when-to-use.md)**

    ---

    Workload characteristics that favor Rocklake and scenarios where alternatives are better. Includes specific thresholds and decision criteria rather than vague guidance.

</div>

## Quick Reference: Typical Latency

For readers who want the bottom line before diving deeper:

| Operation | Cache Hit | Cache Miss (S3 Standard) | Cache Miss (S3 Express) |
|-----------|-----------|--------------------------|------------------------|
| Single key lookup | < 2ms | 20–100ms | 3–10ms |
| Prefix scan (50 keys) | 2–8ms | 30–80ms | 5–15ms |
| Write (any batch size) | N/A | 50–150ms | 3–10ms |

After initial warm-up (the first few operations after startup), most reads hit the cache. In steady state, the effective read latency for a moderately active catalog is typically under 5ms. Writes always go to object storage (they cannot be served from cache by definition) and pay the full PUT latency.

## Performance Philosophy

Rocklake's performance engineering follows three principles:

**Optimize the common path, accept the rare path.** The first read of a key from a cold cache is slow (full object storage round-trip). Subsequent reads of the same key are fast (memory lookup). Rather than trying to make the cold path fast (which would require prefetching logic, speculative reads, and complexity), Rocklake focuses on ensuring the hot path stays hot.

**Batch aggressively at the write boundary.** A transaction that registers 1 file and a transaction that registers 1,000 files cost approximately the same — one WAL PUT. This makes write throughput effectively free once you are inside a transaction. The optimization guidance is always "do more work per transaction" rather than "write faster."

**Predictability over peak throughput.** A system that delivers 10,000 ops/sec with occasional 500ms stalls is less useful than one that delivers 1,000 ops/sec with a guaranteed p99 under 50ms. Rocklake's fixed key layout, bounded scan ranges, and deterministic cache behavior aim for predictable latency rather than impressive peak numbers.
