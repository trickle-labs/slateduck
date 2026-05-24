# Monitoring

A well-monitored SlateDuck deployment tells you three things at a glance: Is it healthy? Is it performing well? Is anything trending toward a problem? SlateDuck exposes Prometheus-compatible metrics that give you visibility into catalog operations, resource usage, storage interactions, and session state. Combined with proper alerting, these metrics let you catch issues before they affect users.

This page covers the metrics endpoint configuration, the complete metrics catalog with explanations, alerting rules for common failure modes, Grafana dashboard setup, and integration with cloud-native monitoring services.

## Enabling Metrics

SlateDuck exposes metrics in Prometheus exposition format on a configurable HTTP endpoint:

```bash
slateduck \
    --catalog s3://bucket/catalog/ \
    --bind 0.0.0.0:5432 \
    --metrics-bind 0.0.0.0:9090 \
    --metrics-path /metrics
```

Or via environment variables:

```bash
export SLATEDUCK_METRICS_BIND=0.0.0.0:9090
export SLATEDUCK_METRICS_PATH=/metrics
```

The metrics endpoint is a plain HTTP server (separate from the PG-wire listener) that responds to GET requests with the current metric values in Prometheus text format.

### Prometheus Scrape Configuration

```yaml
scrape_configs:
  - job_name: 'slateduck'
    scrape_interval: 15s
    static_configs:
      - targets: ['slateduck:9090']
    metrics_path: /metrics
```

For Kubernetes with Prometheus Operator:

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: slateduck
  namespace: slateduck
spec:
  selector:
    matchLabels:
      app.kubernetes.io/name: slateduck
  endpoints:
    - port: metrics
      interval: 15s
      path: /metrics
```

## Complete Metrics Catalog

### Operation Metrics

These metrics track catalog operations — the business-level work SlateDuck performs:

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `slateduck_operations_total` | Counter | `type` (read, write, ddl) | Total catalog operations executed |
| `slateduck_operation_duration_seconds` | Histogram | `type` | Time per operation (buckets: 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 5.0) |
| `slateduck_snapshots_created_total` | Counter | — | Total snapshots (transactions) committed |
| `slateduck_snapshot_size_keys` | Histogram | — | Keys modified per snapshot |
| `slateduck_queries_total` | Counter | `statement_type` | SQL statements by type (SELECT, CREATE, INSERT, etc.) |
| `slateduck_query_errors_total` | Counter | `sqlstate` | Errors by SQLSTATE code |

### Object Storage Metrics

These track interactions with the underlying storage (S3/GCS/Azure):

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `slateduck_storage_requests_total` | Counter | `operation` (get, put, delete, list) | Requests to object storage |
| `slateduck_storage_request_duration_seconds` | Histogram | `operation` | Latency per storage request |
| `slateduck_storage_bytes_read_total` | Counter | — | Total bytes read from storage |
| `slateduck_storage_bytes_written_total` | Counter | — | Total bytes written to storage |
| `slateduck_storage_throttles_total` | Counter | — | 429/503 responses from storage (rate limiting) |
| `slateduck_storage_retries_total` | Counter | — | Retried requests (transient failures) |
| `slateduck_storage_errors_total` | Counter | `error_type` | Non-retryable storage errors |

### Session Metrics

These track client connections:

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `slateduck_sessions_active` | Gauge | — | Currently connected clients |
| `slateduck_sessions_max` | Gauge | — | Configured maximum sessions |
| `slateduck_sessions_total` | Counter | — | Total sessions created since start |
| `slateduck_sessions_rejected_total` | Counter | — | Sessions rejected (at capacity) |
| `slateduck_session_duration_seconds` | Histogram | — | Session lifetimes |

### Writer Metrics

These track the single-writer state:

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `slateduck_writer_epoch` | Gauge | — | Current writer epoch number |
| `slateduck_writer_epoch_acquired_timestamp` | Gauge | — | Unix timestamp when epoch was acquired |
| `slateduck_writer_active` | Gauge | — | 1 if this instance is the writer, 0 if read-only |
| `slateduck_writer_commits_total` | Counter | — | Successful commits (writes flushed to storage) |
| `slateduck_writer_commit_duration_seconds` | Histogram | — | Time per commit operation |

### Cache Metrics

These track the hot key cache effectiveness:

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `slateduck_cache_hits_total` | Counter | — | Key lookups served from cache |
| `slateduck_cache_misses_total` | Counter | — | Key lookups that required storage reads |
| `slateduck_cache_size_bytes` | Gauge | — | Current cache memory usage |
| `slateduck_cache_entries` | Gauge | — | Number of entries in cache |

### GC Metrics

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `slateduck_gc_last_run_timestamp` | Gauge | — | Unix timestamp of last GC run |
| `slateduck_gc_duration_seconds` | Gauge | — | Duration of last GC run |
| `slateduck_gc_snapshots_collected` | Gauge | — | Snapshots made inaccessible in last GC |
| `slateduck_gc_retained_snapshots` | Gauge | — | Total accessible snapshots |
| `slateduck_gc_pinned_count` | Gauge | — | Number of pinned snapshots blocking GC |

### Process Metrics

Standard process metrics (automatically exposed by Rust's prometheus crate):

| Metric | Type | Description |
|--------|------|-------------|
| `process_resident_memory_bytes` | Gauge | RSS memory usage |
| `process_cpu_seconds_total` | Counter | Total CPU time consumed |
| `process_open_fds` | Gauge | Open file descriptors |
| `process_start_time_seconds` | Gauge | Process start timestamp |

## Alerting Rules

### Critical Alerts (Page Immediately)

```yaml
groups:
  - name: slateduck-critical
    rules:
      - alert: SlateDuckDown
        expr: up{job="slateduck"} == 0
        for: 30s
        labels:
          severity: critical
        annotations:
          summary: "SlateDuck is down"
          description: "No metrics received from SlateDuck for 30 seconds"

      - alert: SlateDuckStorageUnreachable
        expr: rate(slateduck_storage_errors_total{error_type="connection"}[5m]) > 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "SlateDuck cannot reach object storage"

      - alert: SlateDuckSessionsExhausted
        expr: slateduck_sessions_active / slateduck_sessions_max > 0.95
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "SlateDuck session capacity >95% — new connections will be rejected"
```

### Warning Alerts (Investigate Within Hours)

```yaml
      - alert: SlateDuckHighStorageLatency
        expr: histogram_quantile(0.99, rate(slateduck_storage_request_duration_seconds_bucket[5m])) > 0.5
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Object storage P99 latency exceeds 500ms"

      - alert: SlateDuckStorageThrottling
        expr: rate(slateduck_storage_throttles_total[5m]) > 1
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Object storage is throttling SlateDuck requests"

      - alert: SlateDuckEpochChange
        expr: changes(slateduck_writer_epoch[1h]) > 0
        labels:
          severity: warning
        annotations:
          summary: "Writer epoch changed — failover occurred"

      - alert: SlateDuckGCStale
        expr: time() - slateduck_gc_last_run_timestamp > 172800
        labels:
          severity: warning
        annotations:
          summary: "GC has not run in 48 hours"

      - alert: SlateDuckHighErrorRate
        expr: rate(slateduck_query_errors_total[5m]) / rate(slateduck_operations_total[5m]) > 0.05
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Error rate exceeds 5% of operations"

      - alert: SlateDuckCacheMissRate
        expr: rate(slateduck_cache_misses_total[5m]) / (rate(slateduck_cache_hits_total[5m]) + rate(slateduck_cache_misses_total[5m])) > 0.5
        for: 30m
        labels:
          severity: warning
        annotations:
          summary: "Cache hit rate below 50% — performance may be degraded"
```

## Grafana Dashboard

### Recommended Panels

A comprehensive SlateDuck dashboard includes these panels:

**Row 1: Overview**

- Current sessions (gauge) — `slateduck_sessions_active`
- Operations/sec (graph) — `rate(slateduck_operations_total[1m])`
- Error rate (graph) — `rate(slateduck_query_errors_total[1m])`
- Writer epoch (stat) — `slateduck_writer_epoch`

**Row 2: Latency**

- Operation P50/P95/P99 (graph) — `histogram_quantile(0.5/0.95/0.99, ...)`
- Storage request latency (graph) — per operation type
- Commit latency (graph) — `slateduck_writer_commit_duration_seconds`

**Row 3: Storage**

- Storage requests/sec (graph) — by operation type
- Bytes read/written (graph)
- Throttle rate (graph) — `rate(slateduck_storage_throttles_total[1m])`
- Cache hit ratio (graph) — hits / (hits + misses)

**Row 4: Resources**

- Memory usage (graph) — `process_resident_memory_bytes`
- CPU usage (graph) — `rate(process_cpu_seconds_total[1m])`
- Session count over time (graph)
- GC status (stat) — time since last run

### Dashboard JSON

A pre-built Grafana dashboard JSON is available in the repository at `docs/assets/grafana-dashboard.json`. Import it into your Grafana instance:

```bash
curl -X POST http://grafana:3000/api/dashboards/db \
    -H "Content-Type: application/json" \
    -d @docs/assets/grafana-dashboard.json
```

## Cloud-Native Monitoring Integration

### AWS CloudWatch

Use the CloudWatch Agent's Prometheus scraping to forward metrics:

```json
{
  "metrics": {
    "metrics_collected": {
      "prometheus": {
        "prometheus_config_path": "/etc/cwagent/prometheus.yaml",
        "emf_processor": {
          "metric_namespace": "SlateDuck",
          "metric_unit": {
            "slateduck_operation_duration_seconds": "Seconds",
            "slateduck_sessions_active": "Count"
          }
        }
      }
    }
  }
}
```

### Google Cloud Managed Prometheus

On GKE with Managed Prometheus, the ServiceMonitor configuration works automatically — Google scrapes Prometheus endpoints and stores metrics in Cloud Monitoring.

### Datadog

Use the Datadog Agent's OpenMetrics integration:

```yaml
# datadog-agent/conf.d/openmetrics.d/conf.yaml
instances:
  - prometheus_url: http://slateduck:9090/metrics
    namespace: slateduck
    metrics:
      - slateduck_*
```

## What "Normal" Looks Like

Understanding baseline behavior helps identify anomalies:

| Metric | Healthy Range | Concerning |
|--------|--------------|------------|
| Operation latency P50 | 1–10ms | >50ms |
| Operation latency P99 | 10–50ms | >200ms |
| Storage request latency P50 | 5–20ms | >100ms |
| Cache hit ratio | >80% | <50% |
| Error rate | <0.1% | >1% |
| Sessions | <80% of max | >95% |
| Throttle rate | 0 | Any sustained |
| Memory growth | Stable | Monotonically increasing |

## Recommended Alert Rules

Based on operational experience, these alerts catch the most common issues:

```yaml
groups:
  - name: slateduck
    rules:
      - alert: SlateDuckDown
        expr: up{job="slateduck"} == 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "SlateDuck instance unreachable"

      - alert: SlateDuckHighLatency
        expr: histogram_quantile(0.99, rate(slateduck_operation_duration_seconds_bucket[5m])) > 0.2
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "P99 operation latency exceeds 200ms"

      - alert: SlateDuckHighErrorRate
        expr: rate(slateduck_operations_total{status="error"}[5m]) / rate(slateduck_operations_total[5m]) > 0.01
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Error rate exceeds 1%"

      - alert: SlateDuckSessionsNearLimit
        expr: slateduck_active_sessions / slateduck_max_sessions > 0.9
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Active sessions at 90% of maximum"

      - alert: SlateDuckWriterFenced
        expr: slateduck_writer_fenced == 1
        for: 0m
        labels:
          severity: critical
        annotations:
          summary: "Writer has been fenced — cannot process writes"
```

## Capacity Planning with Metrics

Use historical metrics to predict when resources will be exhausted:

### Storage Growth Rate

```promql
# Bytes written per day (projected)
rate(slateduck_storage_bytes_written_total[7d]) * 86400

# Days until storage quota reached (if you have a quota)
(storage_quota_bytes - slateduck_storage_bytes_total) / rate(slateduck_storage_bytes_written_total[7d])
```

### Connection Growth

```promql
# Peak sessions over the last 7 days
max_over_time(slateduck_active_sessions[7d])

# Growth trend (sessions per week)
deriv(max_over_time(slateduck_active_sessions[1d])[7d:1d])
```

### Catalog Growth

```promql
# Snapshot creation rate (commits per hour)
rate(slateduck_snapshots_created_total[1h]) * 3600

# Estimated time until GC is needed
# (If you have a policy of retaining N snapshots)
(target_retention_snapshots - (slateduck_latest_snapshot - slateduck_retain_from))
  / rate(slateduck_snapshots_created_total[1h])
```

## Grafana Dashboard Configuration

A comprehensive SlateDuck dashboard should include these panels:

### Overview Row
- **Uptime:** `time() - process_start_time_seconds`
- **Writer Status:** Single stat showing "Active" (green) or "Fenced" (red)
- **Latest Snapshot:** Current snapshot ID with rate of change
- **Active Sessions:** Gauge with max capacity reference line

### Performance Row
- **Operation Latency (P50/P95/P99):** Time series with heatmap
- **Operations per Second:** Stacked by operation type (read/write/scan)
- **Cache Hit Ratio:** Gauge targeting >80%
- **Storage Request Latency:** Time series showing S3/GCS round-trip times

### Resource Row
- **Memory Usage:** Process RSS with limit overlay
- **CPU Usage:** User/system time
- **Storage Throughput:** Bytes read/written per second
- **Network I/O:** In/out bytes (useful for identifying bandwidth bottlenecks)

### Health Row
- **Error Rate:** Percentage time series
- **Session Utilization:** Active vs max capacity
- **GC Status:** Last GC time, rows collected
- **Writer Epoch:** Step graph showing ownership changes

## Further Reading

- **[Health Checks](health-checks.md)** — Probing operational readiness
- **[Logging](logging.md)** — Complementary diagnostic information
- **[Troubleshooting](troubleshooting.md)** — Investigating alerts
- **[Configuration](../deployment/configuration.md)** — Metrics endpoint configuration
