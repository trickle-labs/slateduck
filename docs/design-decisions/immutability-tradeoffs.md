# Immutability Trade-offs

Rocklake's append-only, immutable data model is its most distinctive architectural property — and its most controversial. Catalog entries are never modified in place. Updates create new versions, and old versions remain indefinitely until explicitly excised through a deliberate, audited process. This page honestly examines the costs of this approach, quantifies the trade-offs, compares them to mutable alternatives, and explains why immutability is the right choice for lakehouse catalogs despite its costs.

## What You Get

The benefits of immutability are substantial and systemic. They are not nice-to-haves; they are structural properties that permeate the entire system design:

### Free Time Travel

Every past state of the catalog is queryable without additional infrastructure. You can ask "what did this table look like last Tuesday?" or "which files were registered before the ETL bug was introduced?" without maintaining separate audit logs, change data capture pipelines, or point-in-time backup systems.

In a mutable system, time travel requires either:

- WAL-based point-in-time recovery (complex, slow, limited retention)
- Change data capture to a separate store (operational overhead, lag)
- Periodic snapshots (coarse granularity, storage-expensive)

Rocklake provides snapshot-level time travel as a natural consequence of its data model — no additional infrastructure required.

### Automatic Crash Safety

Because rows are never modified, there is no concept of a "partial update." A transaction either commits (new rows appear) or does not commit (no new rows appear). There is no intermediate state where some rows are updated and others are not. The catalog cannot be "half-written" after a crash.

In a mutable system, crash safety requires:

- Write-ahead logging (WAL) with careful fsync semantics
- Checkpoint recovery procedures
- Crash-consistent index maintenance
- Undo/redo logic for partially-applied transactions

Rocklake needs none of this. SlateDB's atomic `WriteBatch` either makes all new rows visible or none of them. Old rows are never touched.

### Lock-Free Readers

Readers never conflict with writers because readers access immutable data (existing rows that will never change) while writers only create new data (new rows). There is no need for read locks, shared locks, MVCC read sets, or lock escalation. Readers proceed in parallel with zero coordination.

In a mutable system, read-write conflicts require:

- Shared/exclusive locking (reduces concurrency)
- MVCC with visibility checks and vacuum (complex, requires tuning)
- Snapshot isolation with rollback segments (garbage generation)

Rocklake readers simply seek to the appropriate key prefix and iterate. No visibility check is needed beyond comparing snapshot IDs (two integer comparisons per row).

### Horizontal Read Scale-Out

Because SlateDB's SST files are immutable objects in cloud storage, any number of readers can access them concurrently via GET requests. There is no replication to set up, no replica lag to manage, no read-after-write consistency concerns. If you need 100 DuckDB instances reading the same catalog, they all read the same SST files independently.

## What You Pay

### 1. Storage Growth

Every catalog mutation creates new rows that are never automatically reclaimed. Old versions accumulate indefinitely.

**Quantifying the cost:**

Consider a realistic scenario:

- 100 tables, each with 50 columns = 5,000 column rows
- Each table is altered 10 times (adding/removing columns, renaming) over a year
- Each alteration creates new versions of the table row + affected column rows

After one year:

| Entity Type | Live Rows | Superseded Rows | Total | Storage |
|------------|-----------|-----------------|-------|---------|
| Tables | 100 | 1,000 | 1,100 | 165 KB |
| Columns | 5,000 | 50,000 | 55,000 | 8.25 MB |
| Data Files | 10,000 | 2,000 | 12,000 | 3.6 MB |
| Statistics | 50,000 | 10,000 | 60,000 | 12 MB |
| **Total** | **65,100** | **63,000** | **128,100** | **~24 MB** |

24 MB of total catalog storage, with approximately half being superseded versions. At S3 Standard pricing ($0.023/GB/month), this costs $0.0006/month — less than a penny per year.

For a much larger catalog (10,000 tables, 500,000 columns, 1,000 changes per day):

| Metric | After 1 Year |
|--------|-------------|
| Live rows | ~560,000 |
| Superseded rows | ~5,000,000 |
| Total storage | ~800 MB |
| Monthly cost (S3) | $0.018 |

Still negligible. Storage costs become meaningful only at extreme scales (100,000+ tables with thousands of daily mutations for years without GC).

**Mitigation:** Run `rocklake gc --retain-days 30` to advance the retention horizon. This makes old versions inaccessible via time travel but does not physically delete them. Run `rocklake excise` periodically to physically remove superseded rows beyond the retention horizon. For most catalogs, monthly GC with 30-day retention keeps storage growth well-bounded.

### 2. Read Amplification (Scan Overhead)

Every prefix scan must examine all versions of each entity and filter for the currently-visible version. If a column has been modified 100 times, the reader reads 100 rows from SlateDB and discards 99 of them.

**Quantifying the cost:**

For a typical catalog (10 versions per entity on average):

| Operation | Rows Read | Rows Returned | Amplification Factor |
|-----------|-----------|---------------|---------------------|
| List 50 columns | 500 | 50 | 10x |
| List 100 data files | 1,000 | 100 | 10x |
| List 4 schemas | 40 | 4 | 10x |

At ~150 bytes per row, reading 500 rows means reading 75 KB from SlateDB. With a typical SST block size of 4 KB, this requires reading ~19 blocks. If blocks are cached (likely for repeated access), the cost is a few microseconds per block. If blocks must be fetched from S3, each block read costs 5–20ms.

**Mitigation:**

- **GC reduces amplification:** After GC + excision, the amplification factor drops to 1x (only live rows remain). Running GC weekly keeps the factor low.
- **SlateDB's block cache:** Frequently-accessed blocks stay in memory. For catalogs that fit in cache (most catalogs < 100 MB), read amplification has zero I/O cost.
- **The filter is cheap:** Two integer comparisons per row (is `created_snapshot_id` <= target? is `end_snapshot_id` NULL or > target?) — this takes nanoseconds per row, even for millions of rows.

### 3. Operational Complexity

Operators must understand and manage the two-phase GC process:

1. **Advance retention:** `rocklake gc --retain-days 30` moves the `retain_from` marker forward
2. **Excise (optional):** `rocklake excise --before-snapshot N` physically removes old versions

This is an additional operational task that PostgreSQL-backed DuckLake does not require (PostgreSQL has automatic VACUUM).

**Comparing operational burden:**

| Task | Rocklake | PostgreSQL |
|------|-----------|-----------|
| Retention management | Manual `gc` command (automatable) | Automatic vacuum |
| Physical cleanup | Manual `excise` (optional, rare) | Automatic vacuum |
| Monitoring | Check `retain_from`, row counts | Check vacuum stats, bloat |
| Failure mode | Unbounded growth if GC never runs | Table bloat if vacuum is blocked |
| Recovery from failure | Run GC (catches up instantly) | Vacuum may take hours for large tables |

**Mitigation:** GC can be automated via a cron job or Kubernetes CronJob. A single daily command (`rocklake gc --retain-days 30`) is sufficient for most catalogs. The operational burden is comparable to (not worse than) managing PostgreSQL's autovacuum.

### 4. No True Immediate Delete

You cannot immediately and permanently remove a catalog entry. Even after `gc --retain-days 0 && excise`, there is a brief window where the data exists in SlateDB's WAL or SST files before compaction removes it.

**Quantifying the window:**

| Phase | Duration | Data Location |
|-------|----------|---------------|
| Row created | Instant | WAL segment in S3 |
| GC advances past it | Configurable (retain-days) | WAL/SST in S3 |
| Excision writes tombstone | Instant | Tombstone in WAL |
| Compaction removes both | Minutes to hours | SST files removed |

For GDPR compliance, the relevant question is: "when is the data irrecoverable?" The answer is: after compaction completes following excision. Typically 1–24 hours depending on compaction schedule.

**Mitigation:** For strict compliance timelines, configure aggressive compaction and use S3 object versioning controls to ensure deleted objects are purged promptly.

## The Trade-off Matrix

| Concern | Immutable (Rocklake) | Mutable (PostgreSQL DuckLake) |
|---------|----------------------|-------------------------------|
| Time travel | Free, natural, unlimited (until GC) | Expensive, limited by WAL retention |
| Crash safety | Automatic (no partial updates possible) | Requires WAL + checkpoint + recovery |
| Read scale-out | Unlimited, zero coordination | Requires streaming replication + lag |
| Storage efficiency | Grows until GC (manageable) | Automatic via vacuum (usually) |
| Delete latency | Eventual (GC + excise + compaction) | Immediate (DELETE + VACUUM) |
| Operational burden | GC scheduling (simple, automatable) | Vacuum tuning (complex, can stall) |
| Debugging | Full history visible for forensics | History lost after vacuum |
| Audit trail | Built-in (every version is an audit record) | Requires separate audit system |

## Our Assessment

For the lakehouse catalog use case, the trade-off strongly favors immutability:

1. **Catalogs are small.** Even large catalogs (10,000 tables) occupy megabytes, not gigabytes. Storage growth is negligible in absolute terms.

2. **Schema changes are infrequent.** A typical catalog sees a few changes per day, not per second. The accumulation rate of superseded versions is low.

3. **Time travel is a core requirement.** DuckLake's value proposition includes "query your catalog at any point in time." Immutability provides this for free. A mutable system would require building an entirely separate versioning layer.

4. **Crash safety is non-negotiable.** A corrupted catalog means your entire data lake becomes inaccessible. The automatic crash safety of immutability eliminates an entire class of failure modes.

5. **Read scale-out is essential.** Multiple DuckDB instances reading the same catalog is the standard deployment pattern. Immutability enables this without replication infrastructure.

The costs (storage growth, GC scheduling, delete latency) are real but manageable with minimal operational effort. They are the right costs to pay for the benefits received.

## When Immutability Is Wrong

If your use case has these characteristics, Rocklake's immutability model may not be appropriate:

- **Very high catalog churn:** Thousands of schema changes per second (rare, but possible in automated testing environments)
- **Strict immediate-deletion requirements:** Regulations that demand data destruction within seconds, not hours
- **Storage-constrained environments:** Edge devices or embedded systems where every kilobyte matters
- **Write-heavy, read-light workloads:** If the catalog is written to far more than it is read, the accumulation of versions provides no read benefit

For these cases, consider PostgreSQL-backed DuckLake or evaluate whether the constraints can be relaxed.

## Further Reading

- **[Concepts: Immutability](../concepts/immutability.md)** — Detailed explanation of the immutable model
- **[Concepts: Snapshots](../concepts/snapshots.md)** — How snapshots enable time travel
- **[Operations: Garbage Collection](../operations/garbage-collection.md)** — Managing storage growth
- **[Operations: Excision](../operations/excision.md)** — Physical deletion when required
