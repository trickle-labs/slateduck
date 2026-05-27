# Writer Fencing

The single-writer constraint is one of SlateDB's core architectural guarantees, and it is the property that makes Rocklake's consistency model possible without distributed consensus. At most one process may write to a given catalog at any time. This sounds like a severe limitation — and for some workloads, it is a meaningful constraint to design around — but it is also the reason that Rocklake can provide linearizable writes, crash-safe recovery, and unlimited reader scale-out without any of the infrastructure complexity that multi-writer systems require.

This page explains what the single-writer constraint means operationally, why it was chosen over multi-writer alternatives, how the fencing mechanism works when a writer needs to fail over to a new process, what the recovery latency looks like in practice, and how to design your deployment to work within the constraint rather than fighting against it.

## Why Single-Writer?

The most obvious way to make a distributed system handle concurrent writes from multiple processes is to introduce a consensus protocol — Raft, Paxos, or multi-version optimistic concurrency control with retry logic. These protocols work, but they come with significant complexity: leader election, log replication, split-brain handling, membership changes, and subtle failure modes that can cause data loss if not implemented correctly. Every multi-writer system is also a distributed consensus system, whether it acknowledges that complexity or not.

Rocklake chose the opposite approach: accept the constraint that writes are serialized through a single process, and derive all other properties from that simplification. If there is only one writer, there are no write-write conflicts to resolve. If there are no conflicts, there is no need for optimistic locking, retry loops, or conflict resolution logic. If writes are serialized, readers always see a consistent linear history — there is no "concurrent branch" that needs to be merged.

The single-writer model means that Rocklake's consistency model is trivially linearizable for writes: every write happens in a total order defined by the writer's commit sequence. For a catalog workload — where the write rate is modest (schema changes and file registrations happen at human or batch-pipeline timescales, not at millions of operations per second) — this simplification eliminates an entire category of distributed systems complexity without meaningful throughput limitations.

## What Happens When the Writer Fails

In any single-writer system, the critical question is: what happens when the writer process dies? The catalog cannot accept writes until a new writer takes over, so the duration of the writer vacancy directly affects write availability. Rocklake addresses this through SlateDB's fencing mechanism and a deterministic takeover protocol.

### The Fencing Mechanism

When a Rocklake writer opens a catalog, it registers itself as the current writer by writing a fencing token to the SlateDB manifest. This token is a unique identifier (typically a UUID or epoch number) that identifies the current authoritative writer. Any subsequent write operation includes this token, and SlateDB verifies that the token matches the registered writer before accepting the write.

If a second Rocklake process opens the same catalog for writing (for example, because a Kubernetes pod was restarted and the new pod started before the old one fully terminated), the following sequence occurs:

1. The new process opens the catalog and attempts to register as the writer.
2. SlateDB updates the manifest with the new writer's fencing token.
3. The old writer's next write attempt fails with a fencing error because its token no longer matches the registered writer.
4. The old writer receives this error, logs it, and stops accepting new client connections.

This is a hard fencing mechanism — not a "best effort" lease that might expire — and it guarantees that at no point are two writers both successfully committing writes. The fencing error is immediate and deterministic: once the manifest is updated, the old writer cannot succeed regardless of timing, network delays, or retry attempts.

### The Takeover Protocol

When the new writer takes over, it follows a specific sequence to ensure consistency:

1. **Open the catalog.** The new writer reads the manifest and loads the current state of all SSTs.

2. **Flush.** The new writer calls `flush()` to ensure any committed-but-not-yet-visible writes from the previous writer are made visible. This is important because the previous writer may have committed a transaction to the WAL but crashed before flushing the manifest — without this step, those committed writes might be invisible to readers.

3. **Register the writer endpoint.** If the deployment uses service discovery, the new writer publishes its endpoint (hostname and port) under the `0xFF` system key so that clients know where to reconnect.

4. **Begin accepting connections.** The new writer's PG-wire server starts accepting DuckDB client connections.

### What Happens to In-Flight Requests

Any DuckDB requests that were in-flight to the old writer at the moment of fencing fall into one of two categories:

- **Already committed.** If the request's `DbTransaction` had already committed to the WAL before the fencing token changed, the write is durable and will be visible after the new writer's initial `flush()`. No data is lost.

- **Not yet committed.** If the request was still accumulating in the old writer's `PendingCatalogTxn` (between `BEGIN` and `COMMIT`), the pending mutations are lost. The DuckDB client will receive a connection error and can retry the entire transaction against the new writer.

This behavior is equivalent to what happens when a PostgreSQL server crashes: committed transactions are preserved, in-flight transactions are lost. DuckDB's retry logic handles this gracefully because the `ducklake` extension already implements retry semantics for connection failures.

## Recovery Latency

The time between the old writer failing and the new writer being ready to accept connections depends primarily on object-store latency, because the takeover protocol involves reading the manifest, writing a new fencing token, and calling `flush()` — all of which are object-store operations.

| Storage Backend | Typical Takeover Time | Notes |
|----------------|----------------------|-------|
| S3 Standard | 30–60 seconds | Dominated by PUT latency for manifest update and flush |
| S3 Express One Zone | 10–15 seconds | Lower latency per operation, same protocol |
| GCS | 20–40 seconds | Similar to S3 Standard |
| Azure Blob | 20–40 seconds | Similar to S3 Standard |
| Local filesystem | < 1 second | Primarily for development; not production-relevant |

These numbers represent the catalog-level recovery time — the time from "old writer is fenced" to "new writer is serving requests." The total user-visible downtime also includes the time for whatever orchestration system (Kubernetes, ECS, systemd) to detect the failure and start the new process, which is typically 10–30 seconds additional.

For most analytical workloads (batch loads, schema migrations, ad-hoc queries), a 30–90 second write outage during failover is acceptable. Reads are unaffected during failover because readers operate independently against immutable SST files — they do not need the writer to be running.

## Designing for Single-Writer

The single-writer constraint is not a limitation to work around; it is a constraint to design with. Here are the patterns that work well:

### One Catalog Per Dataset

If you have multiple independent datasets that need concurrent writes (for example, different teams loading data into different logical data products), give each dataset its own catalog. Each catalog has its own writer, and writers for different catalogs are completely independent. This is the recommended approach for multi-tenant platforms.

### Writer as a Managed Service

Run the writer as a managed process (a Kubernetes Deployment with replicas=1, a systemd service with restart policies, an ECS task) that is automatically restarted on failure. The fencing mechanism ensures that at most one writer is active even during the restart overlap. Configure readiness probes against the writer's `/ready` endpoint so that traffic is not routed to the new writer until takeover is complete.

### Readers as Stateless Scale-Out

Deploy readers as stateless processes that can scale horizontally without limit. Readers do not communicate with the writer, so adding or removing readers requires no writer-side configuration change. This is the pattern for high-traffic analytical workloads: one writer handling infrequent schema changes and file registrations, many readers serving concurrent queries.

### Client Retry Configuration

Configure DuckDB clients with retry logic for connection failures (the `ducklake` extension handles this automatically). During a writer failover, clients will experience a brief connection interruption and then reconnect to the new writer. In-flight transactions will be rolled back and must be retried, which is the same behavior clients would experience with a PostgreSQL failover.

## The SQLSTATE Mapping

When a writer is fenced, Rocklake maps the SlateDB fencing error to `SQLSTATE 57P04`, which PostgreSQL defines as "connection failure" in the "connection exception" class. DuckDB interprets this as "the server went away — disconnect and reconnect." This is the correct behavior: the old writer is no longer authoritative, and the client should reconnect (at which point it will reach the new writer via the service discovery or load-balancer endpoint).

Operators monitoring for writer fencing events should watch for `SQLSTATE 57P04` in their logs. A single occurrence during a planned or unplanned failover is normal. Repeated occurrences suggest a "flapping" condition where multiple processes are competing to be the writer — this typically indicates a misconfiguration where more than one writer process is being started against the same catalog.

## Comparison with Multi-Writer Alternatives

For readers who want to understand why Rocklake chose single-writer over multi-writer, here is a brief comparison:

| Property | Single-Writer (Rocklake) | Optimistic Concurrency (OCC) | Raft / Multi-Paxos |
|----------|--------------------------|-------------------------------|---------------------|
| Write consistency | Linearizable (trivially) | Serializable (with retries) | Linearizable (via consensus) |
| Conflict resolution | None needed | Retry on conflict | Leader handles all writes |
| Infrastructure | One writer process | Multiple writer processes + retry logic | 3+ voter nodes + leader election |
| Write throughput | Single process limit | Higher (parallelized with conflicts) | Single leader limit + replication overhead |
| Operational complexity | Minimal | Moderate (retry storms under contention) | High (membership, elections, log replication) |
| Failure mode | Writer death = brief outage | Contention = degraded throughput | Split brain possible if misconfigured |

For Rocklake's workload (catalog mutations at moderate rates, with the dominant performance concern being read latency rather than write throughput), single-writer provides the best complexity-to-correctness ratio.

## Further Reading

- **[Horizontal Read Scale-Out](single-writer-many-readers.md)** — The read-side complement: unlimited readers with zero coordination
- **[Architecture: Transaction Model](../architecture/transaction-model.md)** — How the single writer handles concurrent DuckDB sessions
- **[Design Decisions: Single-Writer Model](../design-decisions/single-writer.md)** — The full trade-off analysis
- **[Operations: Troubleshooting](../operations/troubleshooting.md)** — Diagnosing and resolving writer fencing issues
