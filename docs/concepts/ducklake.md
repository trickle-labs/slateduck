# The DuckLake Format

DuckLake is the lakehouse catalog format that Rocklake implements. Created by the DuckDB team, it defines a catalog as a set of 28 SQL tables — schemas, tables, columns, data files, statistics, snapshots, and more — versioned with monotonically increasing snapshot IDs and accessible over the PostgreSQL wire protocol. If you understand DuckLake, you understand what Rocklake is implementing and why it makes the design choices it does. This page explains the format in full from Rocklake's perspective: what the tables are, how versioning works, what the bounded query set looks like, and how the catalog plane separates from the data plane.

## The 28 Catalog Tables

DuckLake's catalog is defined as 28 relational tables that together describe the complete state of a lakehouse. Each table has a specific purpose, and the relationships between them form a coherent schema for tracking schemas, tables, columns, data files, and their evolution over time. Here are the major categories:

**Core catalog entities.** These tables define the logical structure of the lakehouse:

- `ducklake_snapshot` — Every mutation to the catalog creates a new snapshot. This table records each snapshot's ID, creation timestamp, and the transaction that created it.
- `ducklake_schema` — Schemas (namespaces) that group related tables. Each schema has a unique ID and a name.
- `ducklake_table` — Tables within schemas. Each table has a unique ID, a name, a reference to its containing schema, and metadata like creation time.
- `ducklake_column` — Columns within tables. Each column has an ID, a name, a data type, a position ordinal, and a reference to its containing table.
- `ducklake_view` — Views (saved queries) within schemas.
- `ducklake_macro` — User-defined macros (functions).

**Data file management.** These tables track the physical Parquet files that contain actual data:

- `ducklake_data_file` — Every Parquet file registered with a table. Tracks the file's path in object storage, its row count, byte size, and which table it belongs to.
- `ducklake_data_file_column_statistics` — Per-column statistics for each data file: min values, max values, null counts. These enable predicate pushdown — the query engine can skip files that provably do not contain relevant rows based on the column statistics.
- `ducklake_delete_file` — Delete files that mark specific rows as removed (for merge-on-read patterns).

**Schema evolution tracking.** These tables record how the schema has changed over time:

- `ducklake_table_changes` — Records of schema-altering operations: column additions, removals, renames, type changes.

**Statistics and metadata.** Tables for query optimization:

- `ducklake_table_stats` — Table-level statistics: total row count, total file size, file count.

The full set of 28 tables covers additional concerns like encryption keys, tags, and extended properties, but the tables listed above are the ones most relevant to understanding Rocklake's implementation.

## Snapshot-Based Versioning

DuckLake's versioning model is elegant in its simplicity. Every mutation to the catalog — creating a schema, adding a table, registering a data file, altering a column — creates a new snapshot with a monotonically increasing integer ID. The snapshot ID serves as the version number for the entire catalog: "the catalog at snapshot 15" is a well-defined, immutable state that can be queried at any time in the future.

Versioning is implemented through two columns that appear on most catalog tables: `begin_snapshot` and `end_snapshot`. When a row is created, its `begin_snapshot` is set to the current snapshot ID, and its `end_snapshot` is NULL (meaning it is still current). When a row is superseded — for example, when a table is renamed or a column's type is changed — its `end_snapshot` is set to the snapshot at which the change occurred, and a new row is created with the new values and a new `begin_snapshot`.

This means that the state of the catalog at any snapshot ID can be reconstructed by filtering: a row is visible at snapshot S if and only if `begin_snapshot <= S AND (end_snapshot IS NULL OR S < end_snapshot)`. This is classic Multi-Version Concurrency Control (MVCC), adapted for a catalog that evolves through discrete snapshots rather than concurrent transactions.

The implications are profound. Because old versions of rows are never deleted (only superseded by having their `end_snapshot` set), the entire history of the catalog is available for querying. You can ask "what tables existed at snapshot 5?" or "what columns did this table have before last week's migration?" and get precise answers. This is time travel — not as a special feature, but as the natural consequence of the versioning model.

## The Bounded Query Set

One of the most important things to understand about DuckLake — and about Rocklake's implementation of it — is that the set of SQL queries that a DuckLake client (DuckDB's `ducklake` extension) issues against the catalog is finite and well-defined. DuckDB does not run arbitrary SQL against the catalog. It issues a specific set of queries that fall into predictable categories:

**Point reads.** "Give me the table with ID 5 at the current snapshot." These are simple primary-key lookups that translate directly to key-value `GET` operations.

**Range scans with snapshot filtering.** "Give me all columns for table ID 5 that are visible at snapshot 12." These scan a range of keys (all columns belonging to a specific table) and apply the MVCC visibility filter.

**Aggregations.** "What is the maximum snapshot ID?" or "How many data files does table 5 have?" These are bounded operations over known key ranges.

**Writes.** `INSERT INTO ducklake_data_file (...)` to register a new file, `UPDATE ducklake_table SET end_snapshot = 15 WHERE ...` to supersede a row. Always parameterized, always against known tables with known column sets.

**Session management.** `SET timezone = 'UTC'`, `SHOW server_version`, `SELECT current_schema()`. Protocol-level queries that DuckDB issues during connection setup.

This bounded query set is what makes Rocklake's bounded SQL dispatcher possible. Rather than implementing a general SQL engine (which would be an enormous undertaking), Rocklake implements a pattern-matching classifier that recognizes each known query shape and dispatches it to the corresponding key-value operation. If a query does not match any known shape, it is rejected with `SQLSTATE 0A000` (feature not supported). This is not a limitation in practice — it is a deliberate security and correctness boundary that ensures no unexpected query can modify the catalog in unintended ways.

## Catalog Plane vs. Data Plane

DuckLake draws a clean line between two planes of operation:

**The catalog plane** is everything Rocklake manages: schema definitions, table metadata, column definitions, data file registrations, statistics, snapshot history. This is the "brain" of the lakehouse — it knows what exists and where to find it, but it never handles actual data.

**The data plane** is everything DuckDB manages directly: reading Parquet files, writing Parquet files, computing query results, applying predicates, performing joins and aggregations. DuckDB talks directly to the object store for all data operations — Rocklake is not in the data path.

The connection between the two planes is the `data_path` field in catalog rows. When DuckDB asks "which files contain data for table `events`?", Rocklake returns a list of `data_path` values — S3 URIs pointing to Parquet files. DuckDB then reads those files directly from S3. Rocklake never reads or writes Parquet files, and DuckDB never reads or writes SlateDB's SST files.

This separation has important consequences:

**Performance.** Rocklake is never in the hot path of query execution. A query that scans a terabyte of Parquet data involves Rocklake only for the initial metadata lookup (milliseconds). The actual data scanning is between DuckDB and S3 directly.

**Security.** You can give Rocklake credentials that only allow access to the catalog prefix (`s3://bucket/catalogs/`) and give DuckDB credentials that only allow access to the data prefix (`s3://bucket/data/`). A compromise of either component cannot affect the other.

**Scalability.** Scaling the data plane (more DuckDB instances reading more data) does not require scaling the catalog plane (Rocklake handles metadata lookups that are orders of magnitude smaller than data reads).

## The PostgreSQL Wire Protocol

DuckLake communicates with its catalog backend over the PostgreSQL wire protocol — the same binary protocol that PostgreSQL itself uses. DuckDB's `ducklake` extension opens a TCP connection, performs a PostgreSQL startup handshake, and then sends SQL queries as PostgreSQL `Query` or `Parse/Bind/Execute` messages. The catalog backend responds with `RowDescription`, `DataRow`, and `CommandComplete` messages exactly as a PostgreSQL server would.

Rocklake implements this protocol faithfully enough that DuckDB cannot distinguish it from a real PostgreSQL server. The startup handshake includes the expected responses to DuckDB's probing queries (`pg_catalog.pg_type`, `current_schema()`, `server_version`). The type OIDs in column descriptions match PostgreSQL's standard type catalog. The error responses use standard SQLSTATE codes that DuckDB knows how to interpret.

This is a deliberate compatibility choice. By implementing the wire protocol that DuckDB already speaks, Rocklake becomes a drop-in replacement for any PostgreSQL-backed DuckLake catalog. The migration path is trivial: change the connection string in your `ATTACH` statement, and everything else stays the same.

## How Rocklake Maps DuckLake to SlateDB

The core of Rocklake's implementation is the mapping between DuckLake's 28 relational tables and SlateDB's key-value store. Each catalog table gets a unique 1-byte tag that prefixes all its keys. Within a table's key space, the key structure is designed to make the most common query pattern a single prefix scan:

- For `ducklake_column`, the key is `[tag][table_id][column_id][begin_snapshot]` — so "all columns for table 5" is a prefix scan on `[0x06][0x00000005]`.
- For `ducklake_data_file`, the key is `[tag][table_id][file_id][begin_snapshot]` — so "all files for table 5" is a prefix scan on the data-file tag followed by the table ID.
- For `ducklake_snapshot`, the key is `[tag][snapshot_id]` — so "the latest snapshot" is a reverse scan from the maximum key in the snapshot namespace.

The value for each key is a Protobuf-encoded message containing the row's non-key columns, prefixed with a 5-byte header (`encoding_version` byte + `SDKV` magic) for corruption detection and forward compatibility.

This mapping is what makes Rocklake efficient: the most common DuckLake operations (looking up files for a table, reading columns for a table, finding the current snapshot) translate to single prefix scans or point lookups in the key-value store — operations that SlateDB handles in a small number of object-store GET requests.

## What DuckLake Does Not Require

Understanding what DuckLake does not require helps clarify Rocklake's design boundaries:

**DuckLake does not require a general SQL engine.** The query set is bounded, so a pattern-matching dispatcher suffices. Rocklake does not need a query planner, a cost optimizer, or a general expression evaluator.

**DuckLake does not require multi-writer concurrency control.** The protocol assumes a single catalog connection at a time for writes (though reads can be concurrent). Rocklake's single-writer model is a natural fit.

**DuckLake does not require the catalog to understand Parquet.** The catalog stores file paths and statistics, but it never opens or parses Parquet files. Rocklake has zero Parquet dependencies.

**DuckLake does not require real-time replication.** Readers open snapshots, and consistency is defined at the snapshot level. There is no need for streaming WAL replication or eventual consistency protocols.

These non-requirements are what make Rocklake possible as a lean, focused implementation rather than a general-purpose database. Every feature that DuckLake does not require is infrastructure that Rocklake does not need to build or operate.

## Further Reading

- **[The SlateDB Storage Engine](slatedb.md)** — How SlateDB provides the durability and transaction guarantees that DuckLake requires
- **[Architecture: Key Layout](../architecture/key-layout.md)** — The full binary encoding for all 28 table key structures
- **[Architecture: SQL Dispatcher](../architecture/sql-dispatcher.md)** — How the bounded query set is classified and dispatched
- **[Reference: Catalog Tables](../reference/catalog-tables.md)** — Complete reference for all 28 tables with column definitions
