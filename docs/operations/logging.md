# Logging

SlateDuck uses structured logging via Rust's `tracing` crate, providing configurable verbosity levels, JSON output for machine parsing, contextual spans for correlating related log entries, and per-crate granularity for targeted debugging. Logs are your primary diagnostic tool when something goes wrong — they tell you what the system was doing, what it encountered, and how it responded.

This page covers log level configuration, structured JSON output, per-module filtering, common diagnostic scenarios, log aggregation integration, and best practices for production logging.

## Log Output Destination

SlateDuck writes all log output to **stderr** by default. It does not write to files, rotate logs, or manage log lifecycle. This is intentional — your deployment platform (Docker, systemd, Kubernetes) handles log collection and rotation. SlateDuck's responsibility is producing useful log entries; your platform's responsibility is storing and managing them.

## Log Levels

SlateDuck uses five standard log levels, each with a clear contract about what it captures:

| Level | Purpose | Volume | Production Use |
|-------|---------|--------|----------------|
| `error` | Unrecoverable failures requiring operator attention | Very low (0-1/hour normally) | Always on |
| `warn` | Recoverable issues that may indicate problems | Low (< 10/hour normally) | Always on |
| `info` | Operational milestones and state transitions | Medium (1-5/minute) | Default production level |
| `debug` | Per-operation details for troubleshooting | High (100+/minute) | Temporary, during investigation |
| `trace` | Wire-level protocol details and byte-level state | Very high (1000+/minute) | Never in production |

### What Each Level Captures

**Error** — situations where an operation cannot be completed:

```
ERROR slateduck_catalog: Writer fenced - another writer has a higher epoch (theirs: 42, ours: 41)
ERROR slateduck_pgwire: TLS handshake failed: certificate expired
ERROR slateduck_catalog: Object storage unreachable after 3 retries: connection refused
```

**Warn** — problems that were handled but indicate something is suboptimal:

```
WARN slateduck_catalog: Object storage throttled (429), retrying in 500ms (attempt 2/3)
WARN slateduck_pgwire: Client sent unsupported SSL version (TLS 1.0), rejecting
WARN slateduck_catalog: GC blocked by 3 pinned snapshots older than 90 days
```

**Info** — normal operational events that mark important state transitions:

```
INFO slateduck: SlateDuck v0.8.0 starting
INFO slateduck: Storage: s3://my-bucket/catalog/
INFO slateduck: Listening on 0.0.0.0:5432
INFO slateduck: Writer epoch acquired: 42
INFO slateduck_pgwire: Session connected: remote=10.0.1.5:52431 session_id=abc123
INFO slateduck_pgwire: Session disconnected: session_id=abc123 duration=45m
INFO slateduck_catalog: Snapshot committed: id=1502 keys_written=7 duration=12ms
INFO slateduck_catalog: GC completed: horizon advanced to snapshot 1400, 102 snapshots collected
```

**Debug** — detailed operation traces for investigating specific issues:

```
DEBUG slateduck_catalog: Prefix scan: prefix=t/1/c/ keys_found=12 cache_hits=10 storage_reads=2 duration=3ms
DEBUG slateduck_sql: Statement classified: type=SELECT table=analytics.events
DEBUG slateduck_catalog: Hot key cache miss: key="t/1/latest" fetching from storage
DEBUG slateduck_pgwire: Extended query: parse="SELECT 1" bind=[] describe=true execute=true
```

**Trace** — byte-level details (never for production):

```
TRACE slateduck_pgwire: Received bytes: [51 00 00 00 04]  (Query message, 4 bytes)
TRACE slateduck_catalog: SlateDB get: key=[116 47 49 47 108 97 116 65 115 116] -> Some(152 bytes)
TRACE slateduck_pgwire: Sending: RowDescription [name="count", oid=20, typlen=8]
```

## Configuration

### Environment Variable (RUST_LOG)

The `RUST_LOG` environment variable controls logging with fine-grained, per-module filtering:

```bash
# Standard production
RUST_LOG=info slateduck serve --catalog s3://bucket/catalog/

# Debug catalog operations only
RUST_LOG=info,slateduck_catalog=debug slateduck serve --catalog s3://bucket/catalog/

# Debug PG wire protocol handling
RUST_LOG=info,slateduck_pgwire=debug slateduck serve --catalog s3://bucket/catalog/

# Debug everything (noisy, use briefly)
RUST_LOG=debug slateduck serve --catalog s3://bucket/catalog/

# Multiple specific targets
RUST_LOG=info,slateduck_catalog=debug,slateduck_sql=debug,slateduck_pgwire=trace slateduck ...
```

### CLI Flag

```bash
slateduck serve --catalog s3://bucket/catalog/ --log-level info
```

The `--log-level` flag sets a global level. For per-module control, use `RUST_LOG`.

### Module Names

SlateDuck's log targets correspond to its crate names:

| Target | Crate | What It Logs |
|--------|-------|-------------|
| `slateduck` | Main binary | Startup, shutdown, top-level config |
| `slateduck_catalog` | slateduck-catalog | Key-value operations, MVCC, GC, snapshots |
| `slateduck_pgwire` | slateduck-pgwire | Session lifecycle, protocol messages, TLS |
| `slateduck_sql` | slateduck-sql | SQL parsing, classification, dispatch |
| `slateduck_core` | slateduck-core | Shared types, encoding, protobuf |
| `slateduck_datafusion` | slateduck-datafusion | DataFusion integration |

## Structured JSON Output

For log aggregation systems (Elasticsearch, Loki, CloudWatch Logs, Datadog, Splunk), enable JSON-formatted output:

```bash
SLATEDUCK_LOG_FORMAT=json slateduck serve --catalog s3://bucket/catalog/
```

Or with CLI:

```bash
slateduck serve --catalog s3://bucket/catalog/ --log-format json
```

### JSON Format

Each log line is a single JSON object:

```json
{
  "timestamp": "2024-03-15T14:30:22.456Z",
  "level": "INFO",
  "target": "slateduck_pgwire",
  "message": "Session connected",
  "fields": {
    "remote_addr": "10.0.1.5:52431",
    "session_id": "abc123",
    "tls": true
  },
  "span": {
    "name": "session",
    "session_id": "abc123"
  }
}
```

### Structured Fields

Key fields included automatically:

| Field | Description |
|-------|-------------|
| `timestamp` | ISO 8601 timestamp with millisecond precision |
| `level` | Log level (ERROR, WARN, INFO, DEBUG, TRACE) |
| `target` | Rust module path (crate/module) |
| `message` | Human-readable message |
| `fields` | Contextual key-value pairs specific to the event |
| `span` | Active tracing span context (for request correlation) |

### Filtering JSON Logs

```bash
# Find all errors in the last hour
cat /var/log/slateduck.json | jq 'select(.level == "ERROR")'

# Find all session events for a specific client
cat /var/log/slateduck.json | jq 'select(.fields.remote_addr == "10.0.1.5:52431")'

# Count operations by type
cat /var/log/slateduck.json | jq 'select(.target == "slateduck_sql") | .fields.statement_type' | sort | uniq -c
```

## Request Correlation

SlateDuck uses tracing spans to correlate log entries belonging to the same operation:

```json
{"timestamp":"...","level":"DEBUG","message":"Prefix scan started","span":{"session_id":"abc123","query_id":"q-789"}}
{"timestamp":"...","level":"DEBUG","message":"Cache miss, reading from storage","span":{"session_id":"abc123","query_id":"q-789"}}
{"timestamp":"...","level":"DEBUG","message":"Prefix scan completed","span":{"session_id":"abc123","query_id":"q-789"},"fields":{"keys_found":12,"duration_ms":8}}
```

The `session_id` and `query_id` span fields let you trace a single operation across all log entries it produced.

## Common Diagnostic Scenarios

### Debugging a Slow Operation

```bash
# Temporarily increase verbosity for catalog operations
RUST_LOG=info,slateduck_catalog=debug slateduck ...
```

Look for:

- `cache miss` entries (indicates cold cache, storage reads)
- `keys_scanned` counts (high numbers indicate missing indexes or full scans)
- `storage_request_duration` fields (high values indicate storage latency)

### Investigating Writer Fencing

```bash
RUST_LOG=info,slateduck_catalog=debug slateduck ...
```

Look for:

```
WARN slateduck_catalog: Epoch check failed: stored_epoch=43, our_epoch=42
ERROR slateduck_catalog: Writer fenced - shutting down
```

This means another instance has taken the writer role. Check for accidental duplicate deployments.

### Troubleshooting Connection Failures

```bash
RUST_LOG=info,slateduck_pgwire=debug slateduck ...
```

Look for:

- `TLS handshake failed` — certificate issues
- `Authentication failed` — wrong password
- `Session limit reached` — max sessions exhausted
- `Connection reset by peer` — client disconnected ungracefully

### Diagnosing Object Storage Issues

```bash
RUST_LOG=info,slateduck_catalog=debug slateduck ...
```

Look for:

- `storage throttled (429)` — rate limiting, back off
- `storage error: connection refused` — endpoint unreachable
- `storage error: access denied` — credential issues
- `retry attempt 2/3` — transient failures being handled

## Log Aggregation Integration

### Docker (JSON File Driver)

```yaml
services:
  slateduck:
    environment:
      SLATEDUCK_LOG_FORMAT: json
    logging:
      driver: json-file
      options:
        max-size: "50m"
        max-file: "5"
```

### Kubernetes (stdout → cluster log collector)

Kubernetes captures stdout/stderr automatically. Use a DaemonSet log collector (Fluentd, Fluent Bit, Vector):

```yaml
# Fluent Bit parser for SlateDuck JSON logs
[PARSER]
    Name        slateduck
    Format      json
    Time_Key    timestamp
    Time_Format %Y-%m-%dT%H:%M:%S.%LZ
```

### systemd Journal

When running under systemd, logs go to the journal automatically:

```bash
# View SlateDuck logs
journalctl -u slateduck -f

# Filter by level
journalctl -u slateduck -p err

# Export structured fields
journalctl -u slateduck -o json | jq '.MESSAGE | fromjson | select(.level == "ERROR")'
```

### AWS CloudWatch Logs

```json
{
  "logs": {
    "logs_collected": {
      "files": {
        "collect_list": [
          {
            "file_path": "/var/log/slateduck/*.json",
            "log_group_name": "/slateduck/production",
            "log_stream_name": "{instance_id}"
          }
        ]
      }
    }
  }
}
```

## Performance Impact of Logging

Logging has measurable overhead. The impact by level:

| Level | Overhead | Notes |
|-------|----------|-------|
| `error` + `warn` + `info` | <1% | Negligible, always safe |
| `debug` | 2–5% | Acceptable for short investigation |
| `trace` | 10–30% | Significant, never in production |

The overhead comes from:

1. String formatting (even if the message is not emitted — use `tracing` macros which avoid this)
2. I/O writes to stderr
3. JSON serialization (slightly more than text format)

## Best Practices

1. **Use `info` level in production.** It provides sufficient visibility without noise.
2. **Use JSON format with log aggregation.** Structured logs enable powerful filtering and alerting.
3. **Never use `trace` in production.** It produces gigabytes of output per hour.
4. **Escalate temporarily for debugging.** Change `RUST_LOG` via environment variable, investigate, then revert.
5. **Monitor log volume.** A sudden increase in log volume (especially errors/warnings) is itself a signal.
6. **Retain logs for at least 7 days.** Allows post-incident investigation.

## Common Log Patterns and What They Mean

### Startup Sequence (Healthy)

A healthy startup produces exactly this sequence:

```
INFO slateduck: SlateDuck v0.8.0 starting
INFO slateduck: Storage: s3://my-bucket/catalog/
INFO slateduck_catalog: Opening SlateDB at s3://my-bucket/catalog/
INFO slateduck_catalog: Manifest loaded: format_version=1, latest_snapshot=1247
INFO slateduck_catalog: Writer epoch acquired: 42
INFO slateduck: Listening on 0.0.0.0:5432 (TLS enabled)
INFO slateduck: Metrics endpoint on 0.0.0.0:9090/metrics
INFO slateduck: Ready to accept connections
```

If any of these lines are missing, something interrupted startup.

### Storage Throttling (Transient)

```
WARN slateduck_catalog: Storage throttled (HTTP 429), backing off 200ms
WARN slateduck_catalog: Storage throttled (HTTP 429), backing off 400ms  
INFO slateduck_catalog: Storage request succeeded after 2 retries
```

This is normal under load. S3 throttles at the prefix level. If this occurs frequently, consider using S3 Express One Zone or spreading data across multiple prefixes.

### Client Protocol Error (Non-Critical)

```
WARN slateduck_pgwire: Unrecognized SQL pattern from session abc123: "SHOW search_path"
WARN slateduck_pgwire: Responding with SQLSTATE 42601 (syntax error)
```

This means a client sent SQL that does not match any known DuckDB pattern. If the client is DuckDB and this happens, it may indicate a version incompatibility — check whether the DuckDB version is supported.

### Memory Pressure (Investigate)

```
WARN slateduck: Memory usage approaching limit: 450MB / 512MB
WARN slateduck: Evicting block cache entries to free memory
```

This suggests the instance needs more memory or the `max-sessions` limit should be reduced.

## Correlating Logs Across Components

In a production deployment with multiple components (DuckDB clients, SlateDuck server, object storage), correlating events across systems requires shared identifiers:

| Component | Correlation Key | Where to Find |
|-----------|----------------|---------------|
| DuckDB | Connection ID | Client-side connection string |
| SlateDuck | Session ID | Logged with every session event |
| Object Storage | Request ID | S3 response headers (x-amz-request-id) |
| Load Balancer | Trace ID | X-Request-ID header (if configured) |

When investigating a slow query reported by a DuckDB user:
1. Get the session ID from SlateDuck logs (correlate by timestamp and client IP)
2. Filter all logs for that session: `session_id=abc123`
3. Look for storage latency spikes in that session's operations
4. If storage was slow, use the S3 request ID from debug logs to check CloudTrail

## Further Reading

- **[Monitoring](monitoring.md)** — Metrics-based observability (complements logging)
- **[Troubleshooting](troubleshooting.md)** — Using logs to diagnose specific problems
- **[Configuration](../deployment/configuration.md)** — Log format and level settings
