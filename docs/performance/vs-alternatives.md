# Rocklake vs. Alternatives

Choosing a DuckLake catalog backend is not a question of "which is best" — it is a question of "which trade-offs match my priorities." This page provides an honest, detailed comparison between Rocklake and the alternative backends that DuckLake supports: PostgreSQL, SQLite, and MySQL. Each comparison includes raw performance numbers, operational complexity analysis, cost modeling, and clear guidance on when each option is the better choice.

This page does not claim Rocklake is universally superior. It is faster in some dimensions, slower in others, simpler in some aspects, less capable in others. The goal is to help you make an informed decision based on your specific circumstances — not to sell you on Rocklake when PostgreSQL would serve you better.

## The Alternatives

DuckLake's catalog protocol is backend-agnostic. DuckDB's `ducklake` extension connects to any backend that speaks the DuckLake protocol over PostgreSQL wire format (or implements the catalog tables directly). The practical options are:

| Backend | Description | Maturity |
|---------|-------------|----------|
| **PostgreSQL** | Traditional relational database running DuckLake's schema | Production-ready, default |
| **SQLite** | Embedded database file with DuckLake tables | Development/single-machine |
| **MySQL** | Alternative relational backend | Supported but less common |
| **Rocklake** | Purpose-built serverless catalog on object storage | Production-ready |

## Raw Performance Comparison

### Read Latency (Point Operations)

| Operation | Rocklake (S3 Std) | Rocklake (S3 Express) | PostgreSQL (RDS) | SQLite (local) |
|-----------|-------------------|----------------------|-----------------|---------------|
| Point read (cold) | 30–100ms | 3–10ms | 1–3ms | < 0.1ms |
| Point read (warm) | 0.8–2ms | 0.8–2ms | 1–3ms | < 0.1ms |
| Schema lookup | 1–50ms | 1–5ms | 1–3ms | < 0.1ms |

**Analysis:** For cold reads (first access), PostgreSQL wins against Rocklake-on-S3-Standard by a large margin. For warm reads, Rocklake and PostgreSQL are nearly identical. SQLite dominates both because it is in-process with zero network overhead.

Rocklake's warm-read latency (0.8–2ms) is competitive with PostgreSQL (1–3ms) because both are bound by TCP round-trip time. The storage backend latency is irrelevant when the block cache is warm.

### Read Latency (Scan Operations)

| Operation | Rocklake (S3 Std) | Rocklake (S3 Express) | PostgreSQL (RDS) | SQLite (local) |
|-----------|-------------------|----------------------|-----------------|---------------|
| List 50 columns (cold) | 30–80ms | 5–12ms | 2–5ms | < 1ms |
| List 50 columns (warm) | 3–8ms | 3–8ms | 2–5ms | < 1ms |
| List 1000 files (cold) | 80–250ms | 10–30ms | 5–15ms | 1–5ms |
| List 1000 files (warm) | 15–30ms | 15–30ms | 5–15ms | 1–5ms |

**Analysis:** For large scans, PostgreSQL maintains an advantage because its buffer pool and sequential scan optimization are highly tuned. Rocklake's scan performance is respectable with warm cache but cannot match a database optimized for relational scans over 40 years.

### Write Latency

| Operation | Rocklake (S3 Std) | Rocklake (S3 Express) | PostgreSQL (RDS) | SQLite (local) |
|-----------|-------------------|----------------------|-----------------|---------------|
| Single write | 50–150ms | 3–10ms | 5–15ms | < 1ms |
| Batch 100 writes | 60–160ms | 5–15ms | 20–50ms | 1–5ms |
| Batch 1000 writes | 100–200ms | 10–25ms | 50–200ms | 5–20ms |

**Analysis:** Rocklake on S3 Standard is the slowest writer. On S3 Express, Rocklake is competitive with PostgreSQL for single writes and better for large batches (because Rocklake's batching has constant overhead regardless of batch size, while PostgreSQL's write amplification grows with transaction size).

SQLite dominates write latency because it writes to local disk with fsync — no network involved.

### Write Throughput (Sequential)

| Backend | Writes/second (sequential) | Writes/second (concurrent) |
|---------|---------------------------|---------------------------|
| Rocklake (S3 Standard) | 8–13 | 8–13 (single writer) |
| Rocklake (S3 Express) | 80–150 | 80–150 (single writer) |
| PostgreSQL (RDS) | 500–2,000 | 5,000–20,000 |
| SQLite (local) | 1,000–5,000 | 1,000–5,000 (single writer) |

**Analysis:** PostgreSQL has vastly higher write throughput, especially with concurrent writes. Rocklake's single-writer model limits throughput to one transaction at a time. For catalog workloads (a few writes per minute), Rocklake's throughput is more than sufficient. For high-write scenarios, PostgreSQL is clearly better.

## Operational Complexity Comparison

Performance is not the only dimension. Operational complexity — the ongoing human effort to keep the system running — is often the dominant cost.

### PostgreSQL Operational Requirements

To run PostgreSQL as a DuckLake catalog backend in production:

| Concern | Requirement | Effort |
|---------|-------------|--------|
| Provisioning | RDS instance or self-hosted | Initial setup |
| Availability | Multi-AZ RDS or replication setup | Configuration |
| Backups | Automated daily backups + WAL archiving | Configuration + verification |
| Monitoring | Connection count, replication lag, disk space, vacuum progress | Ongoing |
| Patching | Security patches, minor version upgrades | Monthly |
| Scaling | Instance size changes, read replica addition | As needed |
| Connection management | Connection pooler (PgBouncer/PgCat) | If > 100 connections |
| Vacuum | Monitor and tune autovacuum settings | Quarterly review |
| Extensions | Install and maintain `ducklake` extension | On upgrades |
| Disaster recovery | Cross-region read replicas or S3 backup | HA requirement |
| Cost | RDS instance ($50–$500+/month depending on size) | Ongoing |

**Total ongoing effort:** 2–8 hours/month for a typical team (monitoring, patching, occasional incident response).

### Rocklake Operational Requirements

| Concern | Requirement | Effort |
|---------|-------------|--------|
| Provisioning | Single binary + S3 bucket path | Minutes |
| Availability | Restart on failure (systemd/Kubernetes) | Configuration |
| Backups | None needed (S3 provides 11 nines durability) | Zero |
| Monitoring | Process health + basic metrics | Minimal |
| Patching | Binary replacement, restart | Monthly |
| Scaling | No scaling needed (reads are independent) | Zero |
| Connection management | None needed (lightweight protocol) | Zero |
| Garbage collection | Periodic GC run (weekly/monthly) | 5 min/month |
| Disaster recovery | S3 cross-region replication (infrastructure feature) | Configuration |
| Cost | Compute ($5–$30/month) + S3 ($1–$5/month) | Ongoing |

**Total ongoing effort:** 30 minutes/month (deploy updates, occasional GC).

### SQLite Operational Requirements

| Concern | Requirement | Effort |
|---------|-------------|--------|
| Provisioning | File path | Seconds |
| Availability | Same as the process using it | Zero |
| Backups | File copy (or no backup for dev) | Simple |
| Monitoring | None | Zero |
| Multi-machine access | Not supported (single machine only) | N/A |
| Concurrent writers | Not supported | N/A |
| Cost | Zero | Zero |

**Total ongoing effort:** Zero — but limited to single-machine use.

## Cost Comparison

### Monthly Cost for a Typical Analytics Catalog

Assumptions: 50 tables, 2,500 columns, 50,000 data files, moderate read traffic (1,000 reads/hour), low write traffic (10 writes/hour).

| Component | Rocklake | PostgreSQL (RDS) | SQLite |
|-----------|-----------|-----------------|--------|
| Compute | $15 (t3.micro) | $50 (db.t3.small) | $0 (in-process) |
| Storage | $1 (S3 Standard) | $5 (EBS gp3) | $0 (local disk) |
| Network | $2 (VPC traffic) | $5 (cross-AZ) | $0 |
| Backups | $0 (S3 inherent) | $10 (automated snapshots) | $0 |
| **Total** | **$18/month** | **$70/month** | **$0/month** |

For an enterprise deployment (larger instance, S3 Express, monitoring):

| Component | Rocklake | PostgreSQL (RDS) | SQLite |
|-----------|-----------|-----------------|--------|
| Compute | $50 (t3.medium) | $200 (db.r5.large + replica) | N/A |
| Storage | $30 (S3 Express) | $50 (EBS io1) | N/A |
| Network | $10 | $20 | N/A |
| Backups | $0 | $30 | N/A |
| **Total** | **$90/month** | **$300/month** | **N/A** |

### Cost Scaling

Rocklake's cost scales primarily with storage size and access frequency. PostgreSQL's cost scales with instance size (which must be provisioned for peak, not average). For catalogs that grow over time:

| Catalog Growth | Rocklake Cost Impact | PostgreSQL Cost Impact |
|---------------|----------------------|----------------------|
| 2x tables | +$0.50/month (more S3 storage) | +$0 (unless instance maxes out) |
| 10x reads | +$5/month (more S3 GETs) | May need instance upgrade (+$100–$300) |
| Connection spike | No impact | May need PgBouncer or larger instance |

## Feature Comparison

| Feature | Rocklake | PostgreSQL | SQLite |
|---------|-----------|-----------|--------|
| DuckLake protocol | Full | Full | Full (single machine) |
| Time travel | Built-in (snapshots) | Manual (triggers/temporal tables) | No |
| Multi-reader | Unlimited, zero-config | Limited by max_connections | Single process |
| Multi-writer | No (single writer) | Yes | No (single writer) |
| Cross-region reads | Yes (S3 replication) | Yes (read replicas, complex) | No |
| Durability | 11 nines (S3) | Depends on backup strategy | Depends on disk |
| Ad-hoc SQL | No (bounded SQL only) | Yes (full SQL) | Yes (full SQL) |
| pg_dump export | No | Yes | N/A |
| Extension ecosystem | No | Yes (PostGIS, pg_partman, etc.) | Limited |
| Managed service | Not yet | RDS, Cloud SQL, Azure DB | N/A |

## Decision Framework

### Choose Rocklake When

- **You want minimal operational overhead.** If your team does not have dedicated DBA capacity and you want to deploy once and forget, Rocklake eliminates 90% of the operational surface area of running PostgreSQL.

- **You are already using object storage.** If your data files are in S3/GCS/Azure, adding a Rocklake catalog means your entire data platform (data + metadata) lives in object storage. No additional infrastructure to manage.

- **You need durable time travel.** Rocklake's immutable architecture provides free, reliable time travel to any historical snapshot. PostgreSQL can achieve this with temporal tables or triggers, but it requires additional engineering.

- **Cost matters.** Rocklake costs $18–$90/month. PostgreSQL costs $70–$300/month. For teams running many catalogs (one per environment, one per team), the cost difference multiplied by N is significant.

- **You value simplicity over flexibility.** Rocklake's bounded scope means fewer things can go wrong. There are no vacuum issues, no connection exhaustion, no bloat problems, no lock contention.

### Choose PostgreSQL When

- **You already operate PostgreSQL.** If your team has PostgreSQL expertise, monitoring, backup procedures, and operational runbooks, adding another database to the fleet is trivial. The marginal operational cost is near zero.

- **You need sub-5ms latency consistently.** PostgreSQL's buffer pool serves reads in 1–3ms consistently (no cold-start penalty for warm databases). If every query plan must complete in under 100ms, and catalog lookup is a meaningful fraction of that budget, PostgreSQL is faster.

- **You need high write throughput.** If your pipeline registers hundreds of files per second (continuous streaming ingestion), PostgreSQL's concurrent write capability is necessary. Rocklake's single-writer model tops out at ~10–15 sequential writes per second on S3 Standard.

- **You need ad-hoc catalog queries.** If you regularly query catalog metadata directly (analytics about your data lake — "which tables grew fastest this month?"), PostgreSQL allows full SQL. Rocklake requires exporting to NDJSON first.

- **You need a managed service.** AWS RDS, Google Cloud SQL, and Azure Database for PostgreSQL provide fully managed PostgreSQL. Rocklake does not have a managed offering (though deploying it on Kubernetes or Fly.io is straightforward).

### Choose SQLite When

- **Single-machine development.** For local development and testing, SQLite is unbeatable. Zero setup, zero cost, sub-millisecond everything. There is no reason to run a server process for development.

- **Embedded applications.** If DuckDB is embedded in a desktop application or CLI tool that works with local files only, SQLite is the natural catalog backend.

- **You do not need multi-machine access.** SQLite's limitation (single machine, single process for writes) is only a limitation if you need distributed access. For many use cases, you don't.

## Migration Paths

### PostgreSQL → Rocklake

If you are running PostgreSQL and want to migrate to Rocklake:

1. Export catalog state from PostgreSQL (DuckLake's schema is well-defined)
2. Import into Rocklake (tool provided)
3. Update DuckDB connection strings
4. Decommission PostgreSQL instance

The migration preserves all catalog metadata, including historical snapshots (if you used temporal tables in PostgreSQL).

### Rocklake → PostgreSQL

If Rocklake does not meet your needs and you want to migrate to PostgreSQL:

1. Export catalog state using `rocklake export`
2. Load into PostgreSQL using DuckLake's schema
3. Update DuckDB connection strings

The export includes all current catalog state. Historical snapshots are not migrated (PostgreSQL does not natively support Rocklake's snapshot model).

## Further Reading

- **[When to Use Rocklake](when-to-use.md)** — Detailed workload analysis
- **[Benchmarks](benchmarks.md)** — Raw numbers you can reproduce
- **[Deployment: Configuration](../deployment/configuration.md)** — Setting up Rocklake for production
