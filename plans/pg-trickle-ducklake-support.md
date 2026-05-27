# pg-trickle × RockLake DuckLake Compatibility

**Purpose:** RockLake *is* the DuckLake catalog backend. pg-trickle is a PostgreSQL extension that speaks to DuckLake *through* a PostgreSQL-protocol catalog endpoint. This document defines what "100% pg-trickle compatible" means for RockLake, catalogs every gap between the current state and full compatibility, and maps each gap to a concrete engineering task.

**Related documents:**
- [plans/pg-trickle.md](pg-trickle.md) — IVM algorithm learnings
- [plans/blueprint.md](blueprint.md) — RockLake catalog architecture
- [docs/concepts/ducklake.md](../docs/concepts/ducklake.md) — DuckLake format specification

---

## 1. The Relationship at a Glance

```
┌─────────────────────────────────────────────────────────────────┐
│  PostgreSQL instance (production deployment)                    │
│                                                                 │
│  ┌──────────────────┐       ┌──────────────────────────────┐   │
│  │   pg-trickle     │  SQL  │  RockLake PG-wire sidecar   │   │
│  │  IVM extension   │──────▶│  (rocklake-pgwire)          │   │
│  └──────────────────┘       │  speaks DuckLake catalog SQL │   │
│          │   ▲              └──────────────────────────────┘   │
│          │   │                             │                    │
│          │ CDC                             │ SlateDB (S3/local) │
│          ▼   │                             ▼                    │
│  ┌──────────────────┐         ┌────────────────────────────┐   │
│  │  Source tables   │         │  DuckLake catalog (MVCC)   │   │
│  │  (PG heap / FDW) │         │  + Parquet data files (S3) │   │
│  └──────────────────┘         └────────────────────────────┘   │
│          │                                 ▲                    │
│          │  IVM results as Parquet sink     │                    │
│          └─────────────────────────────────┘                   │
└─────────────────────────────────────────────────────────────────┘
                    │                    │
            DuckDB client         Spark/Trino client
          ATTACH 'ducklake:     (reads Parquet directly
           postgresql://…'        from S3)
```

**RockLake plays two roles in this picture:**

1. **Catalog backend** — replaces the PostgreSQL or SQLite instance that stores the 28 DuckLake catalog tables. pg-trickle connects to RockLake over the PG-wire protocol to issue catalog DDL/DML (`INSERT INTO ducklake_snapshot …`, `SELECT … FROM ducklake_data_file …`).

2. **IVM sink target** — receives Parquet files from pg-trickle's differential refresh engine and registers them as first-class DuckLake data files.

"100% compatible" means both roles work flawlessly so that a pg-trickle user can swap `postgresql://rds-instance/catalog` for `postgresql://rocklake-sidecar/catalog` without changing a single line of their pg-trickle configuration.

---

## 2. What pg-trickle Does with DuckLake

### 2.1 Consuming DuckLake Tables as O(Δ) IVM Sources

**Feature:** `CdcMode::DUCKLAKE_CHANGE_FEED`

When pg-trickle detects that a stream table's source is a DuckLake FDW table, it automatically selects the `DUCKLAKE_CHANGE_FEED` CDC mode. Instead of O(N) polling (`EXCEPT ALL` full diff), it calls DuckLake's `table_changes(table, start_snapshot, end_snapshot)` function to fetch only the rows that changed between two snapshot IDs.

```sql
-- pg-trickle calls this on the DuckLake catalog:
SELECT rowid, change_type, <user_columns>
FROM table_changes('lake.raw_events', start_snapshot := 42, end_snapshot := 45);

-- Returns:
-- (rowid, 'insert', …)
-- (rowid, 'update_preimage', …)
-- (rowid, 'update_postimage', …)
-- (rowid, 'delete', …)
```

The snapshot ID advances atomically with each RockLake write commit — exactly the semantics RockLake already provides via `SnapshotDiff` in `reader.rs`.

**Performance impact:** at 10M rows with a 100-row delta, this is 7 orders of magnitude less work than full polling.

### 2.2 Writing IVM Results Back to DuckLake (the "Sink")

**Feature:** `sink => 'ducklake'` on `create_stream_table()`

After computing a differential delta, pg-trickle:
1. Serializes the delta rows as a Parquet file.
2. Uploads the file to S3 at the configured `ducklake_sink_path`.
3. Issues these SQL statements against the catalog backend (RockLake):

```sql
-- Commit the file into the DuckLake catalog atomically:
INSERT INTO ducklake_data_file (table_id, path, row_count, file_size_bytes, …)
VALUES (…);

INSERT INTO ducklake_snapshot (snapshot_id, schema_version, …)
VALUES (…);

-- Record provenance (pg-trickle internal table in the same DB):
INSERT INTO pgtrickle.pgt_ducklake_provenance
    (stream_table_name, ducklake_snapshot_id, delta_row_count, written_at)
VALUES (…);
```

All three statements are issued in one transaction against the PG-wire protocol.

### 2.3 View Registration

When a stream table is created with a DuckLake sink, pg-trickle automatically registers a DuckLake view so downstream DuckDB/Spark/Trino clients can query it as `lake.stream_table_name`:

```sql
INSERT INTO ducklake_view (schema_id, view_name, view_definition, begin_snapshot)
VALUES (1, 'revenue_by_region', 'SELECT * FROM postgres_scan(…)', <current_snapshot>);
```

### 2.4 Guaranteed Delivery via pg-tide Outbox

**Feature:** `pgtrickle.attach_outbox()` + `tide.relay_set_outbox()`

For production durability, pg-trickle uses the **transactional outbox pattern**: the differential refresh result and the outbox message are written in the **same PostgreSQL transaction**, so either both land or neither does. A separate relay process (pg-tide) then:
1. Polls the outbox with `SELECT … FOR UPDATE SKIP LOCKED`.
2. Encodes rows as Parquet and uploads to S3.
3. Issues `INSERT INTO ducklake_data_file` + `INSERT INTO ducklake_snapshot` against RockLake.
4. Marks the outbox message as delivered.

This relay connects to RockLake over the standard PG-wire protocol — the same surface area that DuckDB uses.

### 2.5 Inlined Data Table CDC

DuckLake stores small writes (below a configurable row threshold) in PostgreSQL heap tables named `ducklake_inlined_data_table_<table_id>_<schema_version>`. When this feature is active, pg-trickle attaches AFTER triggers to these heap tables for sub-millisecond CDC latency. When DuckLake flushes inlined rows to Parquet, a DDL watcher detects the `DROP TABLE ducklake_inlined_data_table_*` event and switches CDC mode.

In RockLake's case, the same functionality maps to the `0xFD` key-space inlined data (documented in `plans/blueprint.md` §9.1). The PG-wire layer exposes these as virtual tables for SELECT, but **trigger-based CDC is architecturally impossible for remote RockLake deployments**.

#### Architectural Constraint: Trigger-Based CDC Requires Local PostgreSQL DML

PostgreSQL `AFTER` triggers only fire when DML executes locally on the host PostgreSQL server. When a DuckDB or other remote client writes inlined data directly to RockLake over PG-wire, the writes bypass the host PostgreSQL entirely — they are translated into RockLake `0xFD` key-space mutations at the executor layer. The host PostgreSQL server never sees the DML, so its trigger machinery is never invoked.

This means:

- **Trigger-based CDC is only viable when RockLake is co-located with a PostgreSQL instance and DML is routed through that PostgreSQL instance.** This deployment topology is not the primary RockLake use case.
- **For standard remote RockLake deployments, pg-trickle must use `DUCKLAKE_CHANGE_FEED` polling mode** (`table_changes()` function, see Gap 1). pg-trickle detects a non-PostgreSQL catalog backend during its connection handshake and automatically selects `DUCKLAKE_CHANGE_FEED` mode rather than trigger mode.
- **Inlined-data rows are included in `table_changes()` output.** RockLake's `SnapshotDiff` computation reads the `0xFD` key space and materializes inlined rows as change records alongside Parquet-backed rows. No trigger attachment is required.

The `DUCKLAKE_CHANGE_FEED` path has higher latency than a local trigger (milliseconds instead of microseconds), but it is correct, durable, and works across any network topology. For workloads where sub-millisecond CDC latency on inlined data is required, consider a co-located PostgreSQL + DuckDB deployment that routes writes through native PostgreSQL — not a remote RockLake endpoint.

#### Tier A Integration Test Contract

The pg-trickle integration test suite (Tier A) must include a test that:

1. Connects pg-trickle to a RockLake endpoint over PG-wire.
2. Asserts that pg-trickle **does not** attempt trigger attachment (i.e., does not issue `CREATE TRIGGER` SQL over the PG-wire connection).
3. Asserts that pg-trickle **does** call `ducklake_latest_snapshot_id($1::regclass)` during connection setup (Mitigation 7 from v0.27.11 roadmap).
4. Asserts that pg-trickle successfully issues `table_changes(…)` calls and receives well-formed change records.

---

## 3. Current RockLake State — What Already Works

| pg-trickle Operation | RockLake Implementation | Status |
|----------------------|--------------------------|--------|
| `INSERT INTO ducklake_snapshot` | `writer.rs::create_snapshot()` | ✅ Done (v0.3+) |
| `INSERT INTO ducklake_data_file` | `writer.rs::register_data_file()` | ✅ Done (v0.3+) |
| `INSERT INTO ducklake_view` | `writer.rs` view support via tag `0x07` | ✅ Done (v0.3+) |
| `INSERT INTO ducklake_delete_file` | `writer.rs` delete file support | ✅ Done (v0.5+) |
| `SELECT … FROM ducklake_data_file LEFT JOIN ducklake_delete_file` | `reader.rs::list_data_files()` | ✅ Done |
| `SELECT snapshot_id … ORDER BY … DESC LIMIT 1` | `reader.rs::get_current_snapshot()` | ✅ Done |
| `UPDATE ducklake_table_stats SET record_count = record_count + ?` | Supported in bounded SQL dispatcher | ✅ Done |
| `UPDATE ducklake_{table|column|view} SET end_snapshot = ?` | MVCC end-snapshot writes | ✅ Done |
| All 28 DuckLake v1.0 catalog tables | Tags `0x01–0x1C`, all implemented | ✅ Done |
| PG-wire `BEGIN`/`COMMIT`/`ROLLBACK` | Transaction plumbing in `rocklake-pgwire` | ✅ Done |
| `SELECT current_schema()`, type OID queries (handshake) | Handled by SQL dispatcher | ✅ Done |
| pg-tide `RockLakeSink` | Documented in v0.10 roadmap | ✅ Done (v0.10) |

The DuckLake catalog-write path that pg-trickle's sink uses is **already fully supported** — that is, pg-trickle can today write its IVM outputs (Parquet file registrations, snapshot commits, view registrations) to a RockLake-backed DuckLake catalog.

---

## 4. Gaps — What Needs to Be Built

### Gap 1 — `table_changes()` SQL Function

**What pg-trickle needs:** a callable SQL function (accessible over PG-wire) that returns per-row change records between two snapshot IDs.

**Current state:** `reader.rs` has `SnapshotDiff` in Rust, but it is not exposed as a SQL function. The PG-wire SQL dispatcher (`rocklake-sql`) handles a bounded set of DuckLake catalog queries; `table_changes(…)` is not among them.

**Impact:** Without this, pg-trickle cannot use `DUCKLAKE_CHANGE_FEED` mode against RockLake. It falls back to O(N) polling, which is correct but ~10⁷× more expensive per refresh cycle.

**Required work:**
1. Expose `SnapshotDiff` as a SQL function via the bounded dispatcher: `SELECT … FROM table_changes('schema.table', start_snapshot := ?, end_snapshot := ?)`.
2. Return the DuckLake change-feed schema: `(rowid BIGINT, change_type TEXT, <user_columns>)`.
3. Ensure that `change_type` maps exactly to DuckLake's `{insert, delete, update_preimage, update_postimage}` vocabulary.
4. Handle the case where `start_snapshot` has been GC'd (return a `SQLSTATE 55000` snapshot-too-old error so pg-trickle can fall back to full refresh).

**Files:** `crates/rocklake-sql/src/`, `crates/rocklake-catalog/src/reader.rs`, `crates/rocklake-pgwire/src/`

---

### Gap 2 — Stable `rowid` Column on Catalog-Managed Tables

**What pg-trickle needs:** every DuckLake table to have a stable `rowid` virtual column that survives UPDATE, compaction, and file movement — so the `DUCKLAKE_CHANGE_FEED` insert/delete pairs can be matched across refresh cycles.

**Current state:** RockLake stores rows as Parquet files; row identity in Parquet is implicit (byte offset) and not stable across compaction or file splits.

**Impact:** Without stable `rowid`, the EC-01 phantom-row fix (see `plans/pg-trickle.md` §4) cannot function correctly when RockLake is the IVM source — pg-trickle cannot match a delete record to its original insert.

**Required work:**
1. Implement a stable `rowid` for DuckLake tables managed by RockLake. The simplest approach: include `rowid` as a hidden column in data file schemas, derived from a per-table monotone counter (already supported by RockLake's counter allocator at key `0xFE | 0x10 | table_id`).
2. Expose `rowid` in `table_changes()` output.
3. Document the `rowid` stability guarantee (survives compaction, GC, file re-registration).

**Files:** `crates/rocklake-catalog/src/writer.rs`, `crates/rocklake-ivm/src/parquet.rs`

---

### Gap 3 — Inlined Data Table PG-wire Exposure

**What pg-trickle needs:** when RockLake writes small deltas as inlined data (key `0xFD`), these rows must be queryable via the PG-wire protocol as a virtual table named `ducklake_inlined_data_table_<table_id>_<schema_version>` so that pg-trickle can attach AFTER triggers to it.

**Current state:** inlined data key space is defined in `plans/blueprint.md` §9.1, but the PG-wire SQL dispatcher does not expose it as a selectable virtual table, and trigger attachment is not supported.

**Impact:** For high-frequency small-write workloads (e.g., OLTP event streams), inlined data is the hot path. Without CDC trigger support on it, pg-trickle must poll at a slower cadence.

**Required work:**
1. Add inlined-data virtual table to the bounded SQL dispatcher: `SELECT * FROM ducklake_inlined_data_table_<table_id>_<schema_version>`.
2. Expose schema-version bump events as a `NOTIFY` on channel `pgt_inlined_flush_<table_id>` (so pg-trickle's DDL watcher can detect Parquet flush).
3. No trigger mechanism needed — pg-trickle switches to `table_changes()` mode when it detects a non-PostgreSQL catalog backend.

**Files:** `crates/rocklake-sql/src/`, `crates/rocklake-pgwire/src/`

---

### Gap 4 — Compaction Safety / Snapshot Hold Mechanism

**What pg-trickle needs:** when it tracks a frontier of `{snapshot_id: 42}` for a DuckLake source, snapshot 42 must still be readable at the next refresh cycle — even if RockLake's GC has advanced the visibility frontier past it.

**Current state:** RockLake's `gc.rs` advances the visibility frontier conservatively (safe-by-default), but there is no explicit "hold" mechanism that an external consumer can acquire. If GC advances past a snapshot that pg-trickle is using as its `start_snapshot`, `table_changes()` will return a snapshot-too-old error.

**Impact:** Without snapshot hold, long refresh intervals or GC being configured aggressively will silently break pg-trickle's incremental mode. pg-trickle can fall back to full refresh, but that defeats the purpose of O(Δ) CDC.

**Required work:**
1. Implement a **snapshot lease** mechanism: a client registers `{consumer_id, min_snapshot}` via a SQL function or catalog insert, and GC refuses to advance past that minimum.
2. Expose as SQL: `SELECT rocklake.hold_snapshot(min_snapshot_id := 42, consumer_id := 'pgtrickle:stream_1')`.
3. pg-trickle periodically renews the lease and releases it when the frontier advances.
4. Lease table stored in RockLake catalog (new key tag `0x22`) with TTL to handle ungraceful pg-trickle shutdowns.

**Files:** `crates/rocklake-catalog/src/gc.rs`, new `src/snapshot_lease.rs`

---

### Gap 5 — `pgtrickle.pgt_ducklake_provenance` Table Support

**What pg-trickle needs:** ability to `INSERT INTO pgtrickle.pgt_ducklake_provenance (…)` through the PG-wire connection, and to `SELECT` from it for monitoring. This is pg-trickle's internal bookkeeping table — it will try to create it in the catalog database at install time.

**Current state:** RockLake's bounded SQL dispatcher handles the 28 DuckLake catalog tables. User-defined tables in arbitrary schemas (`pgtrickle.*`) are outside the bounded set and return `SQLSTATE 0A000`.

**Impact:** pg-trickle's provenance writes fail at startup, preventing it from using the DuckLake sink at all.

**Required work:**
1. Implement a **user-schema extension point**: RockLake should support a restricted form of user-defined tables for external extension catalogs.
2. Minimal viable approach: recognize `CREATE TABLE IF NOT EXISTS pgtrickle.*` DDL and store these tables in a sidecar SQLite or as tagged keys in RockLake (tag `0x23` reserved for extension metadata).
3. Support `INSERT`/`SELECT`/`DELETE` on extension tables.
4. Alternative minimal approach: document that users should point pg-trickle's internal tables to a separate PostgreSQL instance while using RockLake only as the DuckLake catalog endpoint, and add a `pgtrickle.catalog_db` configuration option in the pg-trickle docs.

**Files:** `crates/rocklake-sql/src/`, `crates/rocklake-catalog/src/`

---

### Gap 6 — Mixed Frontier Support (DuckLake Snapshot ID + WAL LSN)

**What pg-trickle plans:** pg-trickle v0.47+ plans to support stream tables that read from both PostgreSQL heap tables (WAL-tracked) and DuckLake FDW tables (snapshot-tracked) within the same view definition. The frontier becomes a JSON vector clock:

```json
{
  "ducklake:lake.events":  {"snapshot_id": 42},
  "wal:postgres":          {"lsn": "0/16A4F08"}
}
```

**Current state:** RockLake's IVM workers use SlateDB sequence numbers as frontiers (`state_store.rs`). DuckLake snapshot IDs are separate. The two are not currently unified.

**Impact:** Mixed-source views will fail at planning time.

**Required work:**
1. Extend the frontier type in `state_store.rs` to carry `BTreeMap<SourceId, SourceFrontier>` where `SourceFrontier` is an enum over `{SequenceNumber(u64), DuckLakeSnapshot(i64), WalLsn(u64)}`.
2. The IVM plan (`plan.rs`) must identify each source's frontier type from its `MatviewInputSource` variant.
3. When a view is refreshed, the frontier is advanced for each source independently.

**Files:** `crates/rocklake-ivm/src/state_store.rs`, `crates/rocklake-ivm/src/plan.rs`, `crates/rocklake-ivm/src/source.rs`

---

### Gap 7 — Encryption Key Pass-Through for Parquet Sink Writes

**What pg-trickle needs:** when DuckLake per-file Parquet encryption is enabled, the `INSERT INTO ducklake_data_file` statement includes an `encryption_key` column. RockLake must store and return this column faithfully without parsing or validating the key material.

**Current state:** `ducklake_data_file` is fully implemented, but `encryption_key` column support has not been explicitly validated against pg-trickle's payload.

**Impact:** encrypted DuckLake deployments fail when pg-trickle writes sink files.

**Required work:**
1. Audit `writer.rs::register_data_file()` against the full `ducklake_data_file` column set from the DuckLake v1.0 spec.
2. Add `encryption_key BYTEA` (or `TEXT`) to the stored schema if absent.
3. Add a test fixture with an encryption-key-bearing `INSERT` from pg-trickle's exact SQL shape.

**Files:** `crates/rocklake-catalog/src/writer.rs`, `tests/fixtures/`

---

### Gap 8 — `NOTIFY` for Refresh-Triggered CDC Events

**What pg-trickle needs:** pg-trickle's event-driven scheduler listens on PostgreSQL `NOTIFY` channels. When a DuckLake table's snapshot advances (i.e., new data landed), pg-trickle wakes up immediately and triggers a refresh. The channel name follows the pattern `pgt_source_changed_<source_relid>`.

**Current state:** RockLake's PG-wire layer does not emit `NOTIFY` events on snapshot advances.

**Impact:** pg-trickle's "event-driven" mode falls back to polling (default 1 s interval), adding up to 1 s of avoidable latency.

**Required work:**
1. When RockLake commits a new snapshot (after any `INSERT INTO ducklake_snapshot`), emit `NOTIFY pgt_source_changed_<table_id>` on all connected PG-wire clients.
2. Implement `LISTEN` support in `rocklake-pgwire` so pg-trickle can subscribe.
3. Implement `UNLISTEN` and connection-close cleanup.

**Files:** `crates/rocklake-pgwire/src/`, new notification plumbing in the PG-wire connection handler

---

### Gap 9 — DuckLake v1.1+ Spec Drift

**Future risk:** the DuckLake spec is actively evolving. pg-trickle tracks the DuckLake spec closely (it contributed to the `table_changes()` API). Any DuckLake catalog DDL change that pg-trickle adopts before RockLake will cause pg-trickle to issue SQL that RockLake returns `0A000` for.

**Required process:**
1. Add pg-trickle to RockLake's DuckLake Spec Upgrade Policy (cross-cutting concern already in ROADMAP §"DuckLake Spec Upgrade Policy").
2. Add a CI job that runs pg-trickle's DuckLake integration test suite against RockLake's PG-wire sidecar (see §6 below).
3. Track pg-trickle's `CHANGELOG.md` in the RockLake dependency review process.

---

## 5. Gap Severity Matrix

| Gap | Severity | Without Fix | Effort |
|-----|----------|-------------|--------|
| 1 — `table_changes()` SQL function | 🔴 Critical | O(N) polling only; correct but expensive | Medium (3–5 days) |
| 2 — Stable `rowid` | 🔴 Critical | EC-01 phantom-row bugs in RockLake-sourced views | Medium (3–5 days) |
| 3 — Inlined data PG-wire exposure | 🟠 Important | No sub-ms CDC for small writes; polling fallback | Medium (2–4 days) |
| 4 — Snapshot hold / lease | 🟠 Important | Aggressive GC breaks incremental mode silently | Medium (2–4 days) |
| 5 — `pgtrickle.*` extension tables | 🟠 Important | pg-trickle sink fails at install time | Small–Medium (1–3 days) |
| 6 — Mixed frontier (snapshot + LSN) | 🟡 Moderate | Mixed-source views fail to plan | Large (1 week) |
| 7 — Encryption key pass-through | 🟡 Moderate | Encrypted DuckLake deployments fail | Small (0.5 days) |
| 8 — `NOTIFY` on snapshot advance | 🟡 Moderate | 1 s polling latency instead of event-driven | Medium (2–3 days) |
| 9 — Spec drift | 🔵 Process | Future breakage; caught by CI gate | Ongoing |

---

## 6. Proposed Testing Strategy: pg-trickle Compatibility Suite

### Tier A — Catalog Write Compatibility (Unit / Integration)

Run pg-trickle's internal DuckLake catalog fixture SQL against RockLake PG-wire sidecar:

1. Parse pg-trickle's SQL test corpus (`tests/` directory in pg-trickle1) for all DuckLake catalog statements.
2. Replay them against RockLake and assert no `0A000` errors.
3. Assert final catalog state (snapshot IDs, file registrations) matches the expected outcome.

**Location:** `tests/compat/pgtrickle_catalog_compat_tests.rs`

### Tier B — `table_changes()` Correctness

Property-based test:
1. Insert / update / delete N rows into a RockLake-managed DuckLake table.
2. Call `table_changes(start_snapshot, end_snapshot)` via PG-wire.
3. Assert that applying the change records to `start_snapshot` produces the `end_snapshot` table state exactly (multiset equality).
4. Include cases: pure insert, pure delete, update (preimage + postimage), compaction in between.

**Location:** `tests/compat/table_changes_property_tests.rs`

### Tier C — End-to-End pg-trickle Pipeline

Run an actual pg-trickle instance against a RockLake PG-wire sidecar (via Testcontainers):

1. Connect pg-trickle to PostgreSQL with source tables.
2. Configure pg-trickle with `sink => 'ducklake'` pointing at RockLake.
3. Run DML on source tables.
4. Assert that RockLake's DuckLake catalog reflects the correct Parquet files.
5. Connect DuckDB to RockLake and query the registered view.

**Location:** `tests/compat/e2e_pgtrickle_pipeline_tests.rs` — requires pg-trickle Docker image

### Tier D — Snapshot Hold Under GC

1. Advance RockLake GC aggressively (override `retain_from` to minimal).
2. Register a snapshot lease for `snapshot_id = current − 5`.
3. Assert GC does not advance past the leased snapshot.
4. Release lease; assert GC can now advance.

**Location:** `tests/compat/snapshot_lease_tests.rs`

---

## 7. Architecture Note: RockLake vs. PostgreSQL as Catalog Backends

pg-trickle was designed with PostgreSQL as the assumed catalog backend. RockLake mimics PostgreSQL's wire protocol precisely enough that pg-trickle sees no difference — this is the key insight that makes all of the above achievable without patching pg-trickle.

The critical compatibility surface is the **SQL query shape**, not the underlying storage:

| Layer | PostgreSQL | RockLake |
|-------|-----------|-----------|
| Wire protocol | PG protocol v3 | PG protocol v3 ✅ |
| DuckLake catalog DDL | Standard PG tables | SlateDB key-value with PG-wire façade ✅ |
| `table_changes()` | DuckLake extension function | To be added to SQL dispatcher (Gap 1) |
| `NOTIFY` | Native PG async notify | To be added to PG-wire handler (Gap 8) |
| Extension tables (`pgtrickle.*`) | PG heap tables | To be handled (Gap 5) |
| Inlined data tables | PG heap tables | Virtual tables via SQL dispatcher (Gap 3) |
| Transaction isolation | MVCC snapshots | SlateDB read-at-sequence ✅ |
| Snapshot hold for readers | Not needed (PG never GCs | Snapshot lease mechanism (Gap 4) |

---

## 8. Summary: The Path to 100% Compatibility

Resolving Gaps 1–5 delivers **functional parity** — pg-trickle can be pointed at RockLake and all documented features work correctly. Resolving Gaps 6–8 adds **production-grade performance and robustness**. Gap 9 is ongoing maintenance.

The four highest-value items to land first:

1. **`table_changes()` SQL function** — unlocks O(Δ) CDC for all RockLake-managed DuckLake tables.
2. **Stable `rowid`** — required for EC-01 correctness in pg-trickle's join delta.
3. **Snapshot lease / hold** — prevents silent fallback from O(Δ) to O(N) under GC.
4. **pg-trickle compatibility test suite (Tier A + B)** — makes the above verifiable and prevents regression.

These four items are proposed as **v0.16 — pg-trickle Compatibility** in the RockLake roadmap.
