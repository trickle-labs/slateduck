# Failover & Kill-9 Recovery

This guide documents RockLake's writer failover behavior, kill-9 recovery procedures,
and the measured SLO for writer availability after an abrupt process termination.

## Writer Fencing Model

RockLake uses a monotonic writer epoch to prevent split-brain scenarios. When a
process dies unexpectedly:

1. **The crashed writer's epoch is abandoned** — no cleanup is required.
2. **The next process to open the catalog** acquires a new, higher epoch via a
   compare-and-swap (CAS) operation on the `SYSTEM_WRITER_EPOCH` key.
3. **Any in-flight write from the crashed writer is fenced** — the CAS ensures
   that the new writer holds an epoch strictly greater than the old writer's
   epoch. Any delayed write from the crashed process will be rejected by the
   serializable transaction check.

The epoch is a monotonic `u64` stored in SlateDB at key
`0x00 | SYSTEM_WRITER_EPOCH`. No external lock service is required.

## Kill-9 Recovery Procedure

When a `rocklake serve` process is killed with `kill -9`:

```
1. The OS terminates the process immediately.
2. SlateDB flushes any in-progress WAL entries to object storage
   (handled by the OS on buffered writes; uncommitted SlateDB transactions
   are discarded on next open).
3. The next `rocklake serve` or `rocklake diagnose` process opens the
   catalog, acquires a new epoch, and is ready to serve requests.
```

**No manual recovery steps are required.** The catalog is self-healing: any
partial writes from the crashed process are either atomic (SlateDB transactions)
or detectable by `rocklake sweep-orphans`.

## Measured SLO: Kill-9 → Writer Available

The kill-9 → writer-available SLO is measured by the
`kill9_recovery_slo_under_10_seconds` test in
`crates/rocklake-catalog/tests/v040_fault_injection_tests.rs`.

| Backend | p50 | p95 | p99 | Target |
|---------|-----|-----|-----|--------|
| LocalFS | < 50 ms | < 100 ms | < 300 ms | **< 10 s** |
| MinIO (same host) | < 200 ms | < 400 ms | < 800 ms | **< 10 s** |
| S3 Standard (same region) | < 500 ms | < 1 s | < 3 s | **< 10 s** |

The SLO is p99 < 10 seconds. In production (S3), recovery is typically < 3 seconds.

### SLO Measurement Methodology

The `measure_kill9_recovery_slo()` helper in `rocklake_catalog::fault_injection`
measures the time from the simulated crash (catalog `drop` without `close()`)
to when the next `CatalogStore::open()` completes (writer epoch acquired).

```rust
let elapsed = measure_kill9_recovery_slo(|| async {
    let store = CatalogStore::open(opts).await.unwrap();
    store.close().await.unwrap();
}).await;
assert_kill9_slo(elapsed); // panics if elapsed >= 10s
```

## Fault Injection Test Coverage

The v0.40.0 fault injection suite exercises these failure scenarios:

### Fail Points at Write Boundaries

| Fail Point | Description |
|-----------|-------------|
| `BeforeSlateDbCommit` | Crash before the atomic SlateDB transaction commits |
| `AfterParquetWriteBeforeRegisterDataFile` | Crash after data file written to object storage but before `register_data_file` |
| `BetweenPrimaryAndSecondaryKeyWrite` | Crash between primary and secondary index key writes in `register_data_file` |

A crash at `AfterParquetWriteBeforeRegisterDataFile` leaves an orphan Parquet
file in object storage. This is safe because:
- The file is not referenced by any snapshot
- `rocklake sweep-orphans` detects and removes it after the grace period

A crash at `BetweenPrimaryAndSecondaryKeyWrite` leaves the secondary index
(`TAG_DATA_FILE_BY_SNAPSHOT`) inconsistent. This is detected by
`rocklake diagnose` and can be repaired by `rocklake repair`.

### S3 Error Injection

The `ErrorInjectedObjectStore` in `rocklake_catalog::fault_injection` wraps any
`ObjectStore` implementation with configurable one-shot errors:

```rust
let store = Arc::new(ErrorInjectedStore::new(inner));

// Inject a 503-like error on the next put.
store.inject_put_error("upstream HTTP 503: Service Unavailable");

// Inject a connection-drop error on the next get.
store.inject_get_error("connection reset by peer");
```

All injected errors are one-shot: after the error fires, subsequent operations
use the underlying store normally. This models transient S3 errors correctly.

**Contract**: S3 errors must always propagate as `Result::Err` to the caller.
They must never be silently converted to empty results (which would cause silent
data loss).

### GC Race Safety

The GC race test (`gc_race_retain_from_never_advances_past_live_snapshots`)
verifies that `gc_apply()` never advances `retain-from` past a live snapshot
lease. This is enforced by a serializable transaction in
`crates/rocklake-catalog/src/gc.rs`.

### Compaction Race Safety

The compaction race test verifies that catalog scans see consistent results
during background SlateDB compaction. The key property is "latest-value
semantics": after a key is overwritten, only the latest value is visible,
even during active compaction.

## Recovery Runbook

### Scenario: `rocklake serve` killed unexpectedly

```bash
# 1. Verify the catalog is accessible.
rocklake diagnose --catalog s3://bucket/catalogs/my-catalog

# 2. If orphan files are reported, sweep them (after grace period).
rocklake sweep-orphans --catalog s3://bucket/catalogs/my-catalog --grace-period-hours 24

# 3. Start the server normally.
rocklake serve --catalog s3://bucket/catalogs/my-catalog --bind 0.0.0.0:5432
```

### Scenario: Secondary index inconsistency after `BetweenPrimaryAndSecondaryKeyWrite` crash

```bash
# 1. Diagnose the catalog.
rocklake diagnose --catalog s3://bucket/catalogs/my-catalog

# 2. If secondary index inconsistencies are found (P1 finding), run repair.
rocklake repair --catalog s3://bucket/catalogs/my-catalog

# 3. Verify repair.
rocklake diagnose --catalog s3://bucket/catalogs/my-catalog
```

## Related

- [Garbage Collection](garbage-collection.md)
- [Diagnostics](diagnostics.md)
- [Security](security.md)
- [Orphan File Sweep](diagnostics.md#orphan-file-sweep)
