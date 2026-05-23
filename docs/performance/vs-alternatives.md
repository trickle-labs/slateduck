# vs. Alternatives

| Dimension | SlateDuck (S3) | PostgreSQL | SQLite |
|-----------|---------------|------------|--------|
| Infrastructure | None (bucket) | Managed server | Local file |
| Read latency (p50) | 20-50 ms | 1-5 ms | <1 ms |
| Write latency (p50) | 100-200 ms | 5-20 ms | <1 ms |
| Time travel | Infinite | Limited | None |
| Read scale-out | Unlimited | Replicas | Single process |
| Write concurrency | Single writer | Multi-writer | Single writer |

## Where SlateDuck Wins

Zero infrastructure, infinite time travel, read scale-out, low cost at rest.

## Where PostgreSQL Wins

Latency (10-50x lower), multi-writer, arbitrary SQL, maturity, observability.

## Honest Assessment

For most deployments, the latency gap is acceptable — DuckDB spends 95%+ of time reading Parquet, not catalog. SlateDuck's value is simpler ops, infinite history, no infrastructure.
