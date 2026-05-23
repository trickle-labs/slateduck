# Architecture

Deep technical detail on SlateDuck's internal architecture.

## Pages in this section

- [Overview](overview.md) — System context and component relationships
- [Crate Structure](crate-structure.md) — The 7-crate workspace and dependency rules
- [Key Layout](key-layout.md) — Binary key encoding for all 28 DuckLake tables
- [Value Encoding](value-encoding.md) — Protobuf encoding with forward/backward compatibility
- [MVCC Implementation](mvcc-implementation.md) — Visibility filter and read paths
- [Transaction Model](transaction-model.md) — Atomic multi-key commits
- [PG-Wire Protocol](pg-wire-protocol.md) — Wire format and session lifecycle
- [SQL Dispatcher](sql-dispatcher.md) — Statement classification and dispatch pipeline
