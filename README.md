# SlateDuck

**A DuckLake catalog on SlateDB — your entire lakehouse in a single S3 bucket, no database server required.**

[![CI](https://github.com/geir-gronmo/slateduck/actions/workflows/ci.yml/badge.svg)](https://github.com/geir-gronmo/slateduck/actions)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange)](https://www.rust-lang.org)

---

## What Is SlateDuck?

Modern data teams are drowning in infrastructure. You want a lakehouse — fast analytical queries over Parquet files in object storage — but every existing solution demands a running database server to hold the catalog: a managed PostgreSQL, a Hive Metastore, a Glue catalog, something that needs to be provisioned, patched, monitored, scaled, and backed up. SlateDuck makes that server disappear.

SlateDuck is a production-ready catalog backend for [DuckLake](https://ducklake.select/), the elegant open-source lakehouse format from the DuckDB team. Instead of routing catalog metadata through an external database, SlateDuck stores it directly in [SlateDB](https://slatedb.io/) — a battle-hardened, LSM-based embedded key-value store that runs entirely inside object storage. The result is a lakehouse where **both your Parquet data files and your catalog live in the same S3 bucket**, connected to DuckDB over the standard PostgreSQL wire protocol, requiring absolutely no servers beyond a lightweight stateless sidecar. Point at a bucket, start the sidecar, and you are querying within seconds.

---

## Why SlateDuck?

### Truly Serverless

There is no "catalog database" to operate. The entire durable state of your catalog — schemas, tables, columns, snapshots, data-file references — is stored as SlateDB key-value pairs that live as ordinary objects in S3 (or GCS, Azure Blob Storage, or even the local filesystem for development). The SlateDuck sidecar is a small, stateless Rust binary that you can run as a Lambda function, a container, or a sidecar pod. If it crashes, you restart it; no recovery ceremony, no WAL replays, no replica promotion. The ground truth is always in object storage.

### Immutable Catalog History

SlateDuck makes a binding architectural promise: **committed catalog facts are never physically deleted by normal operation.** Every schema change, every table creation, every file addition is recorded as a versioned fact with a `begin_snapshot` and an optional `end_snapshot`. The default `slateduck gc` command only advances the query-visibility floor — it never deletes bytes. Physical deletion is reserved for the explicit, audited `slateduck excise` command, which exists for compliance erasure and is designed to be rare. This immutability is not a safety blanket bolted on afterward; it is the load-bearing foundation that enables two capabilities that matter deeply at scale.

### Time Travel That Actually Works

Because every row ever written to the catalog is preserved, time travel is not a special mode — it is the natural way the storage engine works. You can read the complete, consistent state of your catalog at any historical `dl_snapshot_id` with no extra overhead, no snapshot tables, no log-file archaeology. Whether you want to audit what your schema looked like last Tuesday or reproduce the exact table state from which a quarterly report was generated six months ago, you do it with a single snapshot ID and a `SELECT`.

### Horizontal Read Scale-Out

The immutability guarantee has a second payoff: because catalog-data keys are stable once written (the only permitted mutation is a terminal `end_snapshot` mark, which cannot alter a reader's view at the key's own snapshot), an **unbounded number of stateless reader replicas** can serve queries at any historical snapshot without coordinating with the writer or with each other. The catalog is a content-addressable log plus derived indexes. Replicas are pure caches. As your read workload grows, you add readers, not cluster nodes.

---

## Architecture at a Glance

```
┌─────────────────────────────────────────────────────────────┐
│                        DuckDB Client                        │
│                   (ducklake extension)                      │
└─────────────────────┬───────────────────────────────────────┘
                      │ PostgreSQL Wire Protocol
                      ▼
┌─────────────────────────────────────────────────────────────┐
│                   slateduck-pgwire                           │
│              (PG wire protocol sidecar)                     │
├─────────────────────────────────────────────────────────────┤
│                    slateduck-sql                             │
│           (Bounded SQL AST dispatcher)                      │
├─────────────────────────────────────────────────────────────┤
│                  slateduck-catalog                           │
│          (DuckLake catalog operations + MVCC)               │
├─────────────────────────────────────────────────────────────┤
│                   slateduck-core                             │
│       (Key layout, encoding, SlateDB integration)           │
└─────────────────────┬───────────────────────────────────────┘
                      │ SlateDB
                      ▼
┌─────────────────────────────────────────────────────────────┐
│              Object Storage (S3 / GCS / Azure / Local)      │
│                                                             │
│   catalogs/warehouse-a/     data/warehouse-a/               │
│   (SlateDB WAL + SSTs)      (Parquet files)                 │
└─────────────────────────────────────────────────────────────┘
```

DuckDB speaks to SlateDuck using the PostgreSQL wire protocol — the same protocol it already uses for PostgreSQL-backed DuckLake. No changes to DuckDB, no patched extensions, no custom drivers. From DuckDB's perspective it is just talking to a very fast, very opinionated PostgreSQL server that happens to store everything in a bucket.

The sidecar's SQL layer (`slateduck-sql`) is deliberately bounded: it implements exactly the finite set of SQL shapes that DuckDB's `ducklake` extension emits, and nothing more. This makes the parser surface area small, the conformance test suite exhaustive, and the security profile tight.

---

## Getting Started

### Prerequisites

- Rust stable toolchain ([rustup.rs](https://rustup.rs))
- An S3-compatible object store (or just a local directory to start)

### Build

```bash
git clone https://github.com/geir-gronmo/slateduck
cd slateduck
cargo build --release
```

### Run the Sidecar

```bash
# Start with a local catalog directory
./target/release/slateduck serve --catalog /path/to/catalog

# Bind to a specific address
./target/release/slateduck serve --catalog /path/to/catalog --bind 0.0.0.0:5432

# Limit concurrent sessions
./target/release/slateduck serve --catalog /path/to/catalog --max-sessions 16
```

### Connect with DuckDB

```sql
-- Install and load the ducklake extension
INSTALL ducklake;
LOAD ducklake;

-- Attach SlateDuck as your catalog backend
ATTACH 'ducklake:postgres:host=localhost port=5432 dbname=slateduck' AS my_lake;
USE my_lake;

-- You're off to the races
CREATE TABLE events (id BIGINT, name VARCHAR, ts TIMESTAMP);
INSERT INTO events VALUES (1, 'launch', NOW());
SELECT * FROM events;
```

---

## Workspace Layout

SlateDuck is a Cargo workspace of focused crates, each with a clear responsibility boundary:

| Crate | Purpose |
|---|---|
| `slateduck-core` | Foundational types: binary key layout, MVCC visibility logic, protobuf encoding, counter allocation |
| `slateduck-catalog` | All 28 DuckLake v1.0 catalog operations: schemas, tables, columns, snapshots, data files |
| `slateduck-sql` | Bounded SQL parser and AST dispatcher — only the shapes DuckDB actually emits |
| `slateduck-pgwire` | PostgreSQL wire protocol sidecar binary (startup, simple query, extended query) |
| `slateduck-sqlite-vfs` | SQLite VFS layer (planned: Strategy C embedded extension path) |
| `slateduck-ffi` | C/C++ FFI bindings (planned: native DuckDB extension) |

---

## Release Status

The core catalog engine, MVCC layer, and PostgreSQL wire sidecar are complete and passing end-to-end tests against DuckDB's `ducklake` extension. Active development is on the production-hardening and native-extension path; multi-client support, performance optimization, and multi-writer partitioning are staged in v0.6 and v0.7 before the v1.0 GA release.

| Release | Milestone | Status |
|---|---|---|
| **v0.1 — Foundation** | Validated infrastructure, data model, Rust workspace | Done |
| **v0.2 — Catalog Core** | All 28 DuckLake tables, full MVCC, immutability guarantees, Rust API | Done |
| **v0.3 — PG-Wire Sidecar (Alpha)** | Strategy B sidecar, DuckDB end-to-end | Done |
| **v0.4 — Production Hardening** | Visibility GC, excision, backups, observability, encryption, repair tooling | In Progress |
| **v0.5 — Native Extension (Beta)** | Embedded DuckDB extension via FFI, no sidecar required | Planned |
| **v0.6 — Multi-Client & Security** | pg-tide-relay onboarding, TLS/auth, audit log, GCS/Azure validation | Planned |
| **v0.7 — Performance & Ecosystem** | Hot-key reads, secondary indexes, multi-writer partitioning, DataFusion integration | Planned |
| **v1.0 — General Availability** | TPC-H @ SF10 benchmarks, GA polish, full operational story | Planned |

---

## Design Principles

SlateDuck is an opinionated piece of software. It makes strong bets and does not try to be all things to all people. These bets are worth stating plainly:

**One writer, many readers.** SlateDB enforces a single active writer per catalog through writer-epoch fencing. This is a deliberate constraint, not a limitation waiting to be lifted. It eliminates an entire class of catalog corruption bugs that plague systems designed around optimistic concurrent writes.

**Object storage as the only durable medium.** There is no local disk requirement beyond ephemeral caching. Your catalog is as durable and available as your object store. If you trust S3 with your data, you can trust SlateDuck with your catalog.

**Bounded SQL dispatch, not a general query engine.** The PG wire sidecar is not trying to be PostgreSQL. It speaks enough PostgreSQL to satisfy DuckDB's `ducklake` extension, verified against an exhaustive wire corpus captured from real DuckDB sessions. The vocabulary is finite, the conformance tests are thorough, and the failure mode for an unexpected query shape is a clean error rather than a silent wrong answer.

**GC and excision are separate operations.** Garbage collection in SlateDuck only advances the query-visibility floor. It does not delete catalog bytes. Physical deletion is a separate, audited, rare operation reserved for compliance and retention enforcement. This separation makes the common case (GC) safe to automate and the exceptional case (excision) easy to audit.

---

## Testing

```bash
# Run all tests
cargo test --all

# Run the catalog integration tests specifically
cargo test -p slateduck-catalog

# Run benchmarks
cargo bench -p slateduck-catalog
```

The test suite includes unit tests, property-based tests (via `proptest`), integration tests against real SlateDB instances, and golden-file conformance tests derived from actual DuckDB wire captures.

---

## Contributing

Contributions are welcome. Please read [CONTRIBUTING.md](CONTRIBUTING.md) for the development setup, code style requirements, and pull request process. The short version: `cargo fmt --all`, `cargo clippy --all-targets`, tests for new functionality, and a clear PR description.

---

## License

Apache License 2.0 — see [LICENSE](LICENSE) for details.
