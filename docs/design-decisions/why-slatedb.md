# Why SlateDB?

The most fundamental architectural decision in Rocklake is the choice of persistence engine. Everything else — the key encoding, the MVCC model, the deployment topology, the operational procedures — flows from this choice. SlateDB is a Rust-native LSM-tree key-value store that writes directly to object storage (S3, GCS, Azure Blob), with no local disk required for durability. This page explains why SlateDB was chosen over a field of strong alternatives, examines the consequences in detail, and honestly addresses the costs.

## The Requirements

Before evaluating options, we established what the persistence layer must provide:

1. **Object storage as the durable layer.** No local disk dependency for durability. The catalog must survive instance termination, pod eviction, and spot instance reclamation. If the process dies, nothing is lost.

2. **Atomic multi-key writes.** A catalog transaction touches multiple keys (table row + column rows + counter increment + snapshot creation). These must commit atomically — all or nothing. Partial commits would corrupt the catalog.

3. **Efficient prefix scans.** The primary access patterns are "list all tables in schema X" and "list all columns in table Y". These are prefix scans over sorted keys. They must be efficient without secondary indexes.

4. **Fast point reads.** Fetching a single counter or system key (writer epoch, retain_from) must complete in a single read operation, not require scanning.

5. **Crash safety without recovery procedures.** On restart after a crash, the catalog must be immediately usable without running recovery, replaying logs, or rebuilding indexes. The only recovery action should be "start the process again."

6. **Concurrent readers without coordination.** Multiple DuckDB instances (via the native extension) or multiple processes reading the catalog for monitoring should work without acquiring locks or communicating with the writer.

7. **Rust-native with async support.** First-class Rust API with native async/tokio integration. No FFI bridges to C++ libraries, no unsafe blocks for basic operations, no build-time complexity from foreign code.

## Alternatives Considered

### FoundationDB

FoundationDB is arguably the best distributed key-value store ever built. Its transaction model is correct, performant, and well-tested by Apple at enormous scale.

**Why not:** FoundationDB requires a cluster of 3+ servers for its coordination layer. This violates requirement #1 — Rocklake must work with nothing but object storage. Running a FoundationDB cluster adds operational complexity that defeats the purpose of a "deploy one binary and a bucket" system.

### RocksDB

RocksDB is the industry-standard embedded LSM-tree. It is mature, fast, well-understood, and battle-tested at Facebook, Netflix, and thousands of other companies.

**Why not:** RocksDB writes to local disk. To achieve durability on object storage, you would need a replication layer (snapshot to S3 periodically, or use a FUSE filesystem). This is complex, fragile, and introduces a window of potential data loss between local writes and remote replication. Additionally, the Rust bindings (`rust-rocksdb`) are C++ FFI with complex build requirements (requires librocksdb, C++ compiler, platform-specific build scripts). This conflicts with requirement #7.

### SQLite

SQLite is the world's most deployed database. It is lightweight, zero-configuration, and extremely reliable.

**Why not:** SQLite requires a local filesystem with POSIX locking semantics. It does not support concurrent access from multiple processes without a network filesystem (which introduces its own problems). More fundamentally, DuckLake already has a SQLite backend — Rocklake exists specifically to provide an alternative that does not require local filesystem state.

### Custom LSM-Tree Implementation

We could build a custom LSM-tree tailored exactly to Rocklake's needs, writing SST files directly to object storage.

**Why not:** Building a correct, performant LSM-tree is a multi-year engineering effort. It involves compaction scheduling, bloom filter tuning, block cache management, write-ahead log design, and manifest file management. SlateDB provides all of this already, maintained by a dedicated team. The build-vs-buy calculus strongly favors using an existing implementation.

### DynamoDB / Cloud Firestore / Cosmos DB

Cloud-managed key-value stores offer zero-ops persistence with strong consistency guarantees.

**Why not:** These services are proprietary and not self-hostable. Using DynamoDB would tie Rocklake to AWS permanently. Users running on GCS, Azure, or on-premises would be unable to use it. The `object_store` crate provides a provider-agnostic abstraction over S3/GCS/Azure — this portability is essential.

### TiKV

TiKV is a distributed key-value store (the storage layer of TiDB) with strong consistency and transaction support.

**Why not:** Like FoundationDB, TiKV requires a cluster (3+ Placement Driver nodes + multiple TiKV nodes). It is designed for large-scale distributed workloads, not for a single-writer metadata catalog. The operational overhead is disproportionate to the workload.

### Sled

Sled is a Rust-native embedded database with a modern design and good ergonomics.

**Why not:** Sled writes to local disk (like RocksDB) and does not support object storage backends. It also has a history of correctness issues that make it unsuitable for a data integrity-critical system. The project's maintenance status has been uncertain.

## Why SlateDB Won

SlateDB meets all seven requirements directly and elegantly:

| Requirement | How SlateDB Satisfies It |
|-------------|--------------------------|
| Object storage durability | Writes WAL segments and SST files directly to any `object_store` backend |
| Atomic multi-key writes | `WriteBatch` commits multiple key-value pairs atomically in one PUT |
| Efficient prefix scans | Keys are sorted; seek to prefix start, iterate until prefix end |
| Fast point reads | Bloom filters + binary search within SST blocks; single GET path |
| Crash safety | WAL provides atomic PUT semantics; no recovery procedure needed |
| Concurrent readers | Immutable SST files support unlimited concurrent readers via manifest |
| Rust-native async | Pure Rust library with native tokio runtime integration |

Beyond the core requirements, SlateDB offers additional advantages:

**Active maintenance.** SlateDB is maintained by the team building SlateDB Cloud, which means sustained investment in correctness, performance, and cloud-native features. The project has a roadmap that aligns with Rocklake's needs (caching, tiered storage, compaction optimization).

**Clean API.** SlateDB's API is small and well-designed: `get`, `put`, `delete`, `write_batch`, `scan`. There are no complex configuration knobs that require deep LSM expertise to tune correctly.

**Provider agnostic.** Through the `object_store` crate, SlateDB works identically on S3, GCS, Azure Blob Storage, and local filesystem. Rocklake inherits this portability without any provider-specific code.

**Compaction handled internally.** SlateDB manages its own compaction (merging small SST files into larger ones) without Rocklake needing to schedule or monitor it. This keeps the operational model simple.

## Consequences of This Choice

### Positive Consequences

**Zero infrastructure for persistence.** The only infrastructure Rocklake needs is an object storage bucket. No databases to provision, no disks to size, no replicas to configure, no backups to schedule (object storage is the backup). This dramatically reduces operational burden.

**Cloud-provider durability SLAs.** S3 offers 99.999999999% (11 nines) durability. GCS and Azure offer similar guarantees. Rocklake inherits these guarantees directly. No replication strategy we could build ourselves would match this.

**No local state to manage.** The Rocklake process is stateless (all durable state is in object storage). This means:

- Instances can be killed and restarted freely
- Scaling down does not require draining
- Migration between hosts requires no data movement
- Container deployments need no persistent volumes

**Simple deployment model.** One binary + one bucket path = a running catalog. No cluster membership, no configuration files referencing peer nodes, no split-brain scenarios.

**Horizontal read scale-out.** Because SST files are immutable objects in storage, multiple readers can access them concurrently through separate SlateDB instances. The manifest (which lists current SST files) is a small, frequently-cacheable file.

### Negative Consequences

**Higher per-operation latency than local disk.** A point read from S3 Standard takes 20–100ms (first-byte latency). A local-disk RocksDB read takes microseconds. This is a 1,000–10,000x difference for individual operations.

**Dependency on cloud provider availability.** If S3 is down, the catalog is unavailable. S3 has excellent availability (99.99% SLA, often better in practice), but it is not 100%.

**Limited control over compaction.** SlateDB's compaction runs on its own schedule. Rocklake cannot force immediate compaction or fine-tune compaction parameters for specific workload patterns (though this is rarely needed).

**Newer project than alternatives.** SlateDB has less production mileage than RocksDB (which has run at Facebook-scale for over a decade). The risk of undiscovered bugs is higher, though mitigated by extensive testing.

**No built-in secondary indexes.** SlateDB provides sorted key-value pairs and prefix scans. If Rocklake needs a different access pattern (e.g., "find all tables named X across all schemas"), it must implement its own indexing on top of the sorted key space.

## Mitigating the Negatives

### Latency

The latency cost is mitigated through multiple strategies:

- **Write batching:** A catalog transaction batches all mutations into a single `WriteBatch`. One PUT to S3, not one per row. A transaction that creates a table with 50 columns makes one 10KB write, not 51 individual writes.

- **Hot key caching:** SlateDB caches frequently-read blocks in memory. System keys (epoch, retain_from, latest_snapshot) are read on almost every operation and stay cached.

- **S3 Express One Zone:** For latency-sensitive deployments, S3 Express provides 3–10ms first-byte latency (vs. 20–100ms for S3 Standard). This brings SlateDB reads to within 5–15ms.

- **Acceptable in context:** Rocklake serves lakehouse metadata. The DuckDB queries it enables typically scan gigabytes of Parquet data (taking seconds to minutes). 20ms of catalog overhead is negligible in this context.

### Availability

The availability dependency is acceptable because Rocklake targets cloud-native deployments where object storage availability is a given baseline assumption. If S3 is down, your Parquet data files are also inaccessible — DuckDB cannot run queries regardless of whether the catalog is available. The catalog being unavailable during an S3 outage is not an additional failure mode.

### Maturity

The maturity concern is addressed through:

- **Rocklake's own test suite:** Property-based tests, golden tests, and integration tests exercise SlateDB under various failure conditions (crashes mid-write, corrupt manifests, concurrent access).
- **SlateDB's CI:** The SlateDB project runs continuous correctness tests including crash testing and fuzzing.
- **Conservative usage:** Rocklake uses a small subset of SlateDB's API surface (`get`, `put`, `delete`, `write_batch`, `scan`). The risk of hitting edge-case bugs is reduced by using only well-tested code paths.

## Further Reading

- **[Architecture: Overview](../architecture/overview.md)** — How SlateDB fits into the overall architecture
- **[Concepts: SlateDB](../concepts/slatedb.md)** — Technical details of the storage engine
- **[Performance: Latency Model](../performance/latency-model.md)** — Quantified latency analysis
- **[Single-Writer Model](single-writer.md)** — How the writer interacts with SlateDB
