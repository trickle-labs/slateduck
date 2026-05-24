# Troubleshooting

This page provides a comprehensive guide to diagnosing and resolving problems with SlateDuck. It is organized by symptom — start with what you observe, then follow the diagnostic steps to identify root causes and apply fixes.

## Quick Diagnostic Checklist

Before diving into specific symptoms, run these three commands to establish baseline context:

```bash
# 1. Can we reach the catalog at all?
slateduck inspect --catalog s3://bucket/catalog/

# 2. What does the error log say?
slateduck logs --last 50 --level error

# 3. Is there a network/permission issue?
aws s3 ls s3://bucket/catalog/ --region us-east-1
```

If `inspect` succeeds, the catalog is healthy and the problem is likely on the client side (DuckDB connection, query configuration, or application logic). If `inspect` fails, the problem is on the storage or server side.

## Connection Errors

### "connection refused" When DuckDB Tries to Connect

**Symptom:** DuckDB reports `connection refused` when attempting to attach a DuckLake catalog via the PostgreSQL wire protocol.

**Possible Causes:**

1. SlateDuck is not running
2. SlateDuck is listening on a different address or port
3. A firewall or security group is blocking the connection
4. The DuckDB extension is using the wrong host/port

**Diagnostic Steps:**

```bash
# Is SlateDuck running?
ps aux | grep slateduck

# What address is it listening on?
ss -tlnp | grep slateduck
# or on macOS:
lsof -i -P | grep slateduck

# Can we reach the port from the client?
nc -zv <hostname> 5432

# Check SlateDuck's startup log for the actual bind address
slateduck logs --last 10 | grep "listening"
```

**Solutions:**

- If SlateDuck is not running, start it: `slateduck serve --catalog s3://bucket/catalog/`
- If it is listening on `127.0.0.1`, change to `0.0.0.0` for remote access: `--bind 0.0.0.0:5432`
- If a security group blocks port 5432, add an inbound rule for the client's IP range
- Verify the DuckDB connection string matches: `ATTACH 'dbname=ducklake host=<correct-host> port=<correct-port>' AS lake (TYPE ducklake)`

### "WriterFenced" Error (SQLSTATE 57P04)

**Symptom:** DuckDB queries fail with `WriterFenced` error. SlateDuck logs show "fenced by newer epoch."

**What This Means:**

Another SlateDuck instance has taken over the writer role by incrementing the writer epoch in SlateDB. The fenced instance can no longer write — it is permanently blocked until restarted.

**Common Causes:**

1. **Intentional failover.** You deployed a new instance and the old one was fenced. Expected behavior.
2. **Duplicate processes.** Two SlateDuck processes are pointing at the same catalog storage path.
3. **Kubernetes pod restart.** A crashed pod restarted and the new pod fenced the old one (which hadn't fully terminated yet).
4. **Misconfigured health check.** The orchestrator killed a "unhealthy" instance and started a replacement.

**Diagnostic Steps:**

```bash
# Check the current epoch
slateduck inspect --catalog s3://bucket/catalog/ --format json | jq '.writer_epoch'

# Check how many SlateDuck processes exist
ps aux | grep slateduck | grep -v grep

# In Kubernetes
kubectl get pods -l app=slateduck
```

**Solutions:**

- If this is an intentional failover: terminate the old instance. Clients will reconnect to the new one.
- If duplicate processes: kill the older one (the one with the lower epoch).
- If happening repeatedly: review your deployment configuration to ensure only ONE writer instance runs at a time. Use a Deployment with `replicas: 1` and `strategy: Recreate`.

### "FormatVersionMismatch" on Startup

**Symptom:** SlateDuck refuses to start, logging `FormatVersionMismatch: catalog requires format version 2, binary supports version 1`.

**What This Means:**

The catalog was created or migrated by a newer version of SlateDuck that uses a format version your current binary does not understand.

**Solutions:**

- **Upgrade** to the SlateDuck version that created the catalog (check release notes for format version changes)
- **If you recently downgraded:** you cannot downgrade across format version boundaries without restoring from an NDJSON backup taken before the upgrade

### "ObjectStore: 403 Forbidden"

**Symptom:** SlateDuck fails to start or intermittently fails with `ObjectStore: 403 Forbidden`.

**Cause:** Insufficient IAM permissions for the configured storage path.

**Required Permissions:**

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": [
      "s3:GetObject",
      "s3:PutObject",
      "s3:DeleteObject",
      "s3:ListBucket"
    ],
    "Resource": [
      "arn:aws:s3:::bucket",
      "arn:aws:s3:::bucket/catalog/*"
    ]
  }]
}
```

**Diagnostic Steps:**

```bash
# Test permissions directly
aws s3 ls s3://bucket/catalog/ --region us-east-1
aws s3 cp /dev/null s3://bucket/catalog/test-permissions --region us-east-1
aws s3 rm s3://bucket/catalog/test-permissions --region us-east-1

# Check which credentials are being used
aws sts get-caller-identity
```

### "ObjectStore: 429 Too Many Requests"

**Symptom:** Intermittent 429 errors in logs during heavy activity (bulk imports, compaction bursts).

**What This Means:**

S3 is throttling requests. S3 supports 3,500 PUT/COPY/POST/DELETE and 5,500 GET/HEAD requests per second per partitioned prefix.

**Solutions:**

- SlateDuck retries automatically with exponential backoff (usually self-resolving)
- If sustained: reduce concurrent readers, or restructure the catalog path to spread across prefixes
- Consider S3 Express One Zone for higher throughput

## Performance Issues

### Slow Catalog Queries (> 500ms)

**Symptom:** Queries that should be fast (SELECT * FROM a small table) take hundreds of milliseconds or seconds.

**Diagnostic Steps:**

```bash
# Check network latency to object storage
time aws s3api head-object --bucket bucket --key catalog/MANIFEST --region us-east-1

# Check row counts (high superseded count = scan amplification)
slateduck inspect --catalog s3://bucket/catalog/ --format json | jq '.counts'

# Check if GC is needed
slateduck inspect --catalog s3://bucket/catalog/ --format json | jq '.retention'
```

**Common Causes and Solutions:**

| Cause | Diagnostic Sign | Solution |
|-------|----------------|----------|
| High storage latency | `head-object` > 50ms | Use S3 Express One Zone or same-region deployment |
| Scan amplification | Many superseded rows | Run GC: `slateduck gc --retain-days 7` |
| Large table (many files) | Thousands of data files | Expected; consider table partitioning |
| Compaction backlog | Many small SST files | Wait for compaction or trigger manual compaction |
| Cold cache | First query after restart slow | Expected; subsequent queries are faster |

### DuckDB Queries Slow After Connecting

**Symptom:** DuckDB's planning phase takes seconds even for simple queries.

**What This Means:**

DuckDB's `ducklake` extension makes multiple catalog round-trips per query: list schemas, list tables, list columns, list files, get statistics. If each round-trip takes 50–100ms (typical for S3 Standard), a query with 10 catalog calls adds 500–1000ms of overhead before execution even begins.

**Solutions:**

| Approach | Latency Improvement | Trade-off |
|----------|-------------------|-----------|
| S3 Express One Zone | 5–10x faster | Higher per-request cost |
| Co-located deployment | Minimal network hops | Must deploy in same AZ |
| Native extension (Strategy C) | In-process, no network | Early-stage, fewer features |
| Fewer data files per table | Fewer catalog entries to scan | Larger individual files |

### Writer Throughput Too Low

**Symptom:** Bulk operations (registering thousands of files) take minutes.

**Diagnostic:**

```bash
# Check how many snapshots per second are being created
START=$(slateduck inspect --catalog s3://bucket/catalog/ --format json | jq '.latest_snapshot')
sleep 10
END=$(slateduck inspect --catalog s3://bucket/catalog/ --format json | jq '.latest_snapshot')
echo "Snapshots/sec: $(( (END - START) / 10 ))"
```

**Solutions:**

- Batch operations: register multiple files in a single transaction (single snapshot)
- Ensure object storage latency is low (same region, S3 Express)
- Check for lock contention if multiple writers are competing (single-writer architecture means only one can proceed)

## Data Integrity Issues

### Verify Reports Errors

**Symptom:** `slateduck verify` reports errors.

**Steps:**

```bash
# Run verify with verbose output
slateduck verify --catalog s3://bucket/catalog/ --verbose

# Preview repairs
slateduck repair --catalog s3://bucket/catalog/ --dry-run

# If repairs are available and safe, apply them
slateduck repair --catalog s3://bucket/catalog/
```

**If repair cannot fix the issue:**

1. Restore from the most recent NDJSON backup
2. If no backup: check if object storage versioning is enabled (you may recover previous SST files)
3. Contact the SlateDuck maintainers with the verify output

### Unexpected Empty Results

**Symptom:** Queries return no rows for tables/schemas that should have data.

**Diagnostic Steps:**

```bash
# What snapshot is the catalog at?
slateduck inspect --catalog s3://bucket/catalog/ --format json | jq '.latest_snapshot'

# Is the table visible at the current snapshot?
slateduck inspect --catalog s3://bucket/catalog/ --prefix "t/" | grep "table_name"

# Has GC advanced past the creation snapshot?
slateduck inspect --catalog s3://bucket/catalog/ --format json | jq '.retention.retain_from'
```

**Possible Causes:**

1. **Reading at wrong snapshot:** The client is pinned to an old snapshot before the entities existed.
2. **GC too aggressive:** `retain_from` has advanced past the target snapshot (data still exists but is inaccessible via time travel).
3. **Writer fencing:** Writes went to a different catalog instance (check storage paths match).
4. **Catalog path mismatch:** The client is connecting to a different catalog entirely.

### Snapshot ID Not Advancing

**Symptom:** The latest snapshot ID stays the same over time, even though writes should be occurring.

**Possible Causes:**

1. **Writer is fenced:** Check `slateduck inspect` for fencing status
2. **Writer has crashed:** Check process status and logs
3. **No writes happening:** The application may not be generating catalog mutations
4. **Write errors:** The writer is attempting writes but they fail (check error logs)

## Kubernetes-Specific Issues

### Pod Restart Loop (CrashLoopBackOff)

**Common Causes:**

```bash
# Check pod logs
kubectl logs -l app=slateduck --previous

# Check events
kubectl describe pod <pod-name>
```

| Log Message | Cause | Solution |
|------------|-------|----------|
| `FormatVersionMismatch` | Wrong binary version | Update container image |
| `ObjectStore: 403` | Missing IAM role | Check ServiceAccount/IRSA configuration |
| `Address already in use` | Port conflict | Check for zombie processes or conflicting services |
| `OOMKilled` | Insufficient memory | Increase memory limit in pod spec |

### Leader Election Issues

If using a Deployment with `replicas: 1`:

```bash
# Verify only one pod is running
kubectl get pods -l app=slateduck

# If multiple pods exist (during rollout), check rollout strategy
kubectl get deployment slateduck -o yaml | grep -A5 strategy
```

**Solution:** Use `strategy: Recreate` to ensure the old pod is fully terminated before the new one starts.

## Logging and Diagnostics

### Enabling Debug Logging

```bash
# Set log level
export SLATEDUCK_LOG=debug
slateduck serve --catalog s3://bucket/catalog/

# Or for specific modules
export SLATEDUCK_LOG=slateduck_pgwire=debug,slateduck_catalog=trace
```

### Useful Log Patterns to Search For

```bash
# Find all errors in the last hour
slateduck logs --since 1h --level error

# Find writer fencing events
slateduck logs | grep -i "fenced\|epoch"

# Find slow operations
slateduck logs | grep -i "slow\|timeout\|retry"

# Find permission errors
slateduck logs | grep -i "403\|forbidden\|permission"
```

## Getting Help

If the troubleshooting steps above do not resolve your issue:

1. Run `slateduck inspect --catalog <path> --format json` and save the output
2. Run `slateduck verify --catalog <path>` and save the output
3. Collect the last 100 lines of error logs
4. Open an issue on GitHub with this information

## Further Reading

- **[Inspect](inspect.md)** — Detailed catalog state examination
- **[Verify & Repair](verify-repair.md)** — Integrity checking and repair
- **[Monitoring](monitoring.md)** — Proactive issue detection
- **[Health Checks](health-checks.md)** — Automated health verification
- **[Logging](logging.md)** — Log configuration and analysis
