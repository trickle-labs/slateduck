# DuckLake PostgreSQL Query Audit

**Audited repository:** `../ducklake` (DuckDB ducklake extension)
**Source cross-referenced:** `github.com/duckdb/duckdb-postgres` (postgres scanner extension)
**Date:** 2026-05-26
**Context:** DuckDB v1.5.3 connecting to SlateDuck via `ATTACH 'ducklake:postgres:...' AS lake`

---

## How the Attach Sequence Works

When DuckDB executes `ATTACH 'ducklake:postgres:host=... dbname=...' AS lake`, it triggers
two separate code paths that both send SQL over the PostgreSQL wire protocol to SlateDuck:

1. **Postgres scanner extension** (`duckdb/duckdb-postgres`) — manages the underlying
   PostgreSQL connection. Sends version probes, catalog scans, and connection resets before
   any DuckLake logic runs.

2. **DuckLake extension** (`duckdb/ducklake`) — once the connection is established, sends
   queries against the DuckLake metadata tables (`ducklake_snapshot`, `ducklake_schema`, etc.).

---

## Phase 1 — Postgres Scanner Connection Initialization

These queries are sent unconditionally by the postgres scanner when a connection is opened
or reset. They originate from `postgres_connection.cpp` and `postgres_secret_storage.cpp`.

### 1.1 Version / Instance Type Detection

**Source:** `PostgresConnection::GetPostgresVersion()` — `src/postgres_connection.cpp`

```sql
SELECT version(), (SELECT COUNT(*) FROM pg_settings WHERE name LIKE 'rds%')
```

**Columns expected:**
- col 0 — `TEXT` — PostgreSQL version string (e.g. `"PostgreSQL 15.0 on x86_64-..."`)
- col 1 — `INT8` — count of RDS settings (0 = not RDS, >0 = Aurora/RDS)

**SlateDuck status:** ✅ **Fixed** — `SelectVersionWithRdsCheck` handler returns
`("PostgreSQL 15.0 on x86_64-pc-linux-gnu", 0)`.

**Failure mode if broken:** "Postgres scanner - failed to fetch value for row 0 col 1"

---

### 1.2 Health Check

**Source:** `PostgresConnection::PingServer()` — `src/postgres_connection.cpp`

```sql
SELECT 1
```

**Columns expected:**
- col 0 — `INT4` — literal `1`

**SlateDuck status:** ✅ **Fixed** — `SelectOne` handler returns `1`.

**Failure mode if broken:** Connection ping fails, connection pool considers connection dead.

---

### 1.3 Connection Reset

**Source:** `PostgresConnection::Reset()` — `src/postgres_connection.cpp`

```sql
DISCARD ALL
```

**Expected result:** Command tag `DISCARD` (no rows). This is a PostgreSQL session cleanup
command. DuckDB sends this when returning a connection to the pool.

**SlateDuck status:** ❌ **Unsupported** — classified as `Unsupported("DISCARD ALL")`.

**Failure mode if broken:** DuckDB logs a warning but continues. The connection is not
returned cleanly to the pool; a `PQreset()` is attempted as a fallback (which opens a new
TCP connection).

**Required response:** `DISCARD` command tag with zero rows, no error. Since SlateDuck is
stateless per connection this could simply return `Ok` with a command tag response.

---

### 1.4 Secret Storage Table Existence — Fast Path

**Source:** `SecretStorageTable::Exists()` — `src/storage/postgres_secret_storage.cpp`

```sql
SELECT to_regclass('duckdb_secrets')
```

**Columns expected:**
- col 0 — `TEXT` or `NULL` — OID-as-text if table exists, `NULL` if not

**SlateDuck status:** ❌ **Unsupported** — `to_regclass` is not a recognised pattern.

**Failure mode if broken:** DuckDB falls through to the information_schema fallback (see 1.5).
If _both_ 1.4 and 1.5 fail (return errors), the `RegisterSecretStorage()` call is aborted
and no secret storage is registered. This is non-fatal for attach.

**Required response:** Return a single row with `NULL` in col 0 (meaning the table does not
exist). This suppresses DuckDB's attempt to create or use the `duckdb_secrets` table.

---

### 1.5 Secret Storage Table Existence — Fallback

**Source:** `SecretStorageTable::Exists()` — `src/storage/postgres_secret_storage.cpp`

```sql
SELECT EXISTS (
    SELECT 1
    FROM information_schema.tables
    WHERE table_schema =
        CASE WHEN 'duckdb_secrets' LIKE '%.%'
        THEN split_part('duckdb_secrets', '.', 1)
        ELSE 'public'
        END
    AND table_name =
        CASE
        WHEN 'duckdb_secrets' LIKE '%.%'
        THEN split_part('duckdb_secrets', '.', 2)
        ELSE 'duckdb_secrets'
        END
)
```

**Columns expected:**
- col 0 — `BOOL` — `true` if table exists, `false` otherwise

**SlateDuck status:** ❌ **Unsupported** — sub-SELECT against `information_schema.tables` 
is not classified.

**Failure mode if broken:** If this errors (rather than returns `false`), DuckDB silently
skips registering a secret storage backend. Non-fatal for attach.

**Required response:** Return `false`. Since SlateDuck has no `duckdb_secrets` table, the
correct answer is always `false`.

---

### 1.6 Full Catalog Scan (pg_namespace / pg_class / pg_attribute / pg_type / pg_constraint / pg_enum / pg_indexes)

**Source:** `PostgresSchemaSet::LoadEntries()` — `src/storage/postgres_schema_set.cpp`  
This is sent as a **single multi-statement string** over a `PQsendQuery` call (not
individual queries) inside `BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ ... ROLLBACK`.

```sql
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ;

SELECT oid, nspname
FROM pg_namespace
ORDER BY oid;

SELECT pg_namespace.oid AS namespace_id, relname, relpages, attname,
    pg_type.typname type_name, atttypmod type_modifier, pg_attribute.attndims ndim,
    attnum, pg_attribute.attnotnull AS notnull, NULL constraint_id,
    NULL constraint_type, NULL constraint_key
FROM pg_class
JOIN pg_namespace ON relnamespace = pg_namespace.oid
JOIN pg_attribute ON pg_class.oid=pg_attribute.attrelid
JOIN pg_type ON atttypid=pg_type.oid
WHERE attnum > 0 AND relkind IN ('r', 'v', 'm', 'f', 'p')
UNION ALL
SELECT pg_namespace.oid AS namespace_id, relname, NULL relpages, NULL attname, NULL type_name,
    NULL type_modifier, NULL ndim, NULL attnum, NULL AS notnull,
    pg_constraint.oid AS constraint_id, contype AS constraint_type,
    conkey AS constraint_key
FROM pg_class
JOIN pg_namespace ON relnamespace = pg_namespace.oid
JOIN pg_constraint ON (pg_class.oid=pg_constraint.conrelid)
WHERE relkind IN ('r', 'v', 'm', 'f', 'p') AND contype IN ('p', 'u')
ORDER BY namespace_id, relname, attnum, constraint_id;

SELECT n.oid, enumtypid, typname, enumlabel
FROM pg_enum e
JOIN pg_type t ON e.enumtypid = t.oid
JOIN pg_namespace AS n ON (typnamespace=n.oid)
ORDER BY n.oid, enumtypid, enumsortorder;

SELECT n.oid, t.typrelid AS id, t.typname as type, pg_attribute.attname, sub_type.typname
FROM pg_type t
JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace
JOIN pg_class ON pg_class.oid = t.typrelid
JOIN pg_attribute ON attrelid=t.typrelid
JOIN pg_type sub_type ON (pg_attribute.atttypid=sub_type.oid)
WHERE pg_class.relkind = 'c'
AND t.typtype='c'
ORDER BY n.oid, t.oid, attrelid, attnum;

SELECT pg_namespace.oid, tablename, indexname
FROM pg_indexes
JOIN pg_namespace ON (schemaname=nspname)
ORDER BY pg_namespace.oid;

ROLLBACK;
```

**Columns expected per result set:**
1. `pg_namespace`: `(oid INT8, nspname TEXT)`
2. `pg_class` + UNION: `(namespace_id INT8, relname TEXT, relpages INT8, attname TEXT, type_name TEXT, type_modifier INT8, ndim INT8, attnum INT8, notnull BOOL, constraint_id INT8, constraint_type TEXT, constraint_key TEXT)`
3. `pg_enum`: `(oid INT8, enumtypid INT8, typname TEXT, enumlabel TEXT)`
4. `pg_type` composites: `(oid INT8, id INT8, type TEXT, attname TEXT, typname TEXT)`
5. `pg_indexes`: `(oid INT8, tablename TEXT, indexname TEXT)`

**SlateDuck status:** ❌ **Partially handled** — `Begin` and `Rollback` are handled, but the
five inner SELECT statements against system catalog tables are classified as `Unsupported`.

**Failure mode if broken:** DuckDB's `PostgresSchemaSet::LoadEntries()` receives empty or
error results and crashes with "Attempted to access index 0 within vector of size 0" when
trying to look up tables by schema OID.

**Required response:** This is the most complex requirement. The postgres scanner needs to
build its internal schema map from these results. SlateDuck must return:

| Query | Required behavior |
|---|---|
| `pg_namespace` | At minimum one row: `(1, 'public')`. For ducklake compatibility, also expose `(2, 'main')` or whatever schema the ducklake catalog uses. |
| `pg_class` UNION | Rows for every table SlateDuck exposes (all `ducklake_*` tables). Each row needs `namespace_id`, `relname`, column info or constraint info. |
| `pg_enum` | Empty result set (no rows, but valid schema). |
| `pg_type` composites | Empty result set. |
| `pg_indexes` | Empty result set or index info for ducklake tables. |

> **Note:** The multi-statement batch is sent via `PQsendQuery` (simple query protocol),
> which means SlateDuck receives it as a single SQL string and must respond with _multiple_
> result sets in sequence before the final command-complete. This requires significant
> protocol-level work.

---

### 1.7 Database Size Query

**Source:** `PostgresCatalog::GetDatabaseSize()` — `src/storage/postgres_catalog.cpp`

```sql
SELECT pg_database_size(current_database());
```

**Columns expected:**
- col 0 — `INT8` — database size in bytes

**SlateDuck status:** ❌ **Unsupported** — neither `pg_database_size` nor `current_database()`
are recognised.

**Failure mode if broken:** Non-fatal; DuckDB uses this only for informational `duckdb_databases()` reporting.

**Required response:** Return a single integer row (e.g. `0` or a reasonable approximation).

---

## Phase 2 — DuckLake Metadata Initialization

These queries come from the ducklake extension itself (`duckdb/ducklake`) and access the
DuckLake metadata tables. They only run if the catalog scan (Phase 1) succeeds.

### 2.1 Load Secrets

**Source:** `DuckLakeInitializer::Initialize()` — `src/storage/ducklake_initializer.cpp`

```sql
FROM duckdb_secrets()
```

**Columns expected:** Multiple columns from DuckDB's built-in secrets table function.

**SlateDuck status:** ❌ **Unsupported** — `duckdb_secrets()` is a DuckDB-internal table
function, not a standard SQL construct.

**Note:** This query runs over the _DuckDB internal connection_, not SlateDuck — it is
routed to DuckDB's own metadata. It appears in debug logs because the underlying connection
happens to be the same postgres connection object. This may not actually reach SlateDuck.

---

### 2.2 Metadata Existence Check

**Source:** `DuckLakeMetadataManager::MetadataExists()` — `src/storage/ducklake_metadata_manager.cpp`

```sql
SELECT NULL FROM {METADATA_CATALOG}.ducklake_metadata LIMIT 1
```

Where `{METADATA_CATALOG}` is resolved to the schema name (e.g. `main`).

**Columns expected:**
- col 0 — any type — `NULL` (ignored; DuckDB only checks for error vs. success)

**SlateDuck status:** ⚠️ **Depends on Phase 1 success** — this query is routed through
the postgres scanner and calls `Query()` on the metadata connection. If the catalog scan
(Phase 1, 1.6) fails, DuckLake never reaches this step. If Phase 1 passes, this query
hits SlateDuck's `ducklake_metadata` table handler.

---

### 2.3 DuckLake Metadata Queries (postgres backend variant)

**Source:** `PostgresMetadataManager::GetLatestSnapshotQuery()` — `src/metadata_manager/postgres_metadata_manager.cpp`

When using the postgres backend, ducklake wraps all queries in `postgres_query(...)` or
`postgres_execute(...)` CALL statements:

```sql
SELECT * FROM postgres_query('slateduck',
    'SELECT snapshot_id, schema_version, next_catalog_id, next_file_id
     FROM main.ducklake_snapshot WHERE snapshot_id = (
         SELECT MAX(snapshot_id) FROM main.ducklake_snapshot
     );')
```

**SlateDuck status:** ⚠️ **Not applicable in current test** — `postgres_query` is a 
DuckDB postgres extension function that DuckDB calls _locally_. These queries are only
relevant when ducklake is attached with a DuckDB postgres catalog underneath, not when
SlateDuck is the server. When SlateDuck is the server, standard SQL queries are sent
directly without the `postgres_query()` wrapper.

---

## Summary of Issues

### Critical (block attach)

| # | Query | Issue |
|---|---|---|
| C1 | `BEGIN ... pg_namespace ... pg_class ... pg_enum ... pg_indexes ... ROLLBACK` | Multi-statement catalog scan returns errors; DuckDB crashes with index-out-of-bounds |

### High (degrade secret storage / connection pool)

| # | Query | Issue |
|---|---|---|
| H1 | `DISCARD ALL` | Connection reset fails; postgres scanner falls back to full reconnect on every pool return |
| H2 | `SELECT to_regclass('duckdb_secrets')` | Returns error instead of `NULL`; falls through to H3 |
| H3 | `SELECT EXISTS(... information_schema.tables ...)` | Returns error; secret storage not registered (non-fatal but noisy) |

### Low (informational / non-fatal)

| # | Query | Issue |
|---|---|---|
| L1 | `SELECT pg_database_size(current_database())` | Returns error; database size shows as 0 in `duckdb_databases()` |

---

## Recommended Implementation Order

### Step 1 — `DISCARD ALL` (easy, high impact)

Return a command-complete tag with no rows. SlateDuck is already stateless per connection so
no actual session state needs to be cleared.

```sql
-- Response: CommandComplete "DISCARD"
```

### Step 2 — `SELECT to_regclass(...)` (easy, unblocks step 3)

Return a single-row, single-column NULL result:

```sql
-- Response: one row, col "to_regclass" TEXT = NULL
```

This tells DuckDB the `duckdb_secrets` table does not exist and it can skip secret storage.

### Step 3 — `SELECT EXISTS(... information_schema.tables ...)` (medium)

Return a single boolean `false`:

```sql
-- Response: one row, col "exists" BOOL = false
```

### Step 4 — pg_namespace mock (medium, unblocks Phase 2)

Return a minimal namespace list matching SlateDuck's schema:

```sql
-- Response: (1, 'public'), (2, 'main')  [or whatever schemas SlateDuck exposes]
```

### Step 5 — pg_class / pg_attribute catalog scan (hard, critical)

This is the most complex piece. The postgres scanner needs a complete column/constraint
listing for every table it will query. Two options:

**Option A — Full catalog emulation:**  
Implement `pg_namespace`, `pg_class`, `pg_attribute`, `pg_type`, `pg_constraint`,
`pg_enum`, and `pg_indexes` as virtual tables returning real metadata about the
ducklake tables SlateDuck manages.

**Option B — Multi-statement batch handler:**  
Intercept the specific 6-query batch that DuckDB sends (recognisable by the `BEGIN
TRANSACTION ISOLATION LEVEL REPEATABLE READ` prefix followed by the five catalog
queries and `ROLLBACK`) and return:
- `pg_namespace`: rows for each known schema (e.g. `public`, `main`)
- `pg_class` UNION: rows for each known ducklake table with its column definitions
- `pg_enum`, `pg_type` composites, `pg_indexes`: empty result sets

Option B is easier to implement in the short term but fragile if DuckDB changes the
query shape. Option A is more maintainable long-term.

### Step 6 — `SELECT pg_database_size(current_database())` (easy, low priority)

Return `0::int8` or an approximation of the SlateDB store size.

---

## Protocol Note: Multi-Statement Batches

The catalog scan (Step 5) is sent as a **single string via the simple query protocol**
containing `BEGIN; query1; query2; ... ROLLBACK;`. PostgreSQL handles this by sending
back multiple result sets in sequence. SlateDuck currently processes one statement at a
time (splitting on `;`) and routes each through `classify_statement`.

To support this, SlateDuck either needs to:
1. Detect the catalog scan batch as a named pattern (e.g. starts with
   `BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ`) and return all 6 result sets
   as a pre-built response sequence, or
2. Implement a general multi-result-set response path for the simple query protocol.

The `pgwire` crate supports multiple responses per query; SlateDuck's `Begin` handler
already returns `Response::TransactionStart`. The challenge is accumulating the five
inner query results and returning them before the `ROLLBACK`.

---

## Files to Modify

| File | Change needed |
|---|---|
| `crates/slateduck-sql/src/classifier/mod.rs` | Add `DiscardAll`, `SelectToRegclass`, `SelectExistsInfoSchema`, `SelectPgDatabaseSize`, `PgCatalogScan` variants |
| `crates/slateduck-sql/src/classifier/ast.rs` | Classify the new patterns |
| `crates/slateduck-pgwire/src/executor/helpers.rs` | Add response builders for each new kind |
| `crates/slateduck-pgwire/src/executor/mod.rs` | Add match arms and handlers |
| `crates/slateduck-pgwire/src/handler.rs` | Handle multi-statement batch detection if needed |
