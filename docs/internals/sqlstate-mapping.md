# SQLSTATE Mapping

When Rocklake encounters an error — a table that does not exist, a write attempt on a read-only connection, a storage backend failure — it must communicate that error to the client (DuckDB) over the PostgreSQL wire protocol. The PostgreSQL protocol uses SQLSTATE codes: standardized 5-character strings that classify errors into categories. This page documents how Rocklake maps its internal Rust error types to SQLSTATE codes, the principles behind these mappings, and how clients should handle errors programmatically.

SQLSTATE codes are one of those features that most developers never think about until something goes wrong. But they are the backbone of programmatic error handling in the PostgreSQL ecosystem. DuckDB, psql, JDBC drivers, and every other PostgreSQL client uses these codes to decide how to react to errors — whether to retry, whether the connection is still usable, whether the transaction can continue. Getting these codes right is essential for clients that need to handle errors gracefully.

## SQLSTATE Code Structure

Every SQLSTATE code is exactly 5 characters, divided into:

- **Characters 1-2:** The error class (broad category)
- **Characters 3-5:** The specific condition within that class

For example, `42P01`:
- Class `42` = "Syntax Error or Access Rule Violation"
- Condition `P01` = "Undefined Table"

The class determines the severity of the error (system error vs. client error vs. transient condition). The specific condition allows targeted handling of particular error types.

## Complete Mapping Table

| Internal Error | SQLSTATE | Class | Class Name | Client Action |
|---------------|----------|-------|------------|---------------|
| `StorageError` | 58000 | 58 | System Error | Retry after delay |
| `StorageTimeout` | 57014 | 57 | Operator Intervention | Retry after delay |
| `WriterFenced` | 57P04 | 57 | Operator Intervention | Reconnect (new writer) |
| `FormatVersionMismatch` | 0A000 | 0A | Feature Not Supported | Upgrade binary |
| `DecodeError` | XX000 | XX | Internal Error | Report bug |
| `InvalidStatement` | 42601 | 42 | Syntax Error | Fix SQL |
| `UnknownStatement` | 42601 | 42 | Syntax Error | Statement not supported |
| `TableNotFound` | 42P01 | 42 | Undefined Table | Fix table reference |
| `SchemaNotFound` | 3F000 | 3F | Invalid Schema Name | Fix schema reference |
| `ColumnNotFound` | 42703 | 42 | Undefined Column | Fix column reference |
| `DuplicateTable` | 42P07 | 42 | Duplicate Table | Table already exists |
| `DuplicateSchema` | 42P06 | 42 | Duplicate Schema | Schema already exists |
| `DuplicateColumn` | 42701 | 42 | Duplicate Column | Column already exists |
| `TransactionActive` | 25001 | 25 | Invalid Transaction State | Commit or rollback first |
| `NoTransaction` | 25P01 | 25 | No Active Transaction | Begin transaction first |
| `ReadOnly` | 25006 | 25 | Read Only Transaction | Use writer connection |
| `SnapshotTooOld` | 22023 | 22 | Invalid Parameter Value | Use more recent snapshot |
| `InvalidParameter` | 22023 | 22 | Invalid Parameter Value | Fix parameter value |
| `CatalogNotInitialized` | 55000 | 55 | Object Not In Prerequisite State | Initialize catalog |
| `AuthenticationFailed` | 28P01 | 28 | Invalid Authorization | Fix credentials |

## Mapping Principles

### Use Standard Codes Where Possible

PostgreSQL defines hundreds of SQLSTATE codes (documented in Appendix A of the PostgreSQL manual). Rocklake reuses existing codes rather than inventing custom ones. This ensures that standard PostgreSQL client libraries (psycopg2, node-postgres, JDBC) can handle errors using their built-in error class hierarchies.

For example, `DuplicateTable` maps to `42P07` — the same code PostgreSQL uses. A Python application that catches `psycopg2.errors.DuplicateTable` will work identically against Rocklake and PostgreSQL.

### Class Accuracy Over Specificity

If no exact code exists for a Rocklake-specific condition, the mapping prioritizes correct error class over a specific but misleading code. The error class (first 2 characters) determines:

- Whether the client should retry (class 57, 58) or not (class 42)
- Whether the connection is still usable (class 08 = connection broken, others = connection OK)
- Whether the transaction has been rolled back (class 40) or is still active (other classes)

Getting the class wrong causes incorrect client behavior. Getting the specific condition wrong just makes error messages less precise.

### Vendor-Specific Conditions

For error conditions unique to Rocklake (like `WriterFenced`), the nearest semantically appropriate PostgreSQL code is used:

- `WriterFenced` → `57P04` (originally "database_dropped" in PostgreSQL). The semantics are similar: "your session is no longer valid because of an administrative action." The `P` in position 3 indicates a PostgreSQL vendor extension. DuckDB treats any class-57 error as an operator intervention, prompting retry behavior.

### Never Invent Non-Standard Codes

Rocklake does not use codes outside the PostgreSQL-defined space. Custom codes (like `SD001`) would confuse client libraries that validate code format and could break error handling in tools that assume all codes follow PostgreSQL's allocation scheme.

## Error Severity

PostgreSQL's error protocol includes a severity field. Rocklake uses:

| Severity | When Used | Connection Impact |
|----------|-----------|-------------------|
| ERROR | All operational errors | Connection remains usable |
| FATAL | Authentication failure only | Connection terminated |
| PANIC | Never | (would terminate all connections) |

All errors except authentication failures are reported as ERROR severity. This means the connection remains usable after any error — the client can send the next query immediately. This is important for DuckDB, which may send many queries in sequence and expects individual query failures not to break the connection.

### Why Not FATAL for Storage Errors?

Storage errors (S3 timeout, permission denied) are reported as ERROR, not FATAL. This is because storage errors are often transient — the next request may succeed. If Rocklake terminated the connection on every S3 timeout, DuckDB would need to re-establish connections frequently during temporary network issues.

DuckDB handles ERROR responses by retrying the query (if applicable) or reporting the error to the user. It handles FATAL responses by tearing down the entire catalog connection and requiring re-initialization.

## Error Response Format

The PostgreSQL wire protocol error response (ErrorResponse message) includes several fields:

```
ErrorResponse:
  S (Severity):  ERROR
  V (Severity):  ERROR  (non-localized)
  C (Code):      42P01
  M (Message):   Table "orders" does not exist
  D (Detail):    The table was referenced in a SELECT query but no table with this name exists in the current schema.
  H (Hint):      Check the table name spelling or create the table first.
  F (File):      catalog/src/reader.rs
  L (Line):      142
  R (Routine):   handle_select
```

Rocklake populates:

- **Severity:** Always ERROR (or FATAL for auth)
- **Code:** The mapped SQLSTATE
- **Message:** A concise human-readable description
- **Detail:** Additional context (when available)
- **Hint:** Suggested resolution (when applicable)

File/Line/Routine fields are included in debug builds but omitted in release builds (to avoid exposing internal code structure).

## Client Error Handling Patterns

### DuckDB (Native Client)

DuckDB's `ducklake` extension handles SQLSTATE codes internally:

```
Class 42 (Syntax/Table errors): Report to user, do not retry
Class 25 (Transaction state):    Handle internally (rollback, retry)
Class 57 (Operator intervention): Retry with backoff
Class 58 (System error):         Retry with backoff, then report to user
Class 08 (Connection error):     Reconnect and retry
```

### Python (psycopg2)

```python
import psycopg2
from psycopg2 import errors

try:
    cursor.execute("SELECT * FROM ducklake_tables ...")
except errors.UndefinedTable:
    # SQLSTATE 42P01 — table does not exist
    print("Table not found — check schema")
except errors.ReadOnlySqlTransaction:
    # SQLSTATE 25006 — attempted write on reader
    print("This is a read-only connection")
except errors.OperationalError as e:
    if e.pgcode and e.pgcode[:2] == '58':
        # System error — retry
        time.sleep(1)
        retry()
```

### Go (pgx)

```go
import "github.com/jackc/pgx/v5/pgconn"

err := conn.Exec(ctx, "INSERT INTO ...")
if err != nil {
    var pgErr *pgconn.PgError
    if errors.As(err, &pgErr) {
        switch pgErr.Code {
        case "42P07":
            // Duplicate table — handle idempotently
        case "25006":
            // Read-only — switch to writer connection
        default:
            if pgErr.Code[:2] == "58" {
                // System error — retry
            }
        }
    }
}
```

## Transaction Behavior After Errors

In PostgreSQL, some errors abort the current transaction (requiring ROLLBACK before new queries). Rocklake's behavior:

| Error Class | Transaction State After Error |
|-------------|------------------------------|
| 42 (Syntax/Table) | Transaction still active (can send more queries) |
| 25 (Transaction state) | Depends on specific error |
| 22 (Invalid parameter) | Transaction still active |
| 23 (Constraint violation) | Transaction still active |
| 57, 58 (System) | Transaction rolled back |
| XX (Internal) | Transaction rolled back |

This matches PostgreSQL's behavior for most error classes. The key difference: Rocklake does not implement `SAVEPOINT`, so there is no way to partially roll back within a transaction.

## Testing SQLSTATE Codes

The test suite verifies SQLSTATE mappings through:

1. **Direct mapping tests:** Each internal error variant is constructed and its SQLSTATE output is verified
2. **Integration tests:** DuckDB operations that trigger errors (CREATE TABLE on existing table, SELECT from missing table) verify the received SQLSTATE matches expectations
3. **Wire corpus tests:** Error responses captured from real DuckDB sessions are replayed and compared

## Adding New Error Codes

When adding a new error variant to Rocklake:

1. Choose the appropriate SQLSTATE from the [PostgreSQL error code list](https://www.postgresql.org/docs/current/errcodes-appendix.html)
2. Verify the error class matches the nature of the error (client mistake vs. system failure vs. transient condition)
3. Add the mapping to `crates/rocklake-pgwire/src/error.rs`
4. Add a test verifying the mapping
5. Document the mapping in this page

## Further Reading

- **[Architecture: PG Wire Protocol](../architecture/pg-wire-protocol.md)** — Wire protocol implementation details
- **[Wire Corpus](wire-corpus.md)** — How error responses are tested
- **[Reference: Error Codes](../reference/error-codes.md)** — User-facing error code reference
- **[Integration: Custom Clients](../integration/custom-clients.md)** — Building clients that handle errors correctly
