# Verify & Repair

SlateDuck includes tools for verifying catalog integrity and performing conservative repairs when issues are detected. These tools are designed for situations where you suspect corruption — unexpected errors, inconsistent query results, or anomalies reported by monitoring — and need to diagnose and potentially fix the problem without risking further damage.

Verify performs a comprehensive read-only scan of the entire catalog, checking every invariant that SlateDuck depends on. Repair takes the findings from verify and applies safe, conservative fixes. The guiding principle is "first, do no harm" — repair will never modify data that might be needed by a valid reader, and it will never act on ambiguous findings.

## When to Use

### Run Verify When:

- `slateduck inspect` reports unexpected counter values
- DuckDB queries return inconsistent results (missing columns, wrong types)
- After an unclean shutdown (power loss, OOM kill, SIGKILL)
- After a failed upgrade or migration
- After running excision (to confirm integrity)
- As a periodic health check (weekly or monthly)
- Before or after restoring from backup

### Run Repair When:

- Verify reports errors (not just warnings)
- You have confirmed the errors are genuine (not transient storage issues)
- You have a recent backup to fall back on if repair makes things worse

## Verify

### Running Verify

```bash
# Basic verify
slateduck verify --catalog s3://bucket/catalog/

# Verbose output (shows every check as it runs)
slateduck verify --catalog s3://bucket/catalog/ --verbose

# JSON output (for automated pipelines)
slateduck verify --catalog s3://bucket/catalog/ --format json

# Verify at a specific snapshot (historical state)
slateduck verify --catalog s3://bucket/catalog/ --at-snapshot 1000
```

### What Verify Checks

Verify performs these checks in order:

#### 1. System Key Integrity

| Check | Description | Error If |
|-------|-------------|----------|
| Format version | Is `sys/format_version` present and recognized? | Missing or unknown value |
| Writer epoch | Is `sys/epoch` present and > 0? | Missing or zero |
| Retain from | Is `sys/retain_from` present? | Missing |
| Counters | Are all counters present? | Any missing |

#### 2. Counter Consistency

For each counter (next_snapshot_id, next_catalog_id, next_file_id):

- Scans all keys of the corresponding type
- Verifies the counter value is strictly greater than the maximum existing ID
- Reports an error if counter <= max existing ID (this can cause duplicate ID allocation)

#### 3. MVCC Invariants

For every versioned row in the catalog:

- `created_snapshot_id` must be > 0
- If `end_snapshot_id` is set: `end_snapshot_id` > `created_snapshot_id`
- `created_snapshot_id` must be <= `latest_snapshot`
- If `end_snapshot_id` is set: `end_snapshot_id` <= `latest_snapshot`
- At most one version of each entity should be "live" (no `end_snapshot_id`) at any given snapshot

#### 4. Referential Integrity

| Parent | Child | Check |
|--------|-------|-------|
| Schemas | Tables | Every table's `schema_id` references an existing schema |
| Tables | Columns | Every column's `table_id` references an existing table |
| Tables | Data Files | Every file's `table_id` references an existing table |
| Tables | Delete Files | Every delete file's `table_id` references an existing table |
| Data Files | Statistics | Every stat's `file_id` references an existing data file |

"Existing" means visible at the same snapshot as the child row.

#### 5. Value Decoding

Every value in the catalog is decoded from its protobuf representation:

- Magic byte is correct for the value type
- Protobuf deserialization succeeds
- Required fields are present
- Field values are within valid ranges (e.g., `ordinal_position` >= 0)

#### 6. Duplicate Detection

Scans for duplicate keys that should not exist:

- Two "latest" pointers for the same entity
- Two live versions of the same entity at the same snapshot
- Duplicate system keys

#### 7. Orphan Detection

Identifies entries that reference deleted parents:

- Columns for tables that no longer exist (at any snapshot)
- Statistics for files that no longer exist
- Inlined inserts for tables that no longer exist

### Verify Output

```
SlateDuck Catalog Verification
═══════════════════════════════════════════════════════════════
Storage:         s3://my-bucket/lakehouse/catalog/
Snapshot:        latest (1,247)
Duration:        2.3 seconds

Checks Performed:
  System keys:       6 checked, 6 passed
  Counters:          3 checked, 2 passed, 1 ERROR
  MVCC invariants:   3,412 rows checked, 3,412 passed
  Referential:       2,145 references checked, 2,140 passed, 5 WARNINGS
  Value decoding:    3,412 values decoded, 3,412 passed
  Duplicates:        0 found
  Orphans:           2 found (WARNINGS)

Summary:
  Errors:    1
  Warnings:  7

═══════════════════════════════════════════════════════════════

ERRORS (require repair):

  [E001] Counter consistency
    next_file_id is 1800, but maximum existing file_id is 1892.
    Impact: New file registrations may allocate duplicate IDs.
    Repair: Advance counter to 1893.

WARNINGS (informational, no immediate action needed):

  [W001] Referential integrity
    5 column rows reference table_id=42, which has no live version
    at current snapshot (table was dropped at snapshot 1200).
    These columns are themselves ended at snapshot 1200, so this
    is consistent — the table and its columns were dropped together.

  [W002] Orphaned inlined inserts
    2 inlined insert rows reference table_id=99, which does not exist
    at any snapshot. These rows are unreachable and waste storage.
    Repair can safely remove them.

═══════════════════════════════════════════════════════════════
Run `slateduck repair --dry-run` to preview fixes for the errors above.
```

### JSON Output

```json
{
  "storage": "s3://my-bucket/lakehouse/catalog/",
  "snapshot": 1247,
  "duration_ms": 2300,
  "errors": [
    {
      "code": "E001",
      "category": "counter_consistency",
      "message": "next_file_id is 1800, but maximum existing file_id is 1892",
      "repairable": true,
      "repair_action": "advance_counter",
      "repair_details": {"counter": "next_file_id", "current": 1800, "proposed": 1893}
    }
  ],
  "warnings": [
    {
      "code": "W001",
      "category": "referential_integrity",
      "message": "5 column rows reference table_id=42, which has no live version",
      "repairable": false,
      "explanation": "Consistent with table drop — no action needed"
    }
  ],
  "summary": {"errors": 1, "warnings": 7, "rows_checked": 3412}
}
```

## Repair

### Running Repair

```bash
# Always start with dry-run
slateduck repair --catalog s3://bucket/catalog/ --dry-run

# Apply repairs (requires --confirm for destructive operations)
slateduck repair --catalog s3://bucket/catalog/ --confirm

# Apply only specific repair types
slateduck repair --catalog s3://bucket/catalog/ --only counters --confirm
slateduck repair --catalog s3://bucket/catalog/ --only orphans --confirm
```

### What Repair Can Fix

| Error Type | Repair Action | Risk Level |
|-----------|---------------|------------|
| Stale counter | Advance counter to max(existing_id) + 1 | None — counters only go forward |
| Orphaned inlined rows | Delete rows referencing non-existent tables | Low — rows are unreachable |
| Orphaned statistics | Delete stats referencing non-existent files | Low — stats are unreachable |
| Dangling "latest" pointer | Remove pointer if no version exists | Low — pointer leads nowhere |

### What Repair Cannot Fix

| Error Type | Why Not | Recommended Action |
|-----------|---------|-------------------|
| Protobuf decode failure (retained row) | Row might be needed by active readers | Restore from backup |
| Magic byte mismatch | Deep corruption — data is unreadable | Restore from backup |
| Missing system keys | Catalog structure is fundamentally broken | Reinitialize from backup |
| Duplicate live versions | Cannot determine which is correct | Manual intervention |
| MVCC invariant violation (retained) | Cannot safely modify rows in retention window | Restore from backup |

When repair encounters an unfixable error, it logs the error and continues checking other keys. It never aborts mid-repair (partial repairs are still useful).

### Repair Output

```
SlateDuck Catalog Repair (DRY RUN)
═══════════════════════════════════════════════════════════════

Proposed repairs:

  [R001] Advance counter next_file_id: 1800 → 1893
    Reason: Prevents future duplicate ID allocation
    Risk: None (counter advancement is always safe)

  [R002] Delete 2 orphaned inlined insert rows
    Keys: 0xFD|01|0063|..., 0xFD|01|0063|...
    Reason: Reference non-existent table_id=99
    Risk: Low (rows are unreachable by any query)

Summary:
  Repairs proposed:   2
  Estimated duration: < 1 second
  
To apply these repairs, run without --dry-run and with --confirm.
═══════════════════════════════════════════════════════════════
```

### After Repair

After applying repairs, always re-run verify to confirm the catalog is clean:

```bash
slateduck repair --catalog s3://bucket/catalog/ --confirm
slateduck verify --catalog s3://bucket/catalog/
```

Expected output after successful repair:

```
Summary:
  Errors:    0
  Warnings:  5  (informational only)
```

## Safety Principles

Repair follows strict safety principles:

1. **Never delete data within the retention window.** If a row's `created_snapshot_id` is >= `retain_from`, repair will not touch it — even if it appears orphaned. A reader at that snapshot might need it.

2. **Never modify rows that might be visible.** If there is any chance a valid reader (at any accessible snapshot) could see the row, repair leaves it alone.

3. **Prefer no-ops over guesses.** When repair cannot determine the correct action with certainty, it reports the issue and recommends manual intervention.

4. **All repairs are logged.** Every modification made by repair is recorded in the audit trail (same as excision).

5. **Dry-run is the default mindset.** Repair without `--confirm` does nothing. With `--confirm`, it applies only the repairs that are provably safe.

## Common Scenarios

### After Power Loss / OOM Kill

```bash
# SlateDB's WAL ensures consistency, but verify anyway
slateduck verify --catalog s3://bucket/catalog/

# Usually clean — SlateDB handles crash recovery
# If errors found, repair counters (most common issue)
slateduck repair --catalog s3://bucket/catalog/ --only counters --confirm
```

### After Failed Upgrade

```bash
# Check if the migration left the catalog in a consistent state
slateduck verify --catalog s3://bucket/catalog/

# If format_version was updated but data wasn't fully migrated:
# Restore from pre-upgrade backup
slateduck import --catalog s3://bucket/catalog-restored/ --input pre-upgrade-backup.ndjson
```

### After Excision

```bash
# Verify that excision didn't leave orphans
slateduck verify --catalog s3://bucket/catalog/

# Expected: some warnings about "ended rows beyond retention" — normal
# Unexpected: referential integrity errors — run repair
```

### Periodic Health Check

```bash
# Weekly cron job
0 3 * * 0 slateduck verify --catalog s3://bucket/catalog/ --format json >> /var/log/slateduck-verify.json
```

## Performance

Verify reads every key-value pair in the catalog. Performance scales linearly with catalog size:

| Catalog Size | Verify Time |
|-------------|-------------|
| Small (< 100 tables) | 1–3 seconds |
| Medium (100–1,000 tables) | 3–15 seconds |
| Large (1,000–10,000 tables) | 15–60 seconds |
| Very large (10,000+ tables) | 1–5 minutes |

Repair is typically much faster than verify (it only touches the broken keys).

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Verify: no errors (warnings OK). Repair: all repairs applied successfully. |
| 1 | Verify: errors found. Repair: some repairs failed. |
| 2 | Cannot reach storage or catalog does not exist. |
| 3 | Format version mismatch. |

## Further Reading

- **[Inspect](inspect.md)** — Quick catalog state summary
- **[Excision](excision.md)** — Destructive deletion (run verify after)
- **[Backup & Restore](backup-restore.md)** — Recovery when repair cannot fix issues
- **[Troubleshooting](troubleshooting.md)** — Diagnosing the root cause of errors
