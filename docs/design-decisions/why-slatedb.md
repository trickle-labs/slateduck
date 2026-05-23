# Why SlateDB?

## Alternatives Considered

### PostgreSQL
Mature, excellent SQL. But requires a persistent server — contradicts zero-infrastructure goal.

### SQLite
Zero-infrastructure for single machine. But not safe for concurrent access over object storage.

### FoundationDB
Distributed ACID. But requires 3-5 coordinator nodes — real infrastructure.

### TiKV
Distributed KV with transactions. But requires a cluster.

## Why SlateDB Fits

All durable state lives in the object store. No server, no disk, no cluster. Provides atomic `WriteBatch`/`DbTransaction`, single-writer fencing, and consistent reads.

## The Costs

- Higher latency than local-disk databases (20-40 ms on S3)
- Single-writer constraint
- Younger project with less battle-testing
- No built-in SQL
