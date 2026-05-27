# DuckLake v1.0 Query Specification

This file documents every SQL query that Rocklake must support as part of its
DuckLake v1.0 compatibility claim. These examples are the golden reference used
by the conformance test suite in `crates/rocklake-pgwire/tests/`.

Each query must:
- Parse without error
- Return the exact column names listed in the spec
- Return results with compatible types

## Catalog Setup

```sql
-- Attach a Rocklake catalog via the DuckLake PG-Wire sidecar.
ATTACH 'ducklake:postgres://127.0.0.1:5555/' AS my_lake;

-- Create a schema inside the attached catalog.
CREATE SCHEMA my_lake.analytics;

-- Create a table with typed columns.
CREATE TABLE my_lake.analytics.events (
    id      BIGINT NOT NULL,
    ts      TIMESTAMP NOT NULL,
    payload VARCHAR
);

-- Insert rows.
INSERT INTO my_lake.analytics.events VALUES
    (1, '2024-01-01 00:00:00', 'hello'),
    (2, '2024-01-02 00:00:00', 'world');

-- Select rows.
SELECT * FROM my_lake.analytics.events;

-- Time-travel read at snapshot N.
SELECT * FROM my_lake.analytics.events AT (VERSION => 1);

-- Drop table and schema.
DROP TABLE my_lake.analytics.events;
DROP SCHEMA my_lake.analytics;
DETACH my_lake;
```

## Catalog Introspection Queries

All 28 spec tables must be queryable via `SELECT * FROM <table>`.

### ducklake_snapshot

```sql
SELECT snapshot_id, begin_snapshot, end_snapshot, parent_snapshot_id,
       snapshot_sequence, next_catalog_id, next_file_id,
       changes_made, author, commit_message, commit_extra_info
FROM ducklake_snapshot;
```

### ducklake_schema

```sql
SELECT schema_id, begin_snapshot, end_snapshot, schema_name, schema_oid,
       schema_options
FROM ducklake_schema;
```

### ducklake_table

```sql
SELECT table_id, begin_snapshot, end_snapshot, schema_id, table_name,
       table_oid, path_in_schema, table_uuid, table_comment
FROM ducklake_table;
```

### ducklake_column

```sql
SELECT column_id, begin_snapshot, end_snapshot, table_id, column_order,
       column_name, column_type, nullable, column_default, generated_expression,
       column_comment
FROM ducklake_column;
```

### ducklake_data_file

```sql
SELECT file_id, begin_snapshot, end_snapshot, table_id, file_format,
       path, file_size_bytes, footer_size, record_count, row_id_start,
       path_is_relative, partition_id, mapping_id, partial_max, file_order
FROM ducklake_data_file;
```

### ducklake_delete_file

```sql
SELECT file_id, begin_snapshot, end_snapshot, table_id, data_file_id,
       file_format, path, file_size_bytes, footer_size, delete_record_count
FROM ducklake_delete_file;
```

### ducklake_file_column_stats

```sql
SELECT file_id, column_id, null_count, contains_nan, lower_bound, upper_bound
FROM ducklake_file_column_stats;
```

### ducklake_table_stats

```sql
SELECT table_id, begin_snapshot, end_snapshot, record_count,
       file_size_bytes, next_row_id
FROM ducklake_table_stats;
```

### ducklake_metadata

```sql
SELECT metadata_id, begin_snapshot, end_snapshot, key, value, table_id
FROM ducklake_metadata;
```

### ducklake_tag

Column names: `tag_id`, `begin_snapshot`, `end_snapshot`, `object_id`,
`tag_name`, `tag_value`.

```sql
SELECT tag_id, begin_snapshot, end_snapshot, object_id, tag_name, tag_value
FROM ducklake_tag;
```

### ducklake_column_tag

Column names: `tag_id`, `begin_snapshot`, `end_snapshot`, `column_id`,
`tag_name`, `tag_value`.

```sql
SELECT tag_id, begin_snapshot, end_snapshot, column_id, tag_name, tag_value
FROM ducklake_column_tag;
```

### ducklake_sort_info

Column names: `sort_id`, `begin_snapshot`, `end_snapshot`, `table_id`,
`sort_order`, `column_id`.

```sql
SELECT sort_id, begin_snapshot, end_snapshot, table_id, sort_order, column_id
FROM ducklake_sort_info;
```

### ducklake_schema_version

Column names: `schema_version`, `schema_version_info`.

```sql
SELECT schema_version, schema_version_info
FROM ducklake_schema_version;
```

### ducklake_view

```sql
SELECT view_id, begin_snapshot, end_snapshot, schema_id, view_name,
       view_oid, sql, view_comment
FROM ducklake_view;
```

### ducklake_macro

```sql
SELECT macro_id, begin_snapshot, end_snapshot, schema_id, macro_name,
       macro_type, return_type, macro_comment
FROM ducklake_macro;
```

### ducklake_macro_impl

```sql
SELECT macro_id, macro_impl_id, begin_snapshot, end_snapshot, implementation
FROM ducklake_macro_impl;
```

### ducklake_macro_parameter

```sql
SELECT macro_id, parameter_id, begin_snapshot, end_snapshot,
       parameter_name, parameter_type, parameter_default
FROM ducklake_macro_parameter;
```

### ducklake_schema_changes

```sql
SELECT changes_id, snapshot_id, table_id, schema_id, change_type, change_info
FROM ducklake_schema_changes;
```

### ducklake_snapshot_changes

```sql
SELECT snapshot_id, change_type, changes_made, author, commit_message,
       commit_extra_info
FROM ducklake_snapshot_changes;
```

### ducklake_partition_info

```sql
SELECT partition_id, begin_snapshot, end_snapshot, table_id,
       partition_key, partition_by
FROM ducklake_partition_info;
```

### ducklake_schema_version

```sql
SELECT schema_version, schema_version_info
FROM ducklake_schema_version;
```

### ducklake_inlined_data_table

```sql
SELECT table_id, begin_snapshot, end_snapshot, data_table_id
FROM ducklake_inlined_data_table;
```

### ducklake_encrypted_secret

```sql
SELECT secret_id, begin_snapshot, end_snapshot, secret_name, secret_type,
       encrypted_secret
FROM ducklake_encrypted_secret;
```

### ducklake_table_column_tags (via ducklake_column_tag)

See `ducklake_column_tag` above.

## DuckLake Tutorial End-to-End

The following is a complete session demonstrating the full DuckLake lifecycle:

```sql
-- Attach catalog.
ATTACH 'ducklake:postgres://127.0.0.1:5555/' AS lake (TYPE DUCKLAKE);

-- Create schema and table.
CREATE SCHEMA lake.main;
CREATE TABLE lake.main.orders (
    order_id BIGINT PRIMARY KEY,
    customer VARCHAR NOT NULL,
    amount   DECIMAL(10, 2) NOT NULL
);

-- Insert rows.
INSERT INTO lake.main.orders VALUES (1, 'Alice', 99.99);
INSERT INTO lake.main.orders VALUES (2, 'Bob', 149.50);

-- Read current state.
SELECT * FROM lake.main.orders ORDER BY order_id;

-- Time travel to snapshot 1.
SELECT * FROM lake.main.orders AT (VERSION => 1) ORDER BY order_id;

-- Delete a row.
DELETE FROM lake.main.orders WHERE order_id = 1;

-- Verify deletion.
SELECT * FROM lake.main.orders ORDER BY order_id;

-- Tag a table.
-- (Via catalog: set_tag(table_id, 'owner', 'data-team'))

-- Introspect tags.
SELECT tag_name, tag_value FROM ducklake_tag;

-- Check schema version.
SELECT schema_version FROM ducklake_schema_version;

-- Drop and detach.
DROP TABLE lake.main.orders;
DROP SCHEMA lake.main;
DETACH lake;
```
