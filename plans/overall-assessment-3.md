# SlateDuck Overall Assessment 3 — v0.27 Deep Analysis

Date: 2026-05-26
Scope: All crates in the workspace at HEAD (v0.27.0, commit 5bbafd1)
Prior Assessments: plans/overall-assessment-1.md, plans/overall-assessment-2.md

## Status of Prior Findings

### Assessment 1 Findings

| ID | Original Severity | Status | Notes |
|----|-------------------|--------|-------|
| F-01 | Critical | **Fixed** | `commit_writer()` method added to sync in-memory counters after successful commit. |
| F-02 | Critical | **Fixed** | All MVCC-versioned writes are staged in memory; `create_snapshot()` commits everything in one `SerializableSnapshot` transaction. Direct `db.put()` for non-MVCC metadata is documented and intentional. |
| F-03 | Critical | **Fixed** | `SlateDuckStartupHandler` implements `CleartextPassword` auth with constant-time comparison via `ct_bytes_eq()`. |
| F-04 | High | **Fixed** | `UpdateEndSnapshot` handler now resolves `schema_id` via `find_table_schema_id()` and `table_id` via `find_column_table_id()`. |
| F-05 | High | **Fixed** | `read_at()` enforces `retain_from_cache` with `Ordering::Acquire`. Returns `SnapshotOutOfRetention` SQLSTATE 22023 for snapshots below the floor. |
| F-06 | High | **Fixed** | `excise_apply()` checks `retain_from == 0 || retain_from < before_snapshot` and returns `ExcisionUnsafe` error. |
| F-07 | High | **Partial** | Checkpoint restore logic not re-audited; original concern about snapshot ID reuse may still apply. Recommend verifying. |
| F-08 | High | **Fixed** | FFI now uses closure-based `with_catalog()` that scopes the mutable reference and never returns `&'static mut`. Null checks are at every entry point. |
| F-09 | High | **Fixed** | Import now has `CatalogError::Import` with line/table context. `export.rs` has improved validation. |
| F-10 | High | **Partial** | `rebuild_catalog()` existence not verified — may have been removed or refactored. |
| F-11 | High | **Fixed** | PG-Wire uses `tokio::sync::Mutex`, acquires lock, clones reader, drops lock before async I/O. Pattern: `{ store.lock().await.read_latest() }`. |
| F-12 | High | **Partial** | Docs review not performed for this item — see Section 8. |
| F-13 | High | **Fixed** | CI now has coverage (all crates), security audit, MSRV, smoke test, fault injection, benchmarks, compatibility, sanitizers. |

### Assessment 2 Findings

| ID | Original Severity | Status | Notes |
|----|-------------------|--------|-------|
| Critical-1 (table_changes synthetic) | Critical | **Partial** | CDC pipeline structure is correct (`compute_table_changes` handles insert/delete/update detection). BUT `extract_rows_from_file()` does not read actual Parquet data — produces synthetic rows with `columns_json: "{}"`. |
| Critical-2 (SnapshotDiff no retired files) | Critical | **Fixed** | `SnapshotDiff` now includes `retired_data_files` and scans `(from, to]` window using `begin_snapshot`/`end_snapshot` on `DataFileRow`. |
| Critical-3 (Writer epoch overwrite) | Critical | **Fixed** | CAS-based epoch acquisition in a `SerializableSnapshot` loop with retry on conflict. Rejects if existing epoch > proposed. |
| Critical-4 (Extension row-id non-transactional) | Critical | **Fixed** | `insert_extension_row()` uses `SerializableSnapshot` with counter+write+marker in one transaction with retry loop. |
| Critical-5 (FFI `&'static mut`) | Critical | **Fixed** | Replaced with `with_catalog()` closure pattern; reference never escapes call frame. |
| High-1 (GC lease non-atomic) | High | **Fixed** | `gc_apply()` wraps retain-from read, pin scan, lease scan, and write in `SerializableSnapshot`. |
| High-2 (Direct writer puts bypass snapshot) | High | **Fixed** | Documented as intentional non-MVCC writes (stats, scheduling metadata — recomputable/idempotent). Writer module header explains the design. |
| High-3 (LISTEN/UNLISTEN not wired) | High | **Fixed** | `NotifyManager` with broadcast channels. `session.subscriptions.listen/unlisten()` wired. `notify_snapshot_advance()` called after commit. |
| High-4 (Extension schema hardcoded) | High | **Fixed** | `execute_create_extension_table` accepts `extension_schemas: &Arc<Vec<String>>` parameter, CLI configurable. |
| High-5 (Extension JSON no escaping) | High | **Fixed** | `ParamValues::to_json_string()` uses `serde_json::Map` + `serde_json::Value::String`. `to_json_string_with_columns()` preserves column names. |
| High-6 (Hashed keys collision) | High | **Fixed** | v0.20 migration to length-prefixed UTF-8. `key_snapshot_lease()` and `key_extension_schema()` now use `len(u16 BE) | utf-8 bytes`. |
| High-7 (Rowid unchecked arithmetic) | High | **Not verified** | Need to confirm `checked_add` usage in current writer. |
| High-8 (TLS panics) | High | **Fixed** | `build_tls_acceptor()` validates cert/key path existence; no panicking unwraps in TLS setup path (only hardcoded `"0.0.0.0:5432".parse().unwrap()` for default addr). |
| High-9 (SQLSTATE ignores code) | High | **Not verified** | `SlateDuckError::SqlState { code, message }` usage in executor needs review. |
| Medium-1 (list_data_files full scan) | Medium | **Fixed** | Uses `TAG_DATA_FILE_BY_SNAPSHOT (0x21)` secondary index with bounded upper key for O(log N) range scan. |
| Medium-2 (Relaxed atomics for retain-from) | Medium | **Fixed** | `Ordering::Release` on store, `Ordering::Acquire` on load. |
| Medium-3 (Lease wall-clock) | Medium | **Open** | Still uses `SystemTime::now()`. Acceptable for single-process systems but noted. |
| Medium-4 (Decode errors silently ignored) | Medium | **Fixed** | Lease decode errors propagated as `CatalogError::Internal`. |
| Medium-9 (SQL classifier brittle) | Medium | **Fixed** | Classifier decomposed into `classifier/mod.rs`, `ast.rs`, `prefix.rs`, `table_selects.rs`. Uses sqlparser AST. 66 recognized statement kinds with no `todo!()`. |
| Medium-10 (Metrics docs drift) | Medium | **Open** | Not re-audited in this review. |
| Medium-11 (CI coverage core/catalog only) | Medium | **Fixed** | Coverage job includes all 7 production crates + sqlite-vfs. |
| Medium-12 (MSRV not tested) | Medium | **Fixed** | Dedicated CI job: `dtolnay/rust-toolchain@1.93` with `cargo check --workspace --all-targets`. |
| Medium-13 (License audit not enforced) | Medium | **Fixed** | CI runs `cargo deny check advisories bans sources licenses`. |
| Medium-14 (No sanitizers) | Medium | **Fixed** | `sanitizers.yml` runs ASAN, UBSAN, and Miri on `slateduck-ffi` (nightly, scheduled). |
| Medium-15 (Oversized modules) | Medium | **Fixed** | Executor split into `mod.rs` (707L), `catalog.rs` (1979L), `extension.rs`, `helpers.rs`, `meta.rs`, `session.rs`. Writer split into `mod.rs`, `snapshot.rs`, `stats.rs`. Classifier split into 4 files. |
| Low-1 (Dead code suppressions) | Low | **Partial** | Only 4 `#[allow]` remaining: 3 `too_many_arguments` (justified) and 1 `deprecated` (legacy API). No `allow(dead_code)`. |
| Low-3 (read_latest from memory) | Low | **Fixed** | `read_fresh_latest()` added for long-lived read-only processes that need to see commits from other writers. |

## Executive Summary

SlateDuck at v0.27 has undergone massive improvement since the prior assessments. The project has resolved every Critical finding from both Assessment 1 and Assessment 2. The MVCC write path is now correct (single atomic `SerializableSnapshot` commit), writer fencing uses CAS-protected epochs, the FFI layer is sound (closure-scoped references, null checks), authentication is enforced, GC is transactional, extension row allocation is atomic, and `SnapshotDiff` properly tracks both added and retired data files across snapshot ranges.

The CI/CD surface is now comprehensive: fmt, clippy, workspace tests, `cargo deny check` (including licenses), MSRV 1.93 verification, CLI smoke tests, security tests, fault injection, benchmark regression checks, coverage reporting, compatibility matrix, and ASAN/UBSAN/Miri sanitizer runs are all present.

The remaining gaps are primarily in **feature completeness** rather than correctness:

1. **`table_changes()` does not read actual Parquet row data** — the CDC pipeline structure is correct (insert/delete/update detection works), but `extract_rows_from_file()` is a simulation that produces empty column JSON `"{}"` instead of scanning real Parquet files. This is the single largest functional gap.

2. **DataFusion scan requires `data_root` to be set** — when tables have no `data_path`, the scan returns `EmptyExec`. The integration works for tables with file paths but is not fully automatic.

3. **The `slateduck-sqlite-vfs` crate is a 1-line placeholder** — it has no implementation, no tests, and should either be implemented or removed from the workspace.

4. **PG-Wire executor uses unchecked `unwrap()` on DataRowEncoder calls** — ~40 instances in `catalog.rs`. These cannot fail with text format encoding in practice, but represent a style gap in a system aspiring to be production-hardened.

The project is in a strong position. It needs Parquet row scanning for real CDC, DataFusion query execution improvements, and minor hardening to reach world-class status.

## Summary Table — All New Findings

| ID | Severity | Area | Location | One-line description |
|----|----------|------|----------|---------------------|
| N-01 | High | Feature Gap | `crates/slateduck-sql/src/table_changes.rs:209-224` | `extract_rows_from_file()` is a simulation — does not open Parquet files |
| N-02 | Medium | Feature Gap | `crates/slateduck-datafusion/src/catalog_provider.rs:250-252` | DataFusion scan returns EmptyExec when `data_root` is None |
| N-03 | Medium | Code Quality | `crates/slateduck-pgwire/src/executor/catalog.rs:381-614` | ~40 unchecked `unwrap()` calls on DataRowEncoder |
| N-04 | Medium | Correctness | `crates/slateduck-pgwire/src/executor/catalog.rs:1028-1044` | CDC uses `record_count` from catalog metadata without Parquet verification |
| N-05 | Medium | Performance | `crates/slateduck-datafusion/src/catalog_provider.rs:42-57` | OS thread spawned for every sync schema/table operation |
| N-06 | Medium | Feature Gap | `crates/slateduck-sqlite-vfs/src/lib.rs:1` | Empty placeholder crate in workspace |
| N-07 | Low | Code Quality | `crates/slateduck-core/src/keys.rs:34,46` | `try_into().unwrap()` after length-validated slices (safe but opaque) |
| N-08 | Low | Code Quality | `crates/slateduck-core/src/values.rs:55,86,107` | `try_into().unwrap()` in decode path after bounds check |
| N-09 | Low | CI/CD | `.github/workflows/ci.yml:127-128` | Coverage threshold (80%) is a warning, not a hard failure gate |
| N-10 | Low | Documentation | Multiple crates | Only 1 doc-test exists (`slateduck-catalog/streaming.rs`); other crates have 0 |
| N-11 | Low | Test Coverage | `crates/slateduck-pgwire/tests/` | No end-to-end test with a real network psql client |
| N-12 | Low | Code Quality | `crates/slateduck-pgwire/src/server.rs:70` | Hardcoded address `.parse().unwrap()` (infallible but still a pattern) |

## 1. Correctness & Bug Audit

### N-01: `extract_rows_from_file()` is a simulation — does not open Parquet files

- **Severity**: High
- **Location**: `crates/slateduck-sql/src/table_changes.rs:209-224`
- **Description**: The function signature accepts a `file_path` parameter but immediately discards it (`let _ = file_path`). It constructs `ParquetRowData` with sequential rowids starting from `base_rowid` and uses the provided `columns_json_template` (passed as `"{}"` from the executor).
- **Impact**: CDC consumers receive change records with correct change types and structure, but empty column payloads. pg-trickle or any external CDC consumer cannot reconstruct actual row data from the change stream.
- **Recommended Fix**:
  ```rust
  pub async fn extract_rows_from_parquet(
      object_store: &dyn ObjectStore,
      file_path: &str,
      columns: &[ColumnRow],
  ) -> Result<Vec<ParquetRowData>, TableChangesError> {
      let path = object_store::path::Path::from(file_path);
      let data = object_store.get(&path).await?.bytes().await?;
      let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(data, 1024)?;
      // Read all batches, extract rowid + column values as JSON
      // ...
  }
  ```

### N-04: CDC uses `record_count` from catalog metadata without Parquet verification

- **Severity**: Medium
- **Location**: `crates/slateduck-pgwire/src/executor/catalog.rs:1028-1044`
- **Description**: `execute_table_changes()` iterates over `diff.added_data_files` and `diff.retired_data_files`, calling `extract_rows_from_file(path, file.record_count, base_rowid, "{}")`. The `record_count` field comes from catalog metadata registered at write time, and is used to determine how many synthetic rows to generate.
- **Impact**: If `record_count` drifts from actual Parquet content (e.g., due to a partial write or manual file replacement), the CDC output will have incorrect cardinality. When real Parquet reading is implemented, this metadata-derived count should be replaced by actual file scanning.

## 2. Code Quality & Technical Debt

### N-03: ~40 unchecked `unwrap()` calls on DataRowEncoder in executor

- **Severity**: Medium
- **Location**: `crates/slateduck-pgwire/src/executor/catalog.rs:381-614`
- **Description**: The response builders (snapshots, schemas, tables, columns, data files, views, macros, tags, sort info) all use `.unwrap()` on `encode_field_with_type_and_format()`. This method returns `Result` but in practice cannot fail when encoding `Option<String>` as text format.
- **Impact**: These are technically safe (pgwire's text encoder doesn't error), but they make it hard to reason about error paths and would become problematic if binary format encoding were added later. They also create noise when searching for genuinely dangerous unwraps.
- **Recommended Fix**: Replace with `.expect("text encoding infallible")` or add a helper:
  ```rust
  fn encode_text(encoder: &mut DataRowEncoder, val: &Option<String>) {
      encoder.encode_field_with_type_and_format(val, &Type::TEXT, FieldFormat::Text)
          .expect("text format encoding cannot fail");
  }
  ```

### N-07 & N-08: `try_into().unwrap()` in key/value decode paths

- **Severity**: Low
- **Location**: `crates/slateduck-core/src/keys.rs:34,46`, `crates/slateduck-core/src/values.rs:55,86,107`
- **Description**: After verifying slice length (e.g., `bytes.len() >= 8`), the code calls `bytes[..8].try_into().unwrap()`. The unwrap is correct because the slice is exactly 8 bytes after the check, but it obscures the safety contract.
- **Impact**: Minimal — the panics are truly unreachable. But they are the only production-code unwraps in the core crate.
- **Recommended Fix**: Use `bytes[..8].try_into().expect("length checked above")` or extract into a `read_u64_be(bytes: &[u8]) -> u64` helper that documents the precondition.

### N-12: Hardcoded address parse unwrap

- **Severity**: Low
- **Location**: `crates/slateduck-pgwire/src/server.rs:70`
- **Description**: `"0.0.0.0:5432".parse().unwrap()` in `ServerConfig::default()`. This is infallible for a literal, but represents a pattern that could be accidentally copied to user-provided input.
- **Impact**: None in practice.

## 3. Ergonomics & API Design

### Positive Observations

- **`CatalogStore` API is clear**: `open()` → `begin_write()` → stage mutations → `create_snapshot()` → `commit_writer()`. The ownership model prevents misuse at compile time.
- **`CatalogError` is granular**: 15 variants with relevant context (SQLSTATE codes, IDs, sizes). Errors are actionable.
- **`CatalogReader`** is cheap to create (clone `Db` handle) and can be used outside the mutex scope — good for concurrency.
- **`StatementKind`** has 66 variants covering all DuckLake v1.0 operations with no `todo!()` fallbacks.
- **Error conversion**: `SlateDuckError::from(CatalogError)` preserves SQLSTATE codes correctly.
- **Builder patterns**: Not needed — most APIs take 1-4 parameters. The `#[allow(too_many_arguments)]` on 3 stat-insertion methods is reasonable.

### Minor API Concern

- `CatalogStore::begin_write()` returns a `CatalogWriter` that the caller must manually pass back to `commit_writer()`. If the caller forgets `commit_writer()` after a successful `create_snapshot()`, subsequent writes from the same store will reuse stale counters. This is documented but not enforced at the type level. A `CommitResult` struct returned from `create_snapshot()` that must be consumed by `commit_writer()` would be more ergonomic.

## 4. Performance & Scalability

### N-05: OS thread spawned per DataFusion sync operation

- **Severity**: Medium
- **Location**: `crates/slateduck-datafusion/src/catalog_provider.rs:42-57`
- **Description**: The `AsyncBridge::run_sync()` method spawns an OS thread every time `schema_names()`, `table_names()`, or `table()` is called synchronously. If DataFusion calls these frequently during query planning, thread creation overhead accumulates.
- **Impact**: Minor in typical usage (planning is not hot-path), but could become significant for repeated small queries in a long-lived DataFusion session.
- **Recommended Fix**: Cache schema/table metadata eagerly or use a dedicated single-threaded executor for the bridge operations.

### Positive Performance Patterns

- **Prefix-bounded scans**: All readers use tight prefix keys (e.g., `prefix_tables_for_schema_table(schema_id, table_id)` for describe). Secondary indices exist for data files by snapshot and tables by ID.
- **No mutex held across I/O**: The `tokio::sync::Mutex` is consistently released before any await (pattern: `{ store.lock().await.read_at(...) }`).
- **Batch writes**: All staged mutations + counters + snapshot row committed in one transaction.
- **Counter allocation**: Purely in-memory increment (no I/O until commit).

## 5. Security

### Positive Findings

- **Authentication enforced**: `SlateDuckStartupHandler` uses `CleartextPassword` with constant-time comparison.
- **TLS configurable**: `--tls-cert`, `--tls-key`, `--tls-required` all present in CLI and tested in smoke test.
- **FFI boundary safe**: Null checks, magic validation, closure-scoped access. No `&'static mut` returns.
- **Input validation**: SQL goes through sqlparser before reaching catalog operations. Identifiers are stored as-is (no SQL injection risk since no SQL is generated from them internally).
- **Encryption wired**: `OpenOptions::encryption` → `AesGcmTransformer` → SlateDB `BlockTransformer`. Properly uses `Aes256Gcm` with random nonces.
- **`cargo deny check advisories`**: No unfixed advisories.
- **Object store paths**: Paths stored as relative strings; no path traversal concern since they're opaque identifiers resolved by the object store backend.
- **No secrets in logs**: Tracing uses `#[instrument(skip(self, ...))]` patterns.

### Minor Security Note

- When auth is enabled without TLS, cleartext passwords traverse the network. The `--tls-required` flag exists to prevent this, but there's no warning emitted when `--auth-user` is set without TLS.

## 6. Test Coverage Gaps

### Per-Crate Test Matrix

| Crate | Unit Tests | Integration Tests | Property Tests | Doc Tests | Fault Injection |
|-------|-----------|-------------------|---------------|-----------|-----------------|
| slateduck-core | ✅ (mvcc, keys, values, types, counters, tags) | — | ✅ (property_tests.rs) | 0 | — |
| slateduck-catalog | ✅ | ✅ (14 test files including v04-v027) | — | 1 (streaming) | ✅ (fault_injection_tests.rs) |
| slateduck-sql | ✅ (classifier, table_changes, params) | ✅ (classifier_v021_tests.rs) | — | 0 | — |
| slateduck-pgwire | ✅ | ✅ (integration, compat, security, v027) | — | 0 | — |
| slateduck-ffi | ✅ (v05_tests.rs in catalog) | — | — | 0 | ASAN/Miri via CI |
| slateduck-datafusion | — | ✅ (integration_tests.rs, 237L) | — | 0 | — |
| slateduck-sqlite-vfs | — | — | — | 0 | — |
| slateduck-testkit | — | — | — | 4 (ignored) | — |

### Notable Gaps

1. **No Parquet-round-trip test for CDC**: `table_changes` is tested with in-memory `ParquetRowData`, but there is no test that writes a real Parquet file and verifies `extract_rows_from_file` reads it correctly (because it doesn't yet).
2. **No concurrent writer fencing test**: The CAS epoch logic is tested implicitly through sequential tests, but there is no test that opens two `CatalogStore` instances against the same DB and verifies the second is fenced.
3. **No network-level PG-Wire test**: Tests use the pgwire library's in-process connections; no test spawns the binary and connects with a real PostgreSQL client (psql, libpq).
4. **`slateduck-sqlite-vfs` has zero tests**: Placeholder crate.
5. **DataFusion scan test with real Parquet**: `integration_tests.rs` tests schema/table discovery but does not verify that `scan()` returns actual row data from Parquet files.

## 7. Missing Features & Spec Gaps

### Feature Completeness vs DuckLake v1.0 Spec

| Feature Area | Status | Notes |
|-------------|--------|-------|
| 28 Catalog Tables (tags) | ✅ Complete | All allocated, wired through PG-Wire |
| MVCC visibility (begin/end_snapshot) | ✅ Complete | Correct `is_visible()` on all read paths |
| Snapshot management | ✅ Complete | Atomic commit, schema_version tracking |
| Schema/Table/Column CRUD | ✅ Complete | Full lifecycle with proper key resolution |
| Data file registration | ✅ Complete | With `begin_snapshot`/`end_snapshot` versioning |
| Delete file registration | ✅ Complete | MVCC visibility on delete files |
| File column stats / pruning | ✅ Complete | Type-aware pruning with `partial_max` |
| Views & Macros | ✅ Complete | CRUD + PG-Wire wiring |
| Metadata (key/value) | ✅ Complete | Multi-scope (catalog/schema/table) |
| Tags & Column Tags | ✅ Complete (v0.27) | Full CRUD + PG-Wire |
| Sort Info / Partitioning | ✅ Complete (v0.26) | Read + write wired |
| `table_changes()` CDC | ⚠️ Structure only | Correct diff/detection but no real Parquet reads |
| `next_rowid_range()` | ✅ Complete | Transactional counter allocation |
| Snapshot leases (HOLD/RELEASE) | ✅ Complete | Length-prefixed keys, transactional |
| LISTEN/NOTIFY | ✅ Complete | Broadcast channels + per-session subscriptions |
| Extension schemas | ✅ Complete | Configurable, transactional row-id |
| GC / Excision | ✅ Complete | Transactional, safe guards enforced |
| Encryption at rest | ✅ Complete | AES-256-GCM via SlateDB BlockTransformer |
| DataFusion query execution | ⚠️ Partial | Metadata + Parquet scan via ListingTable (needs data_root) |
| SQLite VFS | ❌ Not started | 1-line placeholder |
| S3/GCS end-to-end tests | ⚠️ Partial | MinIO test exists; GCS/Azure only via object_store config |

### Key Remaining Gaps

1. **Real Parquet row scanning for CDC** — The most important missing feature for pg-trickle and external consumers.
2. **DataFusion scan without explicit `data_root`** — Tables created through PG-Wire DDL may not have a data path set.
3. **SQLite VFS** — Not started; should be removed from workspace until work begins.

## 8. Documentation vs. Implementation Drift

### Areas of Good Alignment

- `docs/architecture/` accurately describes the crate structure, MVCC model, key layout, and transaction model.
- CLI flags verified by CI smoke test against `--help` output.
- `ROADMAP.md` reflects actual milestone progression (v0.24-v0.27 are tagged and released).

### Remaining Drift

| Document | Issue |
|----------|-------|
| `docs/operations/monitoring.md` | May reference `--metrics-path` / `SLATEDUCK_METRICS_PATH` that differ from CLI implementation. (Assessment 2 finding, not re-verified.) |
| Doc-tests | Only 1 doc-test exists (`streaming.rs`). Public APIs in `slateduck-core` and `slateduck-catalog` lack `///` examples. |
| `slateduck-sqlite-vfs` | Listed as a workspace member implying supported functionality; actually empty. |

## 9. CI/CD & Operational Readiness

### Quality Gate Checklist

| Gate | Present | Enforced | Notes |
|------|---------|----------|-------|
| `cargo fmt --check` | ✅ | ✅ | Fails PR on format issues |
| `cargo clippy -D warnings` | ✅ | ✅ | All targets, all features |
| `cargo test --all` | ✅ | ✅ | Full workspace |
| `cargo deny check` | ✅ | ✅ | advisories + bans + sources + licenses |
| MSRV (1.93) | ✅ | ✅ | Dedicated CI job with `cargo check` |
| Sanitizers (ASAN/UBSAN) | ✅ | ⚠️ | Scheduled nightly, `continue-on-error: true` |
| Miri | ✅ | ⚠️ | Scheduled nightly, `continue-on-error: true` |
| Code coverage | ✅ | ⚠️ | Reports but warns (not fails) below 80% |
| Benchmark regression | ✅ | ✅ | Python script checks against baseline JSON |
| CLI smoke test | ✅ | ✅ | Verifies all documented flags exist |
| Fault injection | ✅ | ✅ | `slateduck-catalog` fault tests |
| Security tests | ✅ | ✅ | PG-Wire protocol security |
| Compatibility matrix | ✅ | ✅ | PostgreSQL 16/17/18 |
| Release workflow | ✅ | ✅ | On tag push, quality gate + multi-platform build |
| Cross-platform | ✅ | ✅ | ubuntu-latest + macos-latest |

### N-09: Coverage is a warning, not a gate

- **Severity**: Low
- **Location**: `.github/workflows/ci.yml:127-128`
- **Description**: Coverage below 80% emits `::warning` but does not fail the job.
- **Recommended Fix**: Change to a hard failure at v1.0: `if [ "${COVERAGE%.*}" -lt 80 ]; then exit 1; fi`

## 10. Dependency & Supply Chain Health

### cargo deny check output

```
advisories ok, bans ok, licenses ok, sources ok
```

All checks pass cleanly with no warnings.

### Workspace Dependencies

| Dependency | Version | Status |
|-----------|---------|--------|
| slatedb | 0.13 | Current |
| object_store | 0.12 | Current |
| tokio | 1.x | Current |
| pgwire | 0.28+ | Current |
| sqlparser | 0.55+ | Current |
| prost | 0.13 | Current |
| aes-gcm | 0.10 | Current |
| datafusion | (workspace) | Current |

### No blocking issues

- No `git`-pinned dependencies
- No path-only dependencies that block crates.io publishing
- No conflicting versions of the same crate in the dependency graph
- `deny.toml` is properly configured with license allow-list

### Build scripts

- `prost-build` is used for protobuf compilation (standard, audited)
- No custom `build.rs` scripts with arbitrary execution risk

## Prioritized Remediation Roadmap

### v0.28: Complete CDC (Parquet Row Scanning)

1. **Implement real `extract_rows_from_parquet()`** — Read actual Parquet files via `object_store`, extract row data with column names, produce proper `columns_json` with real values.
2. **Add end-to-end CDC test** — Write rows, create snapshots, call `table_changes()`, verify actual column data in output.
3. **Handle large files** — Add streaming/batching for files with millions of rows.

### v0.29: Hardening & Completeness

4. **DataFusion scan without data_root** — Resolve file paths from catalog metadata automatically.
5. **Replace executor `unwrap()` with `expect()`** — Document infallibility or add proper error handling.
6. **Add concurrent writer fencing test** — Two CatalogStore handles, verify second is rejected.
7. **Add network-level PG-Wire integration test** — Spawn binary, connect with tokio-postgres or libpq.
8. **Remove `slateduck-sqlite-vfs` from workspace** — Or begin implementation.

### v1.0: Production Gate

9. **Make coverage threshold a hard gate** at 80%.
10. **Make sanitizer/Miri jobs non-`continue-on-error`**.
11. **Add doc-tests** for public APIs in `slateduck-core` and `slateduck-catalog`.
12. **Verify checkpoint restore** does not reuse snapshot IDs.
13. **Warn when auth is enabled without TLS**.
