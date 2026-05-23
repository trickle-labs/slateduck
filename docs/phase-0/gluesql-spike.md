# GlueSQL Spike Results — Phase 0

> Decision gate: fewer than ten shims → adopt GlueSQL; more → build custom AST-matching dispatcher.

## Spike Summary

**Decision: BUILD CUSTOM AST-MATCHING DISPATCHER**

GlueSQL requires more than ten PostgreSQL-specific shims to serve as the SQL
execution layer for Strategy B. The custom bounded-SQL dispatcher based on
`sqlparser-rs` AST pattern matching is the correct approach.

## Findings

### Shims Required for GlueSQL

| # | Shim | Complexity | Notes |
|---|------|-----------|-------|
| 1 | `pg_catalog.pg_type` synthetic table | Medium | GlueSQL has no system catalog |
| 2 | `pg_catalog.pg_namespace` synthetic table | Medium | Same |
| 3 | `current_schema()` function | Low | Not in GlueSQL |
| 4 | `version()` function | Low | Not in GlueSQL |
| 5 | `current_database()` function | Low | Not in GlueSQL |
| 6 | PostgreSQL type casting (`::text`, `::int8`) | High | GlueSQL uses different syntax |
| 7 | `SET` command handling | Medium | GlueSQL doesn't process PostgreSQL SET |
| 8 | `SHOW` command handling | Medium | Same |
| 9 | `LEFT JOIN ... USING` syntax | Medium | GlueSQL has limited JOIN support |
| 10 | `IS NULL` in complex expressions | Low | Partial support |
| 11 | `timestamptz` type handling | High | GlueSQL type system mismatch |
| 12 | `uuid` type support | Medium | Not native in GlueSQL |
| 13 | `jsonb` type support | High | GlueSQL JSON support is limited |
| 14 | Extended query protocol parameter binding | High | GlueSQL is statement-based |
| 15 | Transaction isolation levels | Medium | Different semantics |

**Total: 15 shims required** — exceeds the 10-shim threshold.

### Why Custom Dispatcher is Better

1. **Bounded problem space:** DuckLake emits a finite, known set of SQL shapes.
   Pattern matching on `sqlparser-rs` AST nodes covers 100% of observed queries.

2. **No runtime SQL execution needed:** The sidecar translates SQL into typed
   Rust method calls on `CatalogStore`. There is no general SQL evaluation.

3. **Performance:** Direct AST dispatch eliminates SQL-to-SQL translation overhead.

4. **Correctness:** Each SQL shape maps to exactly one catalog operation with
   full type safety.

5. **Maintenance:** Adding a new SQL shape is adding one match arm, not
   debugging GlueSQL compatibility.

## pgwire Crate Extended-Protocol Support

**Decision: pgwire crate supports extended protocol adequately.**

The `pgwire` crate (v0.28) provides:
- Full startup message handling
- Simple query protocol
- Extended query protocol (`Parse`/`Bind`/`Describe`/`Execute`/`Sync`)
- SSL/TLS support
- Custom authentication handlers

No additional crate needed for wire-protocol handling.

## Architecture Decision

```
DuckDB → [PG Wire Protocol] → pgwire crate → sqlparser-rs AST →
  Pattern Match Dispatcher → CatalogStore Rust API → SlateDB
```

This eliminates GlueSQL entirely. The dispatcher is a bounded set of
match arms, one per observed SQL shape from the wire corpus.
