# The SlateDB Storage Engine

SlateDB is the storage engine that makes Rocklake possible. It is an embedded key-value store written in Rust that persists all durable state to object storage — not to local disk, not to a network-attached volume, but to the same S3, GCS, or Azure Blob Storage bucket where your Parquet data lives. This seemingly simple architectural choice has profound consequences for how Rocklake operates: it means the catalog is as durable as the object store itself, it means the catalog has no local-disk requirements beyond ephemeral caching, and it means the catalog can be opened from any process that has network access to the bucket — a long-lived sidecar, a Lambda function, a Kubernetes pod, an edge worker.

Understanding SlateDB is essential to understanding Rocklake because every property that Rocklake advertises — crash safety, durability, horizontal read scale-out, single-writer consistency — is ultimately provided by SlateDB at the storage layer. Rocklake is the mapping between DuckLake's relational catalog model and SlateDB's key-value API; SlateDB is the engine that actually stores the bytes and guarantees their integrity.

## LSM Trees: The Foundation

SlateDB uses a Log-Structured Merge-tree (LSM tree) as its core data structure. If you have used LevelDB, RocksDB, or Cassandra, you have encountered LSM trees before. If not, the concept is straightforward and elegant.

An LSM tree optimizes for write throughput by never modifying data in place. Instead of updating a record on disk (which requires a random I/O seek to find the record and then a write to change it), an LSM tree appends every write to a sequential log. Writes are fast because they are always sequential appends — no seeking, no read-modify-write cycles. The trade-off is that reads must consult multiple layers of the tree to find the most recent version of a key, and background compaction processes periodically merge and sort these layers to keep read performance bounded.

In SlateDB's case, the "layers" are:

**The Write-Ahead Log (WAL).** Every write is first appended to a WAL segment — a single object in the bucket. WAL writes are durable the moment the object-store PUT returns successfully. This is the foundation of crash safety: if the process dies after the PUT succeeds, the write is recoverable; if it dies before, the write never happened. There is no intermediate state.

**The MemTable.** Recent writes are also held in an in-memory sorted data structure for fast reads. The MemTable is not durable on its own — it exists purely for performance. If the process crashes, the MemTable is reconstructed from the WAL segments on restart.

**Sorted String Tables (SSTs).** When the MemTable reaches a size threshold, it is flushed to the object store as an SST — a sorted, immutable file containing key-value pairs. SSTs are the long-term storage format. They are optimized for binary search and range scans: given a key prefix, you can quickly find all matching entries within an SST.

**Compaction.** Over time, multiple SSTs accumulate. Background compaction merges overlapping SSTs into larger, non-overlapping ones, eliminating obsolete versions of keys and reducing the number of files a read must consult. Compaction is transparent to the application — Rocklake does not need to trigger or manage it.

**The Manifest.** A single small file in the bucket that describes the current state of the database: which SSTs exist, which SST covers which key range, and what the current WAL position is. The manifest is the entry point for opening the database — a reader that opens the manifest knows exactly which SST files to read for any given key range.

## Object-Store-Native Persistence

The key insight of SlateDB's design is that every durable component — WAL segments, SST files, and the manifest — is stored as objects in the bucket. There is no local disk that holds persistent state. The local filesystem is used only for optional caching (keeping recently-read SST blocks in memory or on a local SSD to avoid repeated object-store fetches), but all cache data can be regenerated from the bucket at any time.

This has immediate practical consequences:

**Durability equals object-store durability.** When SlateDB reports that a write is committed, the bytes are stored in S3 with 11 nines of durability. There is no replication to configure, no backup to schedule, no point-in-time recovery to set up. The bucket is the backup.

**No persistent volumes required.** You can run Rocklake in environments that have no persistent local storage — Lambda functions, Fargate containers, spot instances that can be terminated at any time. The catalog survives process termination because it lives in the bucket, not on the instance.

**Any process can open the catalog.** As long as a process has network access to the bucket and appropriate credentials, it can open a SlateDB database and begin reading. There is no server to connect to, no port to expose, no handshake to perform. This is what enables the horizontal read scale-out model: readers are just processes that open the manifest and read from SSTs.

## Atomic Write Batches

SlateDB provides two APIs for writes that Rocklake uses:

**`WriteBatch`** — A set of key-value puts that are committed atomically. Either all puts in the batch succeed, or none do. If the process crashes during a WriteBatch, the entire batch is either present in the WAL (and thus committed) or absent (and thus never happened). There is no partial state.

**`DbTransaction`** — A higher-level transactional API that supports read-your-writes semantics within the transaction, conflict detection, and atomic commit. Rocklake uses `DbTransaction` for catalog operations that need to read existing state before deciding what to write (for example, allocating a counter value and then writing a row that uses that value).

The atomicity of these write operations is what gives Rocklake its crash-safety guarantee. When DuckDB sends a `COMMIT` to Rocklake, the pending catalog mutations are assembled into a single `DbTransaction` that includes every INSERT and UPDATE from the current transaction. That transaction is committed to SlateDB as a single atomic operation — one WAL segment write, one PUT to the object store. Either the entire transaction persists, or it does not. A crash at any point during the process leaves the catalog in a clean state.

## Single-Writer Enforcement and Fencing

SlateDB enforces that at most one process may write to a given database at a time. This is not a soft convention — it is an actively enforced constraint with a fencing mechanism. When a writer opens a SlateDB database, it registers itself as the current writer (by writing a fencing token to the manifest). If a second process tries to open the same database for writing, the first writer's subsequent write attempts will fail with a fencing error.

This mechanism prevents a dangerous failure mode common in distributed systems: split-brain, where two processes both believe they are the authoritative writer and make conflicting modifications. With SlateDB's fencing, this cannot happen. The first writer is fenced off the moment the second writer takes over, and any in-flight writes from the first writer that have not yet committed are guaranteed to fail.

Rocklake maps SlateDB's fencing error to `SQLSTATE 57P04` (connection failure), which DuckDB interprets as "the server went away — reconnect." The DuckDB client automatically retries its connection, reaches the new writer, and continues operation. From the client's perspective, a writer failover looks like a brief connection interruption.

The takeover protocol is deterministic:

1. The new writer opens the catalog and calls `flush()` to establish a durable baseline.
2. The manifest is updated with the new writer's fencing token.
3. Any subsequent write attempts by the old writer fail immediately with a fencing error.
4. The new writer begins accepting client connections.

On S3 Standard, this takeover process completes in roughly 30–60 seconds (dominated by the time to flush and update the manifest). On S3 Express One Zone, it completes in roughly 10–15 seconds due to lower object-store latency.

## Read Paths: DbReader and DbSnapshot

SlateDB provides multiple read APIs that Rocklake uses for different scenarios:

**`DbReader`** opens a read-only view of the database against the current manifest. It can read any key or scan any key range, seeing all data that was committed and flushed at the time it was opened. Readers do not interfere with the writer and do not interfere with each other — they are completely independent processes that read immutable SST files directly from the object store.

**`DbSnapshot`** is similar to `DbReader` but pinned to a specific point in time. It sees exactly the state that was committed at that point, regardless of what the writer does afterward. Rocklake uses snapshots for long-running queries that should not be affected by concurrent writes.

The key property of both read APIs is that they operate entirely on immutable data. SST files, once written, are never modified. A reader that opens a set of SST files will see consistent data regardless of what compaction processes or new writes are happening concurrently. This is the fundamental enabler of Rocklake's horizontal read scale-out: adding more readers does not require any coordination with the writer because readers and writers operate on different (immutable) data.

## The Flush Visibility Barrier

There is a subtle but important concept in SlateDB's consistency model: the distinction between "committed" and "visible to readers." When a `DbTransaction` commits successfully, the write is durable — it is in the WAL and will survive a crash. But it is not yet visible to new readers that open the database after the commit. Visibility requires a `flush()` operation that advances the manifest to include the new WAL entries.

Rocklake calls `flush()` after every committed transaction to ensure that subsequent readers see the latest state. Without this call, a reader that opens immediately after a writer's commit might see a stale state because the manifest has not yet been updated. The `flush()` call is the visibility barrier that separates "written" from "readable."

This is analogous to PostgreSQL's WAL flush versus visibility: in PostgreSQL, a committed transaction is durable in the WAL but not visible to other sessions until the WAL is applied. In SlateDB, a committed write is durable in the WAL but not visible to readers until `flush()` advances the manifest.

## What SlateDB Does Not Provide

Understanding SlateDB's boundaries helps explain why Rocklake is built the way it is:

**No built-in SQL.** SlateDB is a key-value store. It does not parse SQL, plan queries, or evaluate expressions. Rocklake implements its own key layout and bounded SQL dispatcher on top of SlateDB's key-value API.

**No multi-writer support.** One writer per database, period. Rocklake's single-writer model is a direct consequence of this constraint.

**No multi-region replication.** SlateDB reads from a single object-store bucket. Cross-region availability requires object-store-level replication (like S3 Cross-Region Replication), not SlateDB-level replication.

**No built-in encryption.** SlateDB provides a block transformer API that Rocklake can use to encrypt data at the SST level, but key management and encryption policy are the application's responsibility.

**No secondary indexes.** SlateDB supports only primary-key lookups and range scans. Rocklake's key layout is designed to make the most common query patterns efficient without secondary indexes.

These constraints are not shortcomings — they are deliberate boundaries that keep SlateDB small, correct, and focused. Each constraint shaped Rocklake's design in a specific way, and understanding the constraint helps understand the design choice.

## Further Reading

- **[Catalog Immutability](immutability.md)** — How Rocklake leverages SlateDB's immutable SST files to provide infinite time travel
- **[Architecture: Transaction Model](../architecture/transaction-model.md)** — How Rocklake uses `DbTransaction` for catalog atomicity
- **[Design Decisions: Why SlateDB?](../design-decisions/why-slatedb.md)** — The full comparison with PostgreSQL, SQLite, FoundationDB, and other alternatives
- **[Performance: Latency Model](../performance/latency-model.md)** — How SlateDB's object-store-based persistence affects operation latency
