# pg-tide-relay

pg-tide-relay connects streaming sources (Kafka, NATS, SQS) to SlateDuck as a DuckLake sink.

## Connection

```toml
[sink.lakehouse]
type = "ducklake"
connection = "host=slateduck-host port=5432 dbname=warehouse"
```

## Extensions Added

- `ORDER BY ... ASC LIMIT 1` on `ducklake_snapshot`
- `gen_random_uuid()` in INSERT VALUES
- `INSERT INTO ducklake_metadata` / `SELECT value FROM ducklake_metadata`

## Exactly-Once Semantics

pg-tide stores its consumer offset alongside catalog mutations in the same transaction. If commit succeeds, both data registration and offset advance are durable.
