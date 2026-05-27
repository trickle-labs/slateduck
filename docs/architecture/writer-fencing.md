# Writer Fencing: CAS Epoch Acquisition Protocol

## Overview

Rocklake uses a writer epoch to guarantee single-writer access to the catalog.
Only one process may hold the writer epoch at any time; all snapshot commits
verify the epoch before writing.

## Epoch Acquisition (v0.19)

When `CatalogStore::open()` is called, the writer acquires the epoch via a
Compare-And-Swap (CAS) protocol inside a serializable transaction:

```
┌─────────────────────────────────────────┐
│ 1. Generate new epoch (current time ms) │
└────────────────┬────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│ 2. Begin SerializableSnapshot TX        │
└────────────────┬────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│ 3. Read SYSTEM_WRITER_EPOCH from DB     │
│    - None → first open, proceed         │
│    - Some(existing) where existing >    │
│      our epoch → REJECT (fenced)        │
│    - Some(existing) where existing <=   │
│      our epoch → proceed (takeover)     │
└────────────────┬────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│ 4. Write new epoch in TX                │
└────────────────┬────────────────────────┘
                 │
                 ▼
┌─────────────────────────────────────────┐
│ 5. Commit TX                            │
│    - Success → epoch acquired           │
│    - Conflict → retry from step 2       │
└─────────────────────────────────────────┘
```

## Epoch Verification (`check_epoch`)

Every `create_snapshot()` call verifies the writer's epoch before committing:

1. Read `SYSTEM_WRITER_EPOCH` inside the snapshot transaction
2. If the stored epoch equals the writer's epoch → proceed
3. If the stored epoch differs → return `WriterEpochMismatch` (SQLSTATE 57P04)
4. **If the key is missing** → return `WriterEpochMismatch` (fail closed)

The fail-closed behavior on missing key (added in v0.19) prevents a corrupted
or deleted epoch key from allowing uncoordinated writes.

## Failure Modes

| Scenario | Behavior |
|----------|----------|
| Two concurrent `open()` calls | Exactly one wins the CAS; the other retries and either wins a subsequent CAS or gets `WriterEpochMismatch` if the first writer's epoch is newer |
| Writer crashes, new writer opens | New writer sees stale (older) epoch, overwrites it with CAS |
| Epoch key deleted | `check_epoch()` returns `WriterEpochMismatch` — no writes succeed until a new writer opens |
| Network partition during CAS | Transaction conflict triggers retry |

## SQLSTATE Mapping

- `57P04` — Writer has been fenced (another writer holds a newer epoch)

## Relation to Snapshot Leases

Writer epochs are orthogonal to snapshot leases. A snapshot lease prevents
GC advancement past a snapshot; the writer epoch prevents concurrent writes.
Both operate at the catalog level but serve different safety guarantees.
