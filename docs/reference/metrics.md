# Metrics

Exported on `:9090/metrics` in Prometheus format.

## Connections

| Metric | Type |
|--------|------|
| `slateduck_connections_total` | Counter |
| `slateduck_connections_active` | Gauge |

## Queries

| Metric | Type |
|--------|------|
| `slateduck_queries_total` | Counter (by shape) |
| `slateduck_query_duration_seconds` | Histogram |
| `slateduck_queries_errors_total` | Counter (by SQLSTATE) |

## Storage

| Metric | Type |
|--------|------|
| `slateduck_storage_get_requests_total` | Counter |
| `slateduck_storage_put_requests_total` | Counter |
| `slateduck_storage_get_duration_seconds` | Histogram |
| `slateduck_storage_put_duration_seconds` | Histogram |

## Catalog

| Metric | Type |
|--------|------|
| `slateduck_catalog_snapshot_id` | Gauge |
| `slateduck_catalog_tables_count` | Gauge |
| `slateduck_catalog_files_count` | Gauge |
