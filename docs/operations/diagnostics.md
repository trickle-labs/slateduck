# Catalog Diagnostics

The `rocklake diagnose` command produces a structured health report that covers
catalog format integrity, secondary-index consistency, snapshot gaps, and
orphan Parquet files in object storage.  It is designed to be run both manually
and as a CI gate after integration tests.

## Quick Start

```bash
# Human-readable report on a local catalog
rocklake diagnose --catalog ./my-catalog

# JSON output for CI or monitoring ingestion
rocklake diagnose --catalog ./my-catalog --json

# Include orphan-file detection against the data root
rocklake diagnose \
    --catalog s3://bucket/catalog/ \
    --data-root s3://bucket/data/
```

`rocklake diagnose` exits with code **0** when no P0 findings are present and
**1** when at least one P0 finding is found.  This makes it suitable as a CI
gate:

```yaml
# .github/workflows/ci.yml
- name: Catalog health check
  run: rocklake diagnose --catalog ./test-catalog
```

## Report Structure

A diagnostic report contains the following fields:

| Field | Description |
|-------|-------------|
| `format_version` | Catalog format version stored in SlateDB |
| `writer_epoch` | Current monotonic writer epoch |
| `latest_snapshot_id` | Most recently committed snapshot ID |
| `retain_from` | GC retain-from floor (lowest retained snapshot) |
| `schema_count` | Number of schemas in the catalog |
| `table_count` | Number of tables |
| `data_file_count` | Number of data files in the primary index |
| `secondary_index_entries_checked` | Number of primary data-file rows checked |
| `secondary_index_gaps` | Primary rows without a matching secondary-index entry |
| `orphan_files` | Parquet files in object storage not in any live snapshot |
| `snapshot_gaps` | Missing snapshot IDs between `retain_from` and `latest_snapshot_id` |
| `findings` | All P0/P1/P2 findings |
| `overall_status` | `"ok"`, `"degraded"` (P1 only), or `"critical"` (P0 present) |

### Finding Severities

| Severity | Meaning |
|----------|---------|
| **P0** | Critical — catalog may not be queryable; requires immediate action |
| **P1** | Important — degraded state; investigate and repair soon |
| **P2** | Advisory — informational; review at your convenience |

## Example Output

### Human-readable (default)

```
=== RockLake Catalog Diagnostics ===

Overall status:      OK
Format version:      1
Writer epoch:        4
Latest snapshot:     12
Retain-from:         1
Schemas:             2
Tables:              5
Data files:          18
2nd-index checked:   18 (0 gaps)
Snapshot gaps:       0
Orphan files:        0

No findings.
```

### JSON output (`--json`)

```json
{
  "format_version": 1,
  "writer_epoch": 4,
  "latest_snapshot_id": 12,
  "retain_from": 1,
  "schema_count": 2,
  "table_count": 5,
  "data_file_count": 18,
  "secondary_index_entries_checked": 18,
  "secondary_index_gaps": 0,
  "orphan_files": [],
  "snapshot_gaps": [],
  "findings": [],
  "overall_status": "ok"
}
```

## Orphan File Sweep

The companion command `rocklake sweep-orphans` lists (and optionally deletes)
Parquet files in object storage that are not referenced by any live catalog
snapshot.

```bash
# Dry-run: list orphan files without deleting
rocklake sweep-orphans \
    --catalog s3://bucket/catalog/ \
    --data-root s3://bucket/data/

# Increase grace period (default: 24 h)
rocklake sweep-orphans \
    --catalog s3://bucket/catalog/ \
    --data-root s3://bucket/data/ \
    --grace-period-hours 48

# Delete orphan files older than the grace period
rocklake sweep-orphans \
    --catalog s3://bucket/catalog/ \
    --data-root s3://bucket/data/ \
    --apply
```

### Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--catalog <path>` | required | Catalog URL |
| `--data-root <prefix>` | required | Object-store prefix to scan for Parquet files |
| `--grace-period-hours <N>` | `24` | Files younger than this are skipped |
| `--apply` | off | Actually delete orphan files; default is dry-run |

### Safety Rules

- **Default is dry-run.** Pass `--apply` explicitly to delete files.
- The grace period protects in-flight writes: a Parquet file written by an
  ongoing transaction will not be listed as an orphan until the grace period
  has elapsed and it still has no catalog entry.
- Swept files are logged at `INFO` level so deletions are auditable.

### Recommended Periodic Schedule

Run `sweep-orphans` as a weekly maintenance task after `rocklake gc apply`:

```bash
rocklake gc apply --catalog s3://bucket/catalog/
rocklake sweep-orphans \
    --catalog s3://bucket/catalog/ \
    --data-root s3://bucket/data/ \
    --grace-period-hours 48 \
    --apply
```

## CI Integration

The `rocklake diagnose` command is run as a CI gate after all integration tests:

```yaml
# .github/workflows/ci.yml
- name: Catalog diagnostics gate
  run: |
    rocklake diagnose --catalog ./test-catalog
    echo "Diagnostics: exit 0 (ok)"
```

A non-zero exit code (P0 finding) fails the CI step and blocks the release.

## Further Reading

- **[Monitoring](monitoring.md)** — Prometheus metrics and OTLP tracing
- **[Verify & Repair](verify-repair.md)** — Deep catalog verification
- **[Garbage Collection](garbage-collection.md)** — GC and retain-from management
- **[Excision](excision.md)** — Physical deletion of old facts
