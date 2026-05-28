# Glossary

This page defines all terms used throughout the RockLake documentation. If you encounter an unfamiliar word or concept in any page of this documentation, look it up here. Terms are listed alphabetically. Each entry includes a concise definition followed by context explaining when and where the term is relevant.

---

## A

**ABI Version** — The Application Binary Interface version number that ensures compatibility between the RockLake shared library (C FFI) and the DuckDB extension that loads it. When the ABI version changes, both sides must be recompiled. The ABI version is distinct from the RockLake release version — it only increments when the C function signatures or calling conventions change.

**Atomic PUT** — The fundamental property of object storage (S3, GCS, Azure) that a PUT operation either completes entirely or does not happen at all. There is no partial PUT. This property is the foundation of RockLake's crash safety — a WAL segment either exists completely or does not exist. No write-ahead log replay is needed.

---

## B

**Block Cache** — An in-memory cache maintained by SlateDB that stores recently-read SST blocks. Avoids repeated object storage fetches for frequently-accessed data. Configured via `ROCKLAKE_CACHE_SIZE_MB`. Distinct from the hot key cache (which caches specific high-frequency keys).

**Bounded SQL** — RockLake's approach to SQL support: only a finite, enumerated set of SQL statement patterns are recognized. Statements outside this set are rejected with SQLSTATE 42601. This is intentional — RockLake implements the DuckLake protocol, not a general SQL engine. The bounded set covers all SQL patterns that DuckDB's ducklake extension emits.

---

## C

**Catalog** — The metadata store that records what schemas, tables, columns, data files, and other objects exist in a lakehouse. RockLake is a catalog implementation — it stores metadata about data, not the data itself. The actual data lives in Parquet files in object storage; the catalog records where those files are and what they contain.

**Compaction** — The background process by which SlateDB merges multiple small SST files into fewer larger files, removing tombstones and obsolete versions. Compaction improves read performance (fewer files to check) and reclaims storage space. It is crash-safe — the new file is written before the manifest is updated.

**Counter** — A system key in RockLake that tracks the next available ID for each entity type (schemas, tables, columns, files). Counters are incremented atomically within each write batch. Gaps in counter sequences (from crashed transactions) are harmless.

---

## D

**Data File** — A Parquet file (or other columnar format) stored in object storage that contains actual table data. RockLake's catalog tracks which data files belong to which tables and at which snapshots they were registered. DuckDB reads data files directly from object storage; RockLake only stores their metadata.

**Delete File** — A file that records which rows in a data file have been logically deleted. Instead of rewriting the entire data file (which is immutable), a separate delete file marks specific row indices as deleted. DuckDB applies these deletions during query execution.

**DuckDB** — An in-process analytical SQL database. DuckDB is the query engine; RockLake is the catalog backend. DuckDB connects to RockLake over the PostgreSQL wire protocol (or via C FFI) to manage lakehouse metadata.

**DuckLake** — DuckDB's lakehouse extension that manages data as columnar files in object storage with metadata in a catalog backend. DuckLake defines the protocol (what SQL statements are sent, what responses are expected). RockLake implements this protocol. Other backends (PostgreSQL, MySQL, SQLite) also implement DuckLake.

---

## E

**Epoch** — A monotonically increasing counter that identifies the current writer. When a new RockLake writer starts, it reads the current epoch, increments it, and writes the new value. This increment fences (invalidates) any previous writer that may still be running. The epoch mechanism prevents split-brain writes.

**Excision** — The irreversible physical deletion of superseded catalog rows from storage. The second phase of garbage collection. After advancing the retention horizon (which makes old snapshots inaccessible), excision removes the actual key-value pairs from SlateDB. This reclaims storage space but is irreversible — excised data cannot be recovered.

---

## F

**Fencing** — The mechanism by which a new writer invalidates an old writer. When writer B increments the epoch, writer A's next operation reads the new epoch value and discovers it has been fenced. Writer A then refuses to proceed (returns SQLSTATE 57P04). Fencing ensures that exactly one writer can modify the catalog at any time.

---

## G

**Garbage Collection (GC)** — The two-phase process of cleaning up old catalog data. Phase 1: advancing the retain_from horizon (making old snapshots inaccessible to time-travel queries). Phase 2: excision (physically deleting superseded rows whose end_snapshot is below retain_from). GC is necessary to bound storage growth over time.

---

## H

**Hot Key** — A cached system key that is read on nearly every catalog operation (e.g., the current writer epoch, the latest snapshot ID). The hot key cache stores these values in memory to avoid a storage round-trip on every request. Cache invalidation occurs on writes (when the writer updates the hot key value).

---

## I

**Immutability** — The fundamental property that catalog rows, once written to SlateDB, are never modified in place. When an entity is updated (renamed, altered), a NEW row is written with the new values. The old row gets its end_snapshot set (in the same write batch) but is otherwise unchanged. This enables time travel and simplifies concurrency.

**Inlined Data** — Small data (below the inline threshold) that is stored directly within a catalog value rather than as a separate object in storage. This avoids the overhead of a separate storage object for very small payloads. Controlled by `ROCKLAKE_INLINE_THRESHOLD_BYTES`.

---

## K

**Key** — The binary identifier for a catalog entry in SlateDB. Every key starts with a tag byte (identifying the entity type) followed by big-endian u64 components (entity IDs, snapshot IDs). Keys are designed so that byte-level lexicographic sort produces the desired access pattern (related entries sort together).

**Key Layout** — The specific encoding scheme for keys: `[tag][parent_id][entity_id][!snapshot_id]`. The `!` means descending sort (XOR with u64::MAX). This ensures that within a given entity, the most recent version (highest snapshot) sorts first in a prefix scan.

---

## L

**LSM-Tree (Log-Structured Merge Tree)** — The data structure used by SlateDB internally. Writes go to a WAL, then to an in-memory memtable, which is periodically flushed to sorted SST files in object storage. Read operations check the memtable and SST files (using bloom filters and binary search). Compaction merges SST files to maintain read performance.

---

## M

**Manifest** — SlateDB's single source of truth: a file in object storage that lists all current WAL segments and SST files that constitute the database state. The manifest is updated atomically (one PUT). Reading the manifest is the first step on startup — it tells the reader what files to consider.

**MVCC (Multi-Version Concurrency Control)** — The mechanism that allows multiple versions of the same logical entity (schema, table, column) to coexist in storage. Each version has a begin_snapshot and end_snapshot. A reader at snapshot N sees only versions where begin_snapshot <= N AND (end_snapshot IS NULL OR end_snapshot > N). This provides snapshot isolation without locks.

---

## O

**Object Storage** — Cloud storage services (AWS S3, Google Cloud Storage, Azure Blob Storage) that provide durable, scalable, pay-per-use storage accessible via HTTP. RockLake persists all catalog data in object storage via SlateDB. Object storage provides durability (11 nines), scalability (unlimited), and availability (99.99%) without managing disk hardware.

---

## P

**PG-Wire (PostgreSQL Wire Protocol)** — The binary message format used for communication between PostgreSQL clients and servers. RockLake implements the server side of this protocol, allowing DuckDB (which has a PostgreSQL wire client) to communicate with RockLake as if it were a PostgreSQL database. Only a subset of the protocol is implemented (no extended query protocol features like cursors).

**Prefix Scan** — The primary read operation in RockLake. Given a key prefix (e.g., `[0x05][schema_id]`), SlateDB returns all key-value pairs whose keys start with that prefix, in sorted order. This is how RockLake implements "list all tables in schema X" — it scans the prefix `[TAG_TABLE][schema_id]`.

**Protobuf (Protocol Buffers)** — The binary serialization format used for RockLake's values. Each row struct (SchemaRow, TableRow, ColumnRow, etc.) is encoded as a protobuf message. Protobuf provides compact encoding, schema evolution (adding fields without breaking existing data), and fast encode/decode.

---

## R

**Retain From** — The snapshot ID below which time-travel queries are rejected. Set by garbage collection's first phase (advancing retention). If retain_from is 100, queries for snapshots 1–99 return an error. Snapshots 100 and above are accessible. This allows GC to eventually excise data from snapshots below the horizon.

---

## S

**SDKV** — The four-byte magic signature in RockLake's value envelope: `0x53 0x44 0x4B 0x56` (ASCII "SDKV"). Present in every stored value. Serves as a corruption detection mechanism — if a read returns bytes that do not start with the expected format version + SDKV magic, the data is corrupt or the format version is wrong.

**Snapshot** — An atomic point-in-time view of the catalog. Each write transaction that commits creates exactly one snapshot with a unique, monotonically increasing ID. Snapshots are the unit of consistency — readers choose a snapshot and see a fully consistent view of the catalog at that moment. No partial transactions are visible.

**SlateDB** — The Rust-native LSM-tree key-value store that RockLake uses for persistence. SlateDB writes directly to object storage (not local disk). It provides atomic write batches, prefix scans, and byte-level key ordering — the three operations RockLake needs.

**SST (Sorted String Table)** — An immutable, sorted file containing key-value pairs in key order. Written by SlateDB during memtable flush and compaction. SST files are the "permanent" storage format (vs. WAL segments which are temporary). SST files are stored as individual objects in S3/GCS/Azure.

**Strategy B** — One of two deployment strategies for RockLake. Strategy B runs RockLake as a standalone process (sidecar) that DuckDB connects to over the PostgreSQL wire protocol (TCP). This is the primary deployment mode.

**Strategy C** — The legacy name for the native DuckDB extension deployment strategy. Renamed to **Native DuckDB Extension** in v0.35.0. See *Native DuckDB Extension* and *Embedded Client Library*.

**Native DuckDB Extension** — Deployment strategy (formerly Strategy C) where RockLake runs as a native DuckDB extension (`.duckdb_extension` shared library loaded into the DuckDB process). Catalog operations are in-process function calls rather than network round-trips. Lower latency but tighter coupling. Builds on the stable `rocklake.h` C ABI introduced in v0.35.0.

**Embedded Client Library** — A universal C ABI (`rocklake.h`) and language bindings (Rust via `rocklake-client`, Python via PyO3, Go via cgo, Node.js via napi-rs) for embedding the RockLake catalog client in any language ecosystem without a PG-wire sidecar. DuckDB is a consumer but not the only one. Introduced in v0.35.0; provides the foundation for the Native DuckDB Extension (v0.36.0).

---

## T

**Tag** — The first byte of every key in RockLake's storage. Identifies the entity type (what "table" this key-value pair belongs to). For example, tag 0x04 means "schema," tag 0x05 means "table," tag 0x06 means "column." Tags enable prefix-based partitioning of the keyspace.

**Time Travel** — The ability to query the catalog at any historical snapshot, seeing the state as it existed at that point in time. For example, "what tables existed yesterday" or "what columns did this table have before the ALTER." Time travel is bounded by the retain_from horizon — snapshots below that boundary are inaccessible.

**Tombstone** — A special value written to SlateDB to indicate that a key has been deleted. In RockLake's context, tombstones represent excised entries (physically removed by GC). SlateDB removes tombstones during compaction.

---

## V

**Value Envelope** — The wrapper format around protobuf-encoded row data. Structure: `[format_version: u8][magic: 4 bytes "SDKV"][protobuf_payload: variable]`. The envelope enables format version detection and corruption checking.

---

## W

**WAL (Write-Ahead Log)** — SlateDB's durability mechanism. Before data is compacted into SST files, it exists as WAL segments — individual objects in storage containing one or more write batches. Each WAL segment is an atomic PUT. WAL segments are temporary (removed after compaction) but provide immediate durability.

**Wire Corpus** — A collection of recorded SQL statements captured from actual DuckDB sessions, stored as test fixtures in `tests/fixtures/wire-corpus/`. Organized by DuckDB version. Used to verify that RockLake's SQL classifier correctly handles real DuckDB output.

**Writer** — The single process authorized to create new snapshots (modify the catalog). Identity is established by holding the highest epoch. There is always exactly one active writer (or zero, if the system is idle). Multiple processes may ATTEMPT to be the writer, but the epoch mechanism ensures only one succeeds.

**Write Batch** — A set of key-value operations (puts and deletes) that are committed atomically in a single SlateDB WAL segment. This is RockLake's transaction mechanism — all mutations in a catalog transaction (creating a table with columns, for example) go into one write batch. Either ALL of them are durable, or NONE of them are.

---

## Numbers and Symbols

**!snapshot_id** — In key encoding documentation, the `!` prefix indicates descending sort order. Achieved by XOR-ing the u64 value with `u64::MAX` before encoding to big-endian bytes. This makes higher values sort before lower values in byte-level lexicographic order.

**0x01–0xFF** — Hexadecimal tag byte values. Tags 0x01–0x1F are reserved for DuckLake protocol tables. Tags 0xFE–0xFF are system tables. Unallocated tags between these ranges are available for future use.
