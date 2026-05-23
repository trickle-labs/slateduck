# Single-Writer Model

## Why Single-Writer Is Correct

Catalogs are coordination points — all mutations must be serialized. Single-writer eliminates: write-write conflicts, lost updates, distributed locking, split-brain.

For DuckLake's workload (infrequent, small catalog writes), single writer is not a bottleneck.

## The Cost

Write throughput bounded by one process.

## Workaround: Multi-Writer Partitioning (v0.7)

One SlateDB per dataset. Global registry maps dataset names to catalog paths. Independent writers per dataset — no cross-dataset contention.
