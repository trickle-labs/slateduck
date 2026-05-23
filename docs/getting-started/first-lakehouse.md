# Your First Lakehouse

This guide walks through a complete lakehouse workflow: creating schemas, tables, querying with time travel, and inspecting the catalog.

## Setup

Start SlateDuck and connect DuckDB as described in the [quickstart](quickstart.md).

## Create a Schema

```sql
CREATE SCHEMA analytics;
```

## Create Tables

```sql
CREATE TABLE analytics.events (
  id BIGINT, event_type VARCHAR, source VARCHAR, ts TIMESTAMP
);
CREATE TABLE analytics.users (
  user_id BIGINT, name VARCHAR, email VARCHAR, created_at TIMESTAMP
);
```

## Insert Data

```sql
INSERT INTO analytics.events VALUES
  (1, 'signup', 'web', '2024-01-01 10:00:00'),
  (2, 'click', 'mobile', '2024-01-01 10:05:00');
INSERT INTO analytics.users VALUES
  (1, 'Alice', 'alice@example.com', '2024-01-01 09:00:00');
```

## Time Travel

```sql
SELECT snapshot_id, snapshot_time FROM ducklake_snapshot ORDER BY snapshot_id;
SELECT * FROM analytics.events AT (SNAPSHOT 2);
```

## Alter Schema

```sql
ALTER TABLE analytics.events ADD COLUMN user_id BIGINT;
```

## Inspect

```bash
./target/release/slateduck inspect --catalog-path /tmp/my-lakehouse
```
