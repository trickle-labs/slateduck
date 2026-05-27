# Testing

Rocklake has a multi-layered testing strategy that ensures correctness from individual functions up to full protocol interactions. The test suite is not an afterthought — it is a first-class artifact of the project, maintained with the same care as production code. Every feature has tests. Every bug fix has a regression test. Every encoding decision is verified with property-based tests. And the wire protocol is validated against real DuckDB output captured in a corpus of test fixtures.

This page describes the testing philosophy, the different types of tests in the project, how to run them, how to write effective new tests, and how to decide what kind of test your change needs.

## Testing Philosophy

Four principles guide Rocklake's testing strategy:

### 1. Every Bug Fix Requires a Regression Test

If a bug was found in production (or by a user), it means the existing test suite was insufficient. The fix must include a test that would have caught the bug if it had existed before. This test serves two purposes: it verifies the fix is correct, and it prevents the bug from being reintroduced in future changes.

The test should reproduce the exact conditions that triggered the bug. Do not write a test for "a similar scenario" — write a test for the exact scenario that failed.

### 2. Property-Based Tests for Encoders

Key encoding, value encoding, MVCC visibility, and sort order preservation are properties that must hold for ALL inputs, not just the inputs we can think of in example-based tests. Property-based tests (using the `proptest` crate) generate random inputs and verify that invariants hold:

- Encoding is reversible: `decode(encode(x)) == x` for all `x`
- Sort order is preserved: `x < y` implies `encode(x) < encode(y)` for all `x, y`
- MVCC visibility is consistent: a key is either visible or invisible at a given snapshot, never both

These tests find edge cases that humans miss — boundary values, overflow conditions, unusual combinations of fields.

### 3. Wire Corpus for Protocol Compatibility

DuckDB's ducklake extension sends specific SQL patterns over the PostgreSQL wire protocol. Rocklake must recognize and handle these patterns exactly. The wire corpus captures real DuckDB output (actual SQL strings sent by actual DuckDB versions) and uses them as test fixtures:

- When a new DuckDB version changes its SQL patterns, the corpus shows what changed
- When Rocklake adds support for a new statement, the corpus verifies it matches real DuckDB behavior
- When a protocol bug is reported, the exact bytes or SQL can be added to the corpus

The corpus is versioned by DuckDB version, so compatibility across versions is explicitly tracked.

### 4. Integration Tests for Workflows

Unit tests verify individual functions. But many bugs live in the interactions BETWEEN components — a write that is not visible to a subsequent read, a transaction that partially commits, a MVCC filter that drops too many or too few rows. Integration tests exercise complete workflows through the full stack (write → read → verify) to catch these interaction bugs.

## Test Types

### Unit Tests

Located in `#[cfg(test)] mod tests` blocks within source files. These test individual functions in isolation, typically with controlled inputs and explicit expected outputs.

```rust
#[test]
fn test_schema_key_roundtrip() {
    let key = SchemaKey::new(42, 100);
    let bytes = key.encode();
    let decoded = SchemaKey::decode(&bytes).unwrap();
    assert_eq!(key, decoded);
}

#[test]
fn test_schema_key_sort_order() {
    // Keys for the same schema should sort by snapshot ID (descending)
    let key_a = SchemaKey::new(42, 100).encode();  // schema 42, snapshot 100
    let key_b = SchemaKey::new(42, 200).encode();  // schema 42, snapshot 200
    assert!(key_b < key_a);  // Higher snapshot sorts first (descending)
}
```

When to write a unit test:
- Testing a pure function (no side effects, no I/O)
- Verifying encoding/decoding of a specific value
- Checking error handling for specific invalid inputs
- Validating sort order for specific key combinations

### Property-Based Tests

Located in `crates/rocklake-core/tests/property_tests.rs`. These use the `proptest` crate to generate thousands of random inputs and verify that invariants hold:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn key_encoding_preserves_sort_order(a: u64, b: u64) {
        let key_a = encode_u64(a);
        let key_b = encode_u64(b);
        // The byte-level sort order must match the numeric sort order
        assert_eq!(a.cmp(&b), key_a.cmp(&key_b));
    }

    #[test]
    fn value_encoding_is_reversible(
        schema_id in 0u32..1000000,
        name in "[a-z]{1,50}",
        snapshot in 0u64..u64::MAX,
    ) {
        let row = SchemaRow {
            schema_id,
            name: name.clone(),
            snapshot,
        };
        let encoded = row.encode();
        let decoded = SchemaRow::decode(&encoded).unwrap();
        assert_eq!(decoded.schema_id, schema_id);
        assert_eq!(decoded.name, name);
        assert_eq!(decoded.snapshot, snapshot);
    }
}
```

Property-based tests are configured to run at least 256 cases by default (controlled by `PROPTEST_CASES` environment variable). For CI, we run 1024 cases to increase the chance of finding edge cases.

When to write a property-based test:
- Any encoding/decoding function
- Sort order preservation
- Monotonicity guarantees (e.g., snapshot IDs always increase)
- Idempotency (applying an operation twice gives the same result)

### Integration Tests

Located in `crates/*/tests/`. These test complete operations through the public API of each crate, using real storage (local filesystem) rather than mocks:

```rust
#[tokio::test]
async fn test_create_table_full_workflow() {
    // Set up a temporary catalog
    let store = CatalogStore::open_temp().await.unwrap();
    let writer = store.writer().await.unwrap();

    // Create a schema
    let schema_id = writer.create_schema("main").await.unwrap();

    // Create a table in the schema
    let table_id = writer.create_table(schema_id, "users", &[
        Column::new("id", DuckType::BigInt),
        Column::new("name", DuckType::Varchar),
        Column::new("email", DuckType::Varchar),
    ]).await.unwrap();

    // Commit and read back
    let snapshot = writer.commit().await.unwrap();
    let reader = store.reader_at(snapshot).await.unwrap();

    let tables = reader.list_tables(schema_id).await.unwrap();
    assert_eq!(tables.len(), 1);
    assert_eq!(tables[0].name, "users");

    let columns = reader.list_columns(table_id).await.unwrap();
    assert_eq!(columns.len(), 3);
    assert_eq!(columns[0].name, "id");
    assert_eq!(columns[1].name, "name");
    assert_eq!(columns[2].name, "email");
}
```

Integration tests verify that:
- Multi-step operations produce the correct final state
- MVCC isolation works (reads at old snapshots do not see new writes)
- Garbage collection removes old data without affecting live reads
- Concurrent readers and writers do not interfere with each other

When to write an integration test:
- Any new catalog operation (create/alter/drop)
- MVCC visibility changes
- Multi-step workflows that span multiple writes
- Error recovery scenarios (write fails, state should be unchanged)

### Wire Corpus Tests

Located in `tests/golden/`. These verify that Rocklake's SQL classifier recognizes real SQL patterns sent by actual DuckDB versions:

```rust
#[test]
fn test_corpus_create_schema() {
    let sql = include_str!("../fixtures/wire-corpus/duckdb-1.2.0/create-schema.sql");
    let result = classify_statement(sql).unwrap();
    assert_eq!(result.kind, StatementKind::CreateSchema);
}

#[test]
fn test_corpus_create_table_with_columns() {
    let sql = include_str!("../fixtures/wire-corpus/duckdb-1.2.0/create-table.sql");
    let result = classify_statement(sql).unwrap();
    assert_eq!(result.kind, StatementKind::CreateTable);
    assert_eq!(result.table_name, Some("users".to_string()));
}
```

The wire corpus is organized by DuckDB version:

```
tests/fixtures/wire-corpus/
├── duckdb-1.2.0/
│   ├── create-schema.sql
│   ├── create-table.sql
│   ├── drop-table.sql
│   ├── insert-data-file.sql
│   └── ...
├── duckdb-1.3.0/
│   ├── create-schema.sql
│   └── ...
└── README.md
```

When to add a wire corpus entry:
- A new DuckDB version changes SQL patterns
- A new SQL statement type is being implemented
- A protocol bug is reported (capture the exact SQL that triggered it)

### Benchmark Tests

Located in `crates/rocklake-catalog/benches/`. These measure performance and detect regressions:

```rust
fn bench_prefix_scan(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let store = rt.block_on(CatalogStore::open_temp()).unwrap();
    // ... setup data ...

    c.bench_function("prefix_scan_100_tables", |b| {
        b.iter(|| {
            rt.block_on(reader.list_tables(schema_id)).unwrap()
        })
    });
}
```

Run benchmarks with:

```bash
# Run all benchmarks
cargo bench -p rocklake-catalog

# Run a specific benchmark
cargo bench -p rocklake-catalog -- prefix_scan

# Compare to a baseline (for detecting regressions)
cargo bench -p rocklake-catalog -- --save-baseline new
# ... switch to old version ...
cargo bench -p rocklake-catalog -- --baseline new
```

## Running the Test Suite

### Basic Commands

```bash
# All tests, all crates
cargo test

# Specific crate
cargo test -p rocklake-core

# Specific test (by substring match)
cargo test test_schema_key

# Show println! output during tests
cargo test -- --nocapture

# Run tests sequentially (useful for debugging race conditions)
cargo test -- --test-threads=1

# Run only tests that were previously failing
cargo test -- --include-ignored
```

### Environment Variables

| Variable | Purpose | Example |
|----------|---------|---------|
| `PROPTEST_CASES` | Number of property-test cases | `1024` |
| `RUST_LOG` | Log level during tests | `rocklake_catalog=debug` |
| `TEST_S3_BUCKET` | S3 bucket for integration tests | `s3://my-test-bucket/ci/` |
| `ROCKLAKE_TEST_MINIO` | Enable MinIO integration tests | `1` |

### CI Test Matrix

The CI pipeline runs:

- All tests on Linux x86_64, macOS ARM64, Windows x86_64
- Rust stable and nightly (nightly for miri and sanitizer tests)
- Property-based tests with 1024 cases (vs. 256 locally)
- Clippy with `-D warnings` (treat warnings as errors)
- Format check with `cargo fmt -- --check`

## Writing Effective Tests

### What Makes a Good Test

A good test has these properties:

1. **Descriptive name.** The test name explains what it verifies: `test_mvcc_filter_hides_future_snapshots` not `test_mvcc_1`.

2. **Focused assertion.** One logical concept per test. If you are testing "create table" and "list tables," those are two tests.

3. **Minimal setup.** Only create the state needed for this specific test. Do not reuse "kitchen sink" fixtures.

4. **Deterministic.** The test always passes or always fails given the same code. No reliance on timing, random values (except proptest), or external services.

5. **Fast.** Unit tests should run in milliseconds. Integration tests in under a second. If a test is slow, find out why.

### Common Patterns

**Testing error cases:**

```rust
#[test]
fn test_create_duplicate_schema_returns_error() {
    let store = CatalogStore::open_temp().await.unwrap();
    let writer = store.writer().await.unwrap();

    writer.create_schema("main").await.unwrap();  // First time: succeeds
    let err = writer.create_schema("main").await.unwrap_err();  // Second time: fails
    assert!(matches!(err, CatalogError::AlreadyExists { .. }));
}
```

**Testing MVCC isolation:**

```rust
#[tokio::test]
async fn test_old_snapshot_does_not_see_new_writes() {
    let store = CatalogStore::open_temp().await.unwrap();
    let writer = store.writer().await.unwrap();

    // Create initial state
    writer.create_schema("v1").await.unwrap();
    let snapshot_1 = writer.commit().await.unwrap();

    // Write more data
    writer.create_schema("v2").await.unwrap();
    let snapshot_2 = writer.commit().await.unwrap();

    // Reader at snapshot_1 sees only v1
    let reader_1 = store.reader_at(snapshot_1).await.unwrap();
    let schemas_1 = reader_1.list_schemas().await.unwrap();
    assert_eq!(schemas_1.len(), 1);
    assert_eq!(schemas_1[0].name, "v1");

    // Reader at snapshot_2 sees both
    let reader_2 = store.reader_at(snapshot_2).await.unwrap();
    let schemas_2 = reader_2.list_schemas().await.unwrap();
    assert_eq!(schemas_2.len(), 2);
}
```

**Testing key sort order:**

```rust
#[test]
fn test_table_keys_sorted_within_schema() {
    let mut keys: Vec<Vec<u8>> = vec![];
    for table_id in [10, 5, 20, 1, 15] {
        keys.push(TableKey::new(42, table_id, u64::MAX).encode());
    }
    keys.sort();  // Byte-level sort

    // Tables within a schema should sort by table_id ascending
    let decoded: Vec<u32> = keys.iter()
        .map(|k| TableKey::decode(k).unwrap().table_id)
        .collect();
    assert_eq!(decoded, vec![1, 5, 10, 15, 20]);
}
```

### What NOT to Test

- **Internal implementation details.** If you rename a private function, no test should break. Test the public interface.
- **Third-party library behavior.** Do not test that `serde` serializes correctly or that `tokio` spawns tasks.
- **Trivial getters/setters.** If a function just returns a field value, it does not need its own test.
- **Platform-specific behavior** (unless your change is platform-specific). Tests should be portable.

## Test Coverage

The project does not enforce a specific coverage percentage, but aims for:

- 100% of public API functions have at least one test
- Every match arm in the SQL classifier has a corpus entry
- Every error variant is tested (provoked and asserted)
- Every encoding format is tested with property-based roundtrip tests

To measure coverage locally:

```bash
# Install cargo-tarpaulin
cargo install cargo-tarpaulin

# Generate coverage report
cargo tarpaulin --out html
# Open tarpaulin-report.html in your browser
```

## Further Reading

- **[Development Setup](development-setup.md)** — Running the test suite
- **[Architecture Guide](architecture-guide.md)** — Where tests live in the codebase
- **[Code Style](code-style.md)** — Test naming and formatting conventions

## v0.15 Test Additions

### Tier 7 — Fault Injection Tests

- **`crates/rocklake-catalog/tests/fault_injection_tests.rs`** — Kill after SST
  before manifest, corrupted WAL recovery, kill during compaction, concurrent
  writer fencing.

### Tier 10 — Security Tests

- **`crates/rocklake-pgwire/tests/security_tests.rs`** — Tests covering
  invalid auth, SQL injection, oversized queries, schema isolation, privilege
  escalation, TLS enforcement, timing attacks, session hijacking, parameter
  injection, error message leaks, and idle timeout.

### Scale Testing Infrastructure (Tier 8)

Scale tests run on dedicated EC2 `c6i.4xlarge` instances via self-hosted GitHub
Actions runners. They are triggered:
- Manually via `workflow_dispatch`
- Automatically on `v*` release tags

**Setup requirements:**
- Instance: `c6i.4xlarge` (16 vCPUs, 32 GB RAM)
- Storage: 100 GB gp3 EBS
- Network: Same-region as S3 bucket (us-east-1 recommended)
- Runner label: `self-hosted-scale`

**TPC-H catalog benchmarks** (`tests/scale/tpch_catalog.rs`):
- Targets: `get_current_snapshot` p99 < 50 ms (SF10), < 100 ms (SF100)
- Requires MinIO or real S3 endpoint
