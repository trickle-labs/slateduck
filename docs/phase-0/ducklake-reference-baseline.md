# DuckLake Reference Baseline — Golden Fixtures

> SQLite-backed DuckLake tutorial output captured as the spec-conformance oracle.
> Version: DuckDB 1.5.2

## Capture Method

The full DuckLake tutorial was executed against a SQLite-backed DuckLake instance.
All output was captured and stored as golden fixtures for conformance testing.

## Tutorial Operations Captured

1. **Attach catalog:**
   ```sql
   INSTALL ducklake;
   LOAD ducklake;
   ATTACH 'ducklake:sqlite:catalog.db' AS my_lake;
   ```

2. **Create schema and table:**
   ```sql
   CREATE SCHEMA my_lake.main;
   CREATE TABLE my_lake.main.test_table (id INTEGER, name VARCHAR, value DOUBLE);
   ```

3. **Insert data:**
   ```sql
   INSERT INTO my_lake.main.test_table VALUES (1, 'alice', 3.14), (2, 'bob', 2.71);
   ```

4. **Query data:**
   ```sql
   SELECT * FROM my_lake.main.test_table;
   -- Expected: 2 rows
   ```

5. **Time travel:**
   ```sql
   SELECT * FROM my_lake.main.test_table AT (SNAPSHOT 1);
   ```

6. **Schema evolution:**
   ```sql
   ALTER TABLE my_lake.main.test_table ADD COLUMN created_at TIMESTAMP;
   ```

7. **Drop and recreate:**
   ```sql
   DROP TABLE my_lake.main.test_table;
   CREATE TABLE my_lake.main.test_table (id INTEGER, name VARCHAR);
   ```

## Golden Output Format

Each operation's expected output is stored as a JSON object:

```json
{
  "operation": "SELECT * FROM my_lake.main.test_table",
  "snapshot_id": 2,
  "columns": ["id", "name", "value"],
  "types": ["INTEGER", "VARCHAR", "DOUBLE"],
  "rows": [[1, "alice", 3.14], [2, "bob", 2.71]]
}
```

## File Index

- `tests/golden/duckdb-1.5.x/create_schema.json`
- `tests/golden/duckdb-1.5.x/create_table.json`
- `tests/golden/duckdb-1.5.x/insert_data.json`
- `tests/golden/duckdb-1.5.x/select_all.json`
- `tests/golden/duckdb-1.5.x/time_travel.json`
- `tests/golden/duckdb-1.5.x/alter_table.json`
- `tests/golden/duckdb-1.5.x/drop_table.json`
