# DuckDB

DuckDB is the primary client. It connects via the `postgres` extension using the PG wire protocol.

## Connection

```sql
ATTACH 'ducklake:postgres:host=localhost port=5432 dbname=warehouse' AS lake;
USE lake;
```

## Common Operations

```sql
CREATE SCHEMA analytics;
CREATE TABLE analytics.events (id BIGINT, event_type VARCHAR, ts TIMESTAMP);
INSERT INTO analytics.events VALUES (1, 'click', '2024-01-01 10:00:00');
SELECT * FROM analytics.events;
SELECT * FROM analytics.events AT (SNAPSHOT 5);
```

## Session Settings

DuckDB sends SET commands during startup. SlateDuck accepts all and stores `timezone`, `client_encoding`, and `DateStyle`.

## What Is Not Supported

Arbitrary SQL queries against catalog tables (JOINs, subqueries, CTEs, window functions) return `SQLSTATE 0A000`.
