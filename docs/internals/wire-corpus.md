# Wire Corpus

The wire corpus is SlateDuck's ground truth — a collection of real PostgreSQL wire protocol sessions captured from actual DuckDB interactions with DuckLake catalogs. Every SQL statement that DuckDB's `ducklake` extension has ever been observed to send is recorded in the corpus. SlateDuck's SQL classifier must correctly handle every statement in the corpus, making it both the specification and the test suite for SQL compatibility.

This page documents the purpose of the wire corpus, its structure, how new entries are captured, how corpus tests work, and the role it plays in maintaining compatibility across DuckDB versions. For contributors working on the SQL classifier, the wire corpus is the single most important artifact — it defines what "correct" means.

## Purpose

SlateDuck does not implement a general SQL parser. It implements a pattern matcher (the SQL classifier) that recognizes specific SQL statement patterns and dispatches to appropriate handlers. But which patterns should it recognize? The answer is not "whatever SQL looks reasonable" — it is "exactly the patterns that DuckDB actually sends."

This distinction is critical. DuckDB's SQL generation may include:

- Unusual whitespace or formatting
- Non-standard quoting conventions
- Column ordering that differs from documentation
- Version-specific syntax variations
- Edge cases in identifier escaping
- Specific parameter placeholder styles

Hand-writing test cases from documentation would miss these details. The wire corpus captures reality — what bytes actually appear on the wire when DuckDB talks to a DuckLake catalog.

### The Corpus as Specification

The wire corpus serves as the authoritative specification of SlateDuck's SQL surface:

- **If a statement exists in the corpus:** SlateDuck MUST handle it correctly
- **If a statement does NOT exist in the corpus:** SlateDuck is NOT required to handle it
- **If a new DuckDB version generates a new pattern:** It must be added to the corpus, and SlateDuck must be updated to handle it

This gives a precise, testable definition of "bounded SQL" — the set of SQL patterns that SlateDuck supports is exactly the set captured in the corpus.

## Structure

The corpus lives in the test fixtures directory and is organized by DuckDB version and operation category:

```
tests/fixtures/wire-corpus/
├── duckdb-1.1.0/
│   ├── connect.sql
│   ├── create-schema.sql
│   ├── create-table.sql
│   ├── alter-table-add-column.sql
│   ├── alter-table-drop-column.sql
│   ├── drop-table.sql
│   ├── insert-data-file.sql
│   ├── insert-delete-file.sql
│   ├── list-schemas.sql
│   ├── list-tables.sql
│   ├── list-columns.sql
│   ├── list-data-files.sql
│   ├── create-view.sql
│   ├── snapshot-operations.sql
│   └── transaction-control.sql
├── duckdb-1.2.0/
│   ├── create-schema.sql
│   ├── create-table.sql
│   ├── ...
│   └── new-feature.sql
└── duckdb-1.5.x/
    ├── create-schema.sql
    └── ...
```

### File Format

Each `.sql` file contains one or more SQL statements exactly as DuckDB emits them, including whitespace, quoting, parameter markers, and trailing semicolons:

```sql
-- File: duckdb-1.2.0/create-table.sql
-- Captured from DuckDB 1.2.0 during CREATE TABLE with 3 columns

BEGIN TRANSACTION;
INSERT INTO ducklake_tables (table_id, schema_id, table_name, begin_snapshot_id)
  VALUES ($1, $2, $3, $4);
INSERT INTO ducklake_columns (column_id, table_id, column_name, data_type, column_index, begin_snapshot_id)
  VALUES ($1, $2, $3, $4, $5, $6);
INSERT INTO ducklake_columns (column_id, table_id, column_name, data_type, column_index, begin_snapshot_id)
  VALUES ($1, $2, $3, $4, $5, $6);
INSERT INTO ducklake_columns (column_id, table_id, column_name, data_type, column_index, begin_snapshot_id)
  VALUES ($1, $2, $3, $4, $5, $6);
INSERT INTO ducklake_table_snapshot (table_id, snapshot_id) VALUES ($1, $2);
INSERT INTO ducklake_snapshot (snapshot_id, snapshot_time) VALUES ($1, $2);
COMMIT;
```

The file preserves the exact column ordering, parameter style, and statement sequence. Metadata comments (prefixed with `--`) provide context but are ignored by the test runner.

### Expected Results

Alongside each `.sql` file, an expected result file (`.expected.json`) defines what the classifier should produce:

```json
{
  "statements": [
    {"kind": "BeginTransaction"},
    {"kind": "InsertTable", "params": {"table": "ducklake_tables"}},
    {"kind": "InsertColumn", "params": {"table": "ducklake_columns"}},
    {"kind": "InsertColumn", "params": {"table": "ducklake_columns"}},
    {"kind": "InsertColumn", "params": {"table": "ducklake_columns"}},
    {"kind": "InsertTableSnapshot"},
    {"kind": "InsertSnapshot"},
    {"kind": "Commit"}
  ]
}
```

## How Corpus Tests Work

The corpus test runner lives in `tests/golden/` and executes during `cargo test`:

### Step 1: Load Corpus Files

For each `.sql` file in the corpus directory:

```rust
let sql_content = fs::read_to_string(corpus_file)?;
let expected = load_expected_results(expected_file)?;
```

### Step 2: Classify Each Statement

Each SQL statement is passed through the SQL classifier:

```rust
for (statement, expected_kind) in statements.zip(expected.statements) {
    let result = classify_statement(&statement);
    assert_eq!(result.kind, expected_kind.kind,
        "Classifier mismatch for statement in {}", corpus_file);
}
```

### Step 3: Verify Parameter Extraction

For statements that extract parameters (table names, column names, schema names), verify they match:

```rust
if let Some(expected_params) = expected_kind.params {
    assert_eq!(result.params, expected_params,
        "Parameter extraction mismatch for {} in {}", 
        expected_kind.kind, corpus_file);
}
```

### Step 4: Report Results

Failed tests report exactly which corpus file failed, which statement within the file, and what the classifier produced vs. what was expected. This makes it straightforward to diagnose compatibility issues.

## Capturing New Corpus Entries

When a new DuckDB version is released that changes SQL generation patterns, new corpus entries must be captured. The process:

### Step 1: Set Up Capture Environment

Run DuckDB with the new version against a PostgreSQL-backed DuckLake catalog with wire protocol logging enabled:

```bash
# Start PostgreSQL with full query logging
docker run -e POSTGRES_LOG_STATEMENT=all -p 5432:5432 postgres:16

# Initialize DuckLake catalog
psql -h localhost -U postgres -c "CREATE DATABASE ducklake_test;"
# (apply DuckLake schema)

# Run DuckDB with the new version
./duckdb-new-version
```

### Step 2: Exercise All Operations

In DuckDB, execute every catalog operation:

```sql
-- In DuckDB:
ATTACH 'ducklake:postgresql://localhost/ducklake_test' AS lake;
CREATE SCHEMA lake.test_schema;
CREATE TABLE lake.test_schema.test_table (id INTEGER, name VARCHAR, created TIMESTAMP);
ALTER TABLE lake.test_schema.test_table ADD COLUMN email VARCHAR;
INSERT INTO lake.test_schema.test_table VALUES (1, 'test', now());
-- ... all operations
```

### Step 3: Extract SQL from Logs

Parse the PostgreSQL logs to extract the SQL statements:

```bash
grep "LOG:  statement:" postgresql.log | \
  sed 's/.*LOG:  statement: //' > captured-statements.sql
```

### Step 4: Organize and Validate

Split the captured statements into category files, add expected results, and run the corpus tests:

```bash
# Create new version directory
mkdir tests/fixtures/wire-corpus/duckdb-X.Y.Z/

# Place organized .sql files
# Create corresponding .expected.json files

# Run tests
cargo test --test corpus_tests
```

### Step 5: Fix Classifier If Needed

If new corpus entries reveal patterns the classifier does not handle, update the classifier first, then verify all corpus tests pass (including the new entries).

## Version Compatibility Matrix

The corpus tracks compatibility across DuckDB versions:

| DuckDB Version | Corpus Status | New Patterns | Breaking Changes |
|---------------|---------------|-------------|-----------------|
| 1.5.x | Complete | Postgres-scanner initialization queries | None |

"Breaking changes" means: DuckDB changed a SQL pattern such that the old classifier no longer handles it. This has not happened yet, but the corpus would detect it immediately if it did.

## Relationship to Bounded SQL

The wire corpus defines the exact boundary of SlateDuck's bounded SQL support:

1. **The corpus is the specification.** If a pattern is in the corpus, it is within bounds.
2. **Everything else is out of bounds.** Statements not matching any corpus pattern are rejected with SQLSTATE 42601 (Syntax Error).
3. **The boundary grows slowly.** New corpus entries are added only when a new DuckDB version introduces new patterns.
4. **The boundary never shrinks.** Old patterns are never removed (backward compatibility with older DuckDB versions).

This gives a precise answer to "what SQL does SlateDuck support?" — run the corpus and see what passes.

## Corpus Maintenance

### When to Add Entries

- New DuckDB minor version released
- DuckLake protocol adds new operations (new table types, new SQL patterns)
- Bug report reveals a pattern not in the corpus (DuckDB sends something we do not handle)

### When NOT to Add Entries

- Random SQL from psql that DuckDB would never generate
- SQL patterns from other tools (Grafana, Metabase) that SlateDuck is not designed to handle
- Hypothetical patterns that might exist in future DuckDB versions

### Keeping the Corpus Clean

- Each file has a clear header comment explaining what operation it captures
- Expected results are manually verified (not auto-generated)
- Duplicate patterns across versions are acceptable (they prove backward compatibility)
- Files are named descriptively (not `test1.sql`, `test2.sql`)

## Corpus Analysis Tooling

SlateDuck provides utilities for analyzing the wire corpus:

### Coverage Report

```bash
# Show which classifier branches are exercised by the corpus
cargo test --test corpus_coverage -- --show-coverage
```

This reports which SQL patterns the classifier can handle and which have at least one corpus entry exercising them. A pattern without corpus coverage is technically supported but not validated against real DuckDB behavior — a risky situation.

### Diff Between Versions

```bash
# Show new patterns in duckdb-1.2.0 that did not exist in duckdb-1.1.0
diff <(ls tests/fixtures/wire-corpus/duckdb-1.1.0/) <(ls tests/fixtures/wire-corpus/duckdb-1.2.0/)
```

This quickly identifies what new operations a DuckDB version introduces, helping contributors prioritize classifier work.

### Statement Frequency Analysis

In production, SlateDuck can log which classifier branches are actually hit. The corpus should reflect real usage patterns:

| Category | Typical Frequency | Corpus Files |
|----------|------------------|-------------|
| Connection/startup | Every connection | 2–3 files |
| Schema listing | Very frequent | 3–5 files |
| Table listing | Very frequent | 3–5 files |
| Column listing | Frequent | 2–4 files |
| Data file registration | Frequent (writes) | 5–8 files |
| Schema/table creation | Infrequent | 3–5 files |
| ALTER operations | Rare | 5–10 files |
| DROP operations | Rare | 3–5 files |
| Transaction control | Every session | 2–3 files |

### Regression Detection

The corpus acts as a regression detector. When updating the SQL classifier, a single failing corpus test means either:

1. **A bug was introduced:** The change broke handling of an existing pattern. Fix the classifier.
2. **The pattern genuinely changed:** DuckDB updated its SQL generation. Update the corpus entry to reflect the new pattern and adjust the classifier.

Case 2 should be rare and always tied to a DuckDB version upgrade with documented changes.

## Security Implications of the Corpus

The bounded SQL design enforced by the wire corpus has security benefits:

- **SQL injection surface is minimal.** SlateDuck only accepts patterns matching the corpus. Arbitrary SQL (even valid SQL) is rejected if it does not match a known pattern. An attacker who achieves SQL injection in a DuckDB client gains nothing — the injected SQL will not match any corpus pattern and will be rejected.
- **No dynamic SQL execution.** SlateDuck never concatenates user input into SQL strings for execution. It pattern-matches incoming SQL and dispatches to typed handlers with validated parameters.
- **Audit trail.** Every classified statement is logged with its category, making anomaly detection straightforward.

## Historical Context

The wire corpus approach was chosen after evaluating alternatives:

- **Full SQL parser (sqlparser-rs):** Rejected because DuckDB generates PostgreSQL-dialect SQL with DuckDB-specific extensions. No off-the-shelf parser handles this hybrid correctly. Maintaining a fork would be as much work as the classifier approach, without the simplicity benefits.
- **Protocol-level mocking:** Rejected because DuckDB's protocol behavior depends on server responses (it adapts its queries based on what the previous query returned). Static mocking cannot capture these multi-step interactions.
- **Specification-driven development:** Rejected because DuckLake's protocol documentation does not specify exact SQL text — only semantic operations. The wire corpus captures the actual bytes.

## Further Reading

- **[Architecture: SQL Dispatcher](../architecture/sql-dispatcher.md)** — How the classifier works
- **[Design Decisions: Bounded SQL](../design-decisions/bounded-sql.md)** — Why SQL is bounded
- **[SQLSTATE Mapping](sqlstate-mapping.md)** — Error codes for rejected statements
- **[Contributing: Testing](../contributing/testing.md)** — How to run corpus tests
