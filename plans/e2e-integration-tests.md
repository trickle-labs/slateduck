# Integration & End-to-End Testing Plan

> **Scope.** Complete testing strategy for SlateDuck covering all integration and E2E layers from unit-level in-process tests through full multi-node deployments against real object stores. Companion to the overall architecture in [plans/blueprint.md](blueprint.md) and the IVM implementation plan in [plans/incremental-view-maintenance-implementation.md](incremental-view-maintenance-implementation.md).
>
> **Status.** Planning. Implement tier-by-tier alongside milestone delivery.
>
> **Audience.** Contributors building or reviewing tests. This document specifies test file names, helper APIs, crate locations, CI job names, and exact assertions. When a test is listed by name here, that name is canonical.

---

## Table of Contents

1. [Philosophy](#1-philosophy)
2. [Testing Tiers](#2-testing-tiers)
3. [Infrastructure & Helpers](#3-infrastructure--helpers)
4. [Tier 1 — Unit & Property Tests](#4-tier-1--unit--property-tests)
5. [Tier 2 — Catalog Integration Tests](#5-tier-2--catalog-integration-tests)
6. [Tier 3 — PG-Wire Integration Tests](#6-tier-3--pg-wire-integration-tests)
7. [Tier 4 — Object Store Integration Tests (MinIO)](#7-tier-4--object-store-integration-tests-minio)
8. [Tier 5 — Client Compatibility Tests (DuckDB, Spark, Trino)](#8-tier-5--client-compatibility-tests-duckdb-spark-trino)
9. [Tier 6 — IVM Integration Tests](#9-tier-6--ivm-integration-tests)
10. [Tier 7 — Fault Injection Tests](#10-tier-7--fault-injection-tests)
11. [Tier 8 — Scale & Soak Tests](#11-tier-8--scale--soak-tests)
12. [Tier 9 — Security Tests](#12-tier-9--security-tests)
13. [Tier 10 — Benchmark Regression Tests](#13-tier-10--benchmark-regression-tests)
14. [CI Matrix](#14-ci-matrix)
15. [Test Crate Layout](#15-test-crate-layout)
16. [Open Questions](#16-open-questions)

---

## 1. Philosophy

Five rules govern every test decision.

**T1 — Each test proves exactly one thing.** Tests are named after the property they assert: `catalog_open_on_existing_s3_bucket_succeeds`, not `test_s3`. Long names are fine. Ambiguous names are not.

**T2 — Real infrastructure beats mocks at the seam.** We mock nothing at the storage boundary. The `LocalFileSystem` adapter is the fast-path default for pure-logic tests. MinIO via Testcontainers is the seam test for S3-compatible behaviour. Real AWS/GCS/Azure are optional gates behind feature flags and CI secrets.

**T3 — Tests must be deterministic.** Timing-sensitive tests use `tokio::time::pause()` and explicitly advanced clocks. No `sleep`. Retries indicate a timing dependency that must be fixed in the test helper, not in the test body.

**T4 — Failure is a first-class scenario.** Every write path has a corresponding test that injects a failure mid-way and asserts the catalog remains consistent afterward.

**T5 — Testcontainers for real services; `tempfile` for pure FS.** Services that have genuine S3 API semantics (MinIO), genuine PostgreSQL wire semantics (for client compatibility), or genuine Kafka/NATS behaviour use Testcontainers. Filesystem tests remain on `tempfile`. Never try to fake S3 semantics in a `LocalFileSystem` test and call it an S3 test.

---

## 2. Testing Tiers

| Tier | Focus | Transport | Trigger | Runner |
|------|-------|-----------|---------|--------|
| 1 — Unit / Property | Logic correctness, key encoding, MVCC | In-process | Every PR | GitHub Actions standard |
| 2 — Catalog Integration | Catalog API correctness on `LocalFS` | In-process | Every PR | GitHub Actions standard |
| 3 — PG-Wire Integration | Protocol correctness, executor, wire corpus | TCP loopback | Every PR | GitHub Actions standard |
| 4 — Object Store Integration | S3-compatible behaviour against MinIO | Testcontainers (MinIO) | Every merge to `main` | GitHub Actions large runner |
| 5 — Client Compatibility | DuckDB / Spark / Trino wire corpus replay | TCP loopback + corpus | Every merge to `main` | GitHub Actions standard |
| 6 — IVM Integration | Single-shard → sharded → joins → hardening | MinIO + multiple processes | Every merge to `main` | GitHub Actions large runner |
| 7 — Fault Injection | Kill-9, S3 503, compaction races | MinIO + `fail` crate | Pre-release | GitHub Actions large runner |
| 8 — Scale & Soak | 16-shard scale, 24h soak, TPC-H SF10/SF100 | Real S3 / S3 Express | Pre-release (manual) | Dedicated EC2 |
| 9 — Security | IAM separation, TLS, credential isolation | MinIO + real AWS | Pre-release | GitHub Actions |
| 10 — Benchmark Regression | Latency/throughput vs baseline JSON | `LocalFS` + MinIO | Weekly | GitHub Actions |

---

## 3. Infrastructure & Helpers

### 3.1 New workspace crate: `slateduck-testkit`

A dedicated test-helper crate that every integration test depends on. Lives at `crates/slateduck-testkit/`. Contains no production code. Not shipped in releases.

```
crates/slateduck-testkit/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── catalog.rs      # CatalogHarness: open/teardown helpers
    ├── minio.rs        # MinioHarness: Testcontainers MinIO lifecycle
    ├── pgwire.rs       # PgWireHarness: start/connect/teardown server
    ├── duckdb.rs       # DuckDbHarness: spawn DuckDB process, run SQL
    ├── clock.rs        # DeterministicClock: freeze/advance tokio time
    ├── corpus.rs       # Wire corpus replay helpers
    └── ivm.rs          # IvmWorkerHarness: spawn/kill workers, assert lag
```

#### `Cargo.toml` outline

```toml
[package]
name = "slateduck-testkit"
version.workspace = true
edition.workspace = true
publish = false

[dependencies]
slateduck-core    = { path = "../slateduck-core" }
slateduck-catalog = { path = "../slateduck-catalog" }
slateduck-pgwire  = { path = "../slateduck-pgwire" }
object_store      = { workspace = true }
tokio             = { workspace = true }
tempfile          = { workspace = true }
testcontainers    = { version = "0.23", features = ["blocking"] }
testcontainers-modules = { version = "0.11", features = ["minio"] }
tokio-postgres    = { workspace = true }
tracing-subscriber = { workspace = true }
```

### 3.2 `MinioHarness`

Wraps a Testcontainers MinIO container. One harness per test suite (not per test function). Uses the official `minio/minio` image pinned to a digest for reproducibility.

```rust
pub struct MinioHarness {
    container: ContainerAsync<MinIO>,
    pub endpoint: String,        // e.g. "http://127.0.0.1:49382"
    pub access_key: String,      // "minioadmin"
    pub secret_key: String,      // "minioadmin"
    pub bucket: String,          // assigned per harness
}

impl MinioHarness {
    pub async fn start(bucket: &str) -> Self;
    pub fn open_options(&self, prefix: &str) -> OpenOptions;
    pub async fn put_object(&self, key: &str, data: &[u8]);
    pub async fn list_objects(&self, prefix: &str) -> Vec<String>;
    pub async fn delete_object(&self, key: &str);
}
```

A single `MinioHarness::start` per test module (not per test), shared via `tokio::sync::OnceLock`. Container startup overhead is paid once.

### 3.3 `CatalogHarness`

```rust
pub struct CatalogHarness {
    pub store: Arc<Mutex<CatalogStore>>,
    _dir: Option<tempfile::TempDir>,  // Some for LocalFS, None for MinIO
}

impl CatalogHarness {
    pub async fn local() -> Self;
    pub async fn on_minio(harness: &MinioHarness, prefix: &str) -> Self;
    pub async fn writer(&self) -> CatalogWriter;
    pub async fn reader_latest(&self) -> CatalogReader;
    pub async fn reader_at(&self, snapshot: SnapshotId) -> CatalogReader;
}
```

### 3.4 `PgWireHarness`

Starts a live `slateduck-pgwire` server on a dynamically assigned loopback port. Returns a `tokio-postgres` client.

```rust
pub struct PgWireHarness {
    pub addr: SocketAddr,
    shutdown_tx: oneshot::Sender<()>,
}

impl PgWireHarness {
    pub async fn start(catalog: Arc<Mutex<CatalogStore>>) -> Self;
    pub async fn start_with_config(config: ServerConfig, catalog: ...) -> Self;
    pub async fn connect(&self) -> tokio_postgres::Client;
    pub async fn connect_tls(&self, cert: &str, key: &str) -> tokio_postgres::Client;
}

impl Drop for PgWireHarness {
    fn drop(&mut self) { let _ = self.shutdown_tx.send(()); }
}
```

### 3.5 `IvmWorkerHarness`

Spawns one or more `slateduck-ivm` worker processes as `tokio::process::Command`, captures stdout/stderr, and exposes lag polling.

```rust
pub struct IvmWorkerHarness {
    processes: Vec<tokio::process::Child>,
    catalog_path: String,
    state_prefix: String,
}

impl IvmWorkerHarness {
    pub async fn start_worker(id: &str, shard_limit: u32, minio: &MinioHarness) -> Self;
    pub async fn add_worker(&mut self, id: &str, shard_limit: u32);
    pub async fn kill_worker(&mut self, id: &str);
    pub async fn wait_lag_below_ms(&self, matview: &str, threshold_ms: u64, timeout: Duration);
    pub async fn assert_output_matches(&self, matview: &str, expected_sql: &str);
}
```

### 3.6 `DeterministicClock`

Wraps `tokio::time::pause()` + `tokio::time::advance()` to drive lease expiry, heartbeat, and freshness tests without wall-clock waits.

```rust
pub struct DeterministicClock;
impl DeterministicClock {
    pub async fn pause() -> Self;           // calls tokio::time::pause()
    pub async fn advance(&self, d: Duration);
    pub async fn advance_to_lease_expiry(&self, ttl_ms: u64);
}
```

### 3.7 Shared test binary: `tests/`

A workspace-level `tests/` directory holds E2E tests that span multiple crates and require the full binary. Uses `assert_cmd` + `tokio_postgres` rather than in-process calls.

---

## 4. Tier 1 — Unit & Property Tests

**Location:** `crates/slateduck-core/src/` (inline `#[cfg(test)]` modules) and `crates/slateduck-core/tests/property_tests.rs`.

**Dependencies:** none beyond `proptest`.

### 4.1 Key encoding (inline tests in `keys.rs`)

| Test name | Assertion |
|-----------|-----------|
| `round_trip_all_tags` | Every tag byte encodes and decodes to the same key |
| `unknown_tag_returns_explicit_error` | Byte `0x00` returns `Err(UnknownTag)`, never panics |
| `key_ordering_preserved_for_u64_be` | `encode(a) < encode(b)` iff `a < b` for all u64 pairs |
| `key_ordering_preserved_for_matview_checkpoint_seq` | Same, for the composite `(matview_id, shard_id, seq)` key |

### 4.2 MVCC visibility (inline tests in `mvcc.rs`)

| Test name | Assertion |
|-----------|-----------|
| `row_visible_at_begin_snapshot` | `is_visible(5, 0, 5)` = true |
| `row_not_visible_before_begin` | `is_visible(5, 0, 4)` = false |
| `row_not_visible_at_end_snapshot` | `is_visible(5, 7, 7)` = false |
| `open_row_visible_after_begin` | `is_visible(5, 0, u64::MAX)` = true |
| `gc_eligible_iff_end_lte_retain_from` | Property: `is_insert_gc_eligible(end, retain)` = `end != 0 && end <= retain` |

### 4.3 Value encoding (inline tests in `values.rs`)

| Test name | Assertion |
|-----------|-----------|
| `magic_prefix_present_in_all_encoded_values` | Every `encode()` output starts with `SDKV` |
| `unknown_encoding_version_returns_error` | Version byte `0xFF` returns explicit error |
| `round_trip_all_row_types` | Property: `decode(encode(row)) == row` for all 28 row types + 4 IVM row types |

### 4.4 Property tests (`property_tests.rs`)

| Test name | Generator | Assertion |
|-----------|-----------|-----------|
| `prop_key_snapshot_monotone_under_appends` | Random sequence of snapshot IDs | Each subsequent key is lexicographically greater |
| `prop_mvcc_visible_set_is_total_order` | Random begin/end/snapshot triples | Visibility decisions form a consistent total order |
| `prop_counter_cache_never_reuses_id` | Interleaved `next_snapshot_id()` calls | All returned IDs are distinct |

---

## 5. Tier 2 — Catalog Integration Tests

**Location:** `crates/slateduck-catalog/tests/`
**Backend:** `LocalFileSystem` via `tempfile` (fast, no external services)
**Harness:** `CatalogHarness::local()`

### 5.1 Existing tests (retained, reviewed, extended)

All tests in `integration_tests.rs`, `v091_tests.rs` through `v094_tests.rs`, and `v010_tests.rs` are kept. Each is assigned to this tier.

### 5.2 New tests to add (`v011_catalog_tests.rs`)

IVM catalog primitives. Each method gets: happy path, conflict, idempotence, and at-wrong-state assertion.

| Test name | What it asserts |
|-----------|-----------------|
| `create_matview_persists_and_is_readable` | `create_matview` → `get_matview_by_name` returns same row |
| `create_matview_duplicate_name_returns_42710` | Second `create_matview` with same `(schema, name)` → `SQLSTATE 42710` |
| `drop_matview_sets_end_snapshot` | `drop_matview` → `get_matview` returns row with `end_snapshot != 0` |
| `drop_matview_idempotent` | `drop_matview` twice → second call returns `Ok(())` |
| `claim_matview_shard_cas_acquires` | `claim_matview_shard` on unowned shard → `Acquired` |
| `claim_matview_shard_contended_returns_current_owner` | Two concurrent `claim_matview_shard` calls on same shard → exactly one `Acquired`, one `Contended` |
| `claim_matview_shard_already_owned_by_same_worker` | Same worker re-claims → `AlreadyOwned` |
| `extend_lease_increments_generation` | `extend_matview_lease` with correct generation → `Ok`; `generation` in row incremented |
| `extend_lease_stale_generation_returns_40001` | `extend_matview_lease` with stale generation → `SQLSTATE 40001` |
| `release_lease_clears_owner` | `release_matview_lease` → `list_matview_shards` returns `owner_worker = ""` |
| `release_lease_idempotent_if_taken_over` | Release after takeover by another worker → `Ok(())`, no panic |
| `checkpoint_seq_is_monotone_per_shard` | 100 `update_matview_checkpoint` calls → seq values 1..=100, strictly increasing |
| `matview_lag_ms_reflects_latest_checkpoint` | Create checkpoint with known `durable_at_unix_ms` → `matview_lag_ms` returns plausible value |
| `matview_deps_populated_for_create` | `create_matview` with deps → `list_matview_deps` returns all deps |
| `dropped_matview_invisible_to_list` | After `drop_matview`, `list_matviews` does not return it |
| `dropped_matview_readable_by_id` | After `drop_matview`, `get_matview(id)` still returns the row (MVCC) |
| `set_matview_status_persists` | `set_matview_status(Stale)` → `get_matview` returns `status = 1` |
| `checkpoint_history_ordered_by_seq` | `read_checkpoint_history` returns rows in ascending `seq` order |
| `concurrent_init_convergence_with_matview_tags` | Simulate 5 concurrent `CatalogStore::open` calls; verify exactly one coherent initial state including tag `0x1D`–`0x20` allocations |

### 5.3 Fixture coverage (`tests/fixtures/matview/`)

| File | Contents |
|------|----------|
| `create_view.dat` | Single matview creation, no shards |
| `multi_shard.dat` | View with 8 shards, leases unowned |
| `lease_acquired.dat` | Same, one shard claimed by `worker-0` |
| `checkpoint_history.dat` | 100 checkpoints across 8 shards |
| `dropped.dat` | View with `end_snapshot != 0` |

Each fixture test asserts: round-trip encode/decode matches, key ordering is preserved, MVCC visibility of each row is correct at the fixture's snapshot.

---

## 6. Tier 3 — PG-Wire Integration Tests

**Location:** `crates/slateduck-pgwire/tests/integration_tests.rs`
**Backend:** `LocalFileSystem` (for executor tests) + TCP loopback server
**Harness:** `PgWireHarness`

### 6.1 Existing tests (retained)

All existing tests are retained. The wire corpus replay tests (`test_wire_handshake_replay`, `test_spark_corpus_all_statements_classifiable`, etc.) are extended as new client versions are captured.

### 6.2 SQL surface for IVM (`v011_pgwire_tests.rs`)

| Test name | SQL | Expected outcome |
|-----------|-----|-----------------|
| `create_imv_persists_to_catalog` | `CREATE INCREMENTAL MATERIALIZED VIEW s.v AS SELECT ...` | Catalog contains the matview row |
| `create_imv_duplicate_returns_42710` | Same statement twice | Second returns `42710` |
| `create_imv_if_not_exists_is_idempotent` | `CREATE INCREMENTAL MATERIALIZED VIEW IF NOT EXISTS ...` twice | Second returns success with no state change |
| `drop_imv_marks_dropped` | `DROP INCREMENTAL MATERIALIZED VIEW s.v` | Catalog row has `end_snapshot != 0` |
| `drop_imv_if_exists_on_missing_is_noop` | `DROP INCREMENTAL MATERIALIZED VIEW IF EXISTS s.nonexistent` | Returns success |
| `drop_table_with_matview_dep_returns_2bp01` | `DROP TABLE t` where `t` is a dep of `v` | Returns `SQLSTATE 2BP01` |
| `alter_imv_sets_freshness` | `ALTER INCREMENTAL MATERIALIZED VIEW v SET (freshness = '10s')` | Catalog row updated |
| `show_materialized_views_lists_active` | `SHOW MATERIALIZED VIEWS` | Contains `v` |
| `matview_lag_function_returns_bigint` | `SELECT matview_lag('s.v')` | Returns a BIGINT |
| `matview_status_function_returns_varchar` | `SELECT matview_status('s.v')` | Returns one of `'active'|'stale'|'rebuilding'|'dropped'` |
| `explain_imv_returns_plan` | `EXPLAIN MATERIALIZED VIEW v` | Returns non-empty text row |
| `refresh_imv_full_sets_rebuilding` | `REFRESH INCREMENTAL MATERIALIZED VIEW v FULL` | Status transitions to `rebuilding` then `active` |

### 6.3 Extended query protocol (parameterized statements)

| Test name | Statement | Assertion |
|-----------|-----------|-----------|
| `extended_create_imv_with_params` | `CREATE INCREMENTAL MATERIALIZED VIEW ... WHERE col > $1` | Parse-bind-execute cycle completes without error |
| `extended_matview_lag_with_name_param` | `SELECT matview_lag($1)` bound to `'s.v'` | Returns BIGINT |

### 6.4 Multi-session snapshot isolation

| Test name | Assertion |
|-----------|-----------|
| `two_sessions_see_independent_snapshots` | Session A's `BEGIN` snapshot is stable while session B commits a `DROP SCHEMA`; session A still sees the schema |
| `session_after_drop_imv_sees_dropped_status` | Session started after `DROP INCREMENTAL MATERIALIZED VIEW` sees `matview_status = 'dropped'` |

---

## 7. Tier 4 — Object Store Integration Tests (MinIO)

**Location:** `crates/slateduck-testkit/tests/minio_tests.rs` and `crates/slateduck-catalog/tests/minio_catalog_tests.rs`
**Backend:** MinIO via Testcontainers
**Harness:** `MinioHarness` + `CatalogHarness::on_minio`
**Trigger:** Every merge to `main`

These tests are gated behind `#[cfg(feature = "minio-tests")]` so they do not run on `cargo test` without explicitly enabling the feature. CI enables the feature on the large-runner job.

### 7.1 MinIO harness bootstrap

```rust
static MINIO: OnceLock<MinioHarness> = OnceLock::new();

async fn minio() -> &'static MinioHarness {
    MINIO.get_or_init(|| async {
        MinioHarness::start("slateduck-test").await
    })
    .await
}
```

MinIO starts once per test binary invocation. Individual test functions share the same container but use distinct object-store key prefixes.

### 7.2 Catalog correctness on MinIO

| Test name | Assertion |
|-----------|-----------|
| `catalog_open_and_initialize_on_minio` | `CatalogStore::open` against a fresh MinIO bucket succeeds; version key written |
| `catalog_reopen_preserves_state_on_minio` | Write schema, drop handle, reopen, read schema → still present |
| `create_snapshot_durable_on_minio` | `create_snapshot` with `await_durable = true` → object visible in MinIO bucket listing |
| `concurrent_initialization_convergence_on_minio` | 5 concurrent `CatalogStore::open` on same MinIO prefix → exactly one coherent init |
| `flush_visibility_barrier_on_minio` | Write → `flush()` → fresh reader sees key; measures barrier latency |
| `sequential_snapshot_ids_monotone_on_minio` | 100 sequential writes → all `snapshot_id` values strictly increasing |
| `reader_snapshot_isolation_on_minio` | Reader opened at `S_n` does not see writes committed at `S_{n+1}` |
| `large_file_registration_10k_files_on_minio` | Register 10,000 data files in a single snapshot → all readable via `list_data_files` |
| `prune_files_zone_map_on_minio` | Register files with typed column stats → `prune_files` skips non-matching files |

### 7.3 Writer failover on MinIO

| Test name | Assertion |
|-----------|-----------|
| `writer_failover_on_minio_within_slo` | Open two `CatalogStore` handles; first acquires writer epoch; kill first; second acquires writer epoch within 5 s |
| `stale_writer_epoch_returns_57P04_on_minio` | Old writer attempts `create_snapshot` after epoch stolen → `SQLSTATE 57P04` |
| `new_writer_sees_committed_state_after_takeover` | Second writer's `read_latest()` sees all rows committed by first writer |

### 7.4 Visibility-barrier latency assertion

```rust
#[tokio::test]
#[cfg(feature = "minio-tests")]
async fn flush_visibility_barrier_p99_below_1s() {
    let minio = minio().await;
    let cat = CatalogHarness::on_minio(minio, "barrier-test").await;
    let mut writer = cat.writer().await;
    
    let mut latencies = Vec::with_capacity(100);
    for _ in 0..100 {
        let key = writer.create_schema("s").await.unwrap();
        let start = Instant::now();
        writer.flush().await.unwrap();
        // New reader must see the write
        let reader = cat.reader_latest().await;
        assert!(reader.list_schemas().unwrap().iter().any(|s| s.name == "s"));
        latencies.push(start.elapsed().as_millis());
    }
    latencies.sort();
    let p99 = latencies[98];
    assert!(p99 < 1000, "flush visibility barrier p99 {}ms exceeds 1s SLO", p99);
}
```

---

## 8. Tier 5 — Client Compatibility Tests (DuckDB, Spark, Trino)

**Location:** `crates/slateduck-pgwire/tests/compat_tests.rs`
**Backend:** `LocalFileSystem` for wire corpus replay; `PgWireHarness` for live tests
**Trigger:** Every merge to `main`

### 8.1 Wire corpus replay (existing, extended)

The existing corpus replay infrastructure replays captured JSONL fixtures against a live `PgWireHarness`. Each new client version gets a corresponding `tests/fixtures/wire-corpus/{client}-{version}.jsonl` file and a corresponding replay test.

| Test name | Fixture | Assertion |
|-----------|---------|-----------|
| `duckdb_1_2_2_corpus_replay` | `wire-corpus/duckdb-1.2.2.jsonl` | All responses match expected |
| `duckdb_1_5_2_corpus_replay` | `wire-corpus/duckdb-1.5.2.jsonl` | All responses match expected |
| `spark_3_5_corpus_replay` | `wire-corpus/spark-3.5.jsonl` | All responses match expected |
| `trino_432_corpus_replay` | `wire-corpus/trino-432.jsonl` | All responses match expected |
| `pgtide_0_34_corpus_replay` | `wire-corpus/pgtide-0.34.jsonl` | All responses match expected |

### 8.2 Golden test cross-check

Golden test files under `tests/golden/duckdb-{version}/` are the spec-conformance oracle. Each `.json` file contains expected response shapes for a named DuckLake SQL operation. Tests assert:

- Response row count and column names match golden
- Column types match golden
- On DDL operations, subsequent SELECT verifies the visible catalog state

### 8.3 DuckDB live E2E test (Testcontainers)

A live E2E test spawns an actual DuckDB process using Testcontainers (the official `duckdb/duckdb` Docker image) against a live `PgWireHarness` backed by MinIO. This is the gold-standard DuckDB compatibility test.

```rust
#[tokio::test]
#[cfg(feature = "minio-tests")]
async fn duckdb_full_ducklake_tutorial_against_minio() {
    // 1. Start MinIO
    let minio = MinioHarness::start("compat-duckdb").await;
    // 2. Open catalog on MinIO
    let cat = CatalogHarness::on_minio(&minio, "tutorial").await;
    // 3. Start PG-Wire sidecar
    let pgwire = PgWireHarness::start(cat.store.clone()).await;
    // 4. Run DuckDB SQL via tokio-postgres (simulating the ducklake extension)
    let client = pgwire.connect().await;
    // Full tutorial sequence:
    client.execute("CREATE SCHEMA analytics", &[]).await.unwrap();
    client.execute("CREATE TABLE analytics.events (id BIGINT, ts TIMESTAMP, payload VARCHAR)", &[]).await.unwrap();
    // ... full tutorial assertions including:
    //   - INSERT, SELECT, DESCRIBE TABLE
    //   - Time-travel: SELECT * FROM analytics.events AT SNAPSHOT 1
    //   - ALTER TABLE, DROP TABLE, DROP SCHEMA
    // 5. Assert golden output matches
    let rows = client.query("SELECT schema_name FROM ducklake_schema", &[]).await.unwrap();
    assert!(rows.iter().any(|r| r.get::<_, String>(0) == "analytics"));
}
```

| Test name | Scenario |
|-----------|----------|
| `duckdb_full_ducklake_tutorial_against_minio` | Complete DuckLake tutorial from DuckDB 1.x against MinIO |
| `duckdb_time_travel_at_snapshot` | `SELECT ... AT SNAPSHOT N` returns rows from that snapshot |
| `duckdb_concurrent_reads_snapshot_isolated` | Two DuckDB sessions read at different snapshots simultaneously |
| `duckdb_kill_9_writer_recovers` | `kill -9` on the writer mid-commit; DuckDB reconnects and completes |

---

## 9. Tier 6 — IVM Integration Tests

**Location:** `crates/slateduck-ivm/tests/`
**Backend:** MinIO via Testcontainers + `IvmWorkerHarness`
**Trigger:** Every merge to `main` (Tiers 6a–6b); pre-release (6c–6d)

### 9.1 Tier 6a — Single-shard correctness (v0.11)

All tests use a single shard, single base table, no joins.

| Test name | Assertion |
|-----------|-----------|
| `single_shard_append_only_group_by` | 100 rows appended; matview `SUM(amount) GROUP BY customer_id` converges within `freshness_target` |
| `single_shard_row_deletion_reflected` | Insert row, wait for view, delete row from base, wait for view; row absent from output |
| `single_shard_filter_and_project` | `SELECT col_a FROM t WHERE col_b > 5` maintained correctly under 500 appends |
| `single_shard_distinct` | `SELECT DISTINCT col FROM t` maintained; no duplicates in output ever |
| `single_shard_union_all` | View over `UNION ALL` of two tables maintained under interleaved appends to both |
| `single_shard_having` | `GROUP BY ... HAVING count(*) > 3` maintained; groups below threshold absent |
| `single_shard_worker_restart_resumes_from_checkpoint` | Kill worker mid-batch; restart; view catches up within `60s` SLO |
| `single_shard_stale_on_schema_change` | `ALTER TABLE` on base table → matview status transitions to `Stale` |
| `single_shard_time_travel_before_first_output_returns_empty` | Time-travel `AT SNAPSHOT 1` on a matview that first published at `S_5` → empty |
| `single_shard_matview_lag_bounded` | Under 1000 rows/s ingest rate, `matview_lag_ms` stays below `freshness_target + 2000` ms |
| `two_workers_single_shard_exactly_one_acquires` | Two workers boot within 100 ms; assert exactly one `Acquired`, one `Contended`; no double-ownership |

### 9.2 Tier 6b — Multi-shard scale-out (v0.12)

| Test name | Assertion |
|-----------|-----------|
| `eight_shard_group_by_throughput` | 8-shard GROUP BY view at 50k rows/s; all shards converge; union of shard outputs matches single-shard reference |
| `eight_shard_resharding_preserves_completeness` | `ALTER ... SET shard_count = 4`; re-sharding completes; union of new shards = union of old shards (exact row count and content) |
| `lease_heartbeat_extends_before_expiry` | Single worker with TTL 30s; mock clock advanced 28s; heartbeat fires; `generation` in catalog incremented |
| `lease_expiry_releases_shard_to_second_worker` | First worker stopped (no heartbeat); second worker picks up after TTL |
| `backfill_rate_1m_rows_8_shards` | 8-shard backfill of 1M pre-existing rows; completes within 120s |
| `worker_shard_limit_respected` | Worker with `--shard-limit 4`; 8 shards available; worker claims exactly 4 |
| `output_plane_consistent_across_shards` | `output_mode = consistent`; output snapshot min-frontier matches all shards' latest checkpoints |

### 9.3 Tier 6c — Joins (v0.13)

| Test name | Assertion |
|-----------|-----------|
| `broadcast_join_small_dimension_table` | View joining events (1M rows) with categories (1k rows, broadcast); output correct under appends to both sides |
| `co_partition_join_shared_shard_key` | View joining two tables partitioned on same key; local join; output correct |
| `reshuffle_join_non_collocated` | View joining two tables with different shard keys; exchange operator; output correct |
| `tpch_q1_maintained_incrementally` | TPC-H Q1 against streaming SF0.1 input; output matches DuckDB reference after each batch |
| `tpch_q3_maintained_incrementally` | TPC-H Q3 with broadcast dimension join; output matches reference |
| `tpch_q5_maintained_with_co_partition` | TPC-H Q5 with explicit `WITH (shard_key = ...)` on co-partitionable side; output matches reference |
| `explain_imv_shows_join_strategy` | `EXPLAIN MATERIALIZED VIEW v` returns correct `join_strategy` for each plan type |

### 9.4 Tier 6d — Operational hardening (v0.14)

| Test name | Assertion |
|-----------|-----------|
| `repair_shard_rebuilds_from_base` | Corrupt shard state store; `slateduck-ivm repair --shard N`; shard rebuilds; output correct |
| `refresh_full_drops_all_state_and_rebuilds` | `REFRESH INCREMENTAL MATERIALIZED VIEW v FULL`; all shards rebuild from base; output matches reference |
| `doctor_identifies_stuck_shard` | Kill one worker without releasing lease; `slateduck-ivm doctor` reports shard as `STUCK` |
| `doctor_identifies_expired_lease` | Advance clock past TTL; doctor reports `lease=expired` |
| `output_compaction_reduces_file_count` | After 100 publish cycles, `state_compaction` keeps SST count ≤ 32 |
| `exactly_once_output_under_restart` | Kill output plane mid-publish; restart; output snapshot count is correct (no duplicates) |

---

## 10. Tier 7 — Fault Injection Tests

**Location:** `crates/slateduck-ivm/tests/fault_injection_tests.rs` and `crates/slateduck-catalog/tests/fault_injection_tests.rs`
**Backend:** MinIO + `fail` crate
**Trigger:** Pre-release manual gate

The `fail` crate (`failpoints`) instruments code paths with named `fail_point!` macros. Tests activate fail points remotely via environment variables or a control endpoint.

### 7.1 Catalog fault points

| Fail point | Fault injected | Expected outcome |
|------------|---------------|------------------|
| `catalog::create_snapshot::before_commit` | Panic | Catalog reads at prior snapshot; no partial state |
| `catalog::create_snapshot::after_put_before_flush` | Return `Err(IoError)` | `create_snapshot` returns error; no snapshot committed |
| `catalog::extend_lease::cas_update` | Return `Err(CasConflict)` | `extend_lease` returns `SQLSTATE 40001` |
| `catalog::counter_cache::persist` | Panic | CounterCache reloads from SlateDB on next open; no ID reuse |

### 7.2 IVM worker fault points

| Fail point | Fault injected | Expected outcome |
|------------|---------------|------------------|
| `ivm::worker::after_dbsp_before_flush` | Kill process | State store not corrupted; worker restarts from last checkpoint |
| `ivm::worker::after_flush_before_checkpoint` | Kill process | State and checkpoint mismatch by at most one batch; worker replays correctly |
| `ivm::output::after_parquet_before_catalog_commit` | Kill process | Orphan Parquet file left; next publish is idempotent; orphan GC'd after grace period |
| `ivm::source::read_parquet` | Return `Err(S3Throttled)` | Worker retries with backoff; no data loss; no duplicate ingestion |

### 7.3 MinIO fault injection (simulated via proxy)

Use `toxiproxy` (via Testcontainers) in front of MinIO to inject network faults:

| Scenario | Toxic | Assertion |
|----------|-------|-----------|
| S3 PUT returns 503 | `latency` toxic on write port | Writer retries; eventually succeeds; catalog consistent |
| S3 GET truncated | `slice_body` toxic | Read error propagates as explicit error; no silent data corruption |
| Network partition during heartbeat | `timeout` toxic on catalog port | IVM worker detects no-heartbeat condition, releases lease locally before server-side expiry |
| Slow MinIO (p99 10s) | `latency` toxic 10s | `freshness_lag` degrades gracefully; no panic; no data loss |

---

## 11. Tier 8 — Scale & Soak Tests

**Location:** `tests/scale/` (workspace level) + `crates/slateduck-ivm/benches/`
**Backend:** Real S3 Standard (same region as EC2 runner) or MinIO on large runner
**Trigger:** Weekly scheduled + manual pre-release gate

These tests cannot run in standard GitHub Actions due to time and resource requirements. They run on a dedicated `c6i.4xlarge` EC2 instance with a self-hosted runner.

### 8.1 TPC-H benchmark suite

| Benchmark | Input | Target | Backend |
|-----------|-------|--------|---------|
| `tpch_sf10_catalog_latency` | SF10 data file registration | p99 `get_current_snapshot` < 50ms | S3 Standard |
| `tpch_sf100_catalog_latency` | SF100 data file registration | p99 < 100ms | S3 Standard |
| `tpch_q1_ivm_streaming_sf1` | 100k rows/s synthetic | lag p99 < 5s at 8 shards | MinIO |
| `tpch_q3_ivm_streaming_sf1` | Same | lag p99 < 5s | MinIO |
| `tpch_q5_ivm_streaming_sf1` | Same | lag p99 < 5s | MinIO |

Results written to `benchmarks/v{version}-tpch-{date}.json`. CI comparison job alerts if any metric regresses > 10%.

### 8.2 24-hour soak test

```
slateduck-ivm serve \
  --catalog-path s3://slateduck-scale-tests/soak-{run_id} \
  --state-prefix s3://slateduck-scale-tests/soak-state-{run_id}/ \
  --worker-id soak-0 \
  --shard-limit 8 \
  --lease-ttl-ms 30000
```

Soak test assertions (checked every 15 minutes):
- `matview_lag_ms` never exceeds `2 × freshness_target` for more than two consecutive checks
- Output Parquet row count matches DuckDB reference scan (correctness drift = 0)
- No `ivm_circuit_panic_total` increments after T+1h (allowing for initial warm-up)
- Fault injection fires every 15 minutes; worker recovers within 60s each time

### 8.3 16-shard scale-out benchmark

- 16 IVM workers on 16 separate `c6i.large` EC2 instances
- 1M rows/s synthetic input
- 8-shard GROUP BY view
- Asserts: total throughput ≥ 500k rows/s aggregate; lag p99 ≤ 3s

---

## 12. Tier 9 — Security Tests

**Location:** `crates/slateduck-pgwire/tests/security_tests.rs` and `tests/security/`
**Backend:** MinIO with ACL-enforced credentials (Testcontainers) + real AWS IAM (pre-release only)

### 9.1 Credential isolation (MinIO)

MinIO supports IAM policy simulation via its `mc admin policy` CLI. The `MinioHarness` can create two users: `catalog-role` (read/write on catalog prefix) and `data-role` (read/write on data prefix).

| Test name | Scenario | Expected outcome |
|-----------|----------|-----------------|
| `catalog_role_cannot_write_data_prefix` | `catalog-role` credentials, write to `{bucket}/data/` | `SQLSTATE 42501` |
| `data_role_cannot_write_catalog_prefix` | `data-role` credentials, write to `{bucket}/catalogs/` | `SQLSTATE 42501` |
| `catalog_role_can_read_catalog_prefix` | `catalog-role` credentials, `list_data_files()` | Returns correct list |
| `startup_credential_check_fails_with_wrong_role` | Start sidecar with data-role credentials | Process exits with error code, logs `SQLSTATE 42501` |

### 9.2 TLS tests (existing + extended)

| Test name | Scenario | Expected outcome |
|-----------|----------|-----------------|
| `tls_required_rejects_plaintext_connection` | Existing test, retained | `ErrorResponse` |
| `tls_required_accepts_valid_cert` | Existing test, retained | `AuthenticationOk` |
| `tls_expired_cert_rejected` | Server cert with `not_after` in past | Client receives TLS error |
| `tls_self_signed_ca_validation` | Server uses CA-signed cert; client validates against CA | Connection succeeds |

### 9.3 Authentication tests (existing + extended)

All existing auth tests are retained. Extended with:

| Test name | Assertion |
|-----------|-----------|
| `auth_correct_md5_password_accepted` | MD5-hashed password accepted by `SimplePasswordHandler` |
| `auth_scram_sha_256_accepted` | SCRAM-SHA-256 exchange completes correctly |
| `auth_brute_force_rate_limited` | 10 wrong passwords within 1s → 11th attempt rejected with brief delay (configurable, default 100ms) |

### 9.4 SQL injection guards

| Test name | Input | Expected outcome |
|-----------|-------|-----------------|
| `table_name_with_sql_injection_rejected_at_parse` | `CREATE TABLE "t; DROP TABLE ducklake_metadata"` | Parser returns structured error; no catalog mutation |
| `matview_sql_injection_stored_verbatim` | `CREATE INCREMENTAL MATERIALIZED VIEW v AS SELECT 1; DROP TABLE t` | Stored verbatim; DBSP compilation rejects multi-statement input |
| `view_sql_non_deterministic_function_blocked` | `CREATE INCREMENTAL MATERIALIZED VIEW v AS SELECT random()` | Returns `SQLSTATE 0A000` (feature not supported) |

---

## 13. Tier 10 — Benchmark Regression Tests

**Location:** `crates/slateduck-catalog/benches/catalog_bench.rs` (extended) + `crates/slateduck-ivm/benches/`
**Baseline:** `benchmarks/phase-2-baseline.json`
**Trigger:** Weekly scheduled CI

### 10.1 Catalog benchmark (existing, extended)

Existing benchmarks in `catalog_bench.rs` are extended to run against both `LocalFileSystem` and MinIO, recording results to JSON. New benchmarks:

| Benchmark | What is measured |
|-----------|-----------------|
| `get_current_snapshot_10k_files` | `get_current_snapshot` at 10k registered files |
| `list_data_files_100k_files` | `list_data_files()` scan at 100k files |
| `prune_files_typed_stats_1m_rows` | `prune_files` with typed column stats, 1M-row file range |
| `create_snapshot_100_additions` | `create_snapshot` committing 100 new data file rows |
| `describe_table_100_columns` | `describe_table` on a 100-column table |

### 10.2 CI regression gate

A CI job runs `cargo bench --no-run` to confirm benchmark compilation, and a weekly scheduled job runs the benchmarks against the last 3 commits, comparing each metric against the baseline JSON. If any metric regresses > 10%, the job fails and posts a comment on the relevant commit.

---

## 14. CI Matrix

### 14.1 CI job definitions

```yaml
# .github/workflows/ci.yml additions

# Tier 1-3: runs on every PR (standard runner)
unit-and-integration:
  runs-on: ubuntu-latest
  steps:
    - cargo test --all --exclude slateduck-testkit
    - cargo test -p slateduck-testkit --features local-only

# Tier 4-5: runs on every merge to main (large runner)
minio-and-compat:
  runs-on: ubuntu-latest-8-core
  services:
    docker:
      image: docker:dind
  steps:
    - cargo test --all --features minio-tests
    - cargo test -p slateduck-pgwire --test compat_tests

# Tier 6a-6b: IVM integration (large runner, post-v0.11)
ivm-integration:
  runs-on: ubuntu-latest-8-core
  if: contains(github.event.head_commit.message, 'ivm') || github.ref == 'refs/heads/main'
  steps:
    - cargo test -p slateduck-ivm --features minio-tests --test integration_tests
    - cargo test -p slateduck-ivm --features minio-tests --test sharded_tests

# Tier 7: Fault injection (large runner, pre-release)
fault-injection:
  runs-on: ubuntu-latest-8-core
  if: startsWith(github.ref, 'refs/tags/v')
  steps:
    - cargo test -p slateduck-ivm --features minio-tests,fault-injection --test fault_injection_tests
    - cargo test -p slateduck-catalog --features fault-injection --test fault_injection_tests

# Tier 9: Security (large runner, pre-release)
security:
  runs-on: ubuntu-latest
  if: startsWith(github.ref, 'refs/tags/v')
  steps:
    - cargo test -p slateduck-pgwire --features minio-tests --test security_tests
    - cargo audit
    - cargo deny check advisories bans sources

# Tier 10: Benchmark regression (weekly)
benchmark-regression:
  runs-on: ubuntu-latest-8-core
  schedule:
    - cron: '0 2 * * 1'  # Monday 02:00 UTC
  steps:
    - cargo bench -p slateduck-catalog
    - python3 scripts/check_benchmark_regression.py benchmarks/phase-2-baseline.json
```

### 14.2 Feature flags

| Feature | Enables |
|---------|---------|
| `minio-tests` | All Testcontainers-based tests (Tiers 4, 5, 6, 9) |
| `fault-injection` | `fail_point!` activations (Tier 7) |
| `local-only` | Testkit build with only `LocalFileSystem` harness, no Docker |
| `scale-tests` | Tier 8 tests (should only be enabled on EC2 runner) |

### 14.3 Tier summary

| Tier | Feature flag | Runner | Trigger | SLO (pass/fail time) |
|------|-------------|--------|---------|----------------------|
| 1–3 | (none) | Standard | Every PR | < 5 min |
| 4–5 | `minio-tests` | Large 8-vCPU | Every merge to `main` | < 15 min |
| 6a–6b | `minio-tests` | Large 8-vCPU | Every merge to `main` (IVM path) | < 30 min |
| 6c–6d | `minio-tests` | Large 8-vCPU | Pre-release | < 60 min |
| 7 | `fault-injection` | Large 8-vCPU | Pre-release | < 60 min |
| 8 | `scale-tests` | EC2 c6i.4xlarge | Weekly + manual | < 4 h |
| 9 | `minio-tests` | Large 8-vCPU | Pre-release | < 30 min |
| 10 | (none, bench only) | Large 8-vCPU | Weekly | < 20 min |

---

## 15. Test Crate Layout

Final layout after all tiers are implemented:

```
crates/
  slateduck-testkit/              ← NEW: shared test helpers
    Cargo.toml
    src/
      lib.rs
      catalog.rs                  ← CatalogHarness
      minio.rs                    ← MinioHarness (Testcontainers)
      pgwire.rs                   ← PgWireHarness
      duckdb.rs                   ← DuckDbHarness
      clock.rs                    ← DeterministicClock
      corpus.rs                   ← wire corpus helpers
      ivm.rs                      ← IvmWorkerHarness

  slateduck-core/
    tests/
      property_tests.rs           ← existing, extended

  slateduck-catalog/
    tests/
      integration_tests.rs        ← existing, extended
      v011_catalog_tests.rs       ← NEW: IVM catalog primitives
      minio_catalog_tests.rs      ← NEW: Tier 4 (MinIO)
      fault_injection_tests.rs    ← NEW: Tier 7 catalog faults
    benches/
      catalog_bench.rs            ← existing, extended with MinIO

  slateduck-pgwire/
    tests/
      integration_tests.rs        ← existing, extended
      v011_pgwire_tests.rs        ← NEW: IVM SQL surface
      compat_tests.rs             ← NEW: Tier 5 client compat
      security_tests.rs           ← NEW: Tier 9 security

  slateduck-ivm/                  ← NEW crate (v0.11)
    tests/
      integration_tests.rs        ← Tier 6a single-shard
      sharded_tests.rs            ← Tier 6b multi-shard
      join_tests.rs               ← Tier 6c joins
      hardening_tests.rs          ← Tier 6d operational
      fault_injection_tests.rs    ← Tier 7 IVM faults
    benches/
      ingest_throughput.rs

tests/                            ← workspace-level E2E
  scale/
    tpch_catalog.rs               ← Tier 8 TPC-H catalog
    tpch_ivm.rs                   ← Tier 8 TPC-H IVM
    soak.rs                       ← Tier 8 24h soak
  security/
    credential_isolation.rs       ← IAM / MinIO ACL

scripts/
  check_benchmark_regression.py  ← Tier 10 baseline comparison
```

---

## 16. Open Questions

| # | Question | Recommendation | Decision deadline |
|---|----------|----------------|-------------------|
| 1 | Testcontainers version: `0.23` is async-native; confirm MSRV compatibility | Pin to `0.23`; run on MSRV in CI | Before Tier 4 implementation |
| 2 | MinIO image: `minio/minio:RELEASE.2024-...` pinned to digest vs. rolling | Pin to digest in `minio.rs`; update intentionally | Before Tier 4 implementation |
| 3 | Toxiproxy: Testcontainers module exists (`toxiproxy`); evaluate overhead | Use for Tier 7 network fault tests only | Before Tier 7 implementation |
| 4 | DuckDB Docker image: official image size is ~400 MB; evaluate startup time | Use only in Tier 5 live E2E; skip for corpus replay | Before Tier 5 live test implementation |
| 5 | Scale tests: EC2 self-hosted runner vs. GitHub-managed ARM runner | Prefer managed runner if 8-vCPU available; fall back to self-hosted for TPC-H SF100 | Before Tier 8 implementation |
| 6 | `fail` crate vs. `fail_parallel` for fault injection | Use `fail` for single-process faults; evaluate `fail_parallel` for multi-process IVM worker faults | Before Tier 7 IVM fault tests |
| 7 | `SCRAM-SHA-256` implementation: `tokio-postgres` supports it natively; does `pgwire` crate expose it? | Audit `pgwire` 0.28 API; implement if exposed | Before Tier 9 auth extension |
| 8 | `slateduck migrate-from-ducklake` integration test: requires PostgreSQL source; use Testcontainers `postgres` image | Use `testcontainers-modules::postgres` | Before v1.0 migration test (Tier 5) |

---

*End of integration & E2E test plan.*
