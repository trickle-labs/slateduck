# SlateDuck Architecture

## Overview

SlateDuck is a lakehouse catalog backed by SlateDB — both catalog metadata and
Parquet data files reside in the same object-storage bucket with zero external
infrastructure.

## Component Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        DuckDB Client                        │
│                   (ducklake extension)                       │
└─────────────────────┬───────────────────────────────────────┘
                      │ PostgreSQL Wire Protocol
                      ▼
┌─────────────────────────────────────────────────────────────┐
│                   slateduck-pgwire                           │
│              (PG wire protocol sidecar)                      │
├─────────────────────────────────────────────────────────────┤
│                    slateduck-sql                             │
│           (Bounded SQL AST dispatcher)                       │
├─────────────────────────────────────────────────────────────┤
│                  slateduck-catalog                           │
│          (DuckLake catalog operations)                       │
├─────────────────────────────────────────────────────────────┤
│                   slateduck-core                             │
│       (Key layout, encoding, SlateDB integration)           │
└─────────────────────┬───────────────────────────────────────┘
                      │ SlateDB API
                      ▼
┌─────────────────────────────────────────────────────────────┐
│                       SlateDB                                │
│            (LSM-tree on object storage)                      │
└─────────────────────┬───────────────────────────────────────┘
                      │ object_store API
                      ▼
┌─────────────────────────────────────────────────────────────┐
│              Object Storage (S3/GCS/Azure/Local)            │
│                                                             │
│   catalogs/warehouse-a/     data/warehouse-a/               │
│   (SlateDB WAL + SSTs)      (Parquet files)                 │
└─────────────────────────────────────────────────────────────┘
```

## Key Design Principles

1. **Catalog-data immutability:** Committed catalog facts are never physically
   deleted by normal operation. Physical deletion only via explicit excision.

2. **Single writer, many readers:** One SlateDB writer per catalog; unbounded
   concurrent readers via `DbReader`.

3. **Bounded SQL dispatch:** The sidecar implements only the finite set of SQL
   shapes emitted by DuckDB's `ducklake` extension. No general SQL execution.

4. **Object-store native:** All durable state lives in object storage. No local
   disk requirements beyond ephemeral caching.

## Data Flow

### Read Path
1. DuckDB sends SQL query via PG wire protocol
2. `slateduck-sql` parses and classifies the AST
3. `slateduck-catalog` executes the corresponding read operation
4. `slateduck-core` performs SlateDB `scan_prefix` / `get` with MVCC filter
5. Results encoded as PG wire response rows

### Write Path
1. DuckDB sends `BEGIN` + series of `INSERT`/`UPDATE` + `COMMIT`
2. Statements accumulate in `PendingCatalogTxn`
3. On `COMMIT`: single `DbTransaction` with all mutations
4. `flush()` makes commit visible to readers
5. PG wire `COMMIT` response sent

## For detailed design, see [plans/blueprint.md](../plans/blueprint.md).
