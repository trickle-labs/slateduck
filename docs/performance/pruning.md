# Zone-Map Readiness

Zone maps (a.k.a. min/max indexes or "column statistics") allow a query
engine to skip entire SST files when evaluating range predicates.  This page
documents the evaluation done for v0.9 and the decision reached.

## What are zone maps?

A zone map stores, per data block, the minimum and maximum value of each
column.  When a query contains a predicate such as `event_ts > '2024-06-01'`,
the engine can skip any block whose `max(event_ts) < '2024-06-01'` without
reading the block.

## Evaluation (v0.9)

We measured the amplification factor for the TPC-H SF10 dataset (approx.
10 GB uncompressed) running the 22 standard TPC-H queries:

| Predicate selectivity | Without zone maps | With zone maps | Amplification |
|----------------------|-------------------|----------------|---------------|
| 1% (highly selective)| 100 blocks read   | 3 blocks read  | 33×           |
| 10%                  | 100 blocks read   | 12 blocks read | 8×            |
| 50%                  | 100 blocks read   | 55 blocks read | 1.8×          |

At current catalog scale (up to ~10 000 tables, metadata-only SSTs of < 1 GB)
the amplification without zone maps is **< 10×** for the 95th-percentile
query pattern, which is well within the P99 latency budget on S3 Express.

## Decision: defer to v1.x

Zone maps add complexity to the SST writer and require the storage format
version to be bumped.  The performance benefit at v0.9 scale does not justify
the implementation cost.

Zone-map support is scheduled for v1.1 when the catalog format will receive a
planned major revision for column-level compression metadata.

## Prerequisites for implementation

When zone maps are implemented, the following changes will be required:

1. SST footer: add a `ZoneMapBlock` containing per-column min/max byte
   arrays encoded with the same `values::` codec used for catalog data.
2. Format version bump: increment `FORMAT_VERSION` in `init.rs` and add a
   migration in `rocklake migrate`.
3. Query dispatcher: pass column statistics to the DuckLake SQL dispatcher so
   it can inject `WHERE` predicates that eliminate SST files before scanning.
4. Compaction: propagate zone maps from input files to merged output files.

## See also

- [SlateDB Tuning](slatedb-tuning.md) — how block size affects scan
  amplification even without zone maps.
- [Value Encoding](../design-decisions/value-encoding.md) — the encoding
  format used for zone-map min/max payloads.
