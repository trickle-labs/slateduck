# When to Use SlateDuck

Every tool has a sweet spot — a set of conditions where it outperforms alternatives and a set of conditions where it falls short. SlateDuck's sweet spot is precisely defined by its architecture: single-writer, object-storage-backed, bounded-SQL, serverless. This page provides specific, actionable criteria for evaluating whether SlateDuck is right for your workload. It includes both the scenarios where SlateDuck excels (and you should strongly consider it) and the scenarios where it is a poor fit (and you should look elsewhere).

The goal is to save you time. If your workload matches the "SlateDuck excels" criteria, you can adopt it with confidence that it will serve you well. If your workload matches the "SlateDuck is not ideal" criteria, you now know before investing deployment effort.

## SlateDuck Excels When

### You Want a Serverless Data Lakehouse Catalog

This is SlateDuck's primary use case — the scenario it was designed for from the first line of code.

**The situation:** Your data lives in object storage (S3, GCS, Azure Blob) as Parquet files. You want to query it with DuckDB as a proper lakehouse (schema management, time travel, file-level statistics). You do not want to run a database server for the catalog.

**Why SlateDuck excels:**

- One binary + one bucket path = complete DuckLake catalog
- No database server to manage (no PostgreSQL, no MySQL)
- Catalog metadata lives alongside your data in object storage
- Durability is inherited from the cloud provider (11 nines for S3)
- Start and stop at will — no persistent server process required
- Pay only for storage and API calls — no instance hours when idle

**The alternative (PostgreSQL) requires:** An always-running database instance ($50+/month minimum), backup procedures, patching, connection management, failover configuration, and monitoring. For teams that just want to query Parquet files, this is over-engineering.

**Decision threshold:** If "running a database server" is a meaningful operational burden for your team (small team, no dedicated DBA, infrastructure-light philosophy), SlateDuck removes that burden entirely.

### You Are Building on DuckDB

SlateDuck is designed specifically for DuckDB's `ducklake` extension. The integration is native, the protocol is fully supported, and the deployment model aligns with DuckDB's philosophy (simple, embedded, minimal dependencies).

**Why SlateDuck excels for DuckDB users:**

- DuckDB + SlateDuck gives you a complete lakehouse with two components (no third-party coordination services)
- The native extension (Strategy C) embeds SlateDuck directly in DuckDB — zero network overhead, single-process deployment
- SlateDuck's SQL dialect matches DuckLake's protocol exactly (no compatibility layer, no translation)
- Both DuckDB and SlateDuck share the "do one thing well" philosophy — they compose cleanly

**When this matters less:** If DuckDB is not your primary query engine (you use Spark, Trino, or Presto), SlateDuck provides less value because those engines do not speak the DuckLake protocol natively. You would need a compatibility layer, which reduces the simplicity advantage.

### Your Catalog Workload Is Moderate

A "moderate" catalog workload for DuckLake means:

| Dimension | Moderate Range | SlateDuck Handles Comfortably |
|-----------|---------------|------------------------------|
| Tables | 10–1,000 | Yes |
| Columns per table | 5–200 | Yes |
| Data files | 100–1,000,000 | Yes |
| Reads per minute | 10–10,000 | Yes (cache serves most) |
| Writes per minute | 0.1–100 | Yes (single writer sufficient) |
| Concurrent readers | 1–100 | Yes (unlimited readers) |
| Schema changes | A few per day | Yes |

**The vast majority of analytics workloads fall within these ranges.** A data warehouse with 500 tables, 50,000 columns, and 10 ETL jobs writing every 15 minutes is well within SlateDuck's capacity. The catalog sees maybe 10–20 writes per hour and a few thousand reads.

**Decision threshold:** If your catalog sees fewer than 100 writes per minute and fewer than 10,000 reads per minute, SlateDuck's throughput is not a bottleneck.

### You Need Reliable Time Travel

SlateDuck's immutable architecture provides time travel as an inherent property, not as an add-on feature:

- Every mutation creates a new snapshot with a monotonically increasing ID
- Previous snapshots are never modified (only superseded)
- Query any historical snapshot by ID: "show me the catalog state as of snapshot 500"
- Time travel works across schema changes, file registrations, and drops
- Historical data persists until explicitly garbage collected

**Why this matters:**

- **Pipeline debugging:** "The dashboard broke yesterday. What did the catalog look like at 2pm?" — query snapshot 450 and compare to current.
- **Audit requirements:** "Show the state of all tables at end-of-quarter." — use the snapshot from that date.
- **Reproducibility:** "Re-run last Thursday's analysis with the same data." — attach the catalog at Thursday's snapshot.

**The alternative with PostgreSQL:** Temporal tables or trigger-based audit logs can provide similar functionality, but they require engineering effort (designing the triggers, managing the audit table growth, querying historical state). SlateDuck provides this out of the box.

### You Operate in Multiple Regions

SlateDuck leverages cloud provider infrastructure for multi-region access without application-level replication:

**Cross-Region Read Access:**

```
us-east-1: SlateDuck writer → s3://bucket/catalog/
eu-west-1: SlateDuck reader → s3://bucket/catalog/ (S3 Cross-Region Replication)
ap-southeast-1: SlateDuck reader → s3://bucket/catalog/ (S3 CRR)
```

S3's Cross-Region Replication makes catalog data available globally with infrastructure-level configuration (no application changes). Readers in remote regions access replicated data with local latency.

**The alternative with PostgreSQL:** Cross-region read replicas require configuration (streaming replication, connection routing), introduce replication lag (seconds to minutes), and add operational complexity (monitoring lag, handling failover). SlateDuck delegates all of this to the storage provider.

### You Want Zero-Ops Durability

SlateDuck's durability is inherited from object storage:

| Provider | Durability SLA | Availability SLA | Backup Required? |
|----------|---------------|-----------------|-----------------|
| S3 Standard | 99.999999999% (11 nines) | 99.99% | No |
| GCS Standard | 99.999999999% | 99.95% | No |
| Azure Blob (GRS) | 99.99999999999999% (16 nines) | 99.99% | No |

For comparison, a self-managed PostgreSQL deployment achieves high durability only through careful WAL archiving, point-in-time recovery testing, and backup verification. Most teams do not achieve 11 nines of durability for their PostgreSQL instances — they achieve whatever their backup strategy provides (typically 99.99% — four nines — which means potential data loss during failures).

**Decision threshold:** If "we lost the catalog" is unacceptable and you do not want to invest in backup verification, SlateDuck provides durability without engineering effort.

## SlateDuck Is Not Ideal When

### You Need Sub-Millisecond Catalog Latency

If your use case requires that catalog operations complete in under 1ms — consistently, including cold starts — SlateDuck on S3 Standard cannot provide this. Cold reads take 30–100ms. Even with S3 Express (3–10ms cold), you cannot reach sub-millisecond territory.

**Scenarios where this matters:**

- Interactive BI dashboards with 100ms total query budget (catalog lookup must be < 5ms)
- Real-time applications that issue many small queries with strict latency SLAs
- Benchmarking environments where every millisecond is measured

**What to use instead:**

- **SQLite** for single-machine deployments (sub-0.1ms, in-process)
- **PostgreSQL with warm buffer pool** for multi-machine (1–3ms, stable)
- **SlateDuck native extension** brings latency to 0.3ms for cache-hot reads (but cold reads still hit object storage)

**Mitigation if you still want SlateDuck:** Use S3 Express One Zone (3–10ms cold, < 1ms warm after cache hits) and ensure your working set fits in cache. After warm-up, most reads are under 2ms. But if you need guaranteed sub-millisecond — including the first read after a restart — SlateDuck cannot provide that.

### You Have Extreme Write Throughput

If your pipeline registers more than 100 files per second continuously (not in bursts — continuously), SlateDuck's single-writer model on S3 Standard is a bottleneck:

- S3 Standard: ~10–15 writes/second (sequential)
- At 100 files/batch: ~1,000–1,500 file registrations/second
- At 1,000 files/batch: ~7,000–10,000 file registrations/second

If your requirement exceeds these numbers:

**What to use instead:**

- **PostgreSQL** with concurrent transactions: 5,000–20,000 individual writes/second
- **Multiple SlateDuck catalogs** (one per dataset): each handles independent writes in parallel

**Mitigation if you still want SlateDuck:** Batch more aggressively. If your pipeline can buffer 10,000 file registrations and commit them in one transaction every 10 seconds, SlateDuck handles this easily (one write per 10 seconds). The constraint is not throughput per batch — it is transactions per second.

### You Need Arbitrary Catalog Queries

SlateDuck's bounded SQL means it only supports the ~50 statement patterns that DuckLake needs. You cannot run arbitrary analytical queries against catalog metadata:

```sql
-- These work in PostgreSQL but NOT in SlateDuck:
SELECT table_name, count(*) as file_count
FROM ducklake_tables t JOIN ducklake_data_files f ON t.table_id = f.table_id
GROUP BY table_name
ORDER BY file_count DESC;

SELECT * FROM ducklake_columns WHERE data_type LIKE '%timestamp%';

SELECT schema_name, count(DISTINCT table_id) FROM ducklake_tables GROUP BY schema_name;
```

If you regularly need to query catalog metadata for analytics (data governance dashboards, catalog health reports, metadata search), SlateDuck requires an export-then-query workflow:

```bash
# Export to NDJSON
slateduck export --storage s3://bucket/catalog/ --output catalog.ndjson

# Then query with DuckDB
duckdb -c "SELECT * FROM read_ndjson('catalog.ndjson') WHERE ..."
```

**What to use instead:** PostgreSQL allows full SQL against DuckLake's catalog tables. If ad-hoc catalog queries are a frequent need (daily or more), PostgreSQL provides a significantly better experience.

### You Already Operate Managed PostgreSQL

If your organization already runs managed PostgreSQL (RDS, Cloud SQL, Azure Database) with:

- Established backup procedures
- Monitoring dashboards
- Incident runbooks
- Team expertise

Then the operational advantage of SlateDuck is small. Adding one more database to a fleet of databases you already manage is trivial. The marginal operational cost of "one more PostgreSQL database" is near zero for a team that already operates PostgreSQL.

**Decision threshold:** If adding a PostgreSQL database to your infrastructure takes less than 30 minutes of one-time setup and zero ongoing effort (because all the operational practices already exist), SlateDuck's simplicity advantage is negligible.

### You Need True Multi-Writer

If multiple independent processes must write to the same catalog concurrently without coordination (and dataset partitioning is not acceptable), SlateDuck's single-writer model is a fundamental limitation.

**Scenarios where this matters:**

- Multiple ETL pipelines writing to the same tables simultaneously
- Collaborative environments where multiple users modify schema concurrently
- Event-driven architectures where catalog writes come from many independent sources

**What to use instead:** PostgreSQL with row-level locking handles concurrent writes natively. DuckLake's PostgreSQL backend supports multiple concurrent connections writing to the same catalog.

**Mitigation if you still want SlateDuck:** Dataset partitioning (one catalog per dataset, one writer per catalog) provides write parallelism across datasets. If your concurrent writes are to different datasets, this pattern eliminates the constraint with no loss of functionality.

## Workload Decision Matrix

| Criterion | Favors SlateDuck | Favors PostgreSQL | Favors SQLite |
|-----------|-----------------|-------------------|---------------|
| Team size | Small (no DBA) | Any | One developer |
| Write frequency | < 100/min | > 100/min | Any |
| Read latency need | > 5ms OK | < 5ms required | < 0.1ms required |
| Durability need | Maximum (11 nines) | Managed is OK | Not critical |
| Infrastructure | Object storage only | Database fleet exists | Single machine |
| Time travel | Required | Nice to have | Not needed |
| Ad-hoc catalog SQL | Not needed | Frequently needed | Occasionally |
| Budget | Minimal | Moderate | Zero |
| Multi-region reads | Required | Optional | Not applicable |
| Machine count | Multiple | Multiple | One |

**Scoring:** Count the column with the most matches. That is likely your best choice.

## Hybrid Approaches

You do not have to choose one backend for all use cases:

- **Development:** SQLite (zero setup, instant)
- **Staging:** SlateDuck (matches production architecture)
- **Production:** SlateDuck or PostgreSQL (depending on your team's profile)

DuckDB's `ducklake` extension works identically against all backends. Your SQL queries, data files, and application code do not change when switching backends. Only the connection string changes.

## Further Reading

- **[vs. Alternatives](vs-alternatives.md)** — Detailed performance numbers
- **[Benchmarks](benchmarks.md)** — Reproduce the comparison yourself
- **[Getting Started: Quickstart](../getting-started/quickstart.md)** — Try SlateDuck in 5 minutes
- **[Deployment: Configuration](../deployment/configuration.md)** — Production setup guidance
