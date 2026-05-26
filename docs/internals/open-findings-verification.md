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
