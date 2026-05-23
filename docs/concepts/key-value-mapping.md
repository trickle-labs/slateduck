# Key-Value Mapping

SlateDuck maps DuckLake's relational catalog tables to key-value pairs in SlateDB.

## The Mapping Principle

Each row in a DuckLake catalog table becomes one key-value pair:

- **Key:** Binary-encoded composite of primary key fields, prefixed with a 1-byte tag
- **Value:** Protobuf-encoded message containing all non-key columns

## Example: `ducklake_table`

- **Key:** `0x05 | schema_id (u64 BE) | table_id (u64 BE) | begin_snapshot (u64 BE)`
- **Value:** Protobuf { table_name, end_snapshot }

## Key Ordering

Fields are ordered for the dominant access pattern: *"list all tables in schema X."* This is a prefix scan on `0x05 | schema_id`.

## Rules

1. Tag byte first — ensures table isolation
2. Parent ID before child ID — enables "children of X" queries
3. begin_snapshot last — all versions sort after the parent prefix
4. Big-endian integers — preserves numeric ordering lexicographically
