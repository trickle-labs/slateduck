# S3 API Cost Analysis

RockLake exposes three `--cost-mode` presets that trade write latency for
object-storage API cost. This page explains the cost model, shows the
crossover points, and gives guidance on choosing a mode.

## Cost model

Standard S3 pricing (us-east-1, as of 2024):

| API class | Price per 1,000 requests |
|-----------|--------------------------|
| PUT/POST  | $0.005                   |
| GET/HEAD  | $0.0004                  |
| LIST      | $0.005                   |

RockLake's write path issues **1 PUT** per SST flush, plus **1 LIST** per
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
roughly **$29/month** in us-east-1. RockLake in `balanced` mode at 10
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

## Cost-per-Operation Reference (v0.42.0)

The table below uses S3 Standard pricing as of 2026 (us-east-1):

| Price tier | Rate |
|---|---|
| Storage | $0.023 / GB-month |
| PUT / COPY / POST | $0.005 / 1 000 requests |
| GET / SELECT / other | $0.00035 / 1 000 requests |
| LIST | $0.005 / 1 000 requests |

S3 Express One Zone prices are approximately 50% lower for request fees and
60% lower for storage.

### S3 API Calls per Catalog Operation

These counts are measured by the `object_store` request counter middleware
introduced in v0.42.0, using a LocalFS backend with request interception.
S3 call counts are identical to LocalFS for catalog operations because
RockLake issues the same object-store operations regardless of backend.

| Operation | GET | PUT | LIST | Estimated cost (S3 Standard) |
|---|---|---|---|---|
| `get_current_snapshot()` — warm cache | 0 | 0 | 0 | < $0.0000001 |
| `get_current_snapshot()` — cold open | 1–2 | 0 | 0 | $0.0000007 |
| `list_data_files(n=100)` | 1 | 0 | 0 | $0.00000035 |
| `list_data_files(n=10k)` | 2–4 | 0 | 0 | $0.0000014 |
| `list_data_files(n=100k)` | 4–8 | 0 | 0 | $0.0000028 |
| `create_snapshot(1 file)` | 0 | 1 | 0 | $0.000005 |
| `create_snapshot(100 files)` | 0 | 1 | 0 | $0.000005 |
| `create_snapshot(1 000 files)` | 0 | 1–2 | 0 | $0.000010 |
| `describe_table(50 cols)` | 1 | 0 | 0 | $0.00000035 |
| `prune_files(100k files)` | 4–8 | 0 | 0 | $0.0000028 |
| SlateDB compaction cycle | 2–10 | 1–5 | 1–2 | $0.000015–0.00006 |

*Note: RockLake batches all catalog mutations for a logical commit into a
single SlateDB `WriteBatch`, which flushes as at most 1–2 PUTs per commit
(one memtable SST, one manifest update). Multi-file additions do not
proportionally increase PUT count.*

### Monthly Cost Estimates at Typical Workloads

| Workload | Write ops/s | Monthly S3 cost | Monthly Postgres RDS |
|---|---|---|---|
| Dev / staging | 0.1 | < $0.01 | ~$29 (db.t3.medium) |
| Small analytics | 1 | ~$0.10 | ~$29 |
| Medium analytics | 10 | ~$0.65 | ~$87 (db.t3.large) |
| High-throughput ingest | 100 | ~$6.50 | ~$175 (db.r6g.large) |

*Postgres costs are for an always-on single-AZ RDS instance. RockLake has no
instance cost — only S3 API fees and storage.*

### Storage Cost Crossover

At $0.023/GB-month (S3 Standard), the storage break-even vs. a 100 GB
PostgreSQL RDS `db.t3.medium` allocation (~$14/month storage) is
**≈ 600 GB** of catalog data. Almost no real-world catalog approaches
600 GB; the cost advantage is effectively unconditional.

## Instrumenting Your Own Deployment

RockLake's `object_store` layer includes a request-counting wrapper
(`RequestCounter`) that can be queried via the diagnostics API:

```rust
// From rocklake-client
let cost = catalog.api_cost_report().await?;
println!("GET: {}, PUT: {}, LIST: {}", cost.gets, cost.puts, cost.lists);
```

Or from the CLI:

```bash
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

## See Also

- [S3 Express Validation](s3-express-validation.md) — S3 Express acceptance
  gate results and tuning guidance.
- [SlateDB Tuning](slatedb-tuning.md) — how to set `l0_sst_count_threshold`
  and `max_write_batch_bytes` manually.
- [Benchmarks](benchmarks.md) — TPC-H catalog benchmark results.
