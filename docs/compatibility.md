# Compatibility Matrix

This page describes the tested compatibility between SlateDuck and various ecosystem components.

## SQL Clients

| Client | Version Tested | Status | Notes |
|--------|---------------|--------|-------|
| psql | 16, 17, 18 | ✅ Supported | Standard PostgreSQL client |
| DBeaver | 24.x | ✅ Supported | JDBC PostgreSQL driver |
| pgcli | 4.x | ✅ Supported | |
| Metabase | 0.49+ | ✅ Supported | PostgreSQL connection |

## Apache Spark

| Spark Version | Status | Protocol | Notes |
|--------------|--------|----------|-------|
| 3.5.x | ✅ Supported | pg-wire | Full wire-corpus validated in CI |
| 3.4.x | ⚠️ Untested | pg-wire | Expected to work; not tested |
| 3.3.x | ❌ Not supported | — | Extended query protocol required |

## Trino / Presto

| Version | Status | Protocol | Notes |
|---------|--------|----------|-------|
| Trino 432+ | ✅ Supported | pg-wire | Full wire-corpus validated in CI |
| Trino 400–431 | ⚠️ Untested | pg-wire | Expected to work |
| Presto | ❌ Not tested | — | May work; contributions welcome |

## Apache DataFusion

| DataFusion Version | Status | Notes |
|-------------------|--------|-------|
| 45.0.0 | ✅ Supported | Default version; Parquet scan validated |
| < 45 | ❌ Not supported | API breaking changes in v45 |

## Object Storage Backends

| Backend | Version | Status | Notes |
|---------|---------|--------|-------|
| AWS S3 | — | ✅ Supported | Via `object_store` 0.12 |
| Google Cloud Storage | — | ✅ Supported | Via `object_store` 0.12 |
| Azure Blob Storage | — | ✅ Supported | Via `object_store` 0.12 |
| MinIO | RELEASE.2024+ | ✅ Supported | S3-compatible endpoint |
| Local filesystem | — | ✅ Supported | Development / testing |

## SlateDB

| SlateDB Version | Status | Notes |
|----------------|--------|-------|
| 0.13.x | ✅ Supported | Pinned in workspace |
| 0.12.x | ❌ Not supported | API incompatible |

## TLS

| TLS Version | Status | Notes |
|-------------|--------|-------|
| TLS 1.3 | ✅ Supported | Default |
| TLS 1.2 | ✅ Supported | Via rustls |
| TLS 1.1 or older | ❌ Rejected | Security policy |

## Rust

| Rust Version | Status | Notes |
|-------------|--------|-------|
| Stable latest | ✅ Supported | Recommended |
| 1.80.0 (MSRV) | ✅ Supported | Minimum supported Rust version |
| < 1.80 | ❌ Not supported | |

## Platform

| Platform | Architecture | Status | Notes |
|----------|-------------|--------|-------|
| Linux (Ubuntu 22.04+) | x86-64 | ✅ Supported | Primary CI target |
| Linux | aarch64 | ✅ Supported | Release binary provided |
| macOS 13+ | arm64 (Apple Silicon) | ✅ Supported | Release binary provided |
| Windows | x86-64 | ✅ Supported | Release binary provided |

## Version Policy

SlateDuck follows [Semantic Versioning](https://semver.org/).

- **Major (1.x.x)**: Breaking changes to the pg-wire API, catalog format, or FFI.
- **Minor (0.x.0)**: New features, backward-compatible changes.
- **Patch (0.0.x)**: Bug fixes and security patches.

Until `v1.0.0`, minor version bumps may include breaking changes to internal or experimental APIs.
