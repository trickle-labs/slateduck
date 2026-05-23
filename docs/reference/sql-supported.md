# SQL Supported

## Schema Operations

```sql
CREATE SCHEMA <name>
DROP SCHEMA <name>
```

## Table Operations

```sql
CREATE TABLE <schema>.<table> (<columns>)
DROP TABLE <schema>.<table>
ALTER TABLE ... ADD COLUMN / DROP COLUMN / RENAME COLUMN / ALTER TYPE / RENAME TO
```

## Data Registration

```sql
INSERT INTO ducklake_data_file (...) VALUES (...)
INSERT INTO ducklake_delete_file (...) VALUES (...)
INSERT INTO ducklake_file_column_stats (...) VALUES (...)
INSERT INTO ducklake_inlined_data_insert (...) VALUES (...)
INSERT INTO ducklake_inlined_data_delete (...) VALUES (...)
```

## Snapshot Management

```sql
INSERT INTO ducklake_snapshot (...) VALUES (...)
INSERT INTO ducklake_snapshot_changes (...) VALUES (...)
UPDATE ducklake_table SET end_snapshot = $1 WHERE ...
UPDATE ducklake_column SET end_snapshot = $1 WHERE ...
```

## Queries

```sql
SELECT * FROM ducklake_table WHERE schema_id = $1
SELECT * FROM ducklake_column WHERE table_id = $1
SELECT * FROM ducklake_data_file WHERE table_id = $1
SELECT max(snapshot_id) FROM ducklake_snapshot
SELECT * FROM ducklake_file_column_stats WHERE table_id = $1 AND column_id = $2
SELECT * FROM ducklake_metadata WHERE metadata_key = $1
```

## Not Supported

JOINs, subqueries, CTEs, window functions, GROUP BY, UNION — all return `SQLSTATE 0A000`.
