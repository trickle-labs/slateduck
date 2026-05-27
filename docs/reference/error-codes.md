# Error Codes Reference

This page documents all SQLSTATE error codes that Rocklake may return to clients over the PostgreSQL wire protocol. SQLSTATE codes are the standard mechanism for programmatic error handling — your application code should match on these codes, NOT on error message strings. Error messages are human-readable descriptions intended for logs and debugging; they may change between Rocklake versions without notice. SQLSTATE codes are stable across versions and safe for programmatic use.

Each code consists of five characters: two characters for the error class (the broad category) and three characters for the specific condition within that class. For example, `42P01` belongs to class `42` (Syntax Error or Access Rule Violation) with specific condition `P01` (Undefined Table).

## Error Code Index

| Code | Name | Quick Description |
|------|------|-------------------|
| 00000 | successful_completion | Operation completed successfully |
| 08000 | connection_exception | General connection failure |
| 08003 | connection_does_not_exist | Attempted use of closed connection |
| 08006 | connection_failure | Network error during operation |
| 0A000 | feature_not_supported | Unsupported feature or format version |
| 22023 | invalid_parameter_value | Parameter out of valid range |
| 25001 | active_sql_transaction | Operation invalid during transaction |
| 25006 | read_only_sql_transaction | Write attempted on read-only instance |
| 3F000 | invalid_schema_name | Referenced schema does not exist |
| 42000 | syntax_error_or_access_rule_violation | General syntax/access error |
| 42601 | syntax_error | SQL not recognized by classifier |
| 42P01 | undefined_table | Referenced table does not exist |
| 42P06 | duplicate_schema | Schema with this name already exists |
| 42P07 | duplicate_table | Table with this name already exists |
| 42703 | undefined_column | Referenced column does not exist |
| 42710 | duplicate_object | Object already exists (generic) |
| 57P04 | writer_fenced | Writer invalidated by epoch change |
| 58000 | system_error | Object storage error |
| XX000 | internal_error | Unexpected internal failure |

---

## Class 00 — Successful Completion

### 00000: successful_completion

The operation completed without errors.

| Aspect | Detail |
|--------|--------|
| **When returned** | Every successful operation |
| **Client action** | Process results normally |
| **Notes** | Included in the CommandComplete message |

---

## Class 08 — Connection Exception

Connection errors indicate problems with the network connection between the client and Rocklake.

### 08000: connection_exception

A general connection error that does not fit a more specific category.

| Aspect | Detail |
|--------|--------|
| **When returned** | Miscellaneous connection issues |
| **Common causes** | TLS handshake failure, protocol version mismatch |
| **Client action** | Reconnect and retry |

### 08003: connection_does_not_exist

The client is attempting to use a connection that has been closed or was never properly established.

| Aspect | Detail |
|--------|--------|
| **When returned** | Operations on a terminated session |
| **Common causes** | Server-side timeout closed the session, client using stale connection from pool |
| **Client action** | Establish a new connection |

### 08006: connection_failure

A network error occurred during an operation. The connection is no longer usable.

| Aspect | Detail |
|--------|--------|
| **When returned** | Mid-operation network failure |
| **Common causes** | TCP connection reset, server process crash, network partition |
| **Client action** | Reconnect. If mid-transaction, check whether the transaction committed (query catalog state) before retrying |

---

## Class 0A — Feature Not Supported

### 0A000: feature_not_supported

The requested operation or feature is not supported by this version of Rocklake.

| Aspect | Detail |
|--------|--------|
| **When returned** | Unrecognized catalog format version, unsupported DuckLake protocol feature |
| **Common causes** | Catalog created by a newer Rocklake version, DuckDB using a protocol extension not yet implemented |
| **Client action** | Upgrade Rocklake to a version that supports the requested feature |
| **Example message** | `"catalog format version 3 not supported (maximum supported: 2)"` |

---

## Class 22 — Data Exception

### 22023: invalid_parameter_value

A parameter provided by the client falls outside the valid range.

| Aspect | Detail |
|--------|--------|
| **When returned** | Invalid snapshot ID, invalid entity ID, malformed parameter |
| **Common causes** | Requesting a snapshot below retain_from (garbage-collected), referencing a non-existent ID |
| **Client action** | Verify parameter values. For snapshot IDs, ensure the snapshot is within the retention window |
| **Example message** | `"snapshot 42 is below retention horizon (retain_from = 100)"` |

---

## Class 25 — Invalid Transaction State

### 25001: active_sql_transaction

An operation was attempted that cannot be performed while a transaction is in progress.

| Aspect | Detail |
|--------|--------|
| **When returned** | Operations that require being outside a transaction block |
| **Common causes** | Attempting structural operations while uncommitted changes exist |
| **Client action** | COMMIT or ROLLBACK the current transaction first |

### 25006: read_only_sql_transaction

A write operation was attempted on a connection or instance that is configured as read-only.

| Aspect | Detail |
|--------|--------|
| **When returned** | INSERT, UPDATE, DELETE, or DDL on a read-only instance |
| **Common causes** | Connected to a read-only replica, ROCKLAKE_READ_ONLY=true |
| **Client action** | Connect to the writer instance for write operations |
| **Example message** | `"cannot execute INSERT in a read-only transaction"` |

---

## Class 3F — Invalid Schema Name

### 3F000: invalid_schema_name

A referenced schema does not exist at the requested snapshot.

| Aspect | Detail |
|--------|--------|
| **When returned** | Operations that reference a schema by name or ID |
| **Common causes** | Schema was dropped, schema name misspelled, schema not yet created at the requested snapshot |
| **Client action** | Verify schema name exists at the target snapshot (use list_schemas) |
| **Example message** | `"schema 'analytics' does not exist"` |

---

## Class 42 — Syntax Error or Access Rule Violation

The most common error class in Rocklake. These errors indicate that the SQL statement is either not recognized or references objects that do not exist.

### 42000: syntax_error_or_access_rule_violation

General catch-all for class 42 errors that do not fit a more specific code.

| Aspect | Detail |
|--------|--------|
| **When returned** | Ambiguous syntax or access issues |
| **Client action** | Check the error message for specifics |

### 42601: syntax_error

The SQL statement was not recognized by Rocklake's bounded SQL classifier. This is the most frequently encountered error for clients sending arbitrary SQL.

| Aspect | Detail |
|--------|--------|
| **When returned** | Any SQL statement not in Rocklake's bounded set |
| **Common causes** | Sending SELECT queries (Rocklake is not a query engine), using SQL syntax from a newer DuckDB version not yet supported, typos in DuckLake protocol SQL |
| **Client action** | Verify the statement matches a supported pattern (see [Supported SQL](sql-supported.md)) |
| **Example message** | `"statement not recognized by bounded SQL classifier"` |
| **Hint included** | `"Rocklake only accepts DuckLake protocol statements"` |

**Important:** This error does NOT mean the SQL is syntactically invalid in general — it means Rocklake specifically does not handle this statement pattern.

### 42P01: undefined_table

A referenced table does not exist at the requested snapshot.

| Aspect | Detail |
|--------|--------|
| **When returned** | DROP TABLE, ALTER TABLE, INSERT data file for non-existent table |
| **Common causes** | Table was dropped in a later snapshot, table name misspelled, wrong schema context |
| **Client action** | Verify table exists at the target snapshot |
| **Example message** | `"table 'events' does not exist in schema 'main'"` |

### 42P06: duplicate_schema

Attempted to create a schema with a name that already exists.

| Aspect | Detail |
|--------|--------|
| **When returned** | CREATE SCHEMA when the name is taken |
| **Common causes** | Re-running initialization scripts, schema already exists |
| **Client action** | Use IF NOT EXISTS semantics (if available) or check before creating |
| **Example message** | `"schema 'analytics' already exists"` |

### 42P07: duplicate_table

Attempted to create a table with a name that already exists in the target schema.

| Aspect | Detail |
|--------|--------|
| **When returned** | CREATE TABLE when the name is taken |
| **Common causes** | Re-running migration scripts, table already exists |
| **Client action** | Use IF NOT EXISTS semantics (if available) or check before creating |
| **Example message** | `"table 'users' already exists in schema 'main'"` |

### 42703: undefined_column

A referenced column does not exist in the target table at the requested snapshot.

| Aspect | Detail |
|--------|--------|
| **When returned** | DROP COLUMN, RENAME COLUMN for non-existent column |
| **Common causes** | Column was already dropped, column name misspelled |
| **Client action** | Verify column exists (list_columns) |

### 42710: duplicate_object

A generic duplicate object error for entities other than schemas and tables (views, macros, types, sequences).

| Aspect | Detail |
|--------|--------|
| **When returned** | Creating an object that already exists |
| **Client action** | Check for existing object before creating |

---

## Class 57 — Operator Intervention

### 57P04: writer_fenced

The current writer has been fenced (invalidated) because another writer started with a higher epoch. This is the expected behavior in deployments where a new instance replaces an old one.

| Aspect | Detail |
|--------|--------|
| **When returned** | Any write operation after another writer has taken over |
| **Common causes** | New deployment started, old instance still running, split-brain resolution |
| **Client action** | Disconnect from this instance and reconnect to the new writer. The old writer will never accept writes again. |
| **Example message** | `"writer fenced: current epoch is 5, but epoch 6 was observed"` |
| **Recovery** | This is NOT an error requiring intervention. It is the normal failover mechanism. Connect to the new writer. |

**Important:** Once fenced, a writer NEVER recovers. Do not retry on the same connection. Establish a new connection (which will connect to the new writer).

---

## Class 58 — System Error (External)

### 58000: system_error

An error occurred in the underlying object storage system. Rocklake could not complete the operation due to an infrastructure issue.

| Aspect | Detail |
|--------|--------|
| **When returned** | Object storage request failed |
| **Common causes** | S3 timeout, permission denied (IAM role expired), bucket not found, network connectivity to storage |
| **Client action** | Retry with exponential backoff. If persistent, check storage credentials and connectivity. |
| **Example message** | `"object storage error: PutObject failed: AccessDenied"` |

**Subcauses (included in error detail):**

| Detail | Meaning | Action |
|--------|---------|--------|
| `AccessDenied` | IAM credentials insufficient | Check IAM role/policy |
| `NoSuchBucket` | Storage bucket does not exist | Verify ROCKLAKE_STORAGE path |
| `RequestTimeout` | Storage did not respond in time | Retry (transient) |
| `SlowDown` | Storage is throttling requests | Back off, retry later |
| `ServiceUnavailable` | Storage service is down | Wait and retry |

---

## Class XX — Internal Error

### XX000: internal_error

An unexpected internal error occurred. This indicates a bug in Rocklake or data corruption.

| Aspect | Detail |
|--------|--------|
| **When returned** | Protobuf decode failure, invariant violation, unexpected state |
| **Common causes** | Bug in Rocklake, catalog data corruption (extremely rare), version mismatch between binary and catalog format |
| **Client action** | Report this as a bug. Include the full error message and any relevant context (Rocklake version, operation being performed, storage backend). |
| **Example message** | `"internal error: failed to decode value for key [0x05 0x00...]: unexpected field tag"` |

**If you see XX000 errors repeatedly:**

1. Check Rocklake version matches the catalog format version
2. Run the verify tool (see [Operations: Verify & Repair](../operations/verify-repair.md))
3. Report the issue on GitHub with full error output

---

## Error Handling Best Practices

### For Client Applications

```python
import psycopg2

try:
    cursor.execute(sql)
except psycopg2.errors.SyntaxError:          # 42601
    # Statement not supported — check SQL format
    pass
except psycopg2.errors.UndefinedTable:        # 42P01
    # Table doesn't exist at this snapshot
    pass
except psycopg2.errors.ReadOnlySqlTransaction: # 25006
    # Connected to read-only instance
    pass
except psycopg2.errors.OperationalError as e:
    if e.pgcode == '57P04':
        # Writer fenced — reconnect to new writer
        reconnect()
    elif e.pgcode == '58000':
        # Storage error — retry with backoff
        retry_with_backoff()
    else:
        raise
```

### Retry Logic

| Error Code | Retryable | Strategy |
|-----------|-----------|----------|
| 42601 | No | Fix the SQL statement |
| 42P01 | No | Fix the table reference |
| 42P06 | No | Check before creating |
| 57P04 | No (on same connection) | Reconnect to new writer |
| 58000 | Yes | Exponential backoff (100ms, 200ms, 400ms, ...) |
| 08006 | Yes | Reconnect and retry |
| XX000 | No | Report bug |

### Idempotency

For operations that may fail mid-execution (connection drops after commit but before response):

1. Query the catalog state to check if the operation already succeeded
2. If the object already exists in the expected state, treat as success
3. If not, retry the operation

This is particularly important for `08006` (connection_failure) where the commit may have succeeded but the response was lost.

## Further Reading

- **[Supported SQL](sql-supported.md)** — What SQL statements Rocklake accepts
- **[Internals: SQLSTATE Mapping](../internals/sqlstate-mapping.md)** — How internal errors map to SQLSTATE codes
- **[Operations: Troubleshooting](../operations/troubleshooting.md)** — Diagnosing common error scenarios
