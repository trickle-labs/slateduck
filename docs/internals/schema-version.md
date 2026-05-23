# Schema Version

Tracks schema-mutating operations independently of data operations.

## Increments

- CREATE/DROP/RENAME TABLE
- CREATE/DROP SCHEMA
- ALTER TABLE (ADD/DROP/RENAME/ALTER COLUMN)

## Does NOT Increment

- register_data_file
- register_delete_file
- update_table_stats / upsert_file_column_stats

DuckDB uses this to invalidate its plan cache efficiently.
