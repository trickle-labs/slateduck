# DuckDB Compatibility

## Supported Versions

| DuckDB Version | Status | Corpus Fixture |
|----------------|--------|----------------|
| 1.2.2 | Baseline | `tests/fixtures/wire-corpus/duckdb-1.2.2.jsonl` |

## Validation Process

1. Capture corpus against PostgreSQL-backed DuckLake with new DuckDB version
2. Classify statements against bounded dispatcher taxonomy
3. Implement any new shapes
4. Run replay test — verify byte-for-byte identical responses
5. Update matrix

## CI

The `compatibility.yml` workflow runs wire-corpus replay on every push and PR.
