# Transaction Model

SlateDuck implements a simple but effective transaction model that maps DuckDB's logical catalog transactions to batched atomic writes in SlateDB. This model provides the full ACID guarantees — atomicity (all operations succeed or none do), consistency (MVCC invariants are maintained after every commit), isolation (readers at different snapshots see consistent views without being affected by concurrent writes), and durability (committed transactions survive arbitrary crashes including sudden power loss).

The transaction model is simpler than what you would find in a general-purpose database because SlateDuck exploits two architectural constraints: single-writer (no concurrent write transactions to coordinate) and object-storage atomicity (a PUT to S3/GCS/Azure is either fully written or not written at all). These constraints eliminate entire categories of complexity: no write-write conflict detection, no deadlock handling, no two-phase commit, no undo logs, and no recovery replay.

This page explains how DuckDB uses transactions, how SlateDuck buffers and commits them, what guarantees hold under various failure scenarios, and how the transaction model interacts with MVCC snapshots and garbage collection.

## How DuckDB Uses Transactions

When DuckDB's `ducklake` extension performs a catalog mutation (creating a table, registering data files, recording a snapshot), it wraps all related operations in a single transaction. A typical transaction for creating a table with three columns looks like:

```sql
BEGIN;
INSERT INTO ducklake_table (table_id, schema_id, name, ...) VALUES (5, 1, 'events', ...);
INSERT INTO ducklake_column (column_id, table_id, name, type, ...) VALUES (1, 5, 'id', 'BIGINT', ...);
INSERT INTO ducklake_column (column_id, table_id, name, type, ...) VALUES (2, 5, 'name', 'VARCHAR', ...);
INSERT INTO ducklake_column (column_id, table_id, name, type, ...) VALUES (3, 5, 'ts', 'TIMESTAMP', ...);
INSERT INTO ducklake_snapshot (snapshot_id, snapshot_time, ...) VALUES (42, '2024-06-15', ...);
INSERT INTO ducklake_snapshot_changes (snapshot_id, change_type, ...) VALUES (42, 'create_table', ...);
COMMIT;
```

The critical requirement is that this entire set of operations either succeeds completely (all six rows visible at the new snapshot) or fails completely (no partial state where the table exists but some columns are missing, or the table exists but no snapshot records the fact). SlateDuck guarantees this atomicity.

### Transaction Size in Practice

DuckDB transactions vary in size depending on the operation:

| Operation | Typical Buffered Operations |
|-----------|---------------------------|
| `CREATE SCHEMA` | 1 schema row + 1 snapshot + 1 change = 3 ops |
| `CREATE TABLE` (10 columns) | 1 table + 10 columns + 1 snapshot + 1 change = 13 ops |
| `INSERT INTO` (1 file) | 1 data file + 1 table stats update + column stats + 1 snapshot = 5–15 ops |
| Bulk INSERT (100 files) | 100 data files + 100 × N column stats + 1 snapshot = 500–1500 ops |
| `ALTER TABLE ADD COLUMN` | 1 new column + 1 snapshot + 1 change = 3 ops |
| `DROP TABLE` | Update end_snapshot on table + all columns + 1 snapshot = N+2 ops |

Even the largest transactions (bulk file registration with hundreds of files) produce write batches under 1 MB. The theoretical maximum transaction size is 64 MiB.

## Transaction Buffering

When SlateDuck receives a `BEGIN` message, it transitions the session into "in transaction" state. From this point until `COMMIT` or `ROLLBACK`, write operations are not immediately applied to the catalog. Instead, they are accumulated in a `PendingCatalogTxn` buffer:

```rust
struct PendingCatalogTxn {
    ops: Vec<BufferedOp>,           // All operations in insertion order
    estimated_size: usize,          // Running byte count estimate
    snapshot_row: Option<SnapshotRow>, // The snapshot that will seal this transaction
}
```

Each `BufferedOp` is a fully-classified, parameterized catalog operation ready for execution. The buffer serves two purposes:

1. **Atomicity.** By deferring writes until commit, SlateDuck ensures that either all operations are written (on successful commit) or none are (on rollback or crash during buffering).

2. **Batching efficiency.** Instead of issuing one WAL write per INSERT statement (which would be one S3 PUT per INSERT), SlateDuck combines all operations into a single `WriteBatch` and commits them as one WAL entry (one S3 PUT for the entire transaction).

### Size Monitoring

The `estimated_size` counter tracks the approximate serialized size of all buffered operations. If it exceeds `MAX_BATCH_SIZE` (64 MiB), the next write operation returns an error:

```
ERROR: Transaction exceeds maximum batch size (64 MiB).
SQLSTATE: 54001 (program limit exceeded)
HINT: Split large operations into smaller transactions.
```

This prevents pathological cases (registering millions of tiny files in a single transaction) from consuming excessive memory or creating oversized WAL entries that would degrade SlateDB compaction performance.

## The Commit Sequence

When SlateDuck receives `COMMIT`, it executes a carefully ordered sequence that is the most critical code path in the entire system:

### Step 1: Acquire the Write Lock

The catalog store's write mutex is acquired. This serializes all commits — only one transaction can be in the commit phase at a time. This is acceptable because the single-writer model already guarantees that only one SlateDuck instance is writing.

If another session's commit is already in progress (possible with multiple DuckDB clients connected), the current commit blocks until the mutex is released. In practice, commits are fast (sub-millisecond for the local work, plus one network round-trip for the WAL PUT), so contention is rare.

### Step 2: Check Writer Epoch

Before proceeding, SlateDuck reads the current `writer-epoch` system key from SlateDB and compares it to the epoch this instance claimed at startup. If the stored epoch does not match:

```
ERROR: Writer has been fenced. Another instance has taken over as writer.
SQLSTATE: 57P04 (database dropped)
```

This check prevents split-brain writes. If a new SlateDuck instance started and incremented the epoch while this instance was still running, this instance's commit is rejected. The old writer must reconnect (which will also fail) or shut down.

### Step 3: Allocate IDs

For operations that need unique identifiers (new schemas, tables, columns, files, snapshots), IDs are allocated from the counter system. The allocations are:

- Read current counter value from SlateDB
- Assign IDs to each operation that needs one
- Include counter updates in the write batch (so the incremented counters are committed atomically with the rows they identify)

The snapshot ID is allocated last because it must be the highest ID in the batch — it represents "the commit point" for all other operations in this transaction.

### Step 4: Build the Write Batch

For each buffered operation, the executor:

1. Constructs the binary key (tag + field values in big-endian)
2. Serializes the row as a protobuf message
3. Wraps the protobuf bytes in the value envelope (version + magic + payload)
4. Adds the key-value pair to a SlateDB `WriteBatch`

Counter updates, secondary index entries, and hot key updates are also added to the same batch.

### Step 5: Atomic Commit

The `WriteBatch` is submitted to SlateDB, which writes it as a single entry in the write-ahead log. The WAL entry is then flushed to object storage as a single PUT operation. **This single PUT is the commit point**: if it succeeds, the transaction is committed. If it fails (network error, storage timeout, process crash), no bytes were written and the transaction is aborted.

SlateDB's contract guarantees that the PUT is atomic from the perspective of any subsequent reader — there is no scenario where a reader sees half the batch.

### Step 6: Release the Write Lock

The catalog mutex is released. Subsequent commits (from other connections) can now proceed. The committed data is immediately visible to any reader that requests a snapshot at or after the newly-committed snapshot ID.

## Auto-Commit Mode

When DuckDB sends write statements without an explicit `BEGIN`/`COMMIT` wrapper, SlateDuck operates in auto-commit mode. Each individual statement is treated as its own single-statement transaction:

1. The statement is buffered (single entry)
2. The commit sequence runs immediately
3. The session returns to idle state

This is equivalent to wrapping every statement in `BEGIN; statement; COMMIT;`. Auto-commit mode produces one snapshot per statement, which is correct for DuckDB's usage patterns (each DDL statement is its own logical operation).

## Rollback

On `ROLLBACK` (or connection close while in a transaction), SlateDuck discards the `PendingCatalogTxn` buffer. This is instantaneous and has no side effects because nothing was written to SlateDB during the transaction. The session returns to idle state.

There is no rollback of committed transactions — once a commit succeeds, it is permanent. To "undo" a committed change, you perform a new transaction that creates the desired state (e.g., to undo a DROP TABLE, you would CREATE TABLE again with the same definition).

## Crash Recovery Scenarios

The transaction model's crash behavior depends on where in the sequence the crash occurs:

### Crash During Buffering (Before Commit)

**Scenario:** Process crashes after receiving some INSERT statements but before COMMIT.

**Result:** The in-memory buffer is lost. The catalog is unchanged. From the catalog's perspective, nothing happened. DuckDB receives a connection error and can retry the entire transaction.

### Crash During Commit Sequence (Before WAL PUT)

**Scenario:** Process crashes after acquiring the mutex and allocating IDs but before the WriteBatch is submitted to SlateDB.

**Result:** Same as above — the catalog is unchanged. IDs were allocated in memory but never committed to the counter, so they are not "consumed." The next writer will allocate from the same counter values.

### Crash During WAL PUT

**Scenario:** Process crashes while the WAL entry is being written to object storage.

**Result:** Object storage PUTs are atomic — the write either completes fully or is not visible. If the PUT completed before the crash, the transaction is committed. If it did not complete, the transaction is aborted. There is no intermediate state.

### Crash After WAL PUT

**Scenario:** Process crashes after the WAL PUT succeeds but before the connection sends `CommandComplete` to the client.

**Result:** The transaction is committed (the WAL entry is durable in object storage). The client receives a connection error and may not know whether the transaction succeeded. On retry, it will observe the committed state (the new snapshot is visible).

### Key Insight: No Recovery Phase

SlateDuck has no "recovery" phase on startup. It does not need to replay logs, check for incomplete transactions, or resolve in-doubt states. The catalog state in SlateDB is always consistent because:

- Committed transactions are fully written (atomic PUT)
- Uncommitted transactions leave no trace (in-memory only)
- Counter values are committed atomically with the rows they identify

On startup, SlateDuck opens SlateDB, reads the manifest, and is immediately ready to serve queries. There is nothing to recover.

## Read "Transactions" (Snapshots)

SlateDuck does not implement read transactions in the traditional sense. Instead, reads are bound to a specific snapshot ID. A reader at snapshot N always sees the consistent state at snapshot N, regardless of any concurrent writes creating snapshots N+1, N+2, etc.

This is possible because:

- Writes are additive (they never modify existing key-value pairs)
- The MVCC filter is parameterized by snapshot ID
- A commit at snapshot N+1 cannot affect the visibility of any row at snapshot N

There is no "read lock," no "begin read transaction," and no possibility of a read seeing inconsistent state. Snapshot isolation is structural — a consequence of the key-value encoding — not behavioral (not something that requires runtime coordination between readers and writers).

## Transaction Isolation Level

SlateDuck provides **snapshot isolation** — each transaction sees a consistent snapshot of the catalog as of its start time. Within a single transaction, the visible state does not change regardless of concurrent commits by other sessions.

This is weaker than serializable isolation (which would prevent write skew anomalies) but stronger than read committed (which would allow a transaction to see newly committed data between statements). For a catalog server where writes are serialized by the single-writer model and reads are parameterized by snapshot, snapshot isolation provides the exact guarantees needed without any overhead.

## Relationship to DuckLake Snapshots

Every committed write transaction creates exactly one DuckLake snapshot. The relationship is one-to-one:

- Each snapshot represents the result of exactly one atomic transaction
- Each transaction produces exactly one snapshot (the one it commits)
- The snapshot ID is allocated during commit and becomes the `begin_snapshot` for all rows in that transaction

This means the snapshot sequence is also the commit sequence: snapshot 42 was committed before snapshot 43, which was committed before snapshot 44. There are no "gaps" in the sequence (every integer between 1 and the current snapshot has a corresponding committed transaction) and no "reordering" (the commit order matches the snapshot ID order).

## Writer Protocol State Machine

Every write session follows a strict, non-negotiable lifecycle:

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│  1. CatalogStore::begin_write()                                                  │
│     • Acquire fencing epoch from SlateDB                                         │
│     • Load persisted counters (next_snapshot_id, next_catalog_id, next_file_id) │
│     • Return a CatalogWriter with empty staging buffer                           │
├──────────────────────────────────────────────────────────────────────────────────┤
│  2. Zero or more mutation calls on CatalogWriter                                 │
│     • create_schema / create_table / create_view / etc.                          │
│     • All MVCC-versioned rows are appended to in-memory staging buffer           │
│     • Nothing is written to SlateDB yet                                          │
├──────────────────────────────────────────────────────────────────────────────────┤
│  3. CatalogWriter::create_snapshot()                                             │
│     • Drain staged rows                                                          │
│     • Begin ONE SlateDB SerializableSnapshot transaction                         │
│     • Check fencing epoch (reject if writer has been superseded)                 │
│     • Write all staged mutation rows                                             │
│     • Write ducklake_snapshot row (the commit marker)                            │
│     • Write all three persisted counters atomically                              │
│     • Commit — this single commit is the transaction boundary                   │
├──────────────────────────────────────────────────────────────────────────────────┤
│  4. CatalogStore::commit_writer()                                                │
│     • Sync in-memory counter cache from the writer                               │
│     • Ensures subsequent begin_write() sees the new baseline IDs                 │
│     • Ensures read_latest() returns the newly committed snapshot ID              │
└──────────────────────────────────────────────────────────────────────────────────┘
```

### Protocol Invariants

**Atomicity** — If step 3 is never reached (writer dropped, connection lost, panic), the staging buffer is discarded and no rows reach SlateDB.  There are no partial writes visible to readers.

**Monotonic IDs** — Because all three counter keys are written inside the same transaction as the snapshot row, a crash between step 3 and step 4 leaves the counter state consistent in persistent storage.  A reopened `CatalogStore` reloads the counters from SlateDB, so IDs always advance and never repeat.

**Correct key resolution** — Before calling `drop_table` or `drop_column`, the executor resolves the enclosing `schema_id` / `table_id` by scanning live MVCC rows via `find_table_schema_id()` / `find_column_table_id()`.  IDs are never assumed or inferred from the calling context.

**Fencing** — The epoch check in step 3 guarantees that only one writer can commit to any given generation of the catalog.  A stale writer whose epoch has been superseded receives a `FencingEpochMismatch` error and its transaction is aborted.

### Non-Conformant Patterns (Do Not Use)

| Anti-pattern | Why it breaks the protocol |
|---|---|
| Writing rows directly to SlateDB before `create_snapshot()` | Partial state becomes visible to concurrent readers |
| Omitting `commit_writer()` after a successful `create_snapshot()` | In-memory counter cache goes stale; next session reuses IDs |
| Using a hardcoded `schema_id = 0` in `UpdateEndSnapshot` | Writes the tombstone to the wrong key; original row remains live |
| Beginning a second `CatalogWriter` before committing the first | The second writer loads the same counter baseline; IDs overlap |

## Further Reading

- **[MVCC Implementation](mvcc-implementation.md)** — How version fields interact with the transaction model
- **[Architecture Overview](overview.md)** — Where the transaction model fits in the overall system
- **[Concepts: MVCC & Snapshots](../concepts/mvcc.md)** — The conceptual foundation for snapshot isolation
- **[Internals: Crash Safety](../internals/crash-safety.md)** — Detailed analysis of crash scenarios and durability guarantees
