# Snapshots

A **snapshot** is a point-in-time consistent view of the entire catalog. Every catalog mutation produces a new snapshot with a monotonically-increasing ID.

## Properties

- **Monotonic:** Snapshot IDs always increase
- **Atomic:** A snapshot either fully exists or doesn't
- **Immutable:** Once committed, a snapshot's contents never change
- **Consistent:** Reading at snapshot N always returns the same data

## How Snapshots Enable Time Travel

Because SlateDuck never deletes historical data (by default), every snapshot remains queryable forever.

```sql
SELECT * FROM analytics.events AT (SNAPSHOT 5);
```

## Snapshot Lifecycle

1. Writer begins a transaction — assembles mutations in memory
2. Writer commits — atomically writes all mutations with the new snapshot ID
3. Snapshot becomes visible — readers see the new snapshot
4. Snapshot lives forever — unless GC or excision removes it

## Retention

By default, all snapshots are retained indefinitely. Configure `--retention-days` for bounded storage.
