# Bounded SQL

SlateDuck's SQL layer is a **bounded dispatcher** that recognizes exactly the SQL statement shapes emitted by supported clients and maps each to a catalog operation.

## What "Bounded" Means

The dispatcher has a finite, enumerable set of supported patterns. Anything not matching returns `SQLSTATE 0A000`.

## Why Not a Full SQL Engine?

1. **Security:** Finite attack surface, fully auditable
2. **Correctness:** 100% coverage of the statement space
3. **Performance:** Pattern matching is O(1)
4. **Maintainability:** ~30-40 match arms, not a query optimizer

## What Is Supported

All statement shapes emitted by DuckDB's `ducklake` extension, plus extensions for compatible clients. See [Reference: SQL Supported](../reference/sql-supported.md).
