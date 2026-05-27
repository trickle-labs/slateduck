# Architecture

This section provides a deep technical exploration of Rocklake's internal architecture. It is intended for contributors who want to understand the codebase, operators who want to reason about system behavior under failure, and curious users who want to know exactly what happens when they run a SQL statement against Rocklake.

The architecture is layered: PostgreSQL wire protocol at the top, SQL parsing and dispatch in the middle, MVCC-aware key-value operations at the bottom, and SlateDB handling durable persistence to object storage beneath everything. Each layer has clear boundaries, minimal coupling, and well-defined responsibilities. This is enforced at the Rust compilation level through the workspace crate structure.

## Reading Order

If you are new to the architecture section, we recommend this order:

1. **[Overview](overview.md)** — Start here. The system-level view showing how clients connect, how data flows through the system, and how catalog state is persisted to object storage.

2. **[Crate Structure](crate-structure.md)** — The Rust workspace layout. Seven crates with clear dependency boundaries that enforce separation of concerns at compile time.

3. **[Key Layout](key-layout.md)** — The binary encoding scheme that maps relational catalog concepts (schemas, tables, columns, files) into lexicographically-ordered byte keys in the LSM-tree.

4. **[Value Encoding](value-encoding.md)** — How each catalog row is serialized to bytes: protobuf encoding wrapped in a versioned envelope with magic bytes for corruption detection.

5. **[SQL Dispatcher](sql-dispatcher.md)** — How incoming SQL is parsed, classified into one of approximately 50 known statement kinds, and dispatched to the appropriate catalog operation.

6. **[PG-Wire Protocol](pg-wire-protocol.md)** — The PostgreSQL wire protocol implementation: connection lifecycle, authentication, query execution, result encoding, and type mapping.

7. **[Transaction Model](transaction-model.md)** — How DuckDB's logical transactions map to batched SlateDB writes with snapshot isolation and atomic commit.

8. **[MVCC Implementation](mvcc-implementation.md)** — The storage-level implementation of multi-version concurrency control, including key encoding of version ranges, visibility filtering, and the three versioning behaviors (versioned, append-only, mutable singleton).

## Key Architectural Principles

Several principles guide the architecture:

- **Bounded SQL surface.** Rocklake does not implement a general SQL engine. It recognizes a finite set of statement shapes required by DuckLake and rejects everything else. This makes the system predictable and auditable.

- **Single-writer atomicity.** All catalog mutations flow through a single writer process. This eliminates distributed consensus, conflict resolution, and partial-write recovery from the design.

- **Storage-level MVCC.** Version information is encoded directly in key-value pairs rather than being managed by a separate transaction manager. This means MVCC works even when reading cold SST files from object storage.

- **Fail-fast, fail-safe.** Unknown SQL, unexpected message types, and protocol violations are rejected immediately with appropriate SQLSTATE error codes rather than being silently handled or partially processed.
