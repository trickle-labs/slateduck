# SQL Reference: Incremental Materialized View DDL

## `CREATE INCREMENTAL MATERIALIZED VIEW`

```
CREATE INCREMENTAL MATERIALIZED VIEW [<schema>.]<name>
AS <select_statement>
[WITH (<option> = <value> [, ...])]
```

Creates a new incremental materialized view. The view is registered in the catalog and an empty output table is created. An IVM worker must be running to populate the view.

### Parameters

| Parameter | Description |
|---|---|
| `schema` | Schema name. Defaults to `main` if omitted. |
| `name` | View name. Must be unique within the schema. |
| `select_statement` | A `SELECT ... FROM ... GROUP BY ...` query. |

### WITH Options (v0.11)

| Option | Default | Description |
|---|---|---|
| `shard_count` | `1` | Number of shards (parallel workers). |
| `freshness_target_ms` | `5000` | Target lag in milliseconds before status is set to `Stale`. |

### Supported SELECT Shape

```sql
SELECT <group_by_col> [, ...], <aggregate> [, ...]
FROM <base_table>
GROUP BY <group_by_col> [, ...]
```

Supported aggregate functions:

| Function | Output Type | Notes |
|---|---|---|
| `COUNT(*)` | `i64` | Counts rows in the group. |
| `SUM(col)` | `i64` or `f64` | Sum of the column values. |
| `MIN(col)` | `i64` or `f64` | Minimum value (uses BTreeMap-based tracking). |
| `MAX(col)` | `i64` or `f64` | Maximum value (uses BTreeMap-based tracking). |

### Errors

| Error | Cause |
|---|---|
| `DuplicateName` | A matview with the same schema.name already exists. |
| `ParseError` | The SELECT statement cannot be parsed or is an unsupported shape. |
| `BaseTableNotFound` | The base table referenced in FROM does not exist. |

### Example

```sql
CREATE INCREMENTAL MATERIALIZED VIEW analytics.region_counts
AS SELECT region, COUNT(*) AS cnt, SUM(revenue) AS total
FROM sales
GROUP BY region;
```

---

## `DROP INCREMENTAL MATERIALIZED VIEW`

```
DROP INCREMENTAL MATERIALIZED VIEW [IF EXISTS] [<schema>.]<name>
```

Logically deletes the matview. The output table is not dropped; it remains readable at historical snapshots.

### Parameters

| Parameter | Description |
|---|---|
| `IF EXISTS` | Suppresses error if the view does not exist. |
| `schema` | Schema name. |
| `name` | View name. |

---

## `ALTER INCREMENTAL MATERIALIZED VIEW`

```
ALTER INCREMENTAL MATERIALIZED VIEW [<schema>.]<name>
SET (<option> = <value> [, ...])
```

Changes configuration options without recreating the view. Only options that do not affect the aggregation plan may be altered (e.g. `freshness_target_ms`, `shard_count`).

> **Note**: Altering `shard_count` triggers a re-sharding rebuild in a future tick.

---

## `SHOW MATERIALIZED VIEWS`

```
SHOW MATERIALIZED VIEWS
```

Lists all incremental materialized views in the catalog with their status.

Columns: `schema`, `name`, `status`, `shard_count`, `freshness_target_ms`, `created_at_snapshot`.

Status values: `Active`, `Stale`, `Rebuilding`, `Dropped`.

---

## `SHOW MATVIEW SHARDS`

```
SHOW MATVIEW SHARDS [<schema>.]<name>
```

Returns the per-shard lease state for the named matview.

Columns: `shard_id`, `owner_worker`, `lease_expires_unix_ms`, `generation`, `key_range_lo`, `key_range_hi`.

---

## `EXPLAIN MATVIEW`

```
EXPLAIN MATVIEW [<schema>.]<name>
```

Returns the parsed IVM plan as a JSON object. Useful for verifying that the view SQL was parsed correctly.

Example output:

```json
{
  "view_sql": "SELECT region, COUNT(*) AS cnt FROM sales GROUP BY region",
  "group_by_cols": ["region"],
  "aggregates": [
    { "output_col": "cnt", "kind": "Count", "input_col": null }
  ]
}
```

---

## v0.16 Operator Coverage

### Window Functions

```sql
CREATE INCREMENTAL MATERIALIZED VIEW revenue_ranked AS
SELECT dept, employee, salary,
       ROW_NUMBER() OVER (PARTITION BY dept ORDER BY salary DESC) AS rn,
       RANK() OVER (PARTITION BY dept ORDER BY salary DESC) AS rnk,
       SUM(salary) OVER (PARTITION BY dept ORDER BY hire_date
                         ROWS BETWEEN 2 PRECEDING AND CURRENT ROW) AS rolling_sum
FROM employees
GROUP BY dept, employee, salary, hire_date
WITH (window_mode = 'partitioned');
```

Supported window functions: `ROW_NUMBER`, `RANK`, `DENSE_RANK`, `PERCENT_RANK`, `CUME_DIST`, `NTILE`, `LAG`, `LEAD`, `FIRST_VALUE`, `LAST_VALUE`, `NTH_VALUE`, and aggregate windows (`SUM`, `AVG`, `COUNT`, `MIN`, `MAX` with `OVER` clause).

Supported frames: `ROWS BETWEEN`, `RANGE BETWEEN`, `GROUPS BETWEEN`.

### ORDER BY in Materialized Views

```sql
CREATE INCREMENTAL MATERIALIZED VIEW recent_orders AS
SELECT order_id, customer_id, total, order_date
FROM orders
ORDER BY order_date DESC NULLS LAST
WITH (shard_count = 1);
```

A top-level `ORDER BY` produces physically sorted Parquet output. `shard_count = 1` is auto-enforced for total-order views.

### LIMIT / OFFSET (Top-N)

```sql
CREATE INCREMENTAL MATERIALIZED VIEW top_customers AS
SELECT customer_id, SUM(total) AS lifetime_value
FROM orders
GROUP BY customer_id
ORDER BY lifetime_value DESC
LIMIT 100;
```

`LIMIT N` materializes only the top N rows. State cost is O(N). `OFFSET` is supported but state cost becomes O(OFFSET + LIMIT); `OFFSET > 10000` emits a WARN.

### Correlated Subqueries

```sql
CREATE INCREMENTAL MATERIALIZED VIEW orders_with_items AS
SELECT o.*
FROM orders o
WHERE EXISTS (
    SELECT 1 FROM lineitem l WHERE l.l_orderkey = o.o_orderkey
);
```

Supported patterns: `EXISTS`, `NOT EXISTS`, `IN (SELECT …)`, `NOT IN (SELECT …)`, scalar subquery in SELECT list. Decorrelated to semi-join / anti-join / left join + aggregation.

### Recursive CTEs

```sql
CREATE INCREMENTAL MATERIALIZED VIEW org_hierarchy AS
WITH RECURSIVE reports AS (
    SELECT emp_id, manager_id, 1 AS depth
    FROM employees WHERE manager_id IS NULL
    UNION ALL
    SELECT e.emp_id, e.manager_id, r.depth + 1
    FROM employees e JOIN reports r ON e.manager_id = r.emp_id
)
SELECT * FROM reports
WITH (shard_count = 1, max_iterations = 50);
```

Unbounded recursive CTEs require `shard_count = 1`. `max_iterations` defaults to 100.

### Non-Deterministic Functions (Capture Semantics)

```sql
CREATE INCREMENTAL MATERIALIZED VIEW events_with_ts AS
SELECT *, now() AS captured_at, gen_random_uuid() AS event_id
FROM events;
```

`now()`, `current_timestamp`, `random()`, `gen_random_uuid()` are sampled once per batch. Sampled values are stored in the checkpoint for deterministic repair/replay.

### WITH Options (v0.16)

| Option | Default | Description |
|---|---|---|
| `window_mode` | auto | `'partitioned'` or `'total_order'` |
| `cost_mode` | `'standard'` | `'adaptive'` enables automatic DIFFERENTIAL↔FULL switching |
| `adaptive_threshold` | `0.5` | Threshold for adaptive mode switching |
| `max_iterations` | `100` | Max recursion depth for recursive CTEs |

---

## v0.17: WASM UDFs and Adaptive Mode

### User-Defined Functions (WASM)

```sql
CREATE FUNCTION tokenize(input UTF8) RETURNS UTF8
LANGUAGE WASM AS '<base64-encoded-wasm-module>';

-- Use in a materialized view:
CREATE INCREMENTAL MATERIALIZED VIEW tokenized_events AS
SELECT id, tokenize(event_text) AS tokens FROM events;

-- Drop a UDF:
DROP FUNCTION tokenize;

-- Replace with new version (bumps udf_id):
ALTER FUNCTION tokenize REPLACE AS '<new-wasm-module>';

-- Migrate a view to new UDF version:
ALTER INCREMENTAL MATERIALIZED VIEW tokenized_events
USING FUNCTION tokenize VERSION 2;
```

UDF requirements:
- Must be deterministic
- No WASI imports allowed (sandboxed execution)
- Arrow-compatible scalar types only
- Per-row fuel budget: 10M instructions
- Per-batch memory limit: 64 MiB

### Adaptive Cost Mode

```sql
CREATE INCREMENTAL MATERIALIZED VIEW stats
  WITH (cost_mode = 'adaptive', adaptive_threshold = 0.3)
AS SELECT dept, COUNT(*), SUM(amount) FROM orders GROUP BY dept;
```

Switches between DIFFERENTIAL and FULL refresh automatically based on:
`Δ_rows / N_rows × complexity_multiplier > threshold`

### Reference-Counted DISTINCT (Correct Under Delete)

```sql
CREATE INCREMENTAL MATERIALIZED VIEW unique_users AS
SELECT DISTINCT user_id FROM events;
```

Internally maintains `__sd_ref_count` per row. INSERT increments, DELETE decrements.
Row is visible in output only when `ref_count > 0`.

Set operators use reference counting semantics:
- `UNION DISTINCT`: MAX(count_A, count_B)
- `INTERSECT`: MIN(count_A, count_B)
- `EXCEPT`: count_A - count_B, clamped to 0

## See Also

- [Concepts: Incremental Views](../concepts/incremental-views.md)
- [Operations Guide](../operations/incremental-materialized-views.md)
- [IVM Architecture](../architecture/ivm-plane.md)
- [Recursive CTE Spike](../design-decisions/ivm-recursive-spike.md)
- [UDF Reference](udfs.md)
- [Cost Control](../operations/ivm-cost-control.md)
