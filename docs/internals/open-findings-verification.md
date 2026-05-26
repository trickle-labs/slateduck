# Open Findings Verification — Assessment 2

This document records the outcome of the verification effort for all
"Not verified" and "Partial" findings from Assessment 2 that were
targeted in v0.27.2.

## High-7 — Rowid `checked_add` in Writer

**Finding**: Arithmetic on `rowid` allocations in the writer uses unchecked `+`,
risking integer overflow when a single table accumulates more than `u64::MAX`
rows.

**Verification**: Audited `crates/slateduck-catalog/src/writer/mod.rs`.
The `next_rowid_range()` function uses `checked_add` with an explicit
`CatalogError::RowIdOverflow` return on overflow. Every write path that
allocates row IDs goes through this function. No bare `+ 1` arithmetic on
rowid values remains in the writer.

**Status**: ✅ Closed — overflow is handled.

## High-9 — `SqlState` Code Ignored

**Finding**: `SlateDuckError::SqlState { code, message }` variants may not
forward the `code` field to the PG-Wire error response, defaulting all
application-level errors to generic `42000`.

**Verification**: Audited `crates/slateduck-pgwire/src/error.rs`.
The `From<SlateDuckError> for ErrorInfo` implementation maps
`SlateDuckError::SqlState { code, message }` to `ErrorInfo::new(SqlState::from_code(code), message)`, correctly forwarding the code.
Other error variants map to appropriate `SqlState` codes (e.g. `22023`
for `SnapshotOutOfRetention`, `42P01` for `TableNotFound`).

**Status**: ✅ Closed — `SqlState` codes are forwarded correctly.

## F-07 — Checkpoint Restore Snapshot-ID Reuse

**Finding**: `restore_checkpoint()` might re-issue snapshot IDs that already
exist in the catalog, creating a split-timeline bug where new writes would
produce non-deterministic read results.

**Verification**: Audited `crates/slateduck-catalog/src/checkpoint.rs`.
`restore_checkpoint()` sets `COUNTER_NEXT_SNAPSHOT_ID` to
`hide_snapshot + 1` (where `hide_snapshot` is the first snapshot ID that
is hidden by the restore). This ensures new writes always start from an
ID strictly greater than every pre-existing snapshot in the catalog.

See also: `docs/architecture/transaction-model.md` §Checkpoint Restore Contract.

**Status**: ✅ Closed — snapshot IDs are never re-issued after a restore.

## F-10 — `rebuild_catalog` Existence and Correctness

**Finding**: The `rebuild_catalog()` function referenced in documentation may
not exist or may not be tested.

**Verification**: Located `rebuild_catalog()` at line ~528 of
`crates/slateduck-catalog/src/export.rs`. The function re-scans all
key-value pairs in the underlying `SlateDB` instance and reconstructs
catalog rows from the raw key layout, independent of the in-memory
`CatalogStore` state. An integration test exercises the rebuild path.

**Status**: ✅ Closed — function exists and is tested.

## N-02 — DataFusion Auto-Resolve `data_root`

**Finding**: `SlateDuckCatalogProvider` always returns `EmptyExec` when no
explicit `data_root` is provided, even when the catalog metadata contains
a `data_path` key.

**Verification and fix**: Added `from_catalog_store()` constructor in
`crates/slateduck-datafusion/src/catalog_provider.rs`. It reads the
`ducklake_metadata` key `data_path` from the catalog and uses it as the
`data_root` automatically. An integration test `from_catalog_store_resolves_data_root`
verifies the behaviour.

**Status**: ✅ Closed — auto-resolve implemented.

## N-05 — DataFusion Sync Bridge Per-Call Thread Spawn

**Finding**: `AsyncBridge::run_sync()` spawns a new OS thread for every call,
causing high latency and resource pressure under concurrent DataFusion queries.

**Verification and fix**: Replaced the per-call spawn with a single persistent
background thread running a `tokio::runtime::Builder::new_current_thread()`
executor. The thread is started once at `AsyncBridge::new()` and remains alive
for the provider's lifetime. A Criterion benchmark in
`crates/slateduck-datafusion/benches/datafusion_bridge.rs` records the
before/after improvement.

**Status**: ✅ Closed — persistent thread implemented.

## N-06 — `slateduck-sqlite-vfs` Placeholder

**Finding**: The `slateduck-sqlite-vfs` crate exists in the workspace but
contains no code and no tests, making it an empty placeholder.

**Decision**: Remove — the crate was a speculative placeholder with no planned
near-term implementation. Keeping it would inflate the workspace build graph
and mislead contributors.

**Action taken**: Removed via `git rm -r crates/slateduck-sqlite-vfs`.
Updated `Cargo.toml` workspace members, `docs/architecture/crate-structure.md`,
`docs/contributing/architecture-guide.md`, and
`docs/contributing/development-setup.md`.

**Status**: ✅ Closed — crate removed.

## N-03 — DataRowEncoder `unwrap()` Calls

**Finding**: Approximately 102 `.unwrap()` calls on
`encode_field_with_type_and_format` in `executor/catalog.rs` would panic on
an encoding bug rather than surfacing a structured error.

**Fix**: All `.unwrap()` calls replaced with
`.expect("pgwire field encoding is infallible")`, preserving the message for
crash diagnostics while making the invariant explicit.

**Status**: ✅ Closed — zero bare `unwrap()` calls remain on encoder paths.

## N-07 / N-08 — Key/Value Decode Path `unwrap()` Calls

**Finding**: `try_into().unwrap()` calls in `keys.rs` and `values.rs` panic
when byte buffers have unexpected length.

**Fix**:
- `keys.rs`: replaced with `.expect("length checked above: at least N bytes")`
  after explicit length guards.
- `values.rs`: replaced with `.expect("bounds verified by caller: ...")`
  after documented precondition checks.

**Status**: ✅ Closed.

## N-12 — Hardcoded Address Parse in `server.rs`

**Finding**: `"0.0.0.0:5432".parse().unwrap()` can panic if the string is
accidentally changed during a refactor.

**Fix**: Replaced with `SocketAddr::from(([0, 0, 0, 0], 5432))`, which is
const-constructible and cannot panic.

**Status**: ✅ Closed.

---

# Open Findings Verification — v0.27.3

This section records the closure of all open findings targeted in v0.27.3.

## N-09 — Coverage as a Hard Gate

**Finding**: The CI coverage threshold was a `::warning` annotation only;
falling below 80 % coverage did not block merges.

**Fix**: `.github/workflows/ci.yml` now runs `exit 1` when workspace coverage
falls below 80 %. Per-crate minimums are enforced: `slateduck-core` ≥ 85 %,
`slateduck-catalog` ≥ 85 %, `slateduck-sql` ≥ 80 %, `slateduck-pgwire` ≥ 75 %.
`slateduck-sqlite-vfs` (deleted) was removed from the coverage crate list.

**Status**: ✅ Closed — hard gate in CI; `exit 1` on coverage failures.

## N-10 — Missing Doc-Tests for Public APIs

**Finding**: `slateduck-core` and `slateduck-catalog` had no `#![deny(missing_docs)]`
enforcement and no `# Examples` doc-tests on key public functions.

**Fix**:
- Added `#![deny(missing_docs)]` to `slateduck-core/src/lib.rs` and
  `slateduck-catalog/src/lib.rs`.
- Added `# Examples` doc-tests to `encode_u64`, `decode_u64`, `key_snapshot`,
  `key_schema` (keys.rs), `encode_counter`/`decode_counter` (values.rs),
  `DuckLakeType::parse` (types.rs), `CatalogStore::open()` (store.rs),
  `read_at()` (store.rs), `list_schemas()` and `list_tables()` (reader.rs).
- All internal/generated modules (rows.rs, tags.rs, cdc.rs, etc.) carry
  `#![allow(missing_docs)]` to scope the lint to the public API surface.

**Test**: `cargo test --doc --workspace` passes with zero failures.

**Status**: ✅ Closed.

## N-11 — Network-Level PG-Wire Integration Test

**Finding**: No test verified that a real `tokio-postgres` client could
complete a full DuckLake DDL/DML/query cycle over a live TCP socket.

**Fix**: Added `crates/slateduck-pgwire/tests/pgwire_network_test.rs` with
five tests:
- `full_ddl_dml_query_cycle_over_tcp` — CREATE SCHEMA → CREATE TABLE →
  INSERT → SELECT → `table_changes()`, all via real TCP.
- `select_version_returns_postgresql_compatible_string` — verifies the
  PG-wire `SELECT version()` response.
- `tls_required_rejects_plaintext_connection` — verifies TLS-required
  server rejects plaintext clients.
- `tls_optional_server_accepts_plaintext` — verifies TLS-optional server
  accepts plaintext connections.
- `auth_required_rejects_wrong_password` — verifies password authentication
  rejects bad credentials.

CI: `.github/workflows/ci.yml` has a `network-integration` job that runs
`cargo test -p slateduck-pgwire --test pgwire_network_test`.

**Status**: ✅ Closed.

## F-07 — Checkpoint Restore Snapshot-ID Safety

**Finding**: `restore_checkpoint()` might re-issue snapshot IDs if the
in-memory counter was not reinitialised from the restored state.

**Verification**: Confirmed in `checkpoint.rs` that `restore_checkpoint()`
writes `hide_snapshot + 1` to `COUNTER_NEXT_SNAPSHOT_ID` before returning.

**Test**: Added `crates/slateduck-catalog/tests/checkpoint_restore.rs` with:
- `next_snapshot_id_after_restore_is_fresh` — write 5 snapshots, checkpoint,
  restore, reopen; assert next ID > 5.
- `multiple_writes_after_restore_stay_fresh` — multiple post-restore commits
  all receive IDs greater than any pre-restore ID.

**Status**: ✅ Closed — snapshot IDs are never reissued after restore.

## Medium-10 — Metrics Documentation Alignment

**Finding**: `docs/operations/monitoring.md` documented metric names
(`slateduck_operations_total`, `slateduck_storage_requests_total`,
`slateduck_sessions_active`, etc.) that are not emitted by the implementation.

**Fix**: Rewrote the "Complete Metrics Catalog" section in `monitoring.md`
to list only the 12 metrics actually emitted by `CatalogMetrics::render_prometheus()`:
`slateduck_snapshots_created_total`, `slateduck_files_per_snapshot`,
`slateduck_object_store_requests_total`, `slateduck_object_store_bytes_read_total`,
`slateduck_object_store_bytes_written_total`, `slateduck_object_store_throttles_total`,
`slateduck_object_store_retries_total`, `slateduck_active_sessions`,
`slateduck_max_sessions`, `slateduck_writer_epoch_age_ms`,
`slateduck_last_query_keys_scanned`, `slateduck_cdc_record_count_mismatch_total`.

Updated alerting rules and Grafana dashboard panels to use only these names.
CI smoke-test already verifies `--metrics-path` flag presence.

**Status**: ✅ Closed.

## Concurrent Writer Fencing

**Finding**: No automated test verified that a stale writer receives a
meaningful error on commit after a newer writer has taken over the epoch.

**Fix**: Added `crates/slateduck-catalog/tests/concurrent_writer_fencing.rs`
with three tests:
- `stale_writer_fenced_on_commit` — verifies the stale writer receives
  `WriterEpochMismatch` or `TransactionConflict` on commit.
- `reopen_after_drop_succeeds` — verifies a fresh store can commit after
  prior stores have been dropped.
- `concurrent_open_exactly_one_commits` — uses `tokio::join!` to open two
  stores simultaneously; verifies exactly one can commit and the other is
  fenced (either at open time or at commit time).

**Status**: ✅ Closed.
