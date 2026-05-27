# The Lakehouse Model

If you are reading this documentation, you probably already store analytical data somewhere — perhaps in a data warehouse like Snowflake or BigQuery, perhaps in a collection of CSV or Parquet files on S3, perhaps in a PostgreSQL database that has grown beyond what its hardware can comfortably handle. The lakehouse model offers a different way to organize analytical data that combines the best properties of data lakes (cheap, durable, infinitely scalable object storage) with the best properties of data warehouses (structured schemas, ACID transactions, time travel, efficient query planning). Understanding what a lakehouse is and why the catalog is its most critical component will give you the context to understand everything else in this documentation.

## What is a Data Lake?

A data lake is, at its simplest, a bucket full of files. You store your analytical data — typically as Parquet files, but also CSV, JSON, ORC, or Avro — in an object store like Amazon S3, Google Cloud Storage, or Azure Blob Storage. The storage is cheap (a few cents per gigabyte per month), virtually unlimited in capacity, and extraordinarily durable (S3 provides 99.999999999% durability, meaning you would expect to lose one object out of 10 billion over a 10,000-year period). You can store petabytes of data without worrying about disk capacity, RAID configurations, or storage provisioning.

The appeal is obvious: you decouple storage from compute. Your data lives in the bucket regardless of what query engine you use to read it. You can query the same Parquet files with DuckDB, Spark, Trino, Polars, or any other engine that understands the file format. You can add more compute without moving data, and you can stop paying for compute without losing data. This is fundamentally different from a traditional data warehouse, where the data and the query engine are tightly coupled — your data lives inside Snowflake, and the only way to query it is through Snowflake.

But a data lake has a critical weakness: it has no brain. A bucket full of Parquet files does not know which files belong to which logical table. It does not know what schema each table has. It does not know how the schema has changed over time, or which files are still valid versus which have been replaced by newer versions. It cannot tell you what the data looked like last Tuesday, because it has no concept of versioning. It cannot enforce that two concurrent writers do not corrupt each other's work, because it has no concept of transactions. It is just files.

## What is a Catalog?

A catalog is the brain of the lakehouse. It is a database — sometimes a full relational database, sometimes a specialized metadata service — that tracks everything the data lake cannot track on its own:

**Schema management.** The catalog knows that there is a table called `events` in a schema called `analytics`, that it has five columns (`event_id BIGINT`, `user_id BIGINT`, `event_type VARCHAR`, `created_at TIMESTAMP`, `payload VARCHAR`), and that the schema was last modified three weeks ago when someone added the `payload` column.

**File tracking.** The catalog knows which Parquet files contain data for each table. A table with a billion rows might be spread across 10,000 Parquet files in the bucket; the catalog tracks each file's path, row count, byte size, and column-level statistics (min/max values for each column in each file, enabling predicate pushdown to skip irrelevant files during queries).

**Versioning.** The catalog tracks changes as a sequence of snapshots. Snapshot 1 created the schema. Snapshot 2 created the table. Snapshot 3 added the first batch of data files. Snapshot 4 added a column. Each snapshot is a consistent, queryable state of the catalog. You can ask "what did the catalog look like at snapshot 3?" and get a precise answer.

**Transaction coordination.** When two processes try to modify the catalog concurrently — one adding files to a table while another alters the table's schema — the catalog ensures that these operations are serialized correctly and that no observer sees a partial state.

## Why the Catalog is the Hard Part

Storing files in a bucket is easy. Object storage was designed for exactly this use case, and it works flawlessly at scale. The hard part is the catalog — not because storing metadata is intrinsically difficult, but because doing so correctly requires solving all the same problems that database engineers have spent decades solving: concurrent access, crash safety, snapshot isolation, schema evolution, and efficient querying of metadata that changes over time.

Consider what happens when a data pipeline loads a new batch of data into a lakehouse table. The pipeline writes Parquet files to the bucket, then updates the catalog to register those files. If the pipeline crashes after writing the files but before updating the catalog, you have orphaned files in the bucket that no query will ever find. If it crashes after partially updating the catalog, you might have a table that references files that do not exist or that is missing files that do exist. If two pipelines load data concurrently and both try to update the catalog at the same time, you need a way to ensure that one does not overwrite the other's changes.

These are not hypothetical problems — they are the everyday reality of operating a data platform at scale. Every lakehouse format solves them, but they solve them in different ways with different trade-offs.

## The Landscape of Solutions

**Apache Iceberg** stores its catalog as a tree of JSON manifest files and Avro data files within the data lake itself. The "current" state is defined by a pointer (typically stored in a catalog service like AWS Glue or Hive Metastore) that references the root manifest. Iceberg is powerful and flexible, but the manifest tree can become complex and slow to traverse for large tables, and the reliance on atomic file renames or an external catalog service for the pointer adds operational complexity.

**Delta Lake** stores its catalog as a JSON transaction log alongside the data files. Each transaction appends a new JSON file to the log; the current state is the result of replaying all log entries. Delta Lake is simpler than Iceberg in some ways but introduces its own complexity around checkpoint files (periodic snapshots of the log for faster reads) and the reliance on either atomic renames (which not all object stores support) or a coordination service.

**Apache Hive Metastore** delegates the catalog to a MySQL or PostgreSQL database. This is the oldest approach and the most operationally familiar to teams that already run relational databases. The downside is that you are now running a database server to support your data lake — which partly defeats the "no infrastructure" appeal of object storage.

**DuckLake** takes a different position in this design space. Rather than inventing a new file format for metadata or requiring a specific coordination service, DuckLake defines the catalog as a set of 28 SQL tables with a well-known schema, versioned with snapshot IDs, accessible over the PostgreSQL wire protocol. The catalog is just a database — any database that implements the required tables and speaks the wire protocol can serve as a DuckLake catalog backend. This simplicity is what enables Rocklake to exist: if the catalog is just a database, Rocklake can be that database, implemented on top of object storage instead of traditional infrastructure.

## The Catalog Problem in Practice

To understand why the catalog matters so much, consider the lifecycle of a typical analytical query. When DuckDB receives a query like `SELECT * FROM events WHERE created_at > '2024-01-01'`, it needs to answer several questions before it can begin scanning data:

1. **Which table?** The catalog resolves the name `events` to a specific table ID and retrieves its schema (column names, types, nullability).
2. **Which files?** The catalog returns the list of Parquet files that contain data for this table. A large table might have thousands of files.
3. **Which files are relevant?** Column-level statistics (min/max values for `created_at` in each file) allow the query engine to skip files that cannot possibly contain matching rows. This is called predicate pushdown or partition pruning.
4. **Which version?** If the query is running within a transaction, it sees a consistent snapshot. Files added by concurrent, uncommitted transactions are invisible.

All of this metadata lives in the catalog. A slow or unavailable catalog means slow or failed queries, regardless of how powerful your query engine is or how efficiently your data files are organized. The catalog is the critical path for every single query.

This is also why catalog *correctness* matters more than catalog *performance* for most workloads. A catalog that returns slightly stale file statistics results in reading a few extra files — wasted I/O but correct results. A catalog that returns inconsistent results (mixing data from different transaction states) produces incorrect query results that may go undetected for weeks. Rocklake prioritizes consistency above all else: every read sees a complete, valid snapshot of the catalog state.

## Where Rocklake Fits

The key insight behind Rocklake is that the catalog database itself does not need to be a traditional database server. The DuckLake catalog schema — 28 tables of metadata, versioned with monotonically increasing snapshot IDs, accessed through a bounded set of SQL queries — maps naturally onto a key-value store. And if that key-value store happens to persist its state to object storage (as SlateDB does), then the entire catalog can live in the same bucket as the data files, with no additional infrastructure.

This means you end up with a data platform that consists of exactly two things: a bucket (containing both your Parquet data files and your SlateDB catalog files) and ephemeral compute (Rocklake sidecar processes and DuckDB query engines that start, do work, and stop without leaving any persistent local state). There is no database server to keep running between queries, no replication topology to monitor, no backup schedule to maintain. The bucket is the system of record for both data and metadata.

The rest of this Concepts section explains how Rocklake makes this possible — the DuckLake format it implements, the SlateDB engine it uses for storage, the immutability principle that enables its distinctive properties, and the MVCC model that provides transactional consistency. Each concept builds on the previous one, and by the end of the section you will have a complete mental model of why Rocklake works the way it does.

## Further Reading

- **[The DuckLake Format](ducklake.md)** — The next page in this series, explaining the specific catalog schema that Rocklake implements
- **[Architecture Overview](../architecture/index.md)** — For readers who want to jump straight to the implementation details
- **[Design Decisions: Why SlateDB?](../design-decisions/why-slatedb.md)** — The reasoning behind choosing SlateDB over PostgreSQL, SQLite, or other backends
