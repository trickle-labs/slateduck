# Monitoring

A well-monitored Rocklake deployment tells you three things at a glance: Is it healthy? Is it performing well? Is anything trending toward a problem? Rocklake exposes Prometheus-compatible metrics that give you visibility into catalog operations, resource usage, storage interactions, and session state. Combined with proper alerting, these metrics let you catch issues before they affect users.

This page covers the metrics endpoint configuration, the complete metrics catalog with explanations, alerting rules for common failure modes, Grafana dashboard setup, and integration with cloud-native monitoring services.

## Enabling Metrics

Rocklake exposes metrics in Prometheus exposition format on a configurable HTTP endpoint:

```bash
rocklake \
    --catalog s3://bucket/catalog/ \
    --bind 0.0.0.0:5432 \
    --metrics-bind 0.0.0.0:9090 \
    --metrics-path /metrics
```

Or via environment variables:

```bash
export ROCKLAKE_METRICS_BIND=0.0.0.0:9090
export ROCKLAKE_METRICS_PATH=/metrics
```

The metrics endpoint is a plain HTTP server (separate from the PG-wire listener) that responds to GET requests with the current metric values in Prometheus text format.

### Prometheus Scrape Configuration

```yaml
scrape_configs:
  - job_name: 'rocklake'
    scrape_interval: 15s
    static_configs:
      - targets: ['rocklake:9090']
    metrics_path: /metrics
```

For Kubernetes with Prometheus Operator:

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: rocklake
  namespace: rocklake
spec:
  selector:
    matchLabels:
      app.kubernetes.io/name: rocklake
  endpoints:
    - port: metrics
      interval: 15s
      path: /metrics
```

## Complete Metrics Catalog

The following metrics are emitted by `CatalogMetrics::render_prometheus()` in
`crates/rocklake-catalog/src/metrics.rs`. All are exposed in Prometheus
text format on the configured `--metrics-path` endpoint.

### Snapshot / Catalog Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `rocklake_snapshots_created_total` | Counter | Total catalog snapshots (transactions) committed |
| `rocklake_files_per_snapshot` | Gauge | Data files registered in the most recent snapshot |
| `rocklake_last_query_keys_scanned` | Gauge | SlateDB keys scanned in the last catalog query |

### Object Storage Metrics

These track interactions with the underlying object store (S3/GCS/Azure/local):

| Metric | Type | Description |
|--------|------|-------------|
| `rocklake_object_store_requests_total` | Counter | Total object-store requests issued |
| `rocklake_object_store_bytes_read_total` | Counter | Total bytes read from the object store |
| `rocklake_object_store_bytes_written_total` | Counter | Total bytes written to the object store |
| `rocklake_object_store_throttles_total` | Counter | 429/503 throttle responses from the object store |
| `rocklake_object_store_retries_total` | Counter | Retried object-store requests (transient failures) |

### Session Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `rocklake_active_sessions` | Gauge | Currently connected PG-wire clients |
| `rocklake_max_sessions` | Gauge | Maximum sessions configured via `--max-sessions` |

### Writer Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `rocklake_writer_epoch_age_ms` | Gauge | Milliseconds since the current writer epoch was acquired |

### CDC Data-Quality Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `rocklake_cdc_record_count_mismatch_total` | Counter | Times a Parquet file's scanned row count differed from catalog metadata (N-04 data-quality guard) |

## Alerting Rules

### Critical Alerts (Page Immediately)

```yaml
groups:
  - name: rocklake-critical
    rules:
      - alert: RocklakeDown
        expr: up{job="rocklake"} == 0
        for: 30s
        labels:
          severity: critical
        annotations:
          summary: "Rocklake is down"
          description: "No metrics received from Rocklake for 30 seconds"

      - alert: RocklakeSessionsExhausted
        expr: rocklake_active_sessions / rocklake_max_sessions > 0.95
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "Rocklake session capacity >95% — new connections will be rejected"
```

### Warning Alerts (Investigate Within Hours)

```yaml
      - alert: RocklakeStorageThrottling
        expr: rate(rocklake_object_store_throttles_total[5m]) > 1
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Object storage is throttling Rocklake requests"

      - alert: RocklakeHighRetryRate
        expr: rate(rocklake_object_store_retries_total[5m]) > 5
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Elevated object-store retry rate — transient failures"

      - alert: RocklakeWriterEpochStale
        expr: rocklake_writer_epoch_age_ms > 300000
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Writer epoch is more than 5 minutes old — check for stuck writer"

      - alert: RocklakeCDCMismatch
        expr: increase(rocklake_cdc_record_count_mismatch_total[1h]) > 0
        labels:
          severity: warning
        annotations:
          summary: "CDC record-count mismatch detected — Parquet file row counts differ from catalog metadata"
```

## Grafana Dashboard

### Recommended Panels

A comprehensive Rocklake dashboard includes these panels:

**Row 1: Overview**

- Current sessions (gauge) — `rocklake_active_sessions`
- Session capacity (gauge) — `rocklake_active_sessions / rocklake_max_sessions`
- Snapshots/min (graph) — `rate(rocklake_snapshots_created_total[1m])`

**Row 2: Object Storage**

- Storage requests/sec (graph) — `rate(rocklake_object_store_requests_total[1m])`
- Bytes read/written (graph) — `rate(rocklake_object_store_bytes_read_total[1m])` / `rate(rocklake_object_store_bytes_written_total[1m])`
- Throttle rate (graph) — `rate(rocklake_object_store_throttles_total[1m])`
- Retry rate (graph) — `rate(rocklake_object_store_retries_total[1m])`

**Row 3: Writer Health**

- Writer epoch age (graph) — `rocklake_writer_epoch_age_ms`
- Files per snapshot (graph) — `rocklake_files_per_snapshot`

**Row 4: Data Quality**

- CDC mismatch total (stat) — `rocklake_cdc_record_count_mismatch_total`
- Keys scanned per query (graph) — `rocklake_last_query_keys_scanned`

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
          "metric_namespace": "Rocklake",
          "metric_unit": {
            "rocklake_writer_epoch_age_ms": "Milliseconds",
            "rocklake_active_sessions": "Count"
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
  - prometheus_url: http://rocklake:9090/metrics
    namespace: rocklake
    metrics:
      - rocklake_*
```

## What "Normal" Looks Like

Understanding baseline behavior helps identify anomalies:

| Metric | Healthy Range | Concerning |
|--------|--------------|------------|
| `rocklake_object_store_throttles_total` rate | 0 | Any sustained rate |
| `rocklake_object_store_retries_total` rate | < 1/min | > 5/min |
| `rocklake_active_sessions` / `rocklake_max_sessions` | < 80% | > 95% |
| `rocklake_writer_epoch_age_ms` | < 60 000 ms | > 300 000 ms |
| `rocklake_cdc_record_count_mismatch_total` | 0 | Any increase |

## Further Reading

- **[Health Checks](health-checks.md)** — Probing operational readiness
- **[Logging](logging.md)** — Complementary diagnostic information
- **[Troubleshooting](troubleshooting.md)** — Investigating alerts
- **[Configuration](../deployment/configuration.md)** — Metrics endpoint configuration
