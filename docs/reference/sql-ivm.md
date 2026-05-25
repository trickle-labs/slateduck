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

## See Also

- [Concepts: Incremental Views](../concepts/incremental-views.md)
- [Operations Guide](../operations/incremental-materialized-views.md)
- [IVM Architecture](../architecture/ivm-plane.md)
