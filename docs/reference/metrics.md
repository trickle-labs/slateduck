# Metrics Reference

This page documents all Prometheus metrics exposed by Rocklake's metrics endpoint. When enabled (via `ROCKLAKE_METRICS_BIND`), Rocklake serves metrics in Prometheus exposition format at the `/metrics` path. These metrics provide comprehensive observability into catalog operations, storage performance, caching behavior, and system health.

Monitoring is essential for production deployments. These metrics tell you whether Rocklake is healthy, whether performance is within expectations, whether storage costs are growing, and whether capacity planning assumptions hold. Each metric includes its type (counter, gauge, histogram), labels, description, and guidance on what values are normal and what values indicate problems.

## Metric Types

| Type | Description | Example Use |
|------|-------------|-------------|
| **Counter** | Monotonically increasing value. Only goes up. | Total operations, total bytes transferred |
| **Gauge** | Current value that can go up or down. | Active sessions, cache size |
| **Histogram** | Distribution of observed values in configurable buckets. | Latency percentiles, batch sizes |

## Endpoint Configuration

```bash
# Enable metrics endpoint
ROCKLAKE_METRICS_BIND=0.0.0.0:9090
```

Once enabled, metrics are available at `http://<host>:9090/metrics`. The response is in Prometheus exposition format, compatible with Prometheus, Grafana Agent, Victoria Metrics, Datadog, and other Prometheus-compatible scrapers.

**Scrape configuration (Prometheus):**

```yaml
scrape_configs:
  - job_name: rocklake
    static_configs:
      - targets: ['rocklake:9090']
    scrape_interval: 15s
```

---

## Operation Metrics

These metrics track catalog operations — the core business logic of Rocklake.

### rocklake_operations_total

**Type:** Counter

Total number of catalog operations completed, labeled by operation type.

| Label | Values | Description |
|-------|--------|-------------|
| `operation` | `create_schema`, `create_table`, `create_column`, `drop_schema`, `drop_table`, `drop_column`, `rename_schema`, `rename_table`, `rename_column`, `register_data_file`, `register_delete_file`, `list_schemas`, `list_tables`, `list_columns`, `list_data_files`, `get_column_stats`, `commit`, `rollback` | The operation type |

**Example queries:**

```promql
# Operations per second (rate over 5 minutes)
rate(rocklake_operations_total[5m])

# Write operations vs read operations
sum(rate(rocklake_operations_total{operation=~"create_.*|drop_.*|rename_.*|register_.*"}[5m]))
sum(rate(rocklake_operations_total{operation=~"list_.*|get_.*"}[5m]))

# Most frequent operation type
topk(5, sum by (operation) (rate(rocklake_operations_total[5m])))
```

**Normal values:** Depends entirely on workload. A typical analytical workload produces 10–100 operations/second during active ingestion, near zero during idle periods.

---

### rocklake_operation_duration_seconds

**Type:** Histogram

Latency distribution of catalog operations, labeled by operation type.

| Label | Values |
|-------|--------|
| `operation` | Same as `operations_total` |

**Buckets:** 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0 seconds

**Example queries:**

```promql
# P99 latency for all operations
histogram_quantile(0.99, rate(rocklake_operation_duration_seconds_bucket[5m]))

# P50 latency by operation type
histogram_quantile(0.50, sum by (le, operation) (rate(rocklake_operation_duration_seconds_bucket[5m])))

# Operations slower than 100ms
sum(rate(rocklake_operation_duration_seconds_bucket{le="0.1"}[5m]))
```

**Normal values:**
- Hot-cache reads: < 1ms (P99)
- Cold reads (cache miss): 5–50ms depending on storage latency
- Writes: 10–100ms (dominated by WAL PUT latency)
- If P99 exceeds 500ms, investigate storage performance

---

### rocklake_snapshots_created_total

**Type:** Counter

Total number of snapshots (committed transactions) created since process start.

**Example queries:**

```promql
# Snapshots per minute
rate(rocklake_snapshots_created_total[5m]) * 60

# Total snapshots in the last hour
increase(rocklake_snapshots_created_total[1h])
```

**Normal values:** One snapshot per write transaction. A busy catalog might create 1–10 snapshots per second during bulk operations.

---

### rocklake_files_per_snapshot

**Type:** Gauge

Number of data files registered in the latest snapshot. Indicates the "width" of the catalog.

**Normal values:** Varies by workload. A table with daily Parquet partitions accumulates ~365 files per year per table.

**Alert threshold:** If this grows unexpectedly fast, check whether data ingestion is creating many small files (which hurts scan performance).

---

## Object Store Metrics

These metrics track interactions with the underlying object storage (S3, GCS, Azure).

### rocklake_object_store_requests_total

**Type:** Counter

Total object storage requests by HTTP method.

| Label | Values |
|-------|--------|
| `method` | `GET`, `PUT`, `DELETE`, `HEAD`, `LIST` |

**Example queries:**

```promql
# Total requests per second
sum(rate(rocklake_object_store_requests_total[5m]))

# PUT vs GET ratio (write amplification indicator)
rate(rocklake_object_store_requests_total{method="PUT"}[5m])
  / rate(rocklake_object_store_requests_total{method="GET"}[5m])

# Cost estimation (approximate S3 costs)
increase(rocklake_object_store_requests_total{method="PUT"}[24h]) * 0.000005
  + increase(rocklake_object_store_requests_total{method="GET"}[24h]) * 0.0000004
```

---

### rocklake_object_store_request_duration_seconds

**Type:** Histogram

Object storage request latency by method.

| Label | Values |
|-------|--------|
| `method` | `GET`, `PUT`, `DELETE`, `HEAD`, `LIST` |

**Buckets:** 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0 seconds

**Normal values:**
- Same-region S3: P50 = 10–30ms, P99 = 50–200ms
- Cross-region S3: P50 = 50–150ms, P99 = 200–1000ms
- S3 Express: P50 = 2–5ms, P99 = 10–30ms
- Local filesystem: P50 < 1ms

---

### rocklake_object_store_bytes_read_total

**Type:** Counter

Total bytes read from object storage since process start.

**Example queries:**

```promql
# Read throughput (MB/s)
rate(rocklake_object_store_bytes_read_total[5m]) / 1048576

# Total data read in the last 24h (for cost estimation)
increase(rocklake_object_store_bytes_read_total[24h])
```

---

### rocklake_object_store_bytes_written_total

**Type:** Counter

Total bytes written to object storage since process start.

---

### rocklake_object_store_throttles_total

**Type:** Counter

Number of HTTP 429 (Too Many Requests) or 503 (Service Unavailable) responses from storage.

**Normal values:** Should be 0 or near-zero in normal operation. Non-zero values indicate storage throttling.

**Alert threshold:** > 0 sustained over 5 minutes. Investigate storage tier limits or request rate.

---

### rocklake_object_store_retries_total

**Type:** Counter

Number of retried storage requests (after transient failures).

**Normal values:** Occasional retries are normal (network jitter). Sustained retries indicate storage issues.

---

## Cache Metrics

### rocklake_cache_hits_total

**Type:** Counter

Total cache hits (hot key cache + SlateDB block cache combined).

**Example queries:**

```promql
# Cache hit ratio
rate(rocklake_cache_hits_total[5m])
  / (rate(rocklake_cache_hits_total[5m]) + rate(rocklake_cache_misses_total[5m]))
```

**Normal values:** Hit ratio > 90% indicates healthy caching. Below 80% suggests the cache is too small for the working set.

---

### rocklake_cache_misses_total

**Type:** Counter

Total cache misses requiring a fetch from object storage.

---

### rocklake_cache_size_bytes

**Type:** Gauge

Current memory usage of the block cache in bytes.

**Normal values:** Should approach `ROCKLAKE_CACHE_SIZE_MB * 1048576` under load. If significantly below the configured maximum, the working set fits entirely in cache (good).

---

## Session Metrics

### rocklake_active_sessions

**Type:** Gauge

Number of currently connected client sessions.

**Alert threshold:** When approaching `ROCKLAKE_MAX_SESSIONS`, new connections will be rejected.

---

### rocklake_max_sessions

**Type:** Gauge

The configured session limit (from `ROCKLAKE_MAX_SESSIONS`).

---

### rocklake_sessions_total

**Type:** Counter

Total sessions created since process start (cumulative).

---

## Writer Metrics

### rocklake_writer_epoch

**Type:** Gauge

Current writer epoch. This value increments each time a new writer takes over.

**Alert threshold:** If this changes unexpectedly, it means a new writer started (possibly due to a restart or deployment). This is informational, not necessarily an error.

---

### rocklake_write_batch_size

**Type:** Histogram

Number of key-value mutations per committed write batch.

**Buckets:** 1, 5, 10, 25, 50, 100, 250, 500, 1000, 5000

**Normal values:** Creating a table with 10 columns produces a batch of ~12 keys (1 table + 10 columns + 1 snapshot).

---

### rocklake_last_query_keys_scanned

**Type:** Gauge

Number of keys scanned in the most recent read query. Useful for detecting expensive queries.

---

### rocklake_mean_rows_scanned

**Type:** Gauge

Rolling average of rows scanned per read operation.

---

## Catalog Metrics

### rocklake_schemas_count

**Type:** Gauge

Number of live (non-superseded) schemas in the catalog.

---

### rocklake_tables_count

**Type:** Gauge

Number of live tables across all schemas.

---

### rocklake_latest_snapshot_id

**Type:** Gauge

The highest committed snapshot ID. Useful for monitoring ingestion progress.

---

### rocklake_retain_from

**Type:** Gauge

Current GC retention horizon. Snapshots below this value are no longer accessible via time travel.

---

## Metric Naming Conventions

All metrics follow Prometheus naming best practices:

- **Prefix:** `rocklake_` (distinguishes from other services' metrics)
- **Suffix conventions:**
    - `_total` for counters
    - `_seconds` for durations
    - `_bytes` for sizes
    - `_count` for quantities (gauges)
- **Labels:** lowercase with underscores, short but descriptive

## Recommended Alerts

| Alert | Condition | Severity |
|-------|-----------|----------|
| High latency | P99 operation duration > 1s for 5min | Warning |
| Storage throttling | `throttles_total` rate > 0 for 5min | Warning |
| Cache hit ratio low | Hit ratio < 70% for 15min | Warning |
| Sessions near limit | `active_sessions` > 80% of `max_sessions` | Warning |
| Writer epoch change | `writer_epoch` changed | Info |
| Internal errors | `operations_total{status="error"}` > 0 | Critical |
| Storage bytes growing fast | `bytes_written_total` rate > 10MB/s for 1h | Warning |
| Snapshot ID stale | `latest_snapshot_id` unchanged for 1h during expected activity | Warning |

### Alert Rule Examples (Prometheus)

```yaml
groups:
  - name: rocklake
    rules:
      - alert: RocklakeHighLatency
        expr: histogram_quantile(0.99, rate(rocklake_operation_duration_seconds_bucket[5m])) > 1
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Rocklake P99 latency exceeds 1 second"
          description: "Operation latency has been above 1s for 5 minutes. Check storage performance."

      - alert: RocklakeStorageThrottled
        expr: rate(rocklake_object_store_throttles_total[5m]) > 0
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Object storage is throttling Rocklake requests"
          description: "Sustained 429/503 responses from storage. Consider S3 Express or request limit increase."

      - alert: RocklakeCacheMissRate
        expr: |
          rate(rocklake_cache_misses_total[5m]) /
          (rate(rocklake_cache_hits_total[5m]) + rate(rocklake_cache_misses_total[5m])) > 0.3
        for: 15m
        labels:
          severity: warning
        annotations:
          summary: "Rocklake cache miss rate above 30%"
          description: "Working set may exceed cache size. Consider increasing ROCKLAKE_CACHE_SIZE_MB."

      - alert: RocklakeSessionsNearLimit
        expr: rocklake_active_sessions / rocklake_max_sessions > 0.8
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "Rocklake approaching session limit"
          description: "Active sessions are above 80% of maximum. New connections may be rejected soon."
```

## Grafana Dashboard Configuration

A recommended Grafana dashboard for Rocklake should include these panels:

### Overview Row

| Panel | Type | Query |
|-------|------|-------|
| Operations/sec | Stat | `sum(rate(rocklake_operations_total[5m]))` |
| Active Sessions | Stat | `rocklake_active_sessions` |
| Cache Hit Ratio | Stat | `rate(rocklake_cache_hits_total[5m]) / (rate(rocklake_cache_hits_total[5m]) + rate(rocklake_cache_misses_total[5m]))` |
| Latest Snapshot | Stat | `rocklake_latest_snapshot_id` |
| Writer Epoch | Stat | `rocklake_writer_epoch` |

### Latency Row

| Panel | Type | Query |
|-------|------|-------|
| Operation Latency (P50/P99) | Time series | `histogram_quantile(0.5, ...)` and `histogram_quantile(0.99, ...)` |
| Storage Latency by Method | Time series | `histogram_quantile(0.99, sum by (le, method) (rate(rocklake_object_store_request_duration_seconds_bucket[5m])))` |

### Storage Row

| Panel | Type | Query |
|-------|------|-------|
| Requests/sec by Method | Time series (stacked) | `sum by (method) (rate(rocklake_object_store_requests_total[5m]))` |
| Bytes Read/Written | Time series | `rate(rocklake_object_store_bytes_read_total[5m])` and `rate(...)_written_...` |
| Throttles | Time series | `rate(rocklake_object_store_throttles_total[5m])` |
| Retries | Time series | `rate(rocklake_object_store_retries_total[5m])` |

### Catalog Row

| Panel | Type | Query |
|-------|------|-------|
| Schemas | Stat | `rocklake_schemas_count` |
| Tables | Stat | `rocklake_tables_count` |
| Retention Horizon | Stat | `rocklake_retain_from` |
| Write Batch Size Distribution | Histogram | `rocklake_write_batch_size` |

## Interpreting Metrics for Capacity Planning

### Storage Cost Estimation

Use the object store metrics to estimate monthly storage costs:

```promql
# Estimated monthly S3 Standard costs (us-east-1 pricing)
# PUT/POST requests: $0.005 per 1000
(increase(rocklake_object_store_requests_total{method="PUT"}[30d]) / 1000) * 0.005
# GET requests: $0.0004 per 1000
+ (increase(rocklake_object_store_requests_total{method="GET"}[30d]) / 1000) * 0.0004
```

### Working Set Estimation

If the cache hit ratio is below 90%, calculate the required cache size:

```promql
# Approximate working set size (bytes)
# = cache_size_bytes / cache_hit_ratio
rocklake_cache_size_bytes / (
  rate(rocklake_cache_hits_total[1h]) /
  (rate(rocklake_cache_hits_total[1h]) + rate(rocklake_cache_misses_total[1h]))
)
```

### Connection Pool Sizing

Use session metrics to right-size connection pools:

```promql
# Peak concurrent sessions over the last week
max_over_time(rocklake_active_sessions[7d])

# Average utilization
avg_over_time(rocklake_active_sessions[7d]) / rocklake_max_sessions
```

## Further Reading

- **[Operations: Monitoring](../operations/monitoring.md)** — Setting up monitoring dashboards
- **[Operations: Health Checks](../operations/health-checks.md)** — Liveness and readiness probes
- **[Performance: Tuning](../performance/tuning.md)** — Using metrics to guide tuning decisions
