# Changelog

All notable changes to RockLake are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.45.0] — 2026-05-30

### Added

- **30-Day Dogfood Deployment Report**: Successful 30-day production deployment with zero unresolved P0/P1 findings.
- **External Developer Deployment Verification**: Third-party deployment validation using only published documentation; zero blocker issues.
- **Release Automation Workflow**: `release.yml` GitHub Actions workflow for automated multi-platform binary builds, checksums, and multi-package publishing (crates.io, PyPI, npm, Maven Central).
- **Checkpoint Restore Recovery Validation**: Post-recovery snapshot linearity verification to ensure time-travel consistency after unexpected shutdown.

### Fixed

- **Checkpoint Restore Snapshot Linearity**: Ensure checkpoint recovery re-validates snapshot ordering to maintain MVCC guarantees (fixes inconsistent time-travel reads post-recovery).

### Changed

- **CLI Output Verbosity**: Reduced default verbosity on snapshot commit operations; added `--verbose` flag for detailed debugging output.
- **Error Message Clarity**: Improved time-travel query error messages to reference user-friendly snapshot names instead of internal snapshot IDs.

### Documentation

- ✅ All mkdocs strict-mode warnings eliminated (zero warnings/errors on `mkdocs build --strict`)
- ✅ All documentation stubs removed (no TODO/Coming Soon/placeholder content)
- ✅ All code examples verified to be tested (doctest or integration test coverage)
- ✅ GitHub Pages deployment ready
- Complete operational guides, capacity planning, and disaster recovery procedures
- Environment variable mapping table for configuration documentation

### Infrastructure & Release

- Added Python bindings (PyO3) distribution to PyPI wheels
- Added Go module tag support for go.mod imports
- Added Node.js package distribution to npm
- Added Java/Kotlin Maven artifact distribution to Maven Central
- Multi-platform release artifacts (Linux x86-64/aarch64, macOS arm64, Windows x86-64)

## [0.44.0] — 2026-05-23

### Added

- **JVM Bindings (Java/Kotlin via JNI)**: Complete Java and Kotlin binding layer wrapping stable `rocklake.h` C ABI.
  - `RockLakeCatalog` high-level Java API for catalog operations.
  - `RockLakeNative` JNI method stubs with auto-detected OS/arch library loading.
  - `RockLakeCatalogAsync` Kotlin-idiomatic coroutine wrapper for async operations.
  - Immutable data transfer objects: `DataFileRow`, `ColumnRow`, `RockLakeException`.
  - Apache Spark 3.5 integration example (`SparkCatalogReader`).
  - Apache Flink streaming source stub (`FlinkCatalogSource`) for snapshot-diff-driven ingestion.

- **Maven Publication**: Publish JVM bindings as Maven artifact `io.trickle:rocklake-java:0.44.0` to GitHub Packages; native libraries embedded as JAR resources for Linux x86-64/aarch64, macOS arm64, Windows x86-64.

- **Documentation**: Comprehensive JVM bindings guide with installation, Java API reference, Kotlin async patterns, Spark/Flink integration examples, and configuration reference.

### Changed

- Updated mkdocs.yml with JVM bindings documentation link.
- Fixed external documentation links to GitHub URLs for Go, Node.js, and Python bindings.
- Corrected C API reference link in JVM bindings documentation (c-api.md vs. c-abi.md).

## [0.43.0] — 2026-05-16

### Added

- **Checkpoint Pin/Unpin/List API**: `rocklake-catalog` checkpoint lifecycle management with monotonic pin IDs.
- **SoakHarness Infrastructure**: Reusable test harness in `rocklake-testkit` for long-running durability tests.
- **Lambda Reader Pattern**: Example `rocklake-client` Lambda reader with CDN cache contract validation.
- **24-Hour Scale & Soak Tests**: Tier 7 integration test suite validating 24h sustained write load, reader scale-out to 16 pods, TPC-H SF10 workload on EC2.

### Documentation

- Lambda reader pattern documentation with CDN caching contract.
- Scale and soak testing methodology guide.
- Checkpoint lifecycle management reference.

## [0.42.0] — 2026-05-09

### Added

- **TPC-H Catalog Benchmark Suite**: Standardized benchmark for catalog operation performance across versions.
- **S3 Express Optimization**: Fast path for S3 Express One Zone tier with latency and cost optimizations.
- **Cost-Per-Operation Tooling**: Utility for estimating operational cost per catalog operation, object store tier, and workload profile.
- **Benchmark Regression CI**: Tier 9 & 10 CI gates comparing benchmark results against baseline thresholds.

### Changed

- Tuned SlateDB compaction and memtable settings for improved write throughput.

## [0.41.0] — 2026-04-18

### Added

- **Migration Tooling**: `rocklake migrate-from-ducklake` command for PostgreSQL and SQLite-backed DuckLake source migration.
- **MVCC-Correct Export/Import**: Fixed export to write secondary data-file index; apply MVCC predicate in export; make rebuild atomic.
- **DuckLake v1.1 Forward-Compatibility Gate**: Validation suite ensuring RockLake catalog schema remains compatible with future DuckLake versions.

### Documentation

- Migration guide for moving from PostgreSQL-backed DuckLake to RockLake.
- SQLite to S3 migration procedures.
- Forward-compatibility validation methodology.

## [0.40.0] — 2026-04-04

### Added

- **Tier 6 Fault Injection Suite**: `fail` crate integration, toxiproxy network chaos, kill-9 recovery tests.
- **Tier 8 Security Testing**: IAM credential isolation validation, SQL injection guards, TLS audit suite.

### Fixed

- IAM credential constant-time comparison to prevent timing attacks.
- TLS version gating (1.2/1.3 only) with panic safety guards.

### Documentation

- Fault injection and security testing methodology.

## [0.39.0] — 2026-03-28

### Added

- **Prometheus `/metrics` Endpoint**: Pull-based metrics export for write latency, reader count, snapshot age, GC lease status.
- **OpenTelemetry Tracing**: Structured tracing for all catalog operations with trace context propagation.
- **`rocklake diagnose` CLI**: Comprehensive diagnostics command capturing system state, configuration, recent errors, and recommendations.
- **Orphan File Sweep**: Configurable grace-period-based orphan file cleanup with auditable log of swept files.

### Documentation

- Observability guide with Prometheus/Grafana dashboard examples.
- Troubleshooting guide with diagnostic procedures.
- Operational runbooks for common tasks.

## [0.38.0] — 2026-03-14

### Added

- **Compatibility Manifest System**: TOML-based schema declaring supported dependency versions, cloud providers, and client libraries.
- **MSRV Reconciliation**: Standardized Rust MSRV (1.93) across all crates with CI enforcement.
- **Windows x86-64 CI & Release Artifacts**: Native Windows builds in GitHub Actions with release artifacts (exe, DLL).
- **Release Gates**: Automated checks ensuring compatibility manifest is up-to-date and all CI gates pass before tagging.

### Changed

- Updated CI matrix to include Windows x86-64 test target.
- Standardized all crates to MSRV 1.93 (minimum supported Rust version).

## [0.37.0] — 2026-02-28

### Added

- **Spark 3.5 Integration**: Real Spark SQL connector for RockLake catalogs with partition pushdown and predicate pushdown.
- **Trino 432+ Integration**: Trino connector with federated query support over RockLake catalog.
- **DataFusion Matrix**: Integration tests for DataFusion 45+ with auto-resolved data root and error propagation.
- **Wire-Corpus Replay Golden Tests**: Deterministic replay of DuckDB wire-protocol corpus against RockLake with golden result validation.

### Documentation

- Spark connector installation and usage guide.
- Trino connector setup and tuning.
- DataFusion integration examples.
- Wire protocol compatibility reference.

## [0.36.0] — 2026-02-14

### Added

- **SQL Client Compatibility Suite**: Real-client smoke tests (psql, pgcli, DBeaver, Metabase) against RockLake PG-wire interface.
- **GCS & Azure Emulator Harnesses**: Containerized emulators for Google Cloud Storage and Azure Blob Storage with CRUD/snapshot/epoch fencing tests.
- **Multi-Backend Compat Suite**: Unified test framework for validating catalog operations across LocalFS, MinIO, GCS, and Azure.
- **TLS 1.2/1.3 Protocol Gating**: Version-specific TLS tests ensuring only TLS 1.2 and 1.3 are negotiated.

### Documentation

- Cloud storage setup guides (GCS, Azure).
- SQL client compatibility matrix.
- TLS configuration reference.

## [0.35.0] — 2026-01-31

### Added

- **Embedded Catalog Client Library (`rocklake-client`)**: High-level Rust API for programmatic catalog access without PG-wire dependency.
- **Python Bindings (PyO3)**: Python package wrapping `rocklake-client` for NumPy/Pandas integration and simple catalog queries.
- **Go Bindings (cgo)**: Go package wrapping stable C ABI for Go applications and CLI tools.
- **Node.js Bindings (napi-rs)**: JavaScript/TypeScript package for Node.js and Electron applications.
- **Multi-Language Integration Tests**: Polars, DataFusion, Spark, Trino compatibility validation from all language bindings.

### Documentation

- Embedded client library API reference.
- Language binding installation and usage guides.
- Cross-language integration examples.

## [0.34.0] — 2026-01-17

### Added

- **C/C++ ABI Smoke Test**: Basic C and C++ client linking against `rocklake.h` header to validate FFI safety.
- **CI Test Concurrency Configuration**: Tunable parallelism for Cargo test matrix to balance CI latency and resource usage.
- **Checkpoint/Excision Monotonic IDs**: Sequence counters for checkpoint and excision operations to enable deterministic testing.
- **CLI Docs-Conformance Test**: Automated validation that CLI help output matches reference documentation.

### Fixed

- Checkpoint counter advancement race condition in multi-writer scenarios.
- FFI NUL-string silent truncation vulnerability (now rejects over-length identifiers).

### Documentation

- C header ownership and safety guidelines.
- C++ extension stub disclaimer.
- Checkpoint and excision operation reference.

## [0.33.0] — 2025-12-20

### Added

- **Parameter-Error Redaction**: Raw values redacted from error messages to prevent credential leakage.
- **Identifier Length Validation**: Reject over-length identifiers (> 64 bytes) in key encoding to prevent buffer overflows.
- **Read-Only Query Classification**: All `rocklake_catalog.*` mutations classified as read-only (SQLSTATE 25006) to guide client behavior.

### Fixed

- FFI NUL-string silent truncation (now explicit error on over-length strings).
- Auth-without-TLS warning not shown in all code paths (now consistently warned).

## [0.32.0] — 2025-12-06

### Added

- **Export-Catalog Completeness**: Export now covers all 28+ DuckLake metadata tables with spec-compliant serialization.
- **32-vs-28 Table Count Reconciliation**: Documentation clarifying why RockLake exposes 32 tables (including 4 internal system tables) while DuckLake spec defines 28.

### Changed

- Backup/restore documentation updated to reference correct table counts.
- CLI docs clarified with explicit table schemas.

## [0.31.0] — 2025-11-22

### Added

- **DataFusion AsyncBridge Error Propagation**: Replace `unwrap_or_default()` with explicit error handling in async bridge.
- **Data File Root Validation**: Error if data files have no readable root instead of silently defaulting.
- **Explicit Data Root Carriage**: Pass data root explicitly through DataFusion scan context instead of implicit lookups.
- **Expanded Type Mapping**: Support for geometry, UUID, and variant column types in DataFusion.

## [0.30.0] — 2025-11-08

### Fixed

- **Binary COPY Parser Fail-Closed**: Parser now fails on truncated binary COPY messages instead of silently accepting partial data.
- **CLI Flag Documentation Sync**: All CLI flags now documented and verified to match internal implementations.
- **Migration Docs**: Step-by-step migration procedures for PostgreSQL and SQLite-backed DuckLake.
- **Object Store Listing Error Propagation**: `rebuild` command now propagates object store listing errors instead of silently failing.

## [0.29.0] — 2025-10-25

### Added

- **Atomic Rebuild Operation**: Make catalog rebuild transactional with all-or-nothing semantics.
- **Export/Import Round-Trip Tests**: End-to-end tests verifying export-import cycle with `list_data_files()` secondary index and reader scans.

### Fixed

- Import now writes secondary data-file index for fast lookups.
- Export now applies MVCC predicate to ensure consistent historical snapshots.

## [0.28.0] — 2025-10-11

### Added

- **Transactional Monotonic Counter**: Replace wall-clock millisecond writer epochs with transactional monotonic counter for deterministic ordering.
- **GC Lease/Pin Check Refactor**: Move lease/pin checks outside transaction to prevent stale validations.
- **Deterministic Clock Injection**: Test harness for injecting controlled clock behavior in writer-fencing tests.
- **Atomic Rebuild**: Make rebuild operation transactional.

### Fixed

- Writer fencing no longer relies on wall-clock timestamps; uses transactional monotonic counter instead.

## [0.27.14] — 2025-09-27

### Added

- **Constant-Time Auth**: Authentication comparison using constant-time functions to prevent timing attacks.
- **SCRAM-SHA-256 Support**: Full SCRAM-SHA-256 authentication protocol implementation.
- **TLS Version Gating**: Enforce TLS 1.2+ (reject older versions).
- **Atomic Metadata Commits**: Group all statements in one logical DuckLake commit into atomic batch.
- **Consolidated Stats Deltas**: Efficient column stats delta merging for writes.
- **Repeatable-Read Writer Fencing**: SQLSTATE 40001 for stale writer epochs.

## [0.27.13] — 2025-09-13

### Added

- **Multi-Driver Compat Suite**: Binary format and client schema discovery validation for real drivers (psql, pgcli, Python psycopg, Go pq, Node.js pg).
- **Visibility Constraint Enforcement**: Readers respect begin_snapshot/end_snapshot bounds in all query contexts.
- **Data File Sorting**: Data files sorted by file_order in all catalog responses.
- **DuckLake CDC Contract Reference**: Archived planning docs as generic CDC contract reference for third-party implementations.

## [0.27.12] — 2025-08-30

### Added

- **Containerized Object Store Emulators**: Docker-based GCS and Azure emulators with spec-compliant API.
- **Multi-Backend Compat Suite**: Unified test suite validating catalog CRUD, snapshot commit, and epoch fencing across LocalFS, MinIO, GCS, and Azure.
- **Data & Delete File Spec Fields**: Persist footer_size, partition_id, and encryption_key in data-file and delete-file metadata.

## [0.27.11] — 2025-08-16

### Added

- **DataFusion Virtual Catalog**: Virtual implementation of DuckDB DuckLake catalog over RockLake metadata.
- **AST Visitor Framework**: Generic SQL AST visitor for query classification and optimization.
- **Settings Registry**: Centralized management of all DuckLake PG-wire settings.
- **Fuzzer Suite**: Property-based testing for query parsing and execution edge cases.
- **Schema Registry Refactor**: Full refactor matching all 28 tables exactly; renamed key/value/sql/tag columns for spec compliance.
- **`ducklake_latest_snapshot_id()` SQL Function**: SQL function exposing latest snapshot ID for CDC startup queries.

## [0.27.10] — 2025-08-02

### Added

- **Compatibility CI Matrix**: Pin known-good DuckDB and DuckLake versions; nightly optional jobs.
- **Durable Compatibility Corpus**: PostgreSQL `pg_catalog` scan corpus archived for deterministic replay in CI.
- **Exact Column Schema Checks**: OID and column name validation in RowDescription against golden reference.

## [0.27.9] — 2025-07-19

### Added

- **View, Macro, Tag Metadata**: End-to-end DuckDB tests for CREATE/DROP/ALTER view, macro, and tag operations.
- **Column Tag Support**: Column-level tags with metadata lifecycle tests.
- **Sort Info & Partition Info**: Sort expression and partition column metadata for query optimization hints.
- **DROP/ALTER Cascade**: Cascade behavior for dependent objects (tags on dropped columns, macros in dropped schemas).
- **Time-Travel Tests**: ALTER TABLE time-travel queries reading schema at historical snapshots.
- **Imported DuckLake Support**: Full support for catalogs imported from external DuckLake sources.

## [0.27.8] — 2025-07-05

### Added

- **Atomic Batch Commits**: Group all statements in one logical DuckLake commit into single atomic batch.
- **Spec-Complete `ducklake_snapshot_changes`**: Full implementation with changes_made, author, commit_message, commit_extra_info.
- **Interleaved Writer & Rollback Tests**: Concurrent writer scenarios with rollback validation.
- **Type-Aware Column Stats**: Date, timestamp, and decimal column statistics with correct aggregations.

### Fixed

- Writer fencing now uses transactional counter instead of wall-clock epochs.
- Snapshot changes table now includes all spec-required fields.

## [0.27.7] — 2025-06-21

### Added

- **DuckLakeTableSchema Registry**: Single source of truth for all 28 metadata table schemas.
- **Wire Executor Response Builders**: Response generation code wired to schema registry.
- **Projection-Order Golden Tests**: Validate exact column order for every table across all query contexts.
- **Arbitrary Output Alias Support**: Dynamic inlined tables support arbitrary aliases in SELECT queries.

## [0.27.6] — 2025-06-07

### Added

- **Automated DuckDB/DuckLake Lifecycle Tests**: Fresh attach, INSERT/DELETE/UPDATE, restart reads, stats inspection.
- **Direct SQL Testing**: `postgres_query` of dynamic inlined tables for end-to-end validation.
- **Stats Merge Regression Tests**: Negative numbers, floats, and strings in column stats aggregation.

## [0.27.5] — 2025-05-24

### Added

- **Exact SQL Catalog Facades**: All 28 tables with spec-compliant column names, types, and ordering.
- **Snapshot/Snapshot_Changes Schema Fix**: Corrected schema to match DuckLake v1.0 spec exactly.
- **Spec-Complete Delete-File Semantics**: DELETE files follow DuckLake spec with proper partition and predicate semantics.
- **DROP TABLE Cascade**: Cascade deletion of dependent metadata (indexes, constraints, privileges).
- **Inlined Data SQL Support**: Dynamic inlined tables queryable through PG-wire protocol.
- **Data File Spec Fields**: All required fields present (file_id, path, size, record_count, min_row_id, max_row_id).
- **Metadata Facades**: Virtual views for accessing inlined metadata without additional catalog scans.
- **Column Stats Completeness**: Min, max, null_count, distinct_count for all column types.

## [0.27.4] — 2025-05-10

### Added

- **DuckDB 1.5.x PostgreSQL Scanner Support**: Handle all DuckDB 1.5.x initialization queries (DISCARD ALL, to_regclass, information_schema probes, pg_type composites, pg_indexes, pg_database_size).
- **Wire-Corpus Capture for DuckDB 1.5.x**: Complete wire-protocol corpus for compatibility validation.

### Changed

- Updated compatibility matrix to support DuckDB 1.5.x as primary target.

## [0.27.3] — 2025-04-26

### Added

- **Coverage Threshold Hard Gate**: Code coverage must meet minimum threshold (currently 75%); CI fails if not met.
- **Doc-Tests for Core APIs**: All public APIs in `rocklake-core` and `rocklake-catalog` have doctests.
- **Network-Level PG-Wire Integration Tests**: End-to-end tests over real TCP socket.
- **Concurrent Writer Fencing Test**: Multi-writer scenarios validating epoch conflicts detected.
- **Checkpoint-Restore Snapshot-ID Safety**: Verify restored snapshots maintain correct IDs and ordering.
- **`rebuild_catalog` Behavior Tests**: Comprehensive tests for catalog rebuild operation.
- **CLI Docs-Conformance Test**: Validate CLI help output matches reference documentation.

### Documentation

- Complete operational monitoring guide aligned with CLI flags.
- Security assessment findings closed (Assessments 1 & 2).

## [0.27.2] — 2025-04-12

### Added

- **Real Parquet Row Scanning**: Implement `extract_rows_from_parquet()` via `object_store` to replace synthetic CDC payloads.
- **Streaming/Batching for Large Files**: Handle large Parquet files with streaming decompression for memory efficiency.
- **End-to-End CDC Round-Trip Tests**: Verify CDC column payloads match actual scanned data.
- **`record_count` Verification**: Cross-check metadata record_count against actual scanned row count.

### Changed

- Replaced synthetic CDC column payloads with real Parquet row data.

### Removed

- `rocklake-sqlite-vfs` placeholder crate (no functional code, speculative only).

## [0.27.1] — 2025-03-29

### Added

- **Real CDC Implementation**: `table_changes()` function returns actual row data from Parquet files.
- **Parquet Row Extraction**: Streaming extraction of rows from Parquet files with compression support.

## [0.27.0] — 2025-03-15

### Added

- **DuckLake v1.0 Conformance Test Harness**: Comprehensive test suite for all 28 spec tables.
- **Snapshot/Snapshot_Changes Spec Alignment**: Schema corrected to match DuckLake v1.0 spec.
- **Data File Spec Fields**: All required data file metadata fields implemented and persisted.
- **Delete File Model**: Spec-complete delete file semantics with partition and predicate support.
- **Row ID Tracking**: Stable row ID generation and tracking across snapshots.
- **Table Stats `next_row_id`**: Auto-incrementing row ID counter exposed in table stats.
- **DROP TABLE Cascade**: Cascade deletion of dependent metadata objects.

### Changed

- MVCC implementation aligned with DuckLake v1.0 specification.

---

## [0.1–0.26] — Historical Releases

Earlier releases focused on foundation (v0.1), catalog core (v0.2), wire protocol (v0.3–v0.6), performance (v0.7), documentation (v0.8), production readiness (v0.9–v0.9.4), and standard DuckLake catalog interface (v0.18–v0.26). Full history available in Git commit log and archived roadmap documents.

---

## Release Policy

- **Semantic Versioning**: MAJOR.MINOR.PATCH (v1.0 marks general availability)
- **Release Cycle**: Monthly or as-needed for critical fixes
- **Support Policy**: Latest patch version of each minor release; v1.0+ receives 12-month maintenance
- **Deprecation Policy**: 2 minor versions (or 6 months) advance notice before removing APIs

## Breaking Changes

### v1.0 (Future)

- Stable API guarantee for all public `rocklake-client` methods.
- Stable C ABI signature for `rocklake.h` functions (no breaking changes without major version).

---

## Contributing

See [CONTRIBUTING.md](../CONTRIBUTING.md) for contribution guidelines, development setup, and release procedures.
