# What SlateDuck Is Not

## Not a General SQL Engine

Bounded dispatcher only. No JOINs, subqueries, window functions.

## Not a Multi-Writer Database

One catalog, one writer. Multi-writer partitioning is a workaround, not multi-writer transactions.

## Not a Data-Plane Proxy

Manages catalog metadata only. DuckDB reads Parquet files directly.

## Not a Replacement for PostgreSQL in All Scenarios

If you already run PostgreSQL with low marginal cost and need sub-millisecond latency, use it.

## Choose SlateDuck When

- Zero infrastructure beyond a bucket
- Infinite time travel
- Horizontal read scale-out
- Building on DuckDB + DuckLake
- Moderate write rate

## Choose PostgreSQL When

- Already operating PostgreSQL
- Need sub-millisecond latency
- Need multi-writer
- Need arbitrary SQL against catalog
