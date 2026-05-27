# Garbage Collection

Rocklake's immutable, append-only data model means every catalog change creates new key-value pairs without deleting old ones. A schema alteration does not modify existing rows — it writes new versioned rows that supersede the old ones. Over time, these superseded versions accumulate. Garbage collection (GC) is the process of reclaiming storage by removing data that is no longer accessible to any reader.

GC in Rocklake is a two-phase process with an explicit separation between "making data logically inaccessible" and "physically deleting bytes." This separation is intentional — it gives operators a safety window to change their mind and prevents accidental irrecoverable data loss. Phase 1 (advancing the retention horizon) is reversible. Phase 2 (excision) is permanent.

This page explains when and why GC is needed, how each phase works internally, scheduling strategies, interaction with pinned snapshots, and storage impact analysis.

## Why Garbage Collection Is Necessary

### The Growth Problem

Every catalog operation that modifies state creates new key-value pairs:

- Creating a table: ~10 new keys (table metadata, columns, constraints)
- Adding a column: ~3 new keys (column metadata, table version bump)
- Registering a data file: ~5 new keys (file entry, stats, partition info)
- Running compaction: ~2 new keys (compaction marker, updated stats)

A catalog that undergoes 100 schema changes per day accumulates approximately 500–1000 new key-value pairs daily. Each pair is 50–200 bytes, so raw growth is 50–200 KB/day — modest. But over months and years, this growth compounds:

| Time Period | Approximate Growth | Cumulative |
|-------------|-------------------|------------|
| 1 day | 100 KB | 100 KB |
| 1 month | 3 MB | 3 MB |
| 1 year | 36 MB | 36 MB |
| 3 years | 36 MB/year | 108 MB |

The storage cost itself is negligible (108 MB in S3 costs less than $0.01/month). The real cost is **scan performance**: every prefix scan must read through all versions and filter to the latest visible one. More versions means more data to skip during scans, increasing latency.

### The Snapshot Problem

By default, every committed transaction creates a snapshot that remains queryable forever via time travel. If your catalog commits 50 times per day, after a year you have 18,250 queryable snapshots. While the storage cost is minimal, the operational overhead of maintaining infinite history is unnecessary for most workloads.

## Understanding the Two Phases

### Phase 1: Advance Retention Horizon

The retention horizon (`retain_from`) is a system-level key that defines the oldest snapshot any reader is allowed to access. Think of it as a sliding window — everything before the horizon is "outside the window" and invisible to clients:

```
Time: ──────────────────────────────────────────────────────→

Snapshots: [100] [200] [300] [400] [500] [600] [700] [800]

                           ↑ retain_from = 400
                           |
   Inaccessible (GC'd)    |    Accessible (time travel OK)
```

Advancing the horizon does NOT delete data. It only updates the `retain_from` key, which makes older snapshots return an error when queried:

```sql
-- Before GC: works
SELECT * FROM lake.analytics.events AT SNAPSHOT 300;

-- After GC advances horizon to 400: error
SELECT * FROM lake.analytics.events AT SNAPSHOT 300;
-- ERROR: Snapshot 300 is before retention horizon (400)
```

**This is reversible.** If you made a mistake, you can set `retain_from` back to an earlier value and those snapshots become accessible again (assuming excision has not yet deleted their underlying data).

### Phase 2: Excision (Physical Deletion)

Excision physically removes key-value pairs that are no longer visible to any valid reader. A key-value pair is eligible for excision if:

1. It was superseded by a newer version (a later write to the same key)
2. Its latest-visible snapshot is before the retention horizon
3. No pinned snapshot references it

Excision is **irreversible**. Once the bytes are deleted from object storage, they cannot be recovered (short of restoring from a backup). This is why it is a separate, explicit step.

## Running Garbage Collection

### Phase 1: Advance Horizon

```bash
rocklake gc --catalog s3://bucket/catalog/ --retain-days 30
```

This command:

1. Reads the current time and calculates the timestamp 30 days ago
2. Finds the snapshot ID closest to that timestamp
3. Checks for pinned snapshots that would block advancement
4. If clear, writes the new `retain_from` value
5. Reports the result

Output:

```
Garbage Collection Summary:
  Previous retain_from: snapshot 200 (2024-11-15T10:00:00Z)
  New retain_from:      snapshot 650 (2024-12-16T10:00:00Z)
  Snapshots now inaccessible: 450
  Pinned snapshots respected: 0
  Storage reclaimable (estimated): 4.2 MB
```

### Dry Run (Preview)

Always preview before running GC in production:

```bash
rocklake gc --catalog s3://bucket/catalog/ --retain-days 30 --dry-run
```

Dry run performs all calculations but does not write anything. It shows exactly what would happen.

### Phase 2: Excision

After advancing the horizon, optionally run excision to reclaim physical storage:

```bash
rocklake excise --catalog s3://bucket/catalog/
```

This scans all keys, identifies pairs that are invisible to all valid readers, and deletes them. See the [Excision](excision.md) page for full details.

## Retention Policies

### Choosing a Retention Period

| Retention | Use Case | Trade-off |
|-----------|----------|-----------|
| **7 days** | High-churn catalogs, cost-sensitive | Minimal debugging window |
| **30 days** | Most production workloads | Good balance of history and growth |
| **90 days** | Compliance-sensitive (audit trails) | More storage, slower scans |
| **365 days** | Regulated industries | Significant accumulation |
| **∞ (no GC)** | Development, testing | Unbounded growth |

### Retention and Compliance

Some industries require audit trails:

- **SOX compliance:** 7 years of financial data provenance → 2555 days retention
- **GDPR:** Right to erasure may require excision within specific periods
- **HIPAA:** 6 years of access records

For long retention periods, monitor scan performance and consider whether read-path optimization (bloom filters, index caching) addresses the latency impact.

## Scheduling GC

### Kubernetes CronJob

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: rocklake-gc
  namespace: rocklake
spec:
  schedule: "0 3 * * *"  # Daily at 3 AM UTC
  concurrencyPolicy: Forbid
  successfulJobsHistoryLimit: 7
  failedJobsHistoryLimit: 3
  jobTemplate:
    spec:
      backoffLimit: 2
      activeDeadlineSeconds: 1800
      template:
        spec:
          serviceAccountName: rocklake
          containers:
            - name: gc
              image: ghcr.io/rocklake/rocklake:0.8.0
              command:
                - "rocklake"
                - "gc"
                - "--storage"
                - "s3://my-bucket/catalog/"
                - "--retain-days"
                - "30"
              resources:
                requests:
                  memory: "64Mi"
                  cpu: "50m"
          restartPolicy: OnFailure
```

### systemd Timer (Bare Metal)

```ini
# /etc/systemd/system/rocklake-gc.timer
[Unit]
Description=Rocklake Daily Garbage Collection

[Timer]
OnCalendar=*-*-* 03:00:00
Persistent=true

[Install]
WantedBy=timers.target
```

```ini
# /etc/systemd/system/rocklake-gc.service
[Unit]
Description=Rocklake GC Run

[Service]
Type=oneshot
ExecStart=/usr/local/bin/rocklake gc --catalog s3://bucket/catalog/ --retain-days 30
Environment=AWS_REGION=us-east-1
```

### Crontab

```bash
# Daily GC at 3 AM
0 3 * * * /usr/local/bin/rocklake gc --catalog s3://bucket/catalog/ --retain-days 30 >> /var/log/rocklake-gc.log 2>&1
```

## Pinned Snapshots

Pinned snapshots prevent GC from advancing the retention horizon past them. This is essential for long-running processes that need consistent reads at a specific point in time.

### When to Pin

- **Long-running analytics jobs** that read from a fixed catalog snapshot for hours
- **Audit processes** that need to verify catalog state at a specific past time
- **Cross-system consistency** where an external system references a specific snapshot ID

### Pinning and Unpinning

```bash
# Pin a snapshot (prevents GC from advancing past it)
rocklake pin-snapshot --catalog s3://bucket/catalog/ --snapshot-id 500

# List all pinned snapshots
rocklake list-pins --catalog s3://bucket/catalog/

# Unpin when no longer needed
rocklake unpin-snapshot --catalog s3://bucket/catalog/ --snapshot-id 500
```

### GC Behavior with Pins

If GC encounters a pinned snapshot within the target retention window:

```bash
rocklake gc --catalog s3://bucket/catalog/ --retain-days 7
# WARNING: Cannot advance past snapshot 450 (pinned since 2024-12-10)
# Advancing retain_from to 449 instead of requested 650
```

GC advances as far as it can without violating any pin. It does not fail — it simply advances to the most aggressive position that respects all constraints.

### Automatic Pin Expiry

For safety, pins can have an expiry:

```bash
rocklake pin-snapshot --catalog s3://bucket/catalog/ --snapshot-id 500 --expires 72h
```

After 72 hours, the pin is automatically removed and GC can proceed past it.

## Storage Impact Analysis

### Estimating Reclaimable Space

```bash
rocklake gc --catalog s3://bucket/catalog/ --retain-days 30 --analyze
```

Output:

```
Storage Analysis:
  Total catalog size: 48.2 MB
  Data before retention horizon: 12.8 MB (26.6%)
  Reclaimable after excision: 11.4 MB (23.7%)
  Protected by pins: 1.4 MB (2.9%)
  Post-GC estimated size: 36.8 MB
```

### Understanding What Gets Cleaned

GC removes **superseded versions** — not all old data. If a table was created 6 months ago and never modified, its creation record is still the "latest version" and will never be garbage collected, regardless of age.

Only data that has been **replaced by newer versions** is eligible:

- Old column definitions (replaced by ALTER COLUMN)
- Previous table versions (replaced by schema changes)
- Superseded file registrations (replaced by compaction)
- Old statistics (replaced by newer stats calculations)

## GC and Performance

### Impact on the Running Server

GC is designed to run while the server is active:

- **Phase 1 (horizon advance):** Single key write, negligible impact (<1ms)
- **Phase 2 (excision):** Full key scan, but read-only from SlateDB's perspective (deletes are writes that remove keys). May slightly increase object storage request rate during the scan.

### Impact on Scan Performance

After GC + excision, prefix scans become faster because there are fewer versions to filter:

| Scenario | Scan Time (P95) |
|----------|-----------------|
| No GC (1 year of history) | 45ms |
| 30-day retention + excision | 12ms |
| 7-day retention + excision | 8ms |

The improvement comes from reading fewer SST blocks during scans.

## Monitoring GC

### Key Metrics

| Metric | Alert Condition | Action |
|--------|-----------------|--------|
| `rocklake_gc_last_run_timestamp` | > 48 hours ago | Check CronJob health |
| `rocklake_gc_retained_snapshots` | Growing unbounded | Retention may be misconfigured |
| `rocklake_gc_pinned_count` | Unexpectedly high | Stale pins blocking GC |
| `rocklake_gc_duration_seconds` | > 600 | Catalog may need compaction |
| `rocklake_catalog_size_bytes` | Growing despite GC | Excision not running or pins blocking |

### Alerting Example

```yaml
- alert: RocklakeGCStale
  expr: time() - rocklake_gc_last_run_timestamp > 172800
  labels:
    severity: warning
  annotations:
    summary: "Rocklake GC has not run in 48 hours"
```

## Troubleshooting GC

### "Cannot advance: pinned snapshot blocks"

A pinned snapshot is preventing GC. List pins and determine if they are still needed:

```bash
rocklake list-pins --catalog s3://bucket/catalog/
```

### "GC completed but catalog size unchanged"

GC only advances the horizon. You need to run excision to actually delete bytes:

```bash
rocklake excise --catalog s3://bucket/catalog/
```

### "Excision running slowly"

Excision scans all keys. For very large catalogs, this may take minutes. Run during off-peak hours and set an appropriate timeout.

## Further Reading

- **[Excision](excision.md)** — Physical deletion details and safety guarantees
- **[Backup & Restore](backup-restore.md)** — Backup before aggressive GC
- **[Concepts: Snapshots](../concepts/snapshots.md)** — Time travel and snapshot model
- **[Concepts: Immutability](../concepts/immutability.md)** — Why append-only creates GC need
