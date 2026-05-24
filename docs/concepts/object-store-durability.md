# Object Store Durability

SlateDuck trusts object storage — Amazon S3, Google Cloud Storage, Azure Blob Storage — as its durable persistence layer. This is a deliberate architectural choice, and it is not obvious why a database would delegate durability to a service best known for storing images and backup archives. The answer requires understanding what modern object storage actually provides under the hood, how SlateDB bridges the gap between a key-value API and object storage semantics, and what the practical implications are for latency, cost, failure modes, and operational burden. Get this mental model right, and SlateDuck's entire operational story becomes clear.

## Why Object Storage?

The conventional wisdom in database design is that durable storage means a local disk — an NVMe drive, a RAID array, a SAN. Local disk provides microsecond write latency, byte-granular writes, and the ability to `fsync` individual files for crash safety. It is the foundation that PostgreSQL, MySQL, SQLite, and essentially every other database engine assumes. So why does SlateDuck use S3 instead?

**Extreme durability, built in.** S3 Standard provides 99.999999999% (eleven nines) annual object durability. To understand what this means intuitively: if you store 10 million objects, you can expect to lose one of them once every ten thousand years. Google Cloud Storage and Azure Blob Storage provide comparable guarantees. No self-managed system — no RAID configuration, no replication topology, no tape backup — can match this without heroic effort and significant ongoing investment. Object storage achieves this through cross-availability-zone replication that happens automatically, invisibly, and at no additional configuration cost to you.

**Zero operational burden.** You do not provision disk capacity, manage replication factors, schedule SMART tests, handle disk replacements, patch storage firmware, configure RAID controller cache policies, or worry about what happens when a physical host fails. The cloud provider handles all of this transparently. For a team deploying SlateDuck as part of a larger data platform, eliminating an entire category of infrastructure operations is not a minor convenience — it is a fundamental reduction in the operational surface area.

**Linear cost scaling with no idle cost.** You pay for what you store and for the requests you make. There is no minimum instance size, no reserved capacity charge, and no cost for an idle storage server waiting for work. A catalog that sees only occasional writes costs pennies per month. One that sees thousands of writes per second costs proportionally more, but there is no step function where you suddenly need to provision a larger tier. This aligns perfectly with SlateDuck's serverless deployment model.

**Unlimited capacity without planning.** You do not need to estimate how large your catalog will grow and provision accordingly. Object storage expands to accommodate whatever you store. A catalog that starts with a thousand entries and grows to a hundred million over five years does not require any capacity planning or migration.

**Native concurrent read support.** Object stores are designed from the ground up to serve millions of simultaneous GET requests. Multiple reader processes opening the same SlateDB instance and reading the same SST files are, from the object store's perspective, just multiple clients reading the same objects — a completely native, expected use case. There is no "concurrent reader" configuration needed, no read replica setup, no connection pool tuning. The concurrency is inherent.

## How SlateDB Uses Object Storage

SlateDB is an LSM-tree (Log-Structured Merge-Tree) key-value engine that maps its internal storage abstractions onto object storage operations. Understanding this mapping is key to understanding SlateDuck's performance characteristics.

An LSM-tree works by accumulating writes in an in-memory buffer, periodically flushing that buffer to an immutable sorted file on disk (called an SST — Sorted String Table), and periodically merging smaller SSTs into larger ones through a background compaction process. In traditional implementations, "disk" means a local filesystem. SlateDB replaces every filesystem operation with an equivalent object storage operation:

| SlateDB Concept | Object Storage Operation |
|-----------------|------------------------|
| Write-Ahead Log (WAL) segment | Atomic PUT of a segment file |
| Sorted String Table (SST) | Atomic PUT of an immutable file |
| Manifest | PUT + conditional GET (optimistic locking) |
| Point read | GET + binary search within cached SST block |
| Prefix scan | Sequential GETs across relevant SST blocks |
| Compaction | Read old SSTs + write new SST + update manifest |
| Checkpoint | Manifest PUT that makes current state visible to new readers |

The critical insight is that object storage provides **atomic PUT semantics**: when you PUT an object, either the operation completes and the full object is available, or the operation fails and the object does not exist. There is no partial write, no half-written file, no torn page. This is actually a stronger guarantee than most local filesystem implementations provide for arbitrary files (local crash safety typically requires careful use of `fsync` and rename operations). SlateDB's crash safety derives directly from this property: because every WAL segment and SST is written as an atomic PUT, a crash at any point leaves the catalog in a state where every object is either fully present or fully absent. Partial writes do not exist.

## The Durability Chain

When SlateDuck commits a catalog transaction, durability is established through a sequence of operations:

1. The pending mutations are serialized into Protobuf and assembled into a WAL segment.
2. The WAL segment is PUT to the object store. The PUT operation is synchronous and blocks until the object store confirms durability (at least two AZ-replicated copies for S3 Standard).
3. The `flush()` call updates the SlateDB manifest to include the new WAL segment, making it visible to readers opening new read views.
4. The PG-wire `COMMIT` response is sent to the client.

Only after step 4 does DuckDB consider the transaction committed. This means that when DuckDB receives a successful COMMIT response, the catalog mutation is durable in at least two availability zones. A crash of the SlateDuck process, the host machine, a network switch, or even an entire availability zone cannot lose the mutation.

The object store's eleven-nines durability guarantee kicks in at step 2. From the moment the WAL PUT completes, the data is protected by the cloud provider's cross-AZ replication. SlateDuck's durability is bounded below by the cloud provider's SLA and above by nothing — there is no weaker link in the chain.

## Latency Implications

The trade-off for extreme durability and zero operational burden is latency. Object storage operations are slower than local disk by a large margin:

| Operation | Local NVMe | S3 Standard | S3 Express One Zone |
|-----------|-----------|-------------|---------------------|
| Single small PUT | 50–200 µs | 10–50 ms | 2–5 ms |
| Single small GET | 10–100 µs | 5–30 ms | 1–3 ms |
| LIST (100 objects) | 1–5 ms | 10–50 ms | 5–15 ms |
| Point read (cached) | Sub-millisecond | Sub-millisecond (from cache) | Sub-millisecond (from cache) |

These latency numbers translate directly to SlateDuck's catalog operation latencies. A catalog write (committing a transaction) involves a WAL PUT — typically 10–50 milliseconds on S3 Standard. A catalog read (listing data files for a table) involves reading one or more SST blocks — often served from SlateDB's in-memory block cache after warmup, but cold reads take 5–30 milliseconds.

For the DuckLake use case, these latencies are generally acceptable. A user running `CREATE TABLE` or `COPY ... FROM` is performing a once-per-batch operation; adding 50 milliseconds to a batch job that takes seconds or minutes is inconsequential. A query that reads one terabyte of Parquet data will spend most of its time on the data scan, not on the 50-millisecond catalog lookup. The catalog is a small minority of the total query time for any non-trivial analytical workload.

Where S3 Standard latency does matter is for workflows that issue many short catalog operations in rapid succession — schema inspection tools, catalog browsers, or orchestration systems that rapidly create and drop temporary tables. For these workloads, S3 Express One Zone (which provides 5–10× lower latency at somewhat higher cost) is worth evaluating. See [Deployment: AWS S3](../deployment/aws-s3.md) for configuration details.

## How SlateDuck Manages Latency

SlateDuck uses several strategies to minimize the effective per-operation latency seen by DuckDB:

**Transaction batching.** Multiple catalog mutations within a single DuckDB transaction — for example, registering 50 data files and updating table statistics — are accumulated in memory and written in a single WAL PUT. A single 100-row transaction pays the cost of one PUT, not 100.

**Block cache.** SlateDB maintains a configurable in-memory block cache (default: 128 MiB) that stores recently read SST blocks. Popular tables — ones that many DuckDB clients query frequently — have their catalog entries kept warm in cache, reducing repeated reads to sub-millisecond. Cold reads on first access pay the full GET latency; subsequent reads are served from memory.

**Hot key packing.** The current snapshot ID and table counts are stored in a small number of keys that are accessed on almost every operation. These remain hot in the block cache at all times after the first access.

**Secondary index for file lookups.** The secondary index at tag `0xFC` makes the most common query — "what files are visible at the current snapshot for table X?" — faster for large tables by avoiding a full prefix scan with MVCC filtering. For tables with thousands of files across many snapshots, this difference is significant.

## Consistency Model

Object storage provides two consistency guarantees that SlateDB relies on:

**Read-after-write consistency for new objects.** After a PUT completes, any subsequent GET of the same object returns the current value. There is no eventual-consistency window for new objects on modern object stores — AWS achieved strong read-after-write consistency for S3 in 2020, and GCS and Azure have always provided it.

**Strong list consistency.** A LIST operation returns all objects that have been successfully PUT. There is no "recently PUT objects might not appear in LIST" window. SlateDB uses LIST operations during manifest reconciliation to discover new SST files, and relies on this guarantee for correctness.

These consistency properties, combined with the atomic PUT semantics, give SlateDB the foundation it needs to maintain catalog integrity across crashes and concurrent readers.

## Failure Modes and Recovery

Because SlateDuck delegates durability to object storage, its failure modes are simpler than those of a database with local storage:

**Object storage unavailable (network partition or cloud outage).** SlateDuck cannot read or write catalog entries. Incoming DuckDB connections fail. Connections in progress receive errors. No data is lost — all committed data is safely stored in the object store. When the outage resolves, restart SlateDuck and it resumes from the latest durable state. No recovery procedure is needed.

**SlateDuck process crash.** The latest WAL segments are in the object store. On restart, SlateDB reads the manifest and replays any WAL segments committed but not yet compacted. The catalog is fully consistent as of the last committed transaction. DuckDB clients that were mid-transaction when the crash occurred will see a disconnection error; they need to retry their transactions. Transactions that had committed before the crash are not affected.

**Partial WAL segment (crash during PUT).** The object store's atomic PUT semantics mean this either does not happen (the PUT completed atomically) or the segment does not exist (the PUT failed before completing). SlateDB treats a missing WAL segment as if it was never written and replays from the previous consistent manifest. No partial segment is ever read.

**Host machine loss.** Because all durable state lives in the object store, you can start SlateDuck on a completely different host pointed at the same bucket prefix and it will recover the full catalog state. There is no local state to transfer, no data to copy, no recovery procedure to follow. This is the "serverless" property in its most concrete form.

**Object storage data loss.** This is the only failure mode that results in permanent data loss. Object storage providing eleven-nines durability means the probability of this for any individual object over a year is 0.000000001% — statistically negligible but not zero. For organizations with regulatory requirements that cannot accept any risk of data loss, cross-region replication (S3 Cross-Region Replication, GCS Multi-Region, Azure GRS) provides an additional layer of protection at the storage level, completely transparent to SlateDuck.

## Cross-Region Durability

For the highest durability requirements, you can enable cross-region replication of your bucket. From SlateDuck's perspective, nothing changes: it still reads and writes to the same bucket using the same API. The cloud provider handles the cross-region replication asynchronously in the background. Any catalog mutation committed to the primary region is eventually replicated to the secondary region, providing recovery capability even in the event of a regional catastrophe.

The practical consideration for cross-region replication is additional latency and cost on writes: each PUT must replicate to the secondary region before being marked complete (for synchronous replication) or after (for asynchronous replication with eventual consistency). Most deployments use asynchronous replication and accept a small risk window in exchange for normal write latency. The catalog's MVCC model and transaction semantics are unaffected by replication mode — the semantics are fully determined by what is visible at the primary region.

## Local Development Without Cloud Storage

When developing and testing SlateDuck without cloud credentials, you can use the local filesystem backend. SlateDuck supports a `file://` path prefix that stores all SST files and WAL segments as regular files on disk. This provides the same key-value API and MVCC semantics but with local-filesystem latency. The durability guarantees are different (local filesystem, not cloud object store), but for development purposes this is usually acceptable.

See [Quickstart (Local)](../getting-started/quickstart.md) for setup instructions, and [Deployment: Binary](../deployment/binary.md) for the full configuration options for local-filesystem deployments.

## Further Reading

- **[SlateDB Storage Engine](slatedb.md)** — A deeper treatment of the LSM-tree engine that sits between SlateDuck and object storage
- **[Deployment: AWS S3](../deployment/aws-s3.md)** — Complete guide to deploying against S3, including S3 Express One Zone for low-latency workloads
- **[Deployment: GCS](../deployment/gcs.md)** — Google Cloud Storage deployment guide
- **[Deployment: Azure](../deployment/azure.md)** — Azure Blob Storage deployment guide
- **[Performance: Latency Model](../performance/latency-model.md)** — Quantitative analysis of catalog operation latency across backends

## Why Object Storage?

Object stores like S3 offer a unique combination of properties that make them attractive as a persistence layer for metadata:

**Extreme durability.** S3 Standard provides 99.999999999% (11 nines) durability. This means if you store 10 million objects, you can expect to lose one object every 10,000 years. GCS and Azure Blob provide similar guarantees. No self-managed database can match this without heroic effort.

**Zero operational burden.** You do not provision capacity, manage replication, handle failover, patch operating systems, or worry about disk failures. The cloud provider handles all of this transparently.

**Linear cost scaling.** You pay only for what you store and what you access. There is no minimum instance size, no reserved capacity, no idle cost for an underutilized database server.

**Unlimited storage.** There is no practical limit to how much data you can store in a bucket. Your catalog can grow without capacity planning.

**Built-in availability.** S3 Standard provides 99.99% availability. Multi-AZ replication is handled automatically.

## How SlateDB Uses Object Storage

SlateDB, the LSM-tree engine that SlateDuck uses for catalog persistence, maps its storage abstractions onto object storage operations:

| SlateDB Concept | Object Storage Operation |
|-----------------|------------------------|
| Write-Ahead Log (WAL) entry | Single PUT of a segment file |
| Sorted String Table (SST) | Single PUT of a file |
| Manifest | PUT + conditional GET (optimistic) |
| Point read | GET + binary search within SST |
| Prefix scan | Multiple GETs (range of SSTs) |
| Compaction | Read old SSTs + write new SST + update manifest |

The critical insight is that object storage provides **atomic PUT**: a PUT operation either succeeds completely or has no effect. There is no partial write. This gives SlateDB its crash-safety guarantee: a WAL segment is either fully durable or absent.

## Durability Guarantees

Once SlateDuck's `commit` returns successfully, the catalog mutation is durable. Specifically:

1. The write was accepted by the WAL (a PUT to object storage completed successfully)
2. The cloud provider has replicated the bytes to at least two availability zones (for S3 Standard)
3. The data will survive any single-facility failure, including total loss of a data center

This means SlateDuck's durability is bounded by your cloud provider's SLA, not by any aspect of the SlateDuck software. If S3 loses your data, that's an S3 problem, not a SlateDuck problem.

## Latency Implications

The trade-off for extreme durability is latency. Object storage operations are slower than local disk:

| Operation | Local NVMe | S3 Standard | S3 Express |
|-----------|-----------|-------------|------------|
| Single write | 10-100 us | 20-100 ms | 3-10 ms |
| Single read | 10-100 us | 10-50 ms | 2-8 ms |
| Prefix scan (10 keys) | 100 us | 50-150 ms | 10-30 ms |

SlateDuck mitigates this through several strategies:

- **Write batching:** Multiple catalog operations in a single DuckDB transaction become one WAL segment (one PUT)
- **Hot key caching:** The most frequently accessed metadata (current snapshot, file counts) is packed into a single key
- **Secondary indexes:** Snapshot-scoped file lookups use a purpose-built index that avoids scanning all files
- **SlateDB block cache:** Recently read SST blocks are cached in memory, avoiding repeated GETs

For most interactive workloads, the overhead is acceptable: catalog operations take 50-200ms against S3 Standard, which is fast enough for DDL operations that happen infrequently. For latency-sensitive workloads, S3 Express One Zone reduces this to 5-20ms.

## Consistency Model

Object storage provides **read-after-write consistency** for new objects (you can read an object immediately after writing it) and **strong consistency** for list operations (a list returns all objects that have been successfully PUT). SlateDuck relies on both of these guarantees for correct operation.

SlateDB's manifest provides the additional ordering guarantee: readers discover new SSTs by reading the manifest, which is updated atomically after new SSTs are written. This ensures readers never see partial compaction results or orphaned files.

## Failure Modes

Because SlateDuck delegates durability to object storage, its failure modes are:

1. **Object storage unavailable:** SlateDuck cannot read or write. Operations fail with retriable errors. No data is lost. Resume when the outage resolves.
2. **Network partition:** Same as unavailable — SlateDuck cannot reach storage. Fail, retry, resume.
3. **SlateDuck process crash:** Catalog state is fully persistent. Restart the process and it resumes from the latest durable state.
4. **Object storage data loss:** Extremely unlikely (11 nines durability), but if it occurs, there is no local backup to recover from. This is the same risk you accept with any cloud-native architecture.

## Cross-Region Durability

For the highest durability requirements, you can use cross-region replication features of your cloud provider (S3 Cross-Region Replication, GCS Multi-Region, Azure GRS). SlateDuck does not need to know about this — it is handled transparently at the storage layer.
