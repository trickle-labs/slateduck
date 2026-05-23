---
hide:
  - navigation
  - toc
---

# SlateDuck

<div class="hero" markdown>

## Your entire lakehouse catalog in a single S3 bucket

No PostgreSQL. No SQLite file locks. No infrastructure to manage.
Just a bucket, DuckDB, and infinite time travel.

[Get Started](getting-started/quickstart.md){ .md-button .md-button--primary }
[Architecture](architecture/index.md){ .md-button }

</div>

## What is SlateDuck?

SlateDuck is a **DuckLake catalog implementation** backed by [SlateDB](https://slatedb.io) -- an LSM-tree key-value store that uses object storage (S3, GCS, Azure) as its durable layer.

## Why SlateDuck?

| Dimension | PostgreSQL-backed DuckLake | SlateDuck |
|-----------|--------------------------|-----------|
| | | | | | | | | | | | | | | | | | | | | | | | | | | | | 
|||||||||tra|||||||||tra||||(WA|||||||||tra|||||||nit|||||||||tra|||||||||tra||||(WA|||||||Re|||||||||tra|||||||||tra||||(WA|||||||||tra|||||||niat|||||||||tra|||||||||tra||||nis|||||||||tra|||||||||rless|||||||||tralog latency** | 1-5 ms||same|||||||||-50 ms||||||tandard|||||||*Write concurrency** | Multi-writer (with locks) | Single writer per catalog |

## Quick Navi## Quick Navi## Quick Navi## Quick Navi## Quick Navi## Quick Navlaunch: **[Getting Started](getting-started/index.md## Quick Navi## Quick Navi## Quick Navi## Quick Navi## Quick Navi## Quick Nal-l## Quick Navi## Quick Navi## Quick Navi## Quick Naerstand the## Quick Navi## Quiaterial-crane: **[Architecture](architecture/index.md)** -- Deep-dive into crate structure, key layout, MVCC.
- :material-server: **[Deployment](deployment/index.md)** -- Docker, Kubernetes, - :material-serval.- :material-server: **[Deployment](deployment/index..md- :material-server: **[Deployment](deployment/index- - :material-server: **[Deployment](deployment/index.md)** -- DuckDB, DataFusion, pg-tide-relay.
- :material-scale-balance: **[Design Decisions](design-decisions/index.md)** -- Honest trade-off analysis.
- :material-speedometer: **[Performance](performance/index.md)** -- Benchmarks, latency model, tuning.

</div>
