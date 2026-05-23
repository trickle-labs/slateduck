# DuckDB Compatibility Matrix

SlateDuck targets the standard DuckDB `ducklake` extension via the PostgreSQL
wire protocol. Each DuckDB version requires a wire corpus capture and explicit
verification before being declared compatible.

## Supported Versions

| DuckDB Version | DuckLake Extension | Status | Notes |
|---------------|-------------------|--------|-------|
| 1.5.2 | v1.0 | ✅ Verified | Baseline capture in Phase 0 |

## Version Policy

- **Patch versions** (1.5.x): Expected compatible without re-capture unless
  DuckLake extension is updated.
- **Minor versions** (1.x.0): Require new wire corpus capture + explicit
  sign-off. May introduce new SQL shapes that need dispatcher support.
- **Major versions** (x.0.0): Full new client treatment — new corpus, new
  golden tests, potential protocol changes.

## Wire Protocol Compatibility

SlateDuck implements the following PostgreSQL wire protocol features:

| Feature | Status |
|---------|--------|
| Simple query protocol | ✅ Supported |
| Extended query protocol (Parse/Bind/Execute/Sync) | ✅ Supported |
| SSL/TLS negotiation (rejection) | ✅ Supported |
| Startup parameter handling | ✅ Supported |
| COPY protocol | ❌ Not needed by DuckLake |
| Notification/Listen | ❌ Not needed by DuckLake |

## SQL Dialect Support

The bounded SQL dispatcher recognizes exactly the statement shapes emitted by
the DuckLake extension. Unrecognized SQL returns `SQLSTATE 0A000` (feature not
supported).

### Recognized Shapes

- `SELECT current_schema()`, `SELECT version()`, `SELECT current_database()`
- `SELECT oid, typname FROM pg_catalog.pg_type WHERE ...`
- `SELECT max(snapshot_id) FROM ducklake_snapshot`
- `SELECT ... FROM ducklake_{table} WHERE ... begin_snapshot/end_snapshot ...`
- `INSERT INTO ducklake_{table} VALUES (...)`
- `UPDATE ducklake_{table} SET end_snapshot = ... WHERE ...`
- `UPDATE ducklake_table_stats SET ... WHERE ...`
- `CREATE TABLE ducklake_inlined_*` (no-op DDL)
- `DROP TABLE ducklake_inlined_*` (no-op DDL)
- `BEGIN` / `COMMIT` / `ROLLBACK`
- `SET` / `SHOW`

## PostgreSQL Type OIDs

| OID | Type | Used for |
|-----|------|----------|
| 16 | `bool` | `path_is_relative`, nullability flags |
| 20 / 23 / 21 | `int8` / `int4` / `int2` | IDs, counts, sizes |
| 700 / 701 | `float4` / `float8` | statistics values |
| 25 / 1043 | `text` / `varchar` | names, paths, JSON fields |
| 1114 / 1184 | `timestamp` / `timestamptz` | snapshot timestamps |
| 2950 | `uuid` | table UUIDs |
| 114 / 3802 | `json` / `jsonb` | snapshot change metadata |

## Known Limitations

1. **Single writer**: Only one sidecar instance can write to a catalog at a time.
   Writer fencing ensures correctness (`SQLSTATE 57P04`).
2. **No binary format**: All result values are sent in text format.
3. **No SSL**: TLS termination should be handled by a reverse proxy.
4. **No authentication**: The sidecar accepts all connections. Deploy behind
   a network boundary or add a proxy with auth.
