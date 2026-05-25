# Code Style

This page documents the coding conventions used in SlateDuck. Consistency matters in a codebase — not because any particular convention is inherently superior to another, but because uniformity reduces cognitive load. When every file follows the same patterns, contributors spend less time deciphering style choices and more time understanding logic. Code reviews focus on correctness and design rather than formatting disputes.

These conventions are not arbitrary. Each rule exists because it solves a real problem that arose during development. The formatting rules prevent merge conflicts. The naming conventions make code self-documenting. The error handling patterns ensure useful diagnostics. The module organization keeps files navigable as the codebase grows.

CI enforces everything it can automatically (formatting, linting). The conventions described here include both the automated checks and the softer guidelines that require human judgment.

## Formatting

All Rust code must pass `cargo fmt` with the default rustfmt configuration. This is enforced in CI — a PR that fails the formatting check cannot be merged.

```bash
# Format all code
cargo fmt

# Check formatting without modifying files (what CI does)
cargo fmt -- --check
```

**Do not configure custom formatting rules.** No `.rustfmt.toml` overrides, no per-file exceptions, no nightly-only formatting options. The default configuration produces readable code and eliminates formatting disputes entirely.

**Practical advice:**
- Configure your editor to format on save
- If a large reformatting obscures your logical changes in a PR, split into two commits: one formatting-only commit, then your actual change

## Linting

All code must pass `cargo clippy --all-targets --all-features` with zero warnings. CI runs with `-D warnings` (warnings are errors).

```bash
# Run clippy
cargo clippy --all-targets --all-features

# Auto-fix simple issues
cargo clippy --fix --allow-dirty
```

Clippy suggestions are generally good advice. Follow them unless there is a specific, documented technical reason not to. If you must suppress a warning, add a comment explaining why:

```rust
// We need the explicit type annotation here because the compiler cannot
// infer the lifetime across the async boundary
#[allow(clippy::redundant_closure)]
let handler = move |msg| handle_message(msg);
```

**Common clippy issues in SlateDuck:**
- `clippy::large_enum_variant` — Box large variants in error enums
- `clippy::match_wildcard_for_single_variants` — Be explicit about enum matches
- `clippy::unnecessary_wraps` — Only return Result/Option when needed

## Naming Conventions

### Types

PascalCase for all types (structs, enums, traits, type aliases):

```rust
struct CatalogStore { ... }
struct WriterEpoch { ... }
enum StatementKind { ... }
trait KeyEncoder { ... }
type SnapshotId = u64;
```

### Functions and Methods

snake_case for all functions, methods, and associated functions:

```rust
fn classify_statement(sql: &str) -> Result<Classification, SqlError> { ... }
fn write_batch(&mut self, batch: WriteBatch) -> Result<(), CatalogError> { ... }
fn advance_retention(&self, new_min: SnapshotId) -> Result<(), GcError> { ... }
```

### Constants

SCREAMING_SNAKE_CASE for all constants and statics:

```rust
const ABI_VERSION: u32 = 3;
const MAGIC_BYTES: &[u8] = b"SDCK";
const MAX_KEY_SIZE: usize = 512;
const DEFAULT_RETENTION_SNAPSHOTS: u64 = 100;
```

### Modules

snake_case matching the filename:

```rust
// File: catalog_provider.rs
mod catalog_provider;

// File: key_encoding.rs
mod key_encoding;
```

### Variables and Parameters

snake_case, descriptive names. Avoid single-letter names except for:
- `i`, `j`, `k` in numeric loops (but prefer iterators)
- `n` for counts
- `_` for intentionally unused values

```rust
// Good
let schema_id = reader.lookup_schema(name).await?;
let snapshot_id = writer.current_snapshot();
let encoded_key = SchemaKey::new(schema_id, snapshot_id).encode();

// Avoid
let s = reader.lookup_schema(name).await?;
let snap = writer.current_snapshot();
let k = SchemaKey::new(s, snap).encode();
```

### Tag Constants

Entity tag bytes (used in key prefixes) are named with a `TAG_` prefix and uppercase entity name:

```rust
const TAG_SCHEMA: u8 = 0x01;
const TAG_TABLE: u8 = 0x02;
const TAG_COLUMN: u8 = 0x03;
const TAG_DATA_FILE: u8 = 0x04;
```

## Error Handling

### Return Result from Fallible Operations

Every function that can fail returns `Result<T, E>`. No panics in library code (crates other than the binary).

```rust
// Good: returns Result
pub fn decode_key(bytes: &[u8]) -> Result<SchemaKey, DecodeError> {
    if bytes.len() < SCHEMA_KEY_LEN {
        return Err(DecodeError::TooShort {
            expected: SCHEMA_KEY_LEN,
            actual: bytes.len(),
        });
    }
    // ... decode ...
}

// Bad: panics on invalid input
pub fn decode_key(bytes: &[u8]) -> SchemaKey {
    assert!(bytes.len() >= SCHEMA_KEY_LEN);  // NO! Return an error instead.
    // ...
}
```

### Use Crate-Level Error Enums

Each crate defines its own error enum. Do not use `anyhow::Error` in library code (it is acceptable in tests and the binary):

```rust
#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("schema not found: {name}")]
    SchemaNotFound { name: String },

    #[error("table already exists: {schema}.{table}")]
    TableAlreadyExists { schema: String, table: String },

    #[error("storage error: {0}")]
    Storage(#[from] slatedb::Error),

    #[error("encoding error: {0}")]
    Encoding(#[from] prost::DecodeError),
}
```

### Include Context in Error Messages

Error messages should include enough context to diagnose the problem without looking at the code:

```rust
// Good: includes relevant IDs and context
Err(CatalogError::ColumnNotFound {
    table_id,
    column_name: name.to_string(),
    available: existing_columns.iter().map(|c| c.name.clone()).collect(),
})

// Bad: vague message
Err(CatalogError::NotFound)
```

### No unwrap() in Library Code

`.unwrap()` and `.expect()` are reserved for situations where the invariant is provably upheld and documented:

```rust
// Acceptable: the slice length is guaranteed by the preceding check
let bytes: [u8; 8] = buffer[0..8].try_into().expect("slice length verified above");

// Unacceptable: runtime input might be invalid
let schema = schemas.get(name).unwrap();  // Use .ok_or(Error::NotFound)?
```

In test code, `.unwrap()` is fine for operations that should not fail (it produces a clear panic message with the test name).

## Module Organization

### One Significant Type Per File

Large types (structs with multiple methods, complex enums) get their own file:

```
src/
├── lib.rs          # Re-exports, module declarations
├── reader.rs       # pub struct CatalogReader { ... }
├── writer.rs       # pub struct CatalogWriter { ... }
├── gc.rs           # pub struct GarbageCollector { ... }
├── keys.rs         # Key encoding functions
├── values.rs       # Value encoding functions
└── error.rs        # pub enum CatalogError { ... }
```

### File Size Limit

Keep files under 500 lines where possible. If a file grows beyond 500 lines, consider whether it has multiple responsibilities that can be separated. The exception is files that are inherently large (e.g., a comprehensive match statement over 28 SQL statement types).

### Test Organization

Tests live in a `#[cfg(test)] mod tests` block at the bottom of each file:

```rust
// ... production code above ...

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_roundtrip() {
        // ...
    }
}
```

Integration tests (testing the public API from the outside) live in `crates/*/tests/`:

```
crates/slateduck-catalog/
├── src/
│   ├── lib.rs
│   ├── reader.rs
│   └── writer.rs
└── tests/
    ├── integration_tests.rs
    └── gc_tests.rs
```

### Visibility Rules

- Make everything private by default
- Use `pub(crate)` for items shared within the crate but not exported
- Use `pub` only for items that are part of the crate's public API
- Re-export important types from `lib.rs` so consumers do not need deep imports

## Dependency Policy

### Adding New Dependencies

New dependencies must be justified. In the PR description, explain:
- Why the dependency is needed (what does it provide that is non-trivial to implement?)
- Why this specific crate (vs. alternatives)
- The size impact (does it pull in many transitive dependencies?)

### Version Pinning

Pin to compatible version ranges, not exact versions:

```toml
# Good: allows compatible updates
serde = "1"
tokio = "1.36"
prost = "0.13"

# Bad: too loose
serde = "*"

# Bad: too strict (prevents bug fixes)
serde = "=1.0.196"
```

### Minimizing the Dependency Tree

Do not add crates for trivial functionality:
- String manipulation: use `std` methods
- Small utility functions: implement inline
- Feature-gated large dependencies: put behind cargo features

### Preferred Crates

| Purpose | Crate | Why |
|---------|-------|-----|
| Serialization | `prost` (protobuf) | Wire format for values |
| Async runtime | `tokio` | Industry standard |
| Error handling | `thiserror` | Derive-based, zero cost |
| CLI parsing | `clap` | Robust, well-maintained |
| Logging | `tracing` | Structured, async-aware |
| Testing | `proptest` | Property-based testing |
| HTTP | `hyper` | Already in dependency tree |

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add support for ALTER TABLE SET SCHEMA
fix: correct MVCC visibility for snapshot 0
docs: update deployment guide for Kubernetes
refactor: extract key encoding into separate module
test: add wire corpus entries for DuckDB 1.3.0
perf: reduce allocations in prefix scan hot path
chore: update dependencies to latest compatible versions
```

### Rules

- Use the imperative mood: "add support" not "added support"
- First line under 72 characters
- Body (optional) explains WHY, not WHAT (the diff shows what changed)
- Reference issues: `Fixes #123` or `Closes #456`

### Scope (Optional)

For large changes, add a scope indicating the affected area:

```
feat(catalog): add secondary index support
fix(pgwire): handle extended query protocol correctly
perf(core): optimize key encoding for small integers
```

## Module Size Limit

No new source file in the production crates (`slateduck-catalog`, `slateduck-pgwire`, `slateduck-sql`, `slateduck-core`, `slateduck-datafusion`, `slateduck-ffi`) may exceed **600 lines** (excluding blank lines and doc comments). This limit exists because large files are the single strongest predictor of future maintenance burden in this codebase — they accumulate unrelated concerns, slow incremental compilation, and make diffs harder to review.

When a file approaches 600 lines:

1. **Identify natural sub-concerns.** A 1,200-line file named `writer.rs` mixing MVCC staging, statistics aggregation, and snapshot commit is three files, not one. Split it: `writer/staged.rs`, `writer/stats.rs`, `writer/snapshot.rs`.
2. **Create a directory module.** Rename `writer.rs` → `writer/mod.rs`, then add the new files inside the directory. The parent module re-exports the public API so callers need no changes.
3. **Use `pub(super)` for module-internal items.** Functions called only within the parent module should use `pub(super)`, not `pub` or `pub(crate)`.

CI does not enforce the line limit automatically yet, but reviewers should reject PRs that add to files already over limit without a corresponding split.

## Parameter Struct Conventions

When a function takes more than four arguments of the same or related types (typical of statistics-writing helpers), introduce a **parameter struct** instead of a long argument list:

```rust
// Bad: argument order is easy to confuse, clippy warns at >7 args
fn upsert_file_column_stats(
    table_id: u64, column_id: u64, data_file_id: u64,
    has_null: bool, min_value: Option<&str>, max_value: Option<&str>,
    contains_nan: bool,
) -> ...

// Good: self-documenting, extensible, no order mistakes
pub struct FileColumnStatsInput<'a> {
    pub table_id: u64,
    pub column_id: u64,
    pub data_file_id: u64,
    pub has_null: bool,
    pub min_value: Option<&'a str>,
    pub max_value: Option<&'a str>,
    pub contains_nan: bool,
}

fn upsert_file_column_stats(input: FileColumnStatsInput<'_>) -> ...
```

Rules for parameter structs:

- Place them in the same submodule as the function they serve (`writer/stats.rs` for writer stats helpers).
- Re-export them from the crate root when callers outside the crate need them.
- Use lifetimes sparingly — only borrow fields that are string-like and performance-critical.

## Dead-Code Policy

`#[allow(dead_code)]` is a red flag, not a solution. Every use in production code must be resolved before the containing file is declared stable:

| Scenario | Resolution |
|---|---|
| Code that will be used in a planned release | Replace `#[allow(dead_code)]` with a doc comment: `// Used in v0.22 — see ROADMAP §v0.22`. |
| Code that is only used in tests | Add `#[cfg(test)]` to the function or item. |
| Code that is structurally dead (set but never read, constructed but never inspected) | Delete it. If you want the *idea* preserved, capture it in ROADMAP.md, not in source. |
| Code generated by a macro that the macro consumer does not use | Suppress with `#[allow(dead_code)]` on the generated struct/impl, document why in a comment. |

CI runs `cargo build --workspace` with `RUSTFLAGS="-D warnings"` (warnings become errors). New `#[allow(dead_code)]` attributes that are not accompanied by a justification comment will be rejected in code review.

## Anti-Patterns to Avoid

| Anti-Pattern | Why It's Bad | What to Do Instead |
|-------------|-------------|-------------------|
| `.unwrap()` in library code | Panics are unrecoverable | Return `Result` |
| `String` where `&str` suffices | Unnecessary allocation | Use references |
| `Box<dyn Error>` | Loses type information | Use concrete error enums |
| `pub` on internal helpers | Leaks implementation details | Use `pub(crate)` |
| Giant match arms | Obscures logic | Extract to functions |
| Comments that restate code | Noise, not signal | Write self-documenting code |
| `clone()` to satisfy the borrow checker | Performance cost | Restructure ownership |
| Nested `if let` / `match` | Hard to follow | Use `?` and early returns |

## Further Reading

- **[Testing](testing.md)** — Test naming and organization conventions
- **[Architecture Guide](architecture-guide.md)** — Module structure rationale
- **[Development Setup](development-setup.md)** — Editor configuration for auto-formatting
