# Wire Corpus

## Capture

1. Start PostgreSQL with DuckLake
2. Start capture proxy between DuckDB and PostgreSQL
3. Run full DuckLake tutorial
4. Store as JSONL

## Replay

Read corpus, send client messages to SlateDuck, compare responses byte-for-byte (masking server-generated values).

## Fixtures

| Fixture | Path |
|---------|------|
| DuckDB 1.2.2 | `tests/fixtures/wire-corpus/duckdb-1.2.2.jsonl` |
| Handshake | `tests/fixtures/handshake/duckdb-1.2.2.jsonl` |
