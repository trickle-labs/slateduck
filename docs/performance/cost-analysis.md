# S3 API Cost Analysis

Rocklake exposes three `--cost-mode` presets that trade write latency for
object-storage API cost. This page explains the cost model, shows the
crossover points, and gives guidance on choosing a mode.

## Cost model

Standard S3 pricing (us-east-1, as of 2024):

| API class | Price per 1,000 requests |
|-----------|--------------------------|
| PUT/POST  | $0.005                   |
| GET/HEAD  | $0.0004                  |
| LIST      | $0.005                   |

Rocklake's write path issues **1 PUT** per SST flush, plus **1 LIST** per
compaction cycle.  The memtable size and `l0_sst_count_threshold` setting
together control flush frequency.

## Cost-mode presets

### `conservative`

```
l0_sst_count_threshold = 8
max_write_batch_bytes  = 64 MiB
```

Targets ≤ 100 PUT/hour for a 10 write-ops/s workload. At $0.005/1 000 PUTs
that is **≈ $0.004/day**.  Best for batch-ingest pipelines that can tolerate
200–400 ms p99 write latency.

### `balanced` (default)

```
l0_sst_count_threshold = 4
max_write_batch_bytes  = 32 MiB
```

Tuned on the TPC-H SF10 benchmark. Produces ≈ 240 PUT/hour under the
benchmark load. **≈ $0.009/day**. Recommended starting point for most
deployments.

### `latency`

```
l0_sst_count_threshold = 2
max_write_batch_bytes  = 16 MiB
```

Targets < 50 ms p99 write latency; issues ≈ 720 PUT/hour under load.
**≈ $0.026/day** on standard S3.  Intended for interactive analyst workloads
on S3 Express One Zone (which has lower per-request prices and higher
throughput).

## Monthly cost crossover vs. PostgreSQL

A self-managed PostgreSQL instance on a `db.t3.medium` RDS instance costs
roughly **$29/month** in us-east-1. Rocklake in `balanced` mode at 10
writes/s costs **< $0.30/month** in API fees, plus storage ($0.023/GB-month
for S3 Standard). Storage break-even is around **1 200 GB** for a data
warehouse that does not need online transaction processing.

## Using `rocklake inspect api-costs`

```
rocklake inspect api-costs \
    --catalog s3://my-bucket/catalog \
    --estimate-monthly \
    --compare-postgres
```

Sample output:

```
S3 API Cost Report (last 1 h)
  PUT  count : 248
  GET  count : 1 024
  LIST count : 12
  Estimated cost (1 h)    : $0.0009
  Estimated cost (monthly): $0.65
  Equivalent Postgres (RDS db.t3.medium): ~$29.00/month
  Cost ratio: 2.2% of equivalent Postgres
```

Add `--stream` to emit one JSON object per second for real-time dashboards.

## See also

- [SlateDB Tuning](slatedb-tuning.md) — how to set `l0_sst_count_threshold`
  and `max_write_batch_bytes` manually.
- [Benchmarks](benchmarks.md) — TPC-H SF10 baseline numbers.
