# Inlined Data

`0xFD` stores small data inlined in the catalog.

## Subtypes

### `0xFD | 0x01` — Insert Rows
Key: `0xFD | 0x01 | table_id | schema_version | row_id`

Actual data rows too small for a Parquet file.

### `0xFD | 0x02` — Delete Markers
Key: `0xFD | 0x02 | table_id | data_file_id | row_id`

Marks specific rows as deleted (merge-on-read).

## Size Limit
Maximum 64 MiB per inlined value. Exceeding returns `SQLSTATE 54001`.
