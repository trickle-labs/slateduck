# Monitoring

SlateDuck exports Prometheus metrics on `:9090/metrics`.

## Key Metrics

| Metric | Alert Condition | Meaning |
|--------|----------------|---------|
| `slateduck_connections_active` | > capacity | Connection exhaustion |
| `slateduck_queries_errors_total{code="0A000"}` | Increasing | Unsupported SQL |
| `slateduck_storage_put_duration_seconds` | p99 > 5s | Write latency issue |
| `slateduck_catalog_snapshot_id` | Not increasing | Writer may be down |
| `slateduck_gc_keys_eligible` | Growing unbounded | GC not running |

## Prometheus Scrape Config

```yaml
scrape_configs:
  - job_name: slateduck
    static_configs:
      - targets: ['slateduck:9090']
```

## Alerting

```yaml
groups:
  - name: slateduck
    rules:
      - alert: SlateDuckWriterDown
        expr: rate(slateduck_catalog_snapshot_id[5m]) == 0
        for: 10m
      - alert: SlateDuckHighLatency
        expr: histogram_quantile(0.99, slateduck_query_duration_seconds_bucket) > 5
        for: 5m
```
