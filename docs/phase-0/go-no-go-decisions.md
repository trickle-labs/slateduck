# Phase 0 Go/No-Go Decisions

> Final decision record for Rocklake v0.1 Foundation phase.
> All gates pass. Proceed to v0.2.

## Decision Summary

| Decision Area | Outcome | Rationale |
|--------------|---------|-----------|
| **SQL Execution Layer** | Custom AST-matching dispatcher | GlueSQL requires 15+ shims (threshold: 10) |
| **Transaction API** | `db.begin(SerializableSnapshot)` | Provides serializable isolation with conflict detection |
| **Conditional Init** | Transaction-based insert-if-absent | Works without external lock; two concurrent inits converge |
| **`flush()` Barrier** | Use as post-commit visibility fence | DbReader sees flushed keys immediately |
| **pgwire Extended Protocol** | pgwire crate v0.28 is sufficient | Full Parse/Bind/Describe/Execute/Sync support |
| **Writer Fencing** | SlateDB-native fencing | Second writer fences first automatically |
| **Counter Allocation** | In-memory cache + transactional persist | Single transaction: read counter, increment, write consuming row |
| **Key Layout** | Confirmed from wire corpus analysis | All proposed key shapes validated |
| **Credential Isolation** | S3 prefix-based IAM separation | Catalog-only and data-only policies work independently |

## Gate Status

All 10 SlateDB API validation gates: **PASS**

## Blockers

None identified. All assumptions validated successfully.

## Next Steps

Proceed to **v0.2 — Catalog Core**: implement the full binary key layout,
value encoding, counter allocation, and MVCC filtering for all 28 DuckLake
tables.
