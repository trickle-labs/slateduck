# Immutability

Once a fact is committed to the catalog, it is never physically deleted unless explicitly excised.

## What Immutability Means

- Old versions are preserved (ALTER sets end_snapshot, doesn't delete)
- Dropped tables are not deleted (DROP sets end_snapshot)
- Historical snapshots are queryable forever (unless GC/excision removes them)

## Benefits

1. Time travel for free
2. Horizontal read scale-out (no writer-reader conflicts)
3. Audit trail
4. Crash safety (old data never overwritten)

## Costs

1. Storage growth (requires GC for bounded storage)
2. Read amplification (MVCC filter skips old versions)
3. Operator responsibility (cleanup requires explicit action)
