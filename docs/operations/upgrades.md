# Upgrades

Upgrading SlateDuck involves replacing the binary (or container image) with a newer version and, in some cases, migrating the catalog's internal format. Because SlateDuck stores all state in SlateDB on object storage, upgrades are simpler than traditional database upgrades — there is no local data directory to migrate, no WAL to replay, and no cluster coordination to manage.

This page covers version compatibility guarantees, the upgrade procedure for every deployment model, format migrations, rollback strategies, and best practices for zero-downtime upgrades.

## Version Compatibility

### Semantic Versioning

SlateDuck follows semantic versioning (MAJOR.MINOR.PATCH):

- **PATCH** (0.8.0 → 0.8.1): Bug fixes only. No format changes. Drop-in replacement.
- **MINOR** (0.8.x → 0.9.0): New features, possibly new configuration options. May include a format migration (documented in release notes).
- **MAJOR** (0.x → 1.0): Breaking changes to CLI, configuration, or wire protocol. Will include format migration.

### Format Versions

The catalog format version is a single integer stored as a system key (`sys/format_version`). It determines binary compatibility:

| Format Version | Introduced In | Description |
|---------------|---------------|-------------|
| 1 | 0.1.0 | Initial format: protobuf values, tag-prefixed keys |

**Compatibility Rules:**

- A binary that supports format version N can read/write catalogs at format version N
- A binary that supports format version N+1 can read catalogs at format version N (backward compatible)
- A binary that only supports format version N CANNOT read catalogs at format version N+1 (forward incompatible)

Once a format migration is applied, you cannot downgrade the binary below the version that introduced the new format (without restoring from backup).

### Wire Protocol Compatibility

The PostgreSQL wire protocol interface maintains backward compatibility within a major version:

- DuckDB clients built for SlateDuck 0.7.x will work with SlateDuck 0.8.x
- New protocol features are additive (new message types, new columns in results)
- Breaking wire protocol changes are reserved for major version bumps

## Pre-Upgrade Checklist

Before upgrading, complete these steps:

```bash
# 1. Check current version and catalog state
slateduck --version
slateduck inspect --catalog s3://bucket/catalog/

# 2. Read the release notes for the target version
# Pay attention to: format migrations, breaking changes, deprecations

# 3. Take a backup (critical for format migrations)
slateduck export --catalog s3://bucket/catalog/ --output pre-upgrade-backup.ndjson

# 4. Verify the backup
wc -l pre-upgrade-backup.ndjson
# Should match expected row count from inspect

# 5. Check disk space / storage quota (for migration overhead)
aws s3 ls s3://bucket/catalog/ --recursive --summarize | tail -2
```

## Upgrade Procedures

### Binary Installation

```bash
# 1. Stop SlateDuck
systemctl stop slateduck
# or: kill $(pgrep slateduck)

# 2. Replace the binary
mv /usr/local/bin/slateduck /usr/local/bin/slateduck.old
curl -L https://github.com/slateduck/slateduck/releases/download/v0.9.0/slateduck-$(uname -m)-unknown-linux-gnu -o /usr/local/bin/slateduck
chmod +x /usr/local/bin/slateduck

# 3. Start the new version
systemctl start slateduck
# or: slateduck serve --catalog s3://bucket/catalog/

# 4. Verify
slateduck --version
slateduck inspect --catalog s3://bucket/catalog/
```

### Docker

```bash
# 1. Pull the new image
docker pull ghcr.io/slateduck/slateduck:0.9.0

# 2. Stop the old container
docker stop slateduck

# 3. Start with the new image
docker run -d --name slateduck-new \
    -p 5432:5432 \
    -e SLATEDUCK_STORAGE=s3://bucket/catalog/ \
    -e AWS_REGION=us-east-1 \
    ghcr.io/slateduck/slateduck:0.9.0 \
    serve

# 4. Verify
docker logs slateduck-new | head -20

# 5. Remove old container
docker rm slateduck
docker rename slateduck-new slateduck
```

### Kubernetes

```yaml
# Update the image tag in your deployment
apiVersion: apps/v1
kind: Deployment
metadata:
  name: slateduck
spec:
  replicas: 1
  strategy:
    type: Recreate  # Ensures old pod dies before new one starts
  template:
    spec:
      containers:
        - name: slateduck
          image: ghcr.io/slateduck/slateduck:0.9.0  # Updated
```

```bash
# Apply the update
kubectl apply -f slateduck-deployment.yaml

# Watch the rollout
kubectl rollout status deployment/slateduck

# Verify
kubectl exec deployment/slateduck -- slateduck --version
kubectl exec deployment/slateduck -- slateduck inspect --catalog s3://bucket/catalog/
```

### Fly.io

```bash
# Update fly.toml with new image or rebuild
fly deploy --image ghcr.io/slateduck/slateduck:0.9.0

# Verify
fly ssh console -C "slateduck --version"
fly ssh console -C "slateduck inspect --catalog s3://bucket/catalog/"
```

## Format Migrations

When a new version introduces a format change, the migration is handled as follows:

### Automatic Migration (Default)

Most format migrations are automatic. On first startup with the new binary:

1. SlateDuck detects the old format version
2. It reads all catalog data
3. It rewrites data in the new format
4. It updates `sys/format_version`
5. Normal operation resumes

The migration is atomic — if it fails midway, the catalog remains at the old format version and can be retried.

### Manual Migration (Rare)

For complex migrations (announced in release notes):

```bash
# Run the migration tool explicitly
slateduck migrate --catalog s3://bucket/catalog/ --target-version 2

# Verify
slateduck inspect --catalog s3://bucket/catalog/
```

### Migration Duration

| Catalog Size | Typical Migration Time |
|-------------|----------------------|
| Small (< 100 tables) | < 5 seconds |
| Medium (100–1,000 tables) | 5–30 seconds |
| Large (1,000–10,000 tables) | 30 seconds – 5 minutes |
| Very large (10,000+ tables) | 5–30 minutes |

**During migration, the catalog is unavailable for reads and writes.** Plan accordingly.

## Rollback Strategies

### No Format Migration (Simple Rollback)

If the new version has the same format version as the old one:

```bash
# Simply replace the binary with the old version
systemctl stop slateduck
mv /usr/local/bin/slateduck.old /usr/local/bin/slateduck
systemctl start slateduck
```

### After Format Migration (Backup Restore)

If a format migration was applied, you cannot simply use the old binary (it will refuse with `FormatVersionMismatch`). Your options:

1. **Restore from NDJSON backup** (recommended):
   ```bash
   # Initialize a new catalog with the old binary
   slateduck import --catalog s3://bucket/catalog-restored/ --input pre-upgrade-backup.ndjson
   # Point your application to the restored catalog
   ```

2. **Object storage versioning** (advanced):
   If bucket versioning is enabled, restore the previous versions of all objects under the catalog prefix. This restores the raw SlateDB state.

3. **Continue forward** (pragmatic):
   Often, fixing the issue in the new version (configuration change, bug workaround) is simpler than rolling back.

## Zero-Downtime Upgrades

Because SlateDuck uses a single-writer architecture, true zero-downtime upgrades require careful orchestration:

### Strategy: Quick Restart

For most deployments, the simplest approach:

1. Stop the old instance
2. Start the new instance
3. Total downtime: 2–10 seconds (startup time)

DuckDB clients will receive connection errors during the gap and should retry.

### Strategy: Writer Handoff

For environments requiring minimal disruption:

1. Start the new instance pointing at the same catalog
2. The new instance claims the writer epoch (fences the old instance)
3. The old instance becomes read-only (existing queries complete)
4. Terminate the old instance after a grace period

```bash
# Start new instance on a different port
slateduck serve --catalog s3://bucket/catalog/ --bind 0.0.0.0:5433

# Verify it claimed the epoch
slateduck inspect --catalog s3://bucket/catalog/ --format json | jq '.writer_epoch'

# Update load balancer to point to new port
# ...

# Stop old instance after drain period
kill $(pgrep -f "slateduck.*5432")
```

### Strategy: Blue-Green with DNS

For production environments with DNS-based routing:

1. Deploy new version to "green" environment
2. New instance claims the catalog (fences blue)
3. Switch DNS from blue to green
4. Terminate blue after TTL expires

## Upgrade Testing

Before upgrading production, test in a staging environment:

```bash
# Copy production catalog to staging
slateduck export --catalog s3://prod-bucket/catalog/ --output prod-snapshot.ndjson
slateduck import --catalog s3://staging-bucket/catalog/ --input prod-snapshot.ndjson

# Upgrade staging
# ... (follow upgrade procedure)

# Run integration tests against staging
slateduck verify --catalog s3://staging-bucket/catalog/
# Run your application's test suite
```

## Post-Upgrade Verification

After any upgrade, run through this verification checklist:

### Immediate Checks (First 5 Minutes)

```bash
# 1. Version confirmation
slateduck --version
# Expected: the target version

# 2. Catalog health
slateduck inspect --catalog s3://bucket/catalog/
# Expected: all entity counts match pre-upgrade values

# 3. Verify connectivity
duckdb -c "ATTACH 'ducklake:postgresql://localhost:5432/slateduck' AS lake; SELECT count(*) FROM lake.information_schema.tables;"
# Expected: same table count as before

# 4. Check logs for errors
journalctl -u slateduck --since "5 minutes ago" | grep -i error
# Expected: no errors
```

### Extended Verification (First Hour)

- Monitor latency metrics — are query response times normal?
- Check that scheduled jobs (GC, backups) still trigger successfully
- Verify that DuckDB clients reconnect without intervention
- Watch for any deprecation warnings in logs that may need configuration changes

### Load Testing After Format Migration

After a format migration, run a representative workload to confirm performance characteristics have not regressed:

```bash
# Run the benchmark suite against the migrated catalog
slateduck bench --catalog s3://bucket/catalog/ --operations 1000

# Compare with baseline
# Look for: point-read latency, prefix scan throughput, write batch commit time
```

## Common Upgrade Issues

### Binary Refuses to Start After Upgrade

**Symptom:** `Error: FormatVersionMismatch { expected: 2, found: 1 }`

This means the new binary requires a format version higher than what exists on disk, and automatic migration did not run. Explicitly trigger migration:

```bash
slateduck migrate --catalog s3://bucket/catalog/ --target-version 2
```

### DuckDB Clients Get "Connection Refused" After Upgrade

**Symptom:** DuckDB `ATTACH` fails with connection errors.

Check that the new instance is listening on the expected port. Configuration file format may have changed between versions — verify bind address in logs:

```bash
journalctl -u slateduck | grep -i "listening"
```

### Performance Regression After Upgrade

**Symptom:** Query latency increased significantly after upgrade.

Common causes:

- **Compaction pending:** Format migration writes new SST files. Wait for SlateDB compaction to optimize the layout.
- **Cache cold:** The new process starts with an empty block cache. Performance normalizes after warming.
- **New feature overhead:** Check if new default-enabled features (additional statistics, telemetry) can be disabled if not needed.

### Writer Epoch Keeps Incrementing

**Symptom:** `slateduck inspect` shows rapidly increasing writer epoch.

This indicates multiple instances are competing for the writer lease — typically caused by the old deployment not being fully terminated before the new one starts. Ensure only one writer instance is running:

```bash
# Find all SlateDuck processes
pgrep -la slateduck
# Terminate any old instances
```

## Version History

| Version | Format | Notable Changes |
|---------|--------|----------------|
| 0.8.0 | 1 | Initial stable release |
| 0.7.0 | 1 | Performance improvements, new CLI commands |
| 0.6.0 | 1 | GC and excision support |
| 0.5.0 | 1 | Writer fencing, epoch-based lease |

## Further Reading

- **[Configuration](../deployment/configuration.md)** — Configuration options that may change between versions
- **[Backup & Restore](backup-restore.md)** — Pre-upgrade backup procedures
- **[Verify & Repair](verify-repair.md)** — Post-upgrade integrity verification
- **[Troubleshooting](troubleshooting.md)** — Diagnosing upgrade-related issues
