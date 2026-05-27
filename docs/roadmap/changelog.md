# Changelog

All notable changes to Rocklake are documented in this file. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Each release is listed with the version number, release date, and categorized changes. Categories follow Keep a Changelog conventions:

- **Added** — New features and capabilities
- **Changed** — Changes to existing functionality
- **Fixed** — Bug fixes
- **Deprecated** — Features marked for removal in a future release
- **Removed** — Previously deprecated features that have been deleted
- **Security** — Fixes for security vulnerabilities
- **Performance** — Measurable performance improvements

---

## [0.8.0] - 2025-01-15

This release focuses on production readiness: performance optimization, comprehensive observability, and documentation. The hot key cache and secondary index provide significant latency improvements for large catalogs. Prometheus metrics and health check endpoints make Rocklake monitorable and deployable in Kubernetes environments. The documentation site now covers all aspects of the system with 80+ pages of detailed reference material.

### Added

- **Complete documentation site** with 80+ pages covering architecture, concepts, deployment, operations, integration, design decisions, performance, internals, contributing, and reference material
- **Performance metrics collection** via Prometheus-compatible `/metrics` endpoint with 30+ metrics covering operations, storage, cache, sessions, and writer state
- **Hot key caching** for frequently-read system keys (writer epoch, latest snapshot, retain_from). Eliminates storage round-trip for the most common access pattern. Configurable via `ROCKLAKE_HOT_KEY_CACHE`
- **Secondary index support** for partition-based access patterns, reducing scan amplification for catalogs with many tables per schema
- **Encryption at rest** using AES-256-GCM for value-level encryption before writing to object storage. Keys managed via environment variable or AWS KMS
- **Partitioned writer support** for multi-dataset workloads where independent writers own disjoint keyspace partitions
- **Audit logging** for destructive operations (excision, GC advancement, catalog reset) with structured JSON output
- **DataFusion integration** via Apache DataFusion's `CatalogProvider` trait, enabling SQL queries against catalog metadata
- **Health check endpoints** (`/health/live` and `/health/ready`) for Kubernetes liveness and readiness probes
- **NDJSON export/import** for catalog backup and migration between storage backends
- **Inspect command** for human-readable catalog contents display with filtering and formatting options
- **Checkpoint command** for creating named restore points before risky operations
- **Wire corpus entries** for DuckDB 1.3.0 (13 new statement patterns)

### Changed

- **Error messages** now consistently include SQLSTATE codes, affected entity IDs, and contextual hints throughout the entire codebase
- **Write batching optimization** reduces S3 PUT count by 3–5x for bulk operations (creating tables with many columns is now a single PUT instead of N+1 PUTs)
- **Upgraded SlateDB** dependency to 0.13 for improved compaction scheduling, better manifest handling, and reduced write amplification
- **Upgraded pgwire** dependency to 0.28 for protocol compliance fixes (correct ReadyForQuery handling after errors)
- **Default retention** increased from 50 to 100 snapshots for better time-travel range out of the box
- **Log format** changed to structured fields (key=value pairs in text mode, JSON objects in json mode) for better machine parseability
- **Startup sequence** now validates catalog format version and reports clear errors for version mismatches

### Fixed

- **MVCC visibility filter** edge case for snapshot ID 0 (the initial snapshot). Previously, entities created at snapshot 0 could be invisible to readers requesting snapshot 0
- **Key encoding correctness** for maximum u64 values (u64::MAX). The XOR-based descending sort was incorrect for this boundary value
- **Session cleanup** on abrupt client disconnect. Previously, sessions could leak if the TCP connection was reset without a proper close handshake
- **Wire corpus compatibility** with DuckDB 1.5.x which requires postgres-scanner initialization queries before DuckLake metadata initialization can begin
- **Counter persistence** race condition where rapidly-created transactions could allocate duplicate IDs under extreme concurrency
- **TLS handshake** timeout handling — connections that stalled during TLS negotiation now properly time out after 30 seconds
- **Prefix scan** memory usage for very large result sets (previously buffered all results before filtering; now streams and filters incrementally)

### Performance

- **3x faster prefix scans** through SST block prefetching (configurable depth via `ROCKLAKE_PREFETCH_DEPTH`)
- **40% reduction in key encoding allocations** through stack-based buffers for keys under 128 bytes
- **2x faster value deserialization** by using prost's zero-copy decode path for byte fields
- **50% reduction in write latency** for single-row transactions through WAL segment size optimization

---

## [0.7.0] - 2024-12-01

This release adds operational tooling: garbage collection, integrity verification, and repair. These commands give operators control over catalog lifecycle management and the ability to diagnose and fix issues without manual key-value manipulation.

### Added

- **GC (garbage collection) command** with configurable retention: `rocklake gc --retain-snapshots 100` advances the retention horizon and optionally excises superseded rows
- **Excision command** for explicit physical deletion of superseded rows: `rocklake excise --dry-run` shows what would be deleted; without `--dry-run` deletes permanently
- **Verify command** for catalog integrity checking: validates key encoding, value envelope format, reference integrity, and counter monotonicity
- **Repair command** for conservative auto-repair: fixes counter inconsistencies and removes corrupt entries without operator intervention
- **Checkpoint command** for creating named restore points before risky operations
- **Wire corpus test suite** for DuckDB 1.5.x (comprehensive coverage of all emitted SQL patterns)
- **Export command** for NDJSON backup of catalog contents
- **Import command** for restoring from NDJSON backup
- **Configurable log format** (`text` or `json`) via `ROCKLAKE_LOG_FORMAT` environment variable

### Changed

- **SQL classifier accuracy** improved for edge cases in DuckDB output: quoted identifiers, multi-line statements, and unusual whitespace patterns
- **Memory usage** reduced by 40% through arena allocation for key encoding buffers (reused across operations rather than allocated per-key)
- **Error reporting** for object storage connectivity issues now includes the specific HTTP status code, endpoint URL, and retry count
- **Writer epoch** is now persisted immediately on startup (previously batched with first write), ensuring fencing takes effect even if no writes occur
- **Session timeout** reduced from 300s to 60s for idle connections (configurable via future release)

### Fixed

- **Writer fencing race condition** during rapid failover: if writer B started before writer A's final write completed, both could briefly believe they were the active writer. Now resolved by reading epoch AFTER write batch commit
- **Protobuf decode error** for columns with very long default expressions (>64KB). The value envelope's length prefix was using u16; upgraded to u32
- **Counter overflow handling** for catalogs with > 2^53 snapshots (theoretical edge case, but now handled gracefully with a clear error message)
- **DROP TABLE** not ending associated columns: when a table was dropped, its columns remained with end_snapshot = NULL. Now all child entities are ended atomically in the same write batch
- **List operations** returning superseded entries when the MVCC filter encountered exactly matching begin_snapshot == requested_snapshot values

---

## [0.6.0] - 2024-10-15

This release implements the PostgreSQL wire protocol (Strategy B), making Rocklake accessible to DuckDB over TCP without requiring a native extension. This is the primary deployment mode for production use.

### Added

- **PG-wire protocol implementation** — full simple query protocol support (Query message → response). Extended query protocol (Parse/Bind/Execute) supported for common patterns
- **SQL statement classifier** with ~50 recognized patterns covering all DuckLake operations
- **Session management** with configurable maximum connections (`ROCKLAKE_MAX_SESSIONS`)
- **TLS support** for encrypted client connections (certificate + key via environment variables)
- **Password authentication** support (cleartext password protocol, requires TLS for security)
- **Graceful shutdown** on SIGTERM (drains active sessions before stopping)
- **Read-only mode** (`ROCKLAKE_READ_ONLY=true`) for deploying read replicas

### Changed

- **Protocol migration:** Moved from custom binary protocol to PostgreSQL wire protocol. All clients now use standard PostgreSQL drivers
- **Error handling unified** with SQLSTATE codes throughout the stack (previously used custom numeric codes)
- **Connection lifecycle** now follows PostgreSQL semantics (startup message → authentication → ready for query)

### Fixed

- **Transaction state** not reset on ROLLBACK: session remained in "transaction active" state, rejecting subsequent operations
- **Large query strings** (>8KB) caused buffer overflow in the message parser

---

## [0.5.0] - 2024-08-01

The native DuckDB extension (Strategy C) — Rocklake integrated directly into the DuckDB process via C FFI.

### Added

- **Native DuckDB extension** (Strategy C) via C-compatible FFI interface
- **CatalogStore abstraction** — unified storage layer supporting local filesystem and object storage
- **CatalogReader / CatalogWriter** separation — readers are cheap (concurrent), writers are exclusive (single-writer)
- **Complete DuckLake protocol table support** — all 28 table types (catalog, snapshot, schema, table, column, data_file, delete_file, file_column_stats, table_stats, view, macro, table_macro, index, type, sequence, and system tables)
- **Property-based tests** for key encoding (proptest integration verifying sort order preservation for all key types)
- **C FFI bindings** with stable ABI versioning (ABI_VERSION constant checked on extension load)

### Changed

- **Key encoding** finalized: tag byte + big-endian u64 components with XOR-based descending sort for snapshot IDs
- **Value format** finalized: 1-byte format version + 4-byte SDKV magic + protobuf payload

---

## [0.4.0] - 2024-06-01

Initial implementation: SlateDB integration and core data model.

### Added

- **SlateDB integration** as the storage engine (object_store crate for S3/GCS/Azure access)
- **Key encoding scheme** — tag byte prefix for entity type identification, big-endian u64 for sort-correct numeric encoding
- **Value envelope format** — SDKV magic bytes for corruption detection, format version for future evolution
- **Protobuf row serialization** via prost for all catalog row types
- **MVCC visibility filter** — begin_snapshot / end_snapshot semantics with snapshot-isolated reads
- **Counter-based ID allocation** — monotonic counters for each entity type, allocated within write batches
- **Basic CRUD operations** — create/read/update/delete for schemas, tables, and columns
- **Local filesystem storage** for development (no cloud credentials required)

---

## Upgrading Between Versions

### General Upgrade Process

1. Stop the running Rocklake instance
2. Replace the binary with the new version
3. Start the new instance

Rocklake is designed for zero-downtime upgrades when possible — the new instance picks up where the old one left off. However, some version transitions may require additional steps:

### Breaking Changes by Version

| From | To | Breaking Change | Action Required |
|------|-----|----------------|----------------|
| 0.5.x | 0.6.x | Protocol changed from custom binary to PG-wire | Update client connection code |
| 0.6.x | 0.7.x | None (backward compatible) | Just upgrade |
| 0.7.x | 0.8.x | Default retention changed from 50 to 100 | Adjust if you depend on specific GC behavior |

### Verifying After Upgrade

After upgrading, verify catalog health:

```bash
rocklake verify --catalog s3://bucket/catalog/
```

This confirms the new binary can read the existing catalog format correctly.

---

## Further Reading

- **[Roadmap](index.md)** — What is planned for future releases
- **[Contributing: Release Process](../contributing/release-process.md)** — How releases are built and published
- **[Operations: Upgrades](../operations/upgrades.md)** — Detailed upgrade procedures
