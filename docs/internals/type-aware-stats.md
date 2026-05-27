# Type-Aware Statistics

Rocklake stores per-column statistics for each data file: minimum value, maximum value, null count, and distinct count estimates. These statistics enable partition pruning — DuckDB's ability to skip entire data files when their column statistics prove that no rows in the file can match a query predicate. This is one of the most powerful performance optimizations in a lakehouse architecture, potentially reducing query I/O by orders of magnitude.

The challenge is that "minimum value" means different things for different data types. The minimum of integers is numerical ordering. The minimum of strings is lexicographic ordering. The minimum of timestamps is chronological ordering. The minimum of UUIDs is... well, it depends on whether you sort by string representation or by the underlying 128 bits. Rocklake must store these statistics in a uniform binary format (protobuf bytes) while preserving the type-specific comparison semantics that DuckDB needs for correct pruning.

This page documents how statistics are encoded for each DuckDB type, how the type registry works, and how DuckDB uses these statistics during query planning.

## The Statistics Table

Statistics are stored in the `ducklake_file_column_stats` table (tag 0x09):

```
Key: 0x09 | file_id (u64) | column_id (u64)
Value: SDKV envelope containing:
  - min_value: bytes (type-specific encoding)
  - max_value: bytes (type-specific encoding)
  - null_count: u64
  - has_null: bool
  - distinct_count: u64 (estimated)
  - row_count: u64
```

Each entry describes one column within one data file. A table with 50 columns and 1,000 data files has 50,000 statistics entries. This sounds large, but each entry is compact (typically 50–200 bytes) and only relevant entries are fetched during query planning (filtered by the columns referenced in query predicates).

## Type Encoding

The `min_value` and `max_value` fields are opaque byte arrays. Their encoding depends on the column's DuckDB type. The type registry in `crates/rocklake-core/src/types.rs` defines the encoding for each supported type.

### Integer Types

All integer types use fixed-width big-endian encoding:

| DuckDB Type | Encoding | Width | Example (value 42) |
|-------------|----------|-------|---------------------|
| TINYINT | i8 big-endian | 1 byte | `0x2A` |
| SMALLINT | i16 big-endian | 2 bytes | `0x00 0x2A` |
| INTEGER | i32 big-endian | 4 bytes | `0x00 0x00 0x00 0x2A` |
| BIGINT | i64 big-endian | 8 bytes | `0x00 0x00 0x00 0x00 0x00 0x00 0x00 0x2A` |
| HUGEINT | i128 big-endian | 16 bytes | ... |
| UTINYINT | u8 big-endian | 1 byte | `0x2A` |
| USMALLINT | u16 big-endian | 2 bytes | `0x00 0x2A` |
| UINTEGER | u32 big-endian | 4 bytes | `0x00 0x00 0x00 0x2A` |
| UBIGINT | u64 big-endian | 8 bytes | `0x00 0x00 0x00 0x00 0x00 0x00 0x00 0x2A` |

**Why big-endian?** The same reason as key encoding: big-endian preserves numeric ordering under byte comparison. DuckDB can compare min/max bytes directly without decoding to integers first (though in practice DuckDB decodes and uses native comparison).

**Signed integer encoding:** For signed integers, the encoding uses a bias (adding 2^(n-1)) to convert the signed range to unsigned, ensuring that negative numbers sort before positive numbers in byte comparison. -128 becomes 0x00, 0 becomes 0x80, 127 becomes 0xFF for TINYINT.

### String and VARCHAR Types

| DuckDB Type | Encoding | Width |
|-------------|----------|-------|
| VARCHAR | Raw UTF-8 bytes | Variable |
| BLOB | Raw bytes | Variable |

Strings are stored as-is (UTF-8 bytes). No length prefix, no null terminator. The protobuf field handles length framing.

**Pruning with strings:** DuckDB can prune files using string comparison:
- `WHERE name > 'M'` — skip files where max_value < 'M' (byte comparison)
- `WHERE city = 'Berlin'` — skip files where min_value > 'Berlin' OR max_value < 'Berlin'

### Temporal Types

| DuckDB Type | Encoding | Width | Representation |
|-------------|----------|-------|---------------|
| TIMESTAMP | i64 big-endian | 8 bytes | Microseconds since epoch |
| TIMESTAMP_S | i64 big-endian | 8 bytes | Seconds since epoch |
| TIMESTAMP_MS | i64 big-endian | 8 bytes | Milliseconds since epoch |
| TIMESTAMP_NS | i64 big-endian | 8 bytes | Nanoseconds since epoch |
| DATE | i32 big-endian | 4 bytes | Days since epoch |
| TIME | i64 big-endian | 8 bytes | Microseconds since midnight |
| INTERVAL | 16 bytes | 16 bytes | months (i32) + days (i32) + micros (i64) |

**Timestamp encoding:** The epoch is 1970-01-01T00:00:00Z (Unix epoch). Timestamps before 1970 have negative values, which are encoded with the signed-to-unsigned bias for correct ordering.

**INTERVAL limitations:** Intervals do not have a total ordering (is "1 month" greater or less than "30 days"? It depends on which month). Statistics for INTERVAL columns are stored but DuckDB does not use them for pruning.

### Decimal Types

| DuckDB Type | Encoding | Width | Representation |
|-------------|----------|-------|---------------|
| DECIMAL(p, s) where p ≤ 18 | i64 big-endian | 8 bytes | Scaled integer (value × 10^s) |
| DECIMAL(p, s) where p > 18 | i128 big-endian | 16 bytes | Scaled integer (value × 10^s) |

**Example:** DECIMAL(10, 2) value 123.45 is stored as the integer 12345 (123.45 × 10^2), encoded as i64 big-endian.

The scale factor is not stored in the statistics — it is known from the column's type definition. DuckDB applies the same scale when comparing predicates against statistics.

### UUID Type

| DuckDB Type | Encoding | Width |
|-------------|----------|-------|
| UUID | 16 raw bytes (MSB first) | 16 bytes |

UUIDs are stored as their 16-byte binary representation. Ordering follows byte comparison (which corresponds to the canonical UUID string sort order for version 4 UUIDs).

### Boolean Type

| DuckDB Type | Encoding | Width |
|-------------|----------|-------|
| BOOLEAN | 1 byte (0x00 = false, 0x01 = true) | 1 byte |

Boolean statistics are useful for "has_true" and "has_false" semantics:
- min=false, max=false → file contains only FALSE values
- min=true, max=true → file contains only TRUE values
- min=false, max=true → file contains both

DuckDB can prune: `WHERE flag = TRUE` skips files where max=false (all values are FALSE).

### Float Types

| DuckDB Type | Encoding | Width | Representation |
|-------------|----------|-------|---------------|
| FLOAT | IEEE 754, sign-magnitude adjusted | 4 bytes | Special encoding for ordering |
| DOUBLE | IEEE 754, sign-magnitude adjusted | 8 bytes | Special encoding for ordering |

**Float ordering challenge:** IEEE 754 floats do not sort correctly under raw byte comparison (negative numbers have the sign bit set, making them appear "larger" than positive numbers). The encoding adjusts: positive floats are XORed with 0x80000000 (flipping the sign bit so they sort after negatives), and negative floats are fully inverted (so -1.0 sorts after -2.0).

This adjusted encoding ensures `byte_compare(encode(a), encode(b)) == (a < b)` for all non-NaN float values.

### Complex Types

| DuckDB Type | Statistics Support |
|-------------|-------------------|
| STRUCT | No min/max (composite type) |
| LIST | No min/max (variable-length composite) |
| MAP | No min/max (variable-length composite) |
| UNION | No min/max (heterogeneous type) |
| ENUM | Min/max stored as enum integer values |

Complex types do not have meaningful min/max statistics because they lack a total ordering. Rocklake stores NULL for min_value and max_value for these types. DuckDB cannot prune files based on complex-type columns.

ENUM types are an exception: they are internally represented as integers (the enum variant index), and min/max of the integer representation enables pruning.

## The Type Registry

The type registry is the mapping between DuckDB type names and their encoding/comparison functions:

```rust
pub struct TypeRegistry {
    encoders: HashMap<String, Box<dyn StatEncoder>>,
}

pub trait StatEncoder {
    fn encode(&self, value: &DuckDBValue) -> Vec<u8>;
    fn decode(&self, bytes: &[u8]) -> DuckDBValue;
    fn compare(&self, a: &[u8], b: &[u8]) -> Ordering;
}
```

Each entry in the registry provides:
- **encode:** Convert a DuckDB value to the binary statistics representation
- **decode:** Convert binary statistics back to a DuckDB value (for display/debugging)
- **compare:** Compare two encoded values without decoding (for validation)

The registry is initialized at startup with all known DuckDB types. Unknown types (from future DuckDB versions) default to "no statistics" (NULL min/max) — they are stored but not used for pruning.

## How DuckDB Uses Statistics

During query planning, DuckDB performs predicate pushdown using statistics:

### Step 1: Identify Relevant Predicates

```sql
SELECT * FROM events WHERE timestamp > '2024-06-01' AND category = 'sales'
```

DuckDB identifies two predicates: `timestamp > '2024-06-01'` and `category = 'sales'`.

### Step 2: Request File Statistics

DuckDB asks Rocklake for statistics of the `timestamp` and `category` columns for all data files in the `events` table. Rocklake returns statistics entries (one per file per column).

### Step 3: Apply Pruning Logic

For each data file, DuckDB evaluates:

- **Timestamp predicate:** If `max_timestamp < '2024-06-01'` for a file, ALL rows in that file have timestamps before June 1. The entire file can be skipped.
- **Category predicate:** If `min_category > 'sales'` OR `max_category < 'sales'` for a file, no rows in that file have category='sales'. Skip.

### Step 4: Execute on Remaining Files

Only files that MIGHT contain matching rows are actually read. Files that were provably empty (based on statistics) are never fetched from S3.

### Pruning Effectiveness

The effectiveness of statistics-based pruning depends on data layout:

| Data Layout | Pruning Effectiveness |
|-------------|---------------------|
| Time-partitioned (one file per day) | Excellent — time predicates skip most files |
| Randomly distributed | Poor — every file's range overlaps the predicate |
| Sorted by query column | Excellent — tight min/max ranges enable precise pruning |
| Unsorted, small files | Moderate — some files prunable by chance |

**Example impact:** A table with 1,000 daily files queried for "last 7 days" prunes 993 files (99.3% reduction in I/O). Without statistics, DuckDB would read all 1,000 files.

## Limitations

### Statistics Quality Depends on Source

Rocklake stores whatever statistics DuckDB reports at file registration time. If the Parquet file was written without column statistics (some writers skip them for performance), Rocklake stores NULL — and DuckDB cannot prune based on that column.

**Mitigation:** Most modern Parquet writers (DuckDB, Apache Spark, PyArrow) include column statistics by default.

### Per-File Granularity

Statistics are per-file, not per-row-group. A large Parquet file (1 GB, many row groups) has one set of statistics covering the entire file. DuckDB performs finer-grained pruning by reading row-group-level statistics from the Parquet file metadata after deciding to read the file.

Rocklake's file-level statistics are the "coarse filter" — eliminating obviously irrelevant files. Parquet's internal metadata provides the "fine filter" within relevant files.

### No Histogram or NDV for Pruning

The `distinct_count` field is informational (useful for query optimizer cardinality estimation) but does not enable pruning. Only min/max values enable data skipping. Future DuckLake protocol versions may add more sophisticated statistics (bloom filters, histograms) for better pruning of equality and IN-list predicates.

## Further Reading

- **[Architecture: Key Layout](../architecture/key-layout.md)** — How statistics keys are structured
- **[Tag Allocation](tag-allocation.md)** — Tag 0x09 for file_column_stats
- **[Performance: Tuning](../performance/tuning.md)** — Impact of statistics on query performance
- **[Integration: DuckDB Compatibility](../integration/duckdb-compatibility.md)** — DuckDB type support
