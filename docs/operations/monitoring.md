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

The following metrics are emitted by `CatalogMetrics::render_prometheus()` in
`crates/slateduck-catalog/src/metrics.rs`. All are exposed in Prometheus
text format on the configured `--metrics-path` endpoint.

### Snapshot / Catalog Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `slateduck_snapshots_created_total` | Counter | Total catalog snapshots (transactions) committed |
| `slateduck_files_per_snapshot` | Gauge | Data files registered in the most recent snapshot |
| `slateduck_last_query_keys_scanned` | Gauge | SlateDB keys scanned in the last catalog query |

### Object Storage Metrics

These track interactions with the underlying object store (S3/GCS/Azure/local):

| Metric | Type | Description |
|--------|------|-------------|
| `slateduck_object_store_requests_total` | Counter | Total object-store requests issued |
| `slateduck_object_store_bytes_read_total` | Counter | Total bytes read from the object store |
| `slateduck_object_store_bytes_written_total` | Counter | Total bytes written to the object store |
| `slateduck_object_store_throttles_total` | Counter | 429/503 throttle responses from the object store |
| `slateduck_object_store_retries_total` | Counter | Retried object-store requests (transient failures) |

### Session Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `slateduck_active_sessions` | Gauge | Currently connected PG-wire clients |
| `slateduck_max_sessions` | Gauge | Maximum sessions configured via `--max-sessions` |

### Writer Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `slateduck_writer_epoch_age_ms` | Gauge | Milliseconds since the current writer epoch was acquired |

### CDC Data-Quality Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `slateduck_cdc_record_count_mismatch_total` | Counter | Times a Parquet file's scanned row count differed from catalog metadata (N-04 data-quality guard) |

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

      - alert: SlateDuckSessionsExhausted
        expr: slateduck_active_sessions / slateduck_max_sessions > 0.95
        for: 5m
        labels:
          severity: critical
        annotations:
          summary: "SlateDuck session capacity >95% — new connections will be rejected"
```

### Warning Alerts (Investigate Within Hours)

```yaml
      - alert: SlateDuckStorageThrottling
        expr: rate(slateduck_object_store_throttles_total[5m]) > 1
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Object storage is throttling SlateDuck requests"

      - alert: SlateDuckHighRetryRate
        expr: rate(slateduck_object_store_retries_total[5m]) > 5
        for: 10m
        labels:
          severity: warning
        annotations:
          summary: "Elevated object-store retry rate — transient failures"

      - alert: SlateDuckWriterEpochStale
        expr: slateduck_writer_epoch_age_ms > 300000
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Writer epoch is more than 5 minutes old — check for stuck writer"

      - alert: SlateDuckCDCMismatch
        expr: increase(slateduck_cdc_record_count_mismatch_total[1h]) > 0
        labels:
          severity: warning
        annotations:
          summary: "CDC record-count mismatch detected — Parquet file row counts differ from catalog metadata"
```

## Grafana Dashboard

### Recommended Panels

A comprehensive SlateDuck dashboard includes these panels:

**Row 1: Overview**

- Current sessions (gauge) — `slateduck_active_sessions`
- Session capacity (gauge) — `slateduck_active_sessions / slateduck_max_sessions`
- Snapshots/min (graph) — `rate(slateduck_snapshots_created_total[1m])`

**Row 2: Object Storage**

- Storage requests/sec (graph) — `rate(slateduck_object_store_requests_total[1m])`
- Bytes read/written (graph) — `rate(slateduck_object_store_bytes_read_total[1m])` / `rate(slateduck_object_store_bytes_written_total[1m])`
- Throttle rate (graph) — `rate(slateduck_object_store_throttles_total[1m])`
- Retry rate (graph) — `rate(slateduck_object_store_retries_total[1m])`

**Row 3: Writer Health**

- Writer epoch age (graph) — `slateduck_writer_epoch_age_ms`
- Files per snapshot (graph) — `slateduck_files_per_snapshot`

**Row 4: Data Quality**

- CDC mismatch total (stat) — `slateduck_cdc_record_count_mismatch_total`
- Keys scanned per query (graph) — `slateduck_last_query_keys_scanned`

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
            "slateduck_writer_epoch_age_ms": "Milliseconds",
            "slateduck_active_sessions": "Count"
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
| `slateduck_object_store_throttles_total` rate | 0 | Any sustained rate |
| `slateduck_object_store_retries_total` rate | < 1/min | > 5/min |
| `slateduck_active_sessions` / `slateduck_max_sessions` | < 80% | > 95% |
| `slateduck_writer_epoch_age_ms` | < 60 000 ms | > 300 000 ms |
| `slateduck_cdc_record_count_mismatch_total` | 0 | Any increase |

## Further Reading

- **[Health Checks](health-checks.md)** — Probing operational readiness
- **[Logging](logging.md)** — Complementary diagnostic information
- **[Troubleshooting](troubleshooting.md)** — Investigating alerts
- **[Configuration](../deployment/configuration.md)** — Metrics endpoint configuration
