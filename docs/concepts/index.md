# Concepts

This section explains the mental model behind SlateDuck.

## Pages in this section

- [Catalog vs. Data Plane](catalog-vs-data.md) — The fundamental separation
- [Snapshots](snapshots.md) — Point-in-time consistency and time travel
- [Key-Value Mapping](key-value-mapping.md) — How relational tables become KV pairs
- [MVCC](mvcc.md) — Multi-version concurrency control without a database
- [Single Writer, Many Readers](single-writer-many-readers.md) — The concurrency model
- [Bounded SQL](bounded-sql.md) — Why a bounded dispatcher, not a full SQL engine
- [Object-Store Durability](object-store-durability.md) — How SlateDB uses S3 as a WAL
- [Immutability](immutability.md) — Why committed data is never deleted
