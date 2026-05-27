# Crash Safety

Rocklake achieves crash safety without requiring an explicit recovery process, without WAL replay, without fsck, without repair tools, and without operator intervention. If the process crashes at any point during any operation — mid-read, mid-write, mid-compaction — the catalog remains consistent. Either the operation completed fully (and is durable), or it did not happen at all (and no trace remains). There is no in-between state.

This property is perhaps Rocklake's most operationally significant guarantee. Traditional databases (PostgreSQL, MySQL) require recovery time after an unexpected shutdown — they must replay WAL logs to bring the database to a consistent state. This recovery can take seconds to minutes. Rocklake has zero recovery time. Start the process. It is ready immediately.

This page explains how crash safety is achieved, what guarantees it provides, what it does NOT guarantee, and how it interacts with other system components.

## The Foundation: Atomic PUT

Everything rests on one property of object storage: **a PUT either completes entirely or does not happen at all.** There is no partial PUT. You never read half of an object that was being written when the writer crashed.

This is not a storage-level transaction — it is a property of the HTTP PUT operation to S3/GCS/Azure:

- If the PUT request completes (HTTP 200 response): the entire object is durable and readable
- If the PUT request does not complete (network failure, process crash, timeout): the object does not exist at all

No other durability mechanism is needed. No write-ahead log. No journal. No redo log. The atomic PUT IS the durability mechanism.

### Why This Works for SlateDB

SlateDB (the underlying key-value store) leverages atomic PUT by writing WAL segments as individual objects:

```
s3://bucket/catalog/wal/segment-001.sst
s3://bucket/catalog/wal/segment-002.sst
s3://bucket/catalog/wal/segment-003.sst
```

Each WAL segment contains one or more write batches. When Rocklake commits a transaction, SlateDB:

1. Serializes the write batch into an SST-format buffer
2. PUTs the buffer to object storage as a new WAL segment

If the PUT succeeds: the segment exists, the write batch is durable.
If the PUT fails: the segment does not exist, the write batch is lost.

There is no intermediate state where "the segment partially exists" or "some bytes are written but not others."

## Write Path Crash Safety

A Rocklake write transaction follows these steps:

```
1. Allocate snapshot ID (increment counter in memory)
2. Build key-value pairs for the transaction
3. Create a SlateDB WriteBatch containing all pairs
4. Commit the WriteBatch → one atomic WAL segment PUT
5. Update in-memory state (hot key cache, etc.)
6. Send response to client
```

### Crash at Step 1 (Counter Allocation)

The counter was incremented in memory only (not yet persisted to storage). On restart, the counter's persisted value is still the old value. The next writer reads the old value and allocates the same snapshot ID.

**Result:** The crashed transaction never happened. The snapshot ID gap is harmless (no data references it).

### Crash at Step 2 or 3 (Batch Construction)

The write batch is being constructed in memory. No storage writes have occurred.

**Result:** The crashed transaction never happened. No state changes are visible.

### Crash During Step 4 (PUT in Progress)

The most critical moment. The PUT request is in flight — some bytes may have been sent to S3. Two outcomes:

- **S3 received and acknowledged the complete PUT before the process crashed:** The WAL segment is durable. The transaction will be visible to the next writer (it will see the segment in the WAL).
- **S3 did not receive the complete PUT (or the PUT was not acknowledged):** The WAL segment does not exist. The transaction never happened.

**Result:** Either fully committed or never happened. Never partially committed.

### Crash at Step 5 (Post-Commit Updates)

The WAL segment is already durable (step 4 succeeded). In-memory caches are being updated. The process crashes before completing cache updates.

**Result:** The transaction IS committed (durable in storage). On restart, the new writer reads the WAL and discovers the committed data. In-memory caches are rebuilt from storage. No data is lost.

### Crash at Step 6 (Response to Client)

The transaction is committed. The response was not sent to the client (connection dropped).

**Result:** The transaction IS committed, but the client does not know. This is the standard "in-doubt transaction" scenario in any distributed system. The client must check whether its operation succeeded (by querying the catalog state) before retrying.

## Read Path Crash Safety

Reads are inherently safe because they modify no state:

1. Reader constructs a prefix key
2. Reader scans SlateDB (accessing SST files in object storage)
3. Reader deserializes values from protobuf
4. Reader applies MVCC filter
5. Reader sends results to client

A crash at any point simply terminates the read. No cleanup is needed because:

- SST files are immutable (reading them does not modify them)
- No write-ahead state was created
- The client's TCP connection is broken, which it handles by reconnecting

On restart, the next read starts fresh. There is no "dirty read" state to unwind.

## Compaction Crash Safety

SlateDB periodically compacts WAL segments and SST files — merging small files into larger ones and removing tombstones. Compaction follows the "new before old" pattern:

```
Step 1: Write the new (merged) SST file to object storage
Step 2: Update the manifest to reference the new file
Step 3: Delete old SST files
```

### Crash During Step 1

The new SST file is partially written or not written at all. (If the PUT did not complete, the file does not exist.)

**Result:** The old files are still referenced by the manifest. Compaction simply did not happen. It will be retried on next startup.

**Orphan risk:** If the PUT DID complete but the process crashed before step 2, the new SST file exists but is not referenced by the manifest. This is an "orphaned" file. SlateDB's garbage collection identifies and removes orphaned files (files that exist in storage but are not referenced by the manifest).

### Crash During Step 2

The manifest update is itself an atomic PUT (the manifest is a single object in storage). Either the new manifest (referencing the new file) is written, or the old manifest (referencing the old files) remains.

**Result:** Either the compaction completed (new manifest references new file) or it did not (old manifest references old files). Both states are consistent.

### Crash During Step 3

The manifest now references the new file, but old files have not been deleted yet.

**Result:** The old files are orphaned garbage. They are harmless (no one reads them because the manifest does not reference them) and will be cleaned up by SlateDB's garbage collection.

## Manifest Crash Safety

The manifest is SlateDB's source of truth — it lists which SST files and WAL segments constitute the current database state. The manifest itself is stored as a single object:

```
s3://bucket/catalog/manifest/MANIFEST
```

The manifest is updated atomically (one PUT). This means:

- If you read the manifest, you always get a complete, valid manifest
- There is no "half-written manifest" state
- The manifest is never corrupted by a crash

### Manifest Versioning

SlateDB uses conditional PUTs (if supported by the storage provider) or versioned objects to prevent concurrent manifest updates. This ensures that even if two processes attempt to update the manifest simultaneously, only one succeeds.

## No WAL Replay

Traditional databases use a two-phase approach:

1. Write data to WAL (sequential, fast)
2. Apply WAL entries to the main data structure (later, in background)
3. On crash: replay unapplied WAL entries to recover

Rocklake (via SlateDB) does not need step 3 because:

- WAL segments ARE the data (until compaction merges them into SST files)
- The manifest tracks which WAL segments and SST files are current
- There are no "unapplied" WAL entries — all WAL segments referenced by the manifest are valid data

**Startup sequence:**

```
1. Read manifest from storage (one GET)
2. The manifest lists all current WAL segments and SST files
3. Open the block cache
4. Ready to serve requests
```

Time from process start to serving requests: typically 50–200ms (dominated by the single GET for the manifest). Compare to PostgreSQL's crash recovery, which can take seconds to minutes depending on WAL size.

## What Crash Safety Does NOT Guarantee

### In-Flight Transaction Data

If the process crashes during a write transaction (before the WAL PUT completes), that transaction's data is lost. The client will see a connection error and should retry. This is not data loss — it is an uncommitted transaction that was aborted.

### Client Acknowledgment

If the process crashes after committing (WAL PUT succeeded) but before sending the response to the client, the client does not know the transaction committed. The client should check catalog state before retrying to avoid duplicate operations.

### Hot Key Cache Freshness

The hot key cache is in-memory only. After a crash, it must be rebuilt by reading from storage. This means the first few reads after restart may be slightly slower (cache miss). This is a performance concern, not a correctness concern.

### Counter Monotonicity Across Crashes

Snapshot IDs and entity IDs are allocated from counters that are persisted periodically. If the process crashes between counter persists, some allocated IDs may be "lost" (gaps in the sequence). Gaps are harmless — nothing depends on ID contiguity.

## Crash Safety Verification

The test suite includes explicit crash safety tests:

### Power-Failure Tests

Simulated crashes at specific points in the write path:

```rust
#[test]
fn crash_during_put() {
    // Start a transaction
    // Inject a "crash" before PUT completes
    // Restart
    // Verify: transaction data not visible
    // Verify: catalog is consistent
    // Verify: next transaction can proceed
}
```

### Concurrent Reader During Crash

Verify that readers are not affected by writer crashes:

```rust
#[test]
fn reader_unaffected_by_writer_crash() {
    // Start a reader scan
    // Crash the writer mid-transaction
    // Verify: reader completes normally
    // Verify: reader sees consistent state (pre-crash)
}
```

### Recovery Time Measurement

Verify that startup after crash is fast:

```rust
#[test]
fn startup_after_crash_is_instant() {
    // Write 1000 transactions
    // Kill process (simulating crash)
    // Measure time to first successful query after restart
    // Assert: < 500ms
}
```

## Implications for Operators

### No Recovery Procedures

There is no `pg_resetwal`, no `innodb_force_recovery`, no `VACUUM FULL` after crash. After a crash, start the process. It works.

### No Recovery Monitoring

There is no "recovery progress" to monitor, no "checkpoint completion" to wait for, no "WAL replay percentage" dashboard. The process either starts successfully (ready to serve) or fails to start (manifest cannot be read — check storage connectivity).

### No Data Loss Risk from Crashes

The only data "lost" in a crash is uncommitted transactions (which were never durable by definition). All previously committed snapshots are intact because they are stored as immutable objects in S3.

### Restart Is Always Safe

You can restart Rocklake at any time for any reason (deployment, upgrade, configuration change) without risk. There is no "clean shutdown" procedure — just stop the process.

```bash
# All of these are equally safe:
kill -9 $PID           # Immediate kill
kill -TERM $PID        # Graceful shutdown
systemctl restart rocklake  # Service restart
# Power failure         # Hardware crash
```

## Further Reading

- **[Architecture: Transaction Model](../architecture/transaction-model.md)** — How transactions achieve atomicity
- **[Schema Version](schema-version.md)** — How format version writes are atomic
- **[Operations: Troubleshooting](../operations/troubleshooting.md)** — What to do if startup fails
- **[Design Decisions: Why SlateDB](../design-decisions/why-slatedb.md)** — Why object storage atomicity matters
