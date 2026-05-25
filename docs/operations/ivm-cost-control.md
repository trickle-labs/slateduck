# IVM Cost Control

This guide covers cost management for incremental materialized views (IMVs).

## Cost Modes

SlateDuck supports five cost modes that control the trade-off between freshness
and operational cost:

| Mode | Freshness | S3 Costs | Use Case |
|------|-----------|----------|----------|
| `standard` | Default | Moderate | General workloads |
| `spot` | Variable | Low | Non-critical analytics |
| `conservative` | Relaxed | Minimal | Cost-sensitive environments |
| `balanced` | Moderate | Moderate | Production defaults |
| `latency` | Aggressive | Higher | Real-time dashboards |

## Configuration

Set the cost mode at view creation time:

```sql
CREATE INCREMENTAL MATERIALIZED VIEW revenue_by_dept
  WITH (cost_mode = 'balanced', freshness = '5m')
AS SELECT dept, SUM(amount) FROM orders GROUP BY dept;
```

## Cost Budget

Set a monthly cost budget to prevent runaway spending:

```sql
ALTER MATERIALIZED VIEW revenue_by_dept
  SET (cost_budget_monthly_usd = 50.0);
```

When the budget is approached, SlateDuck automatically degrades freshness
proportionally to stay within budget.

## EXPLAIN MATERIALIZED VIEW

Inspect cost estimates for a view:

```sql
EXPLAIN MATERIALIZED VIEW revenue_by_dept;
```

Output includes:
- Estimated monthly S3 PUT/GET costs
- Flush frequency and coalescing ratio
- Change-buffer compaction effectiveness
- Predicted vs actual cost over last 7 days

## Cost Alerts

SlateDuck emits cost alerts at three levels:

- **Info**: Cost trending above estimate (>120% of budget)
- **Warning**: Cost at 80% of monthly budget
- **Critical**: Cost exceeded monthly budget

Alerts are surfaced through:
- `SHOW MATERIALIZED VIEWS` output
- Prometheus metrics (`slateduck_ivm_cost_monthly_usd`)
- Doctor report (`slateduck-ivm doctor`)

## Freshness Degradation

When cost approaches the budget limit, freshness is widened:

```
effective_freshness = base_freshness × (1 + overshoot_ratio)
```

This ensures the view continues to be maintained (never stops entirely)
while reducing S3 operation frequency.

## Monitoring

Key metrics for cost monitoring:

```
slateduck_ivm_s3_puts_total
slateduck_ivm_s3_gets_total
slateduck_ivm_flush_coalesce_ratio
slateduck_ivm_cost_estimate_monthly_usd
slateduck_ivm_cost_budget_remaining_usd
```

## Adaptive Mode (v0.17)

The `adaptive` cost mode automatically switches between DIFFERENTIAL and FULL
refresh based on the delta ratio and query complexity:

```sql
CREATE INCREMENTAL MATERIALIZED VIEW complex_view
  WITH (cost_mode = 'adaptive', adaptive_threshold = 0.3)
AS SELECT ...;
```

### How it works

At each refresh cycle, the engine computes:

```
switch_score = (Δ_rows / N_rows) × complexity_multiplier
```

When `switch_score > threshold` (default 0.5), the engine switches from
DIFFERENTIAL to FULL refresh for that cycle.

### Complexity Multiplier Table

| Operator Class | Multiplier | Crossover Ratio |
|---|---|---|
| Scan | 1.0× | ~50% |
| Filter | 1.1× | ~45% |
| Aggregate | 1.5× | ~35% |
| Join | 2.5× | ~22% |
| JoinAggregate | 4.0× | ~15% |
| Window | 3.0× | ~18% |
| Recursive | 5.0× | ~12% |

These multipliers were empirically calibrated against TPC-H Q1/Q3/Q5 and TPC-DS
Q4/Q47. See `benchmarks/v0.17-adaptive-calibration.json` for full data.

### Per-view Rolling Statistics

The adaptive mode tracks per-view statistics:

- `rows_in`: Total input rows processed
- `rows_out`: Total output rows emitted
- `ms_spent`: Total compute time
- `last_full_cost`: Last FULL refresh latency
- `smoothed_delta_ratio`: Exponentially-smoothed delta ratio (α=0.3)

These are surfaced via the observability module and can be queried:

```sql
SELECT * FROM slateduck_view_stats WHERE matview_id = 42;
```
