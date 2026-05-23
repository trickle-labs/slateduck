# DuckDB Compatibility Matrix

This document tracks which DuckDB versions are tested and supported.

## Supported Versions

| DuckDB Version | Status | Wire Corpus | Notes |
|----------------|--------|-------------|-------|
| 1.2.2 | Baseline | `tests/fixtures/wire-corpus/duckdb-1.2.2.jsonl` | Phase 0 capture |

## Version Policy

- **Minor version bumps** (e.g., 1.2.x → 1.2.y): New corpus capture + regression test required.
- **Major version bumps** (e.g., 1.x → 2.x): Full new client treatment — complete recapture and validation of all catalog operations.

## Testing

Each new DuckDB version requires:
1. Capture of the full wire corpus against SlateDuck
2. Comparison with the baseline golden output
3. Explicit sign-off before marking as supported

## Known Differences

None at this time. The Phase 0 corpus was captured against DuckDB 1.2.2 connecting to PostgreSQL-backed DuckLake.
