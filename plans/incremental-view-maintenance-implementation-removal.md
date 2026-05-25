# IVM Removal Plan

Remove all Incremental View Maintenance code from SlateDuck. The IVM layer is an architectural mismatch: it bolts a streaming aggregation engine onto a catalog store that was designed to never be in the data path. DuckDB full-query re-execution is simpler and faster for all practical scenarios.

## Phase 1: Delete the IVM Crate

Remove the entire `crates/slateduck-ivm/` directory (36 source files, 13 test files, Cargo.toml).

```
rm -rf crates/slateduck-ivm/
```

## Phase 2: Workspace Cargo.toml

1. Remove `"crates/slateduck-ivm"` from `[workspace].members` (L10).
2. Remove `wasmtime = "43"` from `[workspace.dependencies]` (L47-49) — used exclusively by slateduck-ivm.
3. Remove any IVM-related comments near those lines.

## Phase 3: slateduck-core Cleanup

### tags.rs

Remove the four IVM catalog tags and their registry entries:
- `TAG_MATVIEW = 0x1D` (L92)
- `TAG_MATVIEW_DEP = 0x1E` (L96)
- `TAG_MATVIEW_CHECKPOINT = 0x1F` (L100)
- `TAG_MATVIEW_SHARD = 0x20` (L104)
- Section header comment `// ─── v0.11 IVM Catalog Tables ──` (L88)
- Reservation comment `// Tags 0x24–0x2F reserved for future IVM-related tables.` (L115)
- `TagDescriptor` entries for all four tags in `TAG_REGISTRY` static (L390-420)

NOTE: Do NOT renumber existing tags. Leave a gap at 0x1D-0x20 with a comment `// 0x1D–0x20: removed (formerly IVM)` for forward compatibility with old catalogs.

### rows.rs

Remove the following types (L549-690):
- `MatviewRow` struct
- `OutputMode` enum + `from_u32()`
- `MatviewDepRow` struct
- `MatviewCheckpointRow` struct
- `MatviewShardRow` struct

### keys.rs

Remove the following functions (L430-493):
- `key_matview(matview_id, begin_snapshot)`
- `key_matview_dep(matview_id, base_table_id)`
- `key_matview_checkpoint(matview_id, shard_id, seq)`
- `key_matview_shard(matview_id, shard_id)`
- `prefix_matview(matview_id)`
- `prefix_matview_deps(matview_id)`
- `prefix_matview_checkpoints(matview_id, shard_id)`
- `prefix_matview_shards(matview_id)`

Remove associated tests (L814-878):
- `matview_key_structure`
- `matview_dep_key_structure`
- `matview_checkpoint_key_structure`
- `matview_shard_key_structure`
- `matview_key_prefix_isolation`
- `matview_checkpoint_seq_ordering`

## Phase 4: slateduck-catalog Cleanup

### writer.rs

Remove:
- `ClaimOutcome` enum (L38-40)
- `create_matview()` (L877)
- `drop_matview()` (L935)
- `set_matview_status()` (L957)
- `update_matview_checkpoint()` (L978)
- `claim_matview_shard()` (L1015)
- `extend_matview_lease()` (L1107)
- `release_matview_lease()` (L1153)
- `set_matview_output_mode()` (L1191)
- `re_shard_matview()` (L1239)

### reader.rs

Remove:
- `list_matviews()` (L614)
- `get_matview(matview_id)` (L636)
- `get_matview_by_name(schema, name)` (L657)
- `list_matview_deps(matview_id)` (L671)
- `list_matview_shards(matview_id)` (L686)
- `list_shards_for_worker(worker_id)` (L709)
- `read_checkpoint_history(matview_id)` (L719)
- `matview_lag_ms(matview_id, shard_id)` (L742)
- `matview_max_lag_ms(matview_id)` (L758)

### lib.rs

Remove `ClaimOutcome` from `pub use` if re-exported.

### Tests

Delete `tests/v011_tests.rs` entirely.

Remove the IVM integration test section from `tests/v010_tests.rs` (L791-899: `ivm_integration_ingest_to_cdc_pipeline`).

## Phase 5: slateduck-sql Cleanup

### classifier.rs

Remove:
- Section header `// ─── v0.11 IVM Statements ───` (L83)
- `StatementKind` variants (L85-116):
  - `CreateIncrementalMatview { name, schema, select_sql, options }`
  - `DropIncrementalMatview { name, schema, if_exists }`
  - `AlterIncrementalMatview { name, schema, options }`
  - `RefreshIncrementalMatviewFull { name, schema }`
  - `ShowMaterializedViews`
  - `ShowMatviewShards { name, schema }`
  - `ExplainMatview { name, schema }`
- `classify_ivm_prefix(sql)` function (L180-281)
- Any call site invoking `classify_ivm_prefix` in the main `classify()` function

## Phase 6: slateduck-pgwire Cleanup

### Cargo.toml

Remove `slateduck-ivm = { path = "../slateduck-ivm" }` dependency (L40).

### executor.rs

Remove the IVM match arm (L475-484) that routes IVM DDL statements to `SlateDuckError::Unsupported`. After removing the `StatementKind` variants in Phase 5, these arms will not exist anyway — just ensure no dead code remains.

### Tests

Remove IVM references from:
- `tests/security_tests.rs` (L9: `use slateduck_ivm::rate_limit::{...}`)
- `tests/compat_tests.rs` (L17: IVM join workflow comment)

## Phase 7: slateduck-testkit Cleanup

### Cargo.toml

Remove `slateduck-ivm = { path = "../slateduck-ivm" }` (L10).

### Source files

Remove or gut IVM-specific harness code:
- `src/harness.rs` — `IvmWorkerHarness` struct (delete entire file if IVM-only)
- `src/oracle.rs` — `IvmOracle` struct (delete entire file if IVM-only)
- `src/duckdb_harness.rs` — remove IVM assertion helpers (keep if used for non-IVM SQL testing)
- `src/clock.rs` — remove IVM lease TTL test support (keep if used for non-IVM lease tests)
- `src/catalog_harness.rs` — keep (non-IVM catalog testing)

### lib.rs

Remove `IvmWorkerHarness` and `IvmOracle` re-exports (L6, 10, 11, 34, 36).

## Phase 8: Documentation

### Delete entirely

- `docs/architecture/ivm-plane.md`
- `docs/concepts/incremental-views.md`
- `docs/reference/sql-ivm.md`
- `docs/operations/ivm-join-sizing.md`
- `docs/operations/ivm-cost-control.md`
- `docs/operations/ivm-backup-restore.md`
- `docs/design-decisions/ivm-architecture.md`
- `docs/design-decisions/ivm-on-immutable-substrate.md`
- `docs/design-decisions/ivm-recursive-spike.md`
- `docs/design-decisions/ivm-retrospective.md`

### Edit (remove IVM sections)

- `docs/architecture/streaming-pipeline.md` — remove IVM references at L60, L95
- `docs/architecture/key-layout.md` — remove "v0.11 IVM Tag Extensions" section (L236+)
- `docs/reference/udfs.md` — remove IVM-related lines (L6, 97, 117, 126)

### mkdocs.yml

Remove IVM nav entries (L188: `- IVM Join Sizing: operations/ivm-join-sizing.md` and any others referencing deleted files).

## Phase 9: Benchmarks and Fixtures

### Delete

- `benchmarks/v0.12-ivm-scaleout.json`
- `benchmarks/v0.13-ivm-joins.json`
- `benchmarks/v0.15-ivm-hardening.json`
- `benchmarks/v0.17-ivm-hardening.json`
- `benchmarks/v0.17-adaptive-calibration.json`
- `tests/fixtures/matview/` (entire directory)

## Phase 10: README.md and ROADMAP.md

### README.md

- Remove IVM tagline references (L17, 29)
- Remove IVM architecture diagram annotations (L41, 59)
- Remove `slateduck-ivm` from crate table (L71)
- Remove IVM Getting Started example (L93-97)
- Remove entire "Incremental View Maintenance" section (L145-194)
- Remove IVM roadmap rows from the roadmap table

### ROADMAP.md

- Remove v0.11 through v0.17 IVM milestones (L63, L1689-3716: ~2000 lines)
- Remove IVM test tiers (6a-6f, tier 7)
- Keep non-IVM milestones (v0.1-v0.10, v0.18+)

## Phase 11: CI

### .github/workflows/ci.yml

Remove:
- Tier 7 comment (L113)
- IVM fault injection tests step (L122-123)
- IVM hardening tests step (L128-129)
- IVM property tests step (L131-132)
- Benchmark regression check referencing IVM JSON files (L157)

## Phase 12: deny.toml

Remove the two advisory ignores that are IVM-only transitive deps (L27-30):
- `RUSTSEC-2024-0370` (proc-macro-error via dbsp)
- `RUSTSEC-2025-0057` (fxhash via wasmtime 43)

Verify with `cargo deny check` after removal — if other crates still pull these deps, keep the ignores.

## Phase 13: Plans Directory

These files are historical. Keep them for context but add a header noting IVM was removed:
- `plans/incremental-view-maintenance-implementation.md`
- `plans/slateduck-differential-dataflow.md`
- `plans/slatedb-differential-dataflow.md`
- `plans/pg-trickle.md`
- `plans/pg-trickle-ducklake-support.md`

Or delete them entirely if you prefer a clean break.

Remove IVM references from:
- `plans/e2e-integration-tests.md` — remove IvmWorkerHarness, matview fixtures, IVM test coverage sections
- `plans/overall-assessment-2.md` — remove slateduck-ivm file references
- `plans/blueprint-2.x.md` — remove "## 11. Incremental View Maintenance (IVM)" section (L1229+)

## Phase 14: Verify

1. `cargo build --workspace` — must compile cleanly
2. `cargo test --workspace` — all remaining tests pass
3. `cargo clippy --workspace -- -Dwarnings` — no warnings
4. `cargo deny check` — no new advisories
5. Grep for stragglers: `rg -i "matview|ivm|incremental.materialized|IvmWorker|IvmCircuit|ZDelta"` — should return zero hits outside this plan file
6. Verify `mkdocs build` succeeds (no broken nav links)

## Execution Order

Phases 1-6 must be done together (they form a dependency chain — removing the crate breaks imports). Recommended approach:

1. Do Phase 1 (delete crate)
2. Do Phase 2 (workspace Cargo.toml)
3. Do Phase 3-6 (core/catalog/sql/pgwire — fix compile errors)
4. Do Phase 7 (testkit)
5. `cargo build --workspace` — fix any remaining compile errors
6. `cargo test --workspace` — fix any remaining test failures
7. Do Phases 8-13 (docs, benchmarks, fixtures, README, ROADMAP, CI, deny.toml, plans)
8. Do Phase 14 (verify)

## Expected Impact

- **Lines removed**: ~15,000-20,000 (source) + ~5,000 (tests) + ~3,000 (docs/plans)
- **Dependencies dropped**: wasmtime (43), dbsp transitive deps, several proc-macro crates
- **Compile time reduction**: Significant (wasmtime alone adds ~30s to clean builds)
- **Binary size reduction**: `slateduck-ivm` binary eliminated entirely
- **Cognitive load**: One fewer architectural plane to understand
