# Roadmap

This section outlines SlateDuck's development trajectory — where the project is headed, what has been accomplished, and what is planned for future releases. The roadmap represents current intentions, not commitments. Priorities shift based on real-world feedback, ecosystem changes (DuckDB releases, SlateDB improvements, cloud provider features), and the contributions of the community.

SlateDuck is developed in the open. The roadmap is public. Design decisions are documented. If you disagree with a priority or have a use case that the roadmap does not address, open a GitHub Discussion — concrete scenarios from real users are the strongest input to planning.

## Current Status

SlateDuck is in active development at version 0.8.x. The project has reached a point of architectural stability — the core design decisions (key encoding, MVCC model, bounded SQL, object storage persistence via SlateDB) are settled and unlikely to change. What remains is refinement: performance optimization, operational tooling, ecosystem integration, and the documentation and stability work required to reach 1.0.

**What works today:**

| Capability | Status | Quality |
|-----------|--------|---------|
| DuckLake catalog protocol (28 tables) | Complete | Production-ready |
| PG-wire protocol (Strategy B) | Complete | Production-ready |
| MVCC with snapshot isolation | Complete | Production-ready |
| Time travel / historical queries | Complete | Production-ready |
| Garbage collection | Complete | Production-ready |
| TLS + authentication | Complete | Production-ready |
| Prometheus metrics | Complete | Production-ready |
| Health check endpoints | Complete | Production-ready |
| Native DuckDB extension (Strategy C) | Complete | Beta |
| DataFusion integration | Complete | Beta |
| S3, GCS, Azure Blob Storage | Complete | Production-ready |
| S3 Express One Zone | Complete | Production-ready |
| Local filesystem storage | Complete | Development only |
| Documentation (this site) | Complete | 80+ pages |

**What "production-ready" means:** The feature is implemented, tested (unit + integration + property-based), has been running without issues in internal workloads, and is covered by the test suite. It may still have edge cases that surface under unusual conditions — this is pre-1.0 software.

## Development Phases

### Phase 1: Foundation (v0.1 – v0.5) ✅ Complete

Established the core architecture:

- SlateDB integration for object storage persistence
- Key encoding scheme (tag + big-endian u64 components)
- Value envelope format (SDKV magic + protobuf)
- MVCC visibility filter with begin_snapshot / end_snapshot
- Counter-based ID allocation
- CatalogStore / CatalogReader / CatalogWriter abstractions
- Complete DuckLake protocol table support (all 28 types)
- Native DuckDB extension (C FFI)
- Property-based tests for encoding correctness

### Phase 2: Protocol (v0.6 – v0.7) ✅ Complete

Implemented the network layer and operational tooling:

- PostgreSQL wire protocol server (Strategy B)
- SQL statement classifier (~50 recognized patterns)
- Session management with connection limits
- TLS encryption and password authentication
- Garbage collection (retention advancement + excision)
- Catalog integrity verification and repair
- Wire corpus test suite (DuckDB 1.5.x)
- Improved error reporting with SQLSTATE codes

### Phase 3: Performance & Observability (v0.8) ✅ Current

Focus on production readiness:

- Hot key caching for frequently-read system keys
- Secondary index for partition-based access patterns
- Write batching optimization (3–5x fewer S3 PUTs)
- Prometheus metrics collection
- Health check endpoints (liveness + readiness)
- DataFusion CatalogProvider integration
- Comprehensive documentation (80+ pages)
- Encryption at rest (AES-256-GCM)
- Audit logging for destructive operations

### Phase 4: Automation & Ecosystem (v0.9 – v0.10) — Next

Near-term priorities for the next 2–4 months:

**Automated background GC:**

Currently, garbage collection requires explicit invocation (`slateduck gc`). The planned improvement is a background task within the writer process that automatically advances retention and performs excision on a configurable schedule. Operators would configure:

```yaml
gc:
  retention_snapshots: 1000
  retention_duration: 7d
  excision_interval: 1h
  excision_batch_size: 10000
```

This eliminates the need for external cron jobs or Kubernetes CronJobs to manage catalog size.

**OpenTelemetry tracing:**

Structured tracing with distributed context propagation. Each catalog operation would emit a trace span with attributes (operation type, table name, latency, keys scanned). Integrates with Jaeger, Tempo, Datadog APM, and other OTLP-compatible backends.

**Connection multiplexing:**

A built-in connection pool that allows many concurrent DuckDB clients to share a smaller number of backend connections to SlateDuck. Useful for serverless deployments where many Lambda functions may connect simultaneously. The multiplexer would handle connection lifecycle (authentication, session state initialization) transparently, presenting a pool of pre-warmed sessions to incoming clients. This reduces connection establishment latency from ~50ms (TCP + TLS + PG auth) to near-zero.

**Catalog snapshots API:**

A REST API (alongside the PG-wire interface) for management operations that do not naturally fit the SQL model: listing snapshots with metadata, comparing two snapshots (diff), exporting catalog state as JSON, and triggering administrative actions (GC, verify, backup). This makes integration with automation tooling (Terraform, Pulumi, CI/CD pipelines) cleaner than encoding management operations as SQL statements.

**S3 Express optimizations:**

Leverage S3 Express One Zone's unique capabilities: directory-level locality, lower latency, and higher throughput. Specific optimizations include write path batching (S3 Express supports faster PUTs, enabling smaller batches) and read path parallelism (lower latency makes parallel fetches more effective).

### Phase 5: Stability & 1.0 (v0.11 – v1.0) — Medium-Term

The path to a stable release:

**Catalog format freeze:**

Commit to format version 1 stability. After this, any catalog created with format version 1 will be readable by all future SlateDuck versions. Migration tools will be provided for format upgrades.

**API stability:**

- PG-wire protocol behavior: stable (no breaking changes in supported SQL patterns)
- C FFI ABI: stable (new functions may be added but existing signatures will not change)
- Prometheus metrics: stable (metric names and labels will not change)

**Ecosystem bridges:**

- Apache Iceberg metadata bridge: read Iceberg table metadata and present it through the DuckLake protocol
- Delta Lake compatibility layer: understand Delta Lake transaction logs
- Catalog federation: query across multiple SlateDuck instances

**Multi-catalog management:**

A catalog-of-catalogs pattern where a single SlateDuck deployment manages multiple independent catalogs, each with its own storage location and configuration, but shared operational infrastructure (monitoring, GC, networking).

**Partitioned writers:**

For very large catalogs, allow multiple independent writers that each own a partition of the keyspace. Each partition is its own single-writer with its own epoch. This scales write throughput without compromising the single-writer safety model within each partition.

### Phase 6: Cloud-Managed Service — Long-Term

A managed offering built on the same open-source core:

- Automatic scaling (writer process right-sized based on workload)
- Managed networking (private endpoints, VPC peering)
- Automated backups and disaster recovery
- Cost analytics (storage breakdown by table/schema)
- Web dashboard for catalog browsing
- SLA-backed availability guarantees

The managed service is not a priority until the open-source project reaches 1.0 stability.

## Design Principles for Roadmap Decisions

All proposed features are evaluated against these criteria:

**1. Does it simplify operations?**

Features that reduce operational burden are strongly prioritized. Automated GC removes the need for operator-managed cron jobs. Built-in health checks eliminate the need for external monitoring scripts. Connection pooling avoids deploying PgBouncer.

**2. Does it maintain the single-binary promise?**

SlateDuck ships as one binary. Features that require deploying additional infrastructure (separate GC service, external cache, sidecar proxy) are strongly deprioritized. Everything should work out of the box.

**3. Does it serve the DuckLake use case?**

Features that generalize SlateDuck beyond its core purpose (lakehouse catalog) are carefully evaluated. Adding general SQL support, general key-value access, or non-DuckLake client support would dilute focus.

**4. Is it reversible?**

Decisions that can be undone (configuration options, optional features, additive API changes) are preferred over decisions that cannot (format changes, removed features, behavioral changes).

**5. Does it have real user demand?**

Features motivated by concrete use cases from real users are prioritized over speculative "nice to have" features. This is why GitHub Discussions are the input mechanism — we need to understand the actual problem being solved.

## What SlateDuck Will NOT Do

Some features are intentionally out of scope, permanently:

- **General SQL query execution:** SlateDuck will never execute SELECT queries against user data. It is a catalog, not a query engine.
- **Multi-master writes:** The single-writer model is fundamental. Distributed consensus (Raft, Paxos) will not be added.
- **Local disk storage for production:** Object storage is the durability layer. Local disk is for development only.
- **Non-DuckLake protocols:** Supporting Hive Metastore protocol, Iceberg REST protocol, or Unity Catalog API is not planned (though bridges that translate to/from DuckLake are possible).
- **User management / RBAC:** Fine-grained authorization is not in scope. Access control should be handled at the network layer or through object storage IAM policies.

## Contributing to the Roadmap

If you want to influence the roadmap:

1. **Open a GitHub Discussion** describing your use case (not just "I want feature X" — describe the problem you are solving)
2. **Propose a design** if you have one (but be open to alternative approaches)
3. **Contribute code** for features you need (PRs that include tests and documentation are fast-tracked)
4. **Report bugs** — fixing bugs always takes priority over new features

## Pages

- **[Changelog](changelog.md)** — Detailed history of every release
