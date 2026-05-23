# When to Use SlateDuck

## Good Fit

- Zero infrastructure beyond a bucket
- Infinite time travel by default
- Horizontal read scale-out needed
- Moderate write rate (tens to hundreds of snapshots/sec)
- Building on DuckDB + DuckLake

## Not a Good Fit

- Need sub-5ms catalog latency
- Need multi-writer to same table
- Already operate PostgreSQL with low marginal cost
- Need arbitrary SQL against catalog
