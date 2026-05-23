# Bounded SQL

The SQL dispatcher accepts exactly the statement shapes emitted by supported clients. Not a general SQL engine.

## Why?

- **Security:** Finite, enumerable attack surface
- **Correctness:** 100% test coverage of statement space
- **Performance:** O(1) pattern matching
- **Maintainability:** ~30-40 match arms

## The Cost

Cannot run arbitrary SQL against SlateDuck. `SQLSTATE 0A000` for JOINs, CTEs, subqueries. Use `slateduck inspect` or `slateduck export` for ad-hoc exploration.

## Why This Is Acceptable

DuckLake's spec defines a finite set of catalog operations — all supported. The dispatcher serves the spec completely.
