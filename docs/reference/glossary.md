# Glossary

**Bounded SQL Dispatcher**
: Accepts only a finite set of SQL statement shapes.

**Catalog Plane**
: Metadata layer tracking what data exists and where.

**Data Plane**
: Actual data storage (Parquet files). DuckDB handles this directly.

**DuckLake**
: Open lakehouse catalog format by DuckDB Labs.

**Excision**
: Irreversible deletion for compliance.

**GC**
: Physical deletion of data exceeding retention window.

**Hot Key**
: System key caching derived state for fast cold-start reads.

**MVCC Filter**
: `begin_snapshot <= target AND (end_snapshot IS NULL OR target < end_snapshot)`

**Metadata Packing**
: All per-table metadata in one value for single-read describe_table.

**Secondary Index**
: Skip-index for snapshot-scoped lookups without MVCC scans.

**SlateDB**
: Cloud-native LSM-tree KV store on object storage.

**Snapshot**
: Point-in-time consistent catalog view with monotonic ID.

**Strategy B**
: PG-wire sidecar deployment.

**Strategy C**
: Native DuckDB extension via FFI.

**Tag**
: First byte of every key, identifying the table type.

**Wire Corpus**
: Captured PG wire messages used as compatibility ground truth.

**Writer Epoch**
: Monotonic integer for writer fencing.
