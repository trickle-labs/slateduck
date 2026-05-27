# Bounded SQL

Rocklake's SQL layer is fundamentally different from what you might expect of a database. It is not a general-purpose query engine with a parser, planner, optimizer, and executor. Instead, it is a **bounded dispatcher** that recognizes exactly the SQL statement shapes emitted by supported DuckLake clients and maps each one to a specific catalog operation. Anything outside this bounded set is rejected immediately with SQLSTATE `0A000` (feature not supported). This is not a temporary limitation or a roadmap item — it is a deliberate design choice with significant benefits for security, correctness, performance, and maintainability. Understanding why Rocklake works this way will help you understand why it behaves as it does and how to work within its design.

## What "Bounded" Means

A bounded SQL dispatcher has a finite, enumerable set of supported statement patterns. You can list every single SQL shape it accepts, and that list is complete and verifiable. There are no edge cases lurking in a complex grammar, no unexpected interactions between features, and no ambiguity about what is supported. The current implementation recognizes approximately 50 distinct statement kinds, covering session management, transactions, reads (selecting metadata), and writes (inserting or updating catalog entries).

This is in stark contrast to a general SQL engine like PostgreSQL, which supports an effectively infinite space of valid SQL statements through the combination of subqueries, CTEs, window functions, lateral joins, user-defined functions, extensions, and custom operators. General engines must handle any valid SQL, which means they need sophisticated query planners and optimizers that introduce enormous complexity and potential for subtle bugs. PostgreSQL's query planner alone is hundreds of thousands of lines of C code that has been refined over three decades.

Rocklake does not need any of that complexity. The DuckLake protocol defines a specific, finite set of SQL operations that a catalog client will ever need to perform — and Rocklake implements exactly that set, no more.

## Why Not a Full SQL Engine?

The decision to use a bounded dispatcher rather than a full SQL engine was made for four reinforcing reasons, each of which is strong enough on its own to justify the choice.

**Security through a finite attack surface.** A general SQL engine is an attractive target for attack: SQL injection, privilege escalation through user-defined functions, exfiltration through cleverly crafted subqueries, denial of service through expensive queries. Because Rocklake's statement space is finite and enumerable, it is possible to audit every single code path that processes SQL input. There is no SQL injection risk because there is no dynamic query construction — each incoming statement is matched against a fixed pattern and dispatched to a pre-determined function. There is no way to exfiltrate data through clever query composition because there are no joins, subqueries, or user-defined functions in the first place. The entire SQL surface area is a classifier with roughly 50 arms. You can read it and verify it in an afternoon.

**Correctness through exhaustive coverage.** When the statement space is finite, you can write a test for every single variant, including every error case. Rocklake's test suite covers 100% of the statement kinds, each tested with multiple parameter variations. This level of coverage is impossible for a general SQL engine where the statement space is theoretically infinite. For catalog software, correctness is paramount: a catalog that returns wrong results causes data loss and incorrect query results that may go undetected for weeks. The bounded approach makes it possible to be truly exhaustive in testing.

**Performance through direct dispatch.** A general SQL engine must parse the query, build an AST, plan the query (which may involve statistics-based cost estimation, join ordering, and rule-based optimization), and then execute a query plan that may involve multiple stages. This takes time — typically a few hundred microseconds for simple queries in PostgreSQL. Rocklake's dispatcher classifies an incoming statement in microseconds: it parses the SQL with `sqlparser-rs`, then pattern-matches the AST against known shapes. There is no planning phase, no cost estimation, no optimizer. The match succeeds or fails in constant time, and the corresponding catalog operation is dispatched directly. For a catalog that handles hundreds of metadata requests per second, this difference compounds.

**Maintainability through simplicity.** The entire SQL classifier is a few thousand lines of Rust. It is straightforward pattern-matching logic with no state machines, no recursive descent, and no dynamic dispatch. A new developer can understand the entire SQL layer in a few hours. Adding support for a new statement shape — when DuckDB's `ducklake` extension adds a new catalog operation — means adding one variant to an enum, one arm to a match statement, and a handful of tests. Changing something is never risky because the entire system is visible and auditable.

## What Is Supported

The bounded set covers everything that DuckDB's `ducklake` extension needs to manage a catalog. Here is a categorized overview:

**Session management.** Queries that DuckDB issues during the connection handshake and throughout the session to configure its environment:

```sql
SELECT version()
SELECT current_schema()
SELECT current_database()
SELECT pg_catalog.pg_type.typname FROM pg_catalog.pg_type WHERE ...
SET timezone = 'UTC'
SET extra_float_digits = 3
SHOW server_version
```

These are handled by pre-configured responses. `version()` returns a PostgreSQL-compatible version string. `pg_type` queries return the type OIDs that DuckDB expects for data type resolution. `SET` statements for known parameters are acknowledged and discarded (Rocklake does not use a session timezone internally).

**Transaction control.** The standard SQL transaction primitives:

```sql
BEGIN
COMMIT
ROLLBACK
```

`BEGIN` opens a `PendingCatalogTxn` that accumulates subsequent write operations. `COMMIT` dispatches all accumulated operations as a single atomic `DbTransaction` against SlateDB, then calls `flush()` to advance reader visibility. `ROLLBACK` discards the pending batch.

**Snapshot queries.** DuckDB needs to know the current snapshot before it can issue any MVCC-filtered read:

```sql
SELECT max(snapshot_id) FROM ducklake_snapshot
SELECT snapshot_id, ... FROM ducklake_snapshot WHERE snapshot_id = $1
```

These translate to a point lookup or reverse scan on the snapshot namespace in SlateDB.

**Schema and table reads.** The metadata that DuckDB needs to plan and execute queries:

```sql
SELECT * FROM ducklake_schema
SELECT * FROM ducklake_table WHERE schema_id = $1
SELECT * FROM ducklake_column WHERE table_id = $1
SELECT * FROM ducklake_view
SELECT * FROM ducklake_macro
```

All of these translate to prefix scans against the appropriate tag namespace, filtered by MVCC visibility at the requested snapshot ID.

**Data file reads.** The file lists and statistics that DuckDB needs for query planning:

```sql
SELECT * FROM ducklake_data_file WHERE table_id = $1
SELECT * FROM ducklake_data_file_column_statistics WHERE data_file_id IN (...)
SELECT * FROM ducklake_delete_file WHERE table_id = $1
```

Data file queries are the most performance-critical catalog operation — every table scan triggers at least one. Rocklake optimizes these with a secondary index (tag `0xFC`) that makes snapshot-scoped file lookups fast even for tables with thousands of files.

**Catalog writes.** The mutations that DuckDB issues when creating tables, registering data files, and managing schema:

```sql
INSERT INTO ducklake_schema (schema_id, name, begin_snapshot) VALUES ($1, $2, $3)
INSERT INTO ducklake_table (table_id, schema_id, name, ...) VALUES (...)
INSERT INTO ducklake_data_file (...) VALUES (...)
UPDATE ducklake_table SET end_snapshot = $1 WHERE table_id = $2 AND begin_snapshot = $3
UPDATE ducklake_column SET end_snapshot = $1 WHERE ...
```

These all accumulate in the pending transaction and are committed atomically on `COMMIT`.

**Snapshot creation.** After a transaction commits, DuckDB issues a snapshot registration:

```sql
INSERT INTO ducklake_snapshot (snapshot_id, ...) VALUES (...)
INSERT INTO ducklake_table_changes (...) VALUES (...)
```

These are handled as part of the commit process.

## What Is Not Supported

The explicit non-support is as important as what is supported. These are the most common queries that might arrive at Rocklake and be rejected:

**Arbitrary `SELECT` with `JOIN`:**

```sql
SELECT t.name, c.name, c.type
FROM ducklake_table t
JOIN ducklake_column c ON t.id = c.table_id;
```

Rocklake does not implement SQL joins. This query would be rejected with `SQLSTATE 0A000`. DuckDB never issues this query — it asks for tables and columns separately and performs the join client-side.

**Aggregations beyond `max(snapshot_id)`:**

```sql
SELECT COUNT(*), SUM(row_count) FROM ducklake_data_file WHERE table_id = $1;
```

Custom aggregations are not in the bounded set. Row counts and file counts are returned as part of table statistics, not computed on demand from the data file table.

**Subqueries and CTEs:**

```sql
WITH recent AS (
  SELECT * FROM ducklake_snapshot ORDER BY snapshot_id DESC LIMIT 5
)
SELECT * FROM ducklake_table WHERE begin_snapshot IN (SELECT snapshot_id FROM recent);
```

No subqueries, no CTEs. Every query that Rocklake handles is a flat statement against a single table or a pre-enumerated set of tables.

**`DELETE` statements:**

```sql
DELETE FROM ducklake_table WHERE table_id = $1;
```

Rows are never deleted in Rocklake's catalog model. Instead, rows are superseded by setting their `end_snapshot`. This preserves the complete history (immutability) and makes crash recovery trivially safe (no deletion to roll back). A `DELETE` statement arriving at Rocklake is a bug in the client — DuckDB's `ducklake` extension never issues them.

**User-defined functions, stored procedures, triggers:**

None of these exist in Rocklake's surface area. There is no procedural language, no function registry, and no trigger mechanism. The catalog is a key-value store with a SQL interface, not a programmable database.

## How Classification Works

When a SQL string arrives over the PostgreSQL wire protocol, Rocklake processes it in three stages:

**Parsing.** The raw SQL text is parsed by `sqlparser-rs` using the PostgreSQL dialect. This produces an AST. If the SQL is syntactically invalid, `sqlparser-rs` returns an error and Rocklake responds with `SQLSTATE 42601` (syntax error) before any classification attempt.

**Classification.** The AST is pattern-matched against the registered statement shapes. This happens in a single match statement that covers all ~50 known shapes. The match is structural: it checks the statement type (Select, Insert, Update, Set, etc.), the target table names, the structure of the WHERE clause, and the presence of specific column names. Wildcards are allowed where they do not affect dispatch — for example, the specific column names in a `SELECT *` from `ducklake_data_file` are irrelevant to classification. If no pattern matches, the statement is classified as `Unsupported`.

**Dispatch.** For supported statements, the classifier extracts the bound parameters (table IDs, snapshot IDs, literal values) and calls the corresponding function on `CatalogStore`. For `Unsupported` statements, an error is returned immediately without any catalog operation — there is no partial execution, no side effect, no state change.

The classification logic is deterministic and stateless. The same SQL string always classifies to the same variant, regardless of session state. This makes the classifier easy to test and reason about.

## Living with the Bounded Set in Practice

If you are operating Rocklake as a DuckLake catalog backend for DuckDB, you will never encounter the bounded-SQL limitation in normal usage — DuckDB's `ducklake` extension issues only the statements that are in the bounded set by design. The limitation only becomes visible if you:

1. **Connect to Rocklake directly with a PostgreSQL client** (like `psql`) and try to run arbitrary SQL. You will find that most queries work for catalog inspection purposes (reading `ducklake_table`, `ducklake_column`, etc.) but more complex queries fail.

2. **Build a custom DuckLake client** that issues catalog operations. In this case, you must stay within the bounded query set. The [Custom Clients](../integration/custom-clients.md) guide documents exactly what you can and cannot do.

3. **Use a DuckDB version newer than Rocklake supports.** If DuckDB's `ducklake` extension adds a new catalog operation that Rocklake does not yet recognize, queries that trigger the new operation will fail with `SQLSTATE 0A000`. This surfaces as a DuckDB error like "unsupported catalog operation." Check the compatibility matrix in [DuckDB Compatibility](../integration/duckdb-compatibility.md).

For all three cases, the appropriate response is the same: understand what statement Rocklake is rejecting, determine whether it is something Rocklake should support, and either file an issue or adjust your client accordingly.

## Extending the Bounded Set

When a new version of DuckDB's `ducklake` extension introduces new catalog operations, Rocklake must be updated to recognize the new statement shapes. This is a deliberate design choice: compatibility is explicit, not implicit. Each new statement shape requires:

1. A new variant in the `StatementKind` enum in `rocklake-sql/src/lib.rs`
2. A new match arm in the classifier that pattern-matches the AST and extracts parameters
3. A new handler in the executor that maps the parameters to a `CatalogStore` operation
4. Tests for the new shape, including error cases

This means that Rocklake's compatibility with DuckDB versions is well-defined and testable. You can look at the `StatementKind` enum and know exactly which DuckDB `ducklake` extension operations are supported. You can run the wire corpus replay tests against a specific DuckDB version's corpus and know whether Rocklake handles it correctly. Compatibility is never ambiguous.

The flip side is that adding new SQL capabilities is an intentional act that goes through the normal contribution process. There is no "just make it work" path for unsupported SQL — adding support requires understanding the full implication of the new shape, writing the catalog operation it maps to, and adding tests. This is the right trade-off for catalog software where correctness and auditability matter more than feature velocity.

## Further Reading

- **[Design Decision: Why Bounded SQL?](../design-decisions/bounded-sql.md)** — The full engineering rationale for the bounded dispatcher approach
- **[SQL Dispatcher Architecture](../architecture/sql-dispatcher.md)** — The implementation details of the classifier and dispatcher
- **[DuckDB Compatibility](../integration/duckdb-compatibility.md)** — Which DuckDB versions work with which Rocklake versions
- **[Custom Clients](../integration/custom-clients.md)** — How to build a new DuckLake client that works with Rocklake
