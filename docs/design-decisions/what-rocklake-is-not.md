# What Rocklake Is Not

Every successful software project has a boundary — a clearly articulated set of things it deliberately does not do. These non-goals are not limitations to be apologized for; they are design decisions to be celebrated. They are the reason Rocklake can be small, reliable, and understandable. They are the reason a single Rust binary can replace a PostgreSQL cluster for the specific problem it solves.

This page documents what Rocklake is not, explains why each non-goal was chosen, describes what you should use instead, and provides an honest assessment of when Rocklake is the wrong choice. If you are evaluating Rocklake for a use case and find it listed here as a non-goal — that is valuable information. It means Rocklake was not designed for your problem, and using it anyway will result in frustration.

## Not a Query Engine

Rocklake does not execute analytical queries. It does not scan Parquet files, perform joins, evaluate predicates, compute aggregations, or optimize query plans. It does not implement a columnar execution engine, vectorized processing, or parallel query evaluation. It does not understand your data — it understands your metadata.

The boundary is precise: Rocklake knows that table "sales" has columns "date", "amount", and "region". It knows that the data lives in files `s3://bucket/sales/part-001.parquet` through `part-500.parquet`. It knows the file-level statistics (min/max values, row counts, null counts). But it has never read a single row of actual data. It has never evaluated `WHERE amount > 100`. It has never computed `SUM(amount) GROUP BY region`.

### Why This Boundary Exists

Combining a metadata catalog with a query engine would create a monolithic system with competing concerns:

**Resource contention.** A query engine needs CPU and memory for computation. A metadata catalog needs fast I/O to serve schema lookups. Running both in the same process means they compete for resources. A large analytical query could starve metadata operations, causing other DuckDB clients to time out waiting for schema information.

**Scaling characteristics differ.** Query engines scale horizontally by adding compute nodes. Metadata catalogs scale by caching (the catalog is small enough to fit in memory). Combining them forces you to scale both together, even when only one is under pressure.

**Evolution velocity differs.** Query engines evolve rapidly — new join algorithms, improved statistics, better parallelism. Metadata catalogs should be stable — schema compatibility across versions is critical. Coupling them means catalog upgrades require query engine testing and vice versa.

**Deployment flexibility is lost.** With separation, you can run Rocklake on a tiny ARM instance (or Lambda) while DuckDB runs on a beefy compute node. With coupling, you need a machine that satisfies both requirements.

### What You Should Use Instead

DuckDB is Rocklake's query engine. The architecture is deliberate: Rocklake serves metadata over the PostgreSQL wire protocol, DuckDB's `ducklake` extension consumes that metadata to plan and execute queries. DuckDB handles everything after the metadata lookup: file reading, predicate pushdown, join execution, aggregation, result formatting.

```
DuckDB (query engine)  ←→  Rocklake (metadata catalog)  ←→  SlateDB (storage)
         ↓
    Parquet files in S3 (actual data)
```

If you find yourself wanting Rocklake to "just run the query directly" — you want DuckDB. Rocklake makes DuckDB better by giving it reliable, fast, consistent metadata. But the computation happens in DuckDB.

## Not a General-Purpose Database

You cannot use Rocklake to store application data, serve a web application, run an OLTP workload, or replace PostgreSQL for general use. Rocklake's SQL support is intentionally limited to approximately 50 statement patterns — exactly those needed by DuckDB's DuckLake protocol. It does not support:

- `CREATE USER`, `GRANT`, `REVOKE` (no access control SQL)
- `CREATE INDEX` (no secondary indexes)
- Prepared statements beyond simple parameter binding
- Connection pooling or connection multiplexing
- Stored procedures, functions, or triggers
- Advisory locks or explicit `LOCK TABLE`
- `LISTEN`/`NOTIFY` or any pub/sub mechanism
- `COPY` for bulk data import/export
- Most PostgreSQL system catalogs (`pg_class`, `pg_attribute`, etc.)
- Custom types, domains, or enums
- Window functions, CTEs, or subqueries in catalog operations

Any SQL statement that does not match a recognized DuckLake pattern receives an error response — not a "best effort" parse, but an explicit rejection. This is by design.

### Why This Boundary Exists

**Focus enables reliability.** Supporting 50 SQL patterns means each one can be exhaustively tested. The test suite verifies every valid statement and every known invalid statement. Adding general SQL support (thousands of statement patterns, edge cases, compatibility quirks) would reduce test coverage per pattern by orders of magnitude.

**Bounded complexity enables auditing.** You can read and understand all 50 SQL patterns in an afternoon. You can verify that each one correctly maps to catalog operations. This auditability is impossible with a general SQL engine (PostgreSQL's SQL parser alone is 100,000+ lines).

**Size matters.** Rocklake compiles to a single binary under 50 MB. A general-purpose database requires hundreds of MB and dozens of dependencies. The small binary means fast container starts, low memory usage, and deployment anywhere (including Lambda and embedded devices).

**General-purpose databases already exist.** PostgreSQL, MySQL, SQLite, CockroachDB — these are mature, battle-tested, and excellent. Building a 51st general-purpose database would be hubris. Rocklake exists because these databases are too complex, too heavy, and too operationally demanding for the specific problem of "serve DuckLake metadata from object storage."

### When You Need a General-Purpose Database

If you need to store application data alongside your DuckLake catalog, run Rocklake for the catalog and a separate PostgreSQL instance for application data. They serve different purposes and have different operational requirements. Trying to combine them into one system would produce a system that does both jobs poorly.

## Not a Distributed System

Rocklake does not implement consensus protocols, distributed transactions, multi-node coordination, or any form of cross-process communication. There is no Raft, no Paxos, no gossip protocol, no membership service, no heartbeat mechanism, no split-brain detection, no quorum tracking, no replica synchronization.

One process writes. Any number of processes read. If the writer fails, a new process becomes the writer by incrementing the epoch counter. That is the entirety of Rocklake's distributed systems story.

### Why This Boundary Exists

**Distributed coordination is the primary source of bugs in database systems.** Network partitions, clock skew, message reordering, partial failures, Byzantine faults — distributed systems must handle failure modes that single-node systems simply cannot experience. Each failure mode requires detection logic, recovery logic, and testing infrastructure. The cumulative complexity is enormous.

**The problem does not require distribution.** A DuckLake catalog for a data warehouse with 1,000 tables and 100,000 data files is approximately 50 MB of metadata. This fits trivially in the memory of any modern computer. A single writer handling 100 transactions per second (far more than any catalog workload demands) saturates the requirement. There is no need to distribute the workload across multiple nodes.

**Distributed systems require operational expertise.** Monitoring a distributed database requires understanding of replication lag, consensus latency, quorum health, network topology, and failure domains. Rocklake's operational model is "is the process running? Yes? Good." This is accessible to any engineer, not just distributed systems specialists.

**The failure mode is simpler.** A single-node system either works or it doesn't. Recovery is "restart the process." A distributed system can be in degraded states (one replica lagging, network between two nodes failing, leader election in progress) that require diagnosis and intervention. Simple failure modes mean faster recovery.

### If You Actually Need Distribution

If you genuinely need multi-region writes to the same catalog (rare for a metadata catalog), Rocklake is not the right tool. Consider:

- **PostgreSQL with logical replication** for multi-region metadata (accept eventual consistency)
- **CockroachDB** for strongly consistent multi-region catalog access
- **DuckLake's native PostgreSQL backend** if you already run a distributed PostgreSQL cluster

If what you actually need is high availability (not concurrent writers), Rocklake achieves this through fast failover: the writer restarts in seconds, and readers are unaffected. See [Deployment: High Availability](../deployment/high-availability.md).

## Not a Data Lake Manager

Rocklake records the existence and metadata of Parquet data files but does not manage their lifecycle. It does not:

- Create Parquet files (DuckDB writes them)
- Read Parquet file contents (DuckDB reads them)
- Compact small files into larger files (your ETL pipeline does this)
- Optimize file layout for query patterns (DuckDB's optimizer handles partition pruning)
- Delete data files when they are no longer referenced (your GC pipeline does this)
- Move files between storage tiers (S3 lifecycle policies do this)
- Validate file integrity (DuckDB checks checksums on read)

Rocklake is a catalog — a phone book for data files. It tells DuckDB "table X consists of these files with these schemas and these statistics." What happens to those files before and after their catalog registration is not Rocklake's concern.

### Why This Boundary Exists

**Data lifecycle is workload-specific.** An IoT time-series workload compacts differently than a marketing analytics workload. A real-time pipeline has different compaction triggers than a batch ETL. Embedding lifecycle management in the catalog would require either a one-size-fits-all strategy (suboptimal for everyone) or a complex policy engine (massive additional scope).

**Separation enables flexibility.** With Rocklake as a pure catalog, you can use any compaction strategy: Apache Spark's optimize, DuckDB's merge operations, custom scripts, or no compaction at all. The catalog does not care how files are managed — it only needs to be told about the results.

**Responsibility clarity.** When something goes wrong with data files (corruption, wrong format, missing files), the debugging surface is clear: the problem is either in the pipeline that created the files or in the catalog's record of them. If the catalog also managed file lifecycle, the failure could be in creation, management, or recording — a much larger debugging surface.

### What You Should Use Instead

- **DuckDB's COPY and INSERT** create Parquet files during ETL
- **S3 lifecycle policies** manage storage tiering and expiration
- **Apache Spark / dbt** handle compaction and optimization
- **Custom scripts with Rocklake's snapshot inspection** identify files eligible for cleanup

## Not an Access Control System

Rocklake provides optional password authentication for PostgreSQL wire protocol connections — you can require a password to connect. Beyond that, there is no access control. Once connected, a client has full access to all catalog operations. There are no:

- Table-level permissions (GRANT SELECT ON table TO user)
- Schema-level permissions (GRANT USAGE ON SCHEMA TO role)
- Row-level security policies
- Column-level masking rules
- Role-based access control (RBAC)
- Attribute-based access control (ABAC)
- Audit trails of who accessed what (beyond connection-level logging)

### Why This Boundary Exists

**The DuckLake protocol does not include access control.** DuckDB's `ducklake` extension connects as a single user and expects full catalog access. Even if Rocklake implemented fine-grained permissions, DuckDB would not use them. Implementing access control without client support is security theater — it provides the appearance of control without actual enforcement.

**Access control requires an identity model.** Who is "the user"? In a typical Rocklake deployment, "the user" is a DuckDB process running an ETL job on a compute cluster. That process does not have a human identity — it has a service account. Access control for service accounts is better handled at the infrastructure level (IAM roles, network policies, service mesh) than at the application level.

**The threat model does not match.** Rocklake's threat model is: "Prevent unauthorized network connections from mutating the catalog." Password authentication (or mTLS at the network layer) is sufficient for this threat. The threat model is NOT "allow multiple teams with different permissions to share one catalog instance" — that use case is better served by separate catalog instances per team.

**Access control is hard to get right.** Permission systems have subtle interactions (role inheritance, default permissions, permission caching, privilege escalation paths). Getting them wrong creates security vulnerabilities that are worse than having no access control at all (because users trust the system's permissions are enforced correctly).

### What You Should Use Instead

- **Network-level isolation:** VPCs, security groups, firewalls. Only authorized networks can reach Rocklake.
- **mTLS:** Mutual TLS ensures both client and server identity. Only clients with valid certificates can connect.
- **Separate instances:** Run one Rocklake per team/environment. No cross-team access is possible because there is no shared instance.
- **Proxy layer:** Run a reverse proxy (Envoy, HAProxy) in front of Rocklake that implements authentication, rate limiting, and access logging.

## Not a Streaming System

Rocklake does not support change data capture (CDC), event streaming, publish/subscribe, webhooks, or real-time notification of catalog changes. There is no way to "subscribe" to catalog mutations and receive push notifications when tables are created, schemas are altered, or files are registered.

The catalog provides a snapshot counter that increments on every write transaction. Clients can poll this counter to detect changes — but polling is not streaming. There is no push mechanism.

### Why This Boundary Exists

**Streaming requires persistent connections.** A push notification system requires maintaining open connections to all subscribers, tracking their state (which events have been delivered?), handling subscriber disconnection and reconnection, and managing backpressure when subscribers are slow. This is a full messaging system — significant additional scope.

**Delivery guarantees are complex.** Should notifications be at-most-once, at-least-once, or exactly-once? Each choice has different implementation requirements and failure modes. At-least-once requires acknowledgment tracking and redelivery. Exactly-once requires deduplication at the subscriber. These are solved problems (Kafka, Pulsar, NATS) but they are not simple to implement correctly.

**The use case is infrequent.** Catalog mutations happen a few times per minute (file registrations after ETL, occasional schema changes). At this frequency, polling every 5 seconds is indistinguishable from real-time notification. The latency difference between "push within 10ms" and "poll and discover within 5 seconds" is irrelevant for catalog consumers.

**Streaming systems already exist.** If you need event-driven reactions to catalog changes, write a polling service that publishes to Kafka/SQS/PubSub. This gives you all the delivery guarantees and scalability of a purpose-built messaging system, with Rocklake handling only what it does best (serving metadata).

### What You Should Use Instead

- **Polling:** Query the current snapshot ID every N seconds. If it changed, query the audit log for recent mutations.
- **External event pipeline:** A lightweight service polls Rocklake and publishes events to your messaging system (Kafka, SQS, EventBridge).
- **DuckDB triggers:** In your ETL pipeline, after successful catalog operations, emit events directly from the ETL code.

## Not Multi-Tenant

Rocklake does not support multiple isolated tenants within a single instance. Each instance serves one catalog backed by one SlateDB database at one object storage path. There is no concept of "tenant ID," no resource isolation between workloads, and no per-tenant configuration.

### Why This Boundary Exists

**Multi-tenancy requires isolation guarantees.** If tenant A's large transaction causes latency spikes, tenant B must not be affected. Implementing this requires resource accounting, priority queues, per-tenant memory limits, and circuit breakers. This is a substantial engineering effort that multiplies the testing matrix.

**Multi-tenancy requires security boundaries.** Tenant A must not be able to read or modify tenant B's data, even through bugs or exploits. This requires careful memory management (no shared buffers), separate encryption keys, and audit trails per tenant. A single-tenant system has no cross-tenant attack surface — because there is no other tenant.

**Multi-tenancy complicates operations.** Upgrades affect all tenants simultaneously. A bug in the catalog code potentially corrupts all tenants' data. Capacity planning must account for the sum of all tenants' requirements. Single-tenant instances provide blast radius isolation — one tenant's issues cannot propagate.

**The operational cost of separate instances is low.** Rocklake uses approximately 20 MB of memory at idle. Running 100 separate instances for 100 tenants requires 2 GB of memory — trivial on modern hardware. Container orchestrators (Kubernetes) make managing many small instances straightforward. The operational overhead of separate instances is vastly lower than the engineering complexity of multi-tenancy.

### What You Should Use Instead

Run one Rocklake instance per tenant (or per catalog). Use Kubernetes namespaces, separate AWS accounts, or container-per-tenant isolation to provide security boundaries. This gives you:

- Complete blast radius isolation (one tenant's corruption cannot affect others)
- Independent scaling (give heavy tenants more resources)
- Independent upgrades (canary deploy to one tenant before rolling out)
- Simple debugging (each tenant's logs are separate)
- Natural access control (network isolation per tenant)

## Summary: The Rocklake Scope

Rocklake is intentionally narrow. It does one thing: **serve DuckLake catalog metadata from object storage over the PostgreSQL wire protocol.** Every feature it implements serves this goal. Every feature it rejects would dilute this focus.

| Concern | Rocklake's Role | Better Tool |
|---------|-----------------|-------------|
| Query execution | Serve metadata to DuckDB | DuckDB |
| Application data | Not supported | PostgreSQL, MySQL |
| Distributed writes | Single writer only | CockroachDB, Spanner |
| File lifecycle | Record existence only | Spark, dbt, S3 lifecycle |
| Access control | Password auth only | IAM, mTLS, proxy |
| Event streaming | Polling only | Kafka, SQS, EventBridge |
| Multi-tenancy | One instance per tenant | Kubernetes isolation |

If your use case requires something from the "Better Tool" column, use that better tool alongside Rocklake — not instead of it. Rocklake's value is in solving its specific problem exceptionally well, not in being a mediocre solution to many problems.

## Further Reading

- **[Design Decisions: Bounded SQL](bounded-sql.md)** — Why SQL support is limited
- **[Design Decisions: Single Writer](single-writer.md)** — Why distributed writes are rejected
- **[Concepts: Single Writer, Many Readers](../concepts/single-writer-many-readers.md)** — The concurrency model in depth
- **[Deployment: High Availability](../deployment/high-availability.md)** — How to achieve HA without distribution
