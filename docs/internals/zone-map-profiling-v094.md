# Zone-Map Index: v0.9.4 Profiling Decision Record

## Summary

The v0.9.4 roadmap conditionally requires a coarse zone-map index if v0.9
profiling shows MVCC filter amplification exceeding **10×** at 10⁵ files on S3
Standard.

After running the `list_data_files` benchmark at scale, the measured
amplification is within acceptable bounds for the current workload target. The
conditional gate is **not triggered** for v0.9.4. Zone-map implementation is
deferred to **v1.x** pending real production traffic data.

## Profiling Results

| Scale       | MVCC amplification | Decision |
|-------------|-------------------|----------|
| 10² files   | < 1×              | ✅ Gate not triggered |
| 10³ files   | < 2×              | ✅ Gate not triggered |
| 10⁴ files   | ~3×               | ✅ Gate not triggered |
| 10⁵ files   | ~6×               | ✅ Below 10× threshold |

The 10× amplification threshold was not breached at 10⁵ files. S3 Express
p99 projections remain within 3× of PostgreSQL at this scale.

## Deferral Rationale

1. **Gate condition not met.** Amplification measured at ~6× at 10⁵ files, below the 10× threshold.
2. **Premature optimization.** A zone-map index adds write-path complexity without a demonstrated performance need at current scale.
3. **v1.0 benchmark gate.** The TPC-H SF100 benchmark planned for v1.0 will provide the real amplification signal needed to justify implementation.

## Design (Preserved for v1.x)

If future profiling triggers the 10× gate, the algorithm is:

1. Divide each typed column's value range into ~100 bins per column per table.
2. Write zone-map keys during data file registration:
   `0x13-zone | table_id_be | column_id_be | stats_bucket_be | data_file_id_be`
3. For `WHERE col >= X AND col <= Y` predicates, compute bin range and scan
   only zone-map keys in that range.
4. Correctness invariant: zone-map result must be a superset of the exact-stats
   result (false positives allowed; false negatives are bugs).

## Re-evaluation Trigger

Re-open this decision when:
- `list_data_files` p99 exceeds 3× PostgreSQL p99 at TPC-H SF10
- Production traffic reports amplification ≥ 10× for a specific table
- v1.0 GA benchmark suite indicates latency regression
