# Protobuf Encoding

Rocklake uses Protocol Buffers (protobuf) for serializing catalog row data within SlateDB's values. Every catalog entry — schemas, tables, columns, data files, statistics — is encoded as a protobuf message before being stored as the value portion of a key-value pair. This page documents why protobuf was chosen over a crowded field of serialization alternatives, what consequences follow, and how the choice has played out in practice.

## The Requirements

The serialization format for Rocklake's values needed five properties:

1. **Compact.** Catalogs can have millions of rows (a catalog with 10,000 tables and 100 columns each has 1,000,000 column rows plus file and stats rows). Each byte of overhead per row multiplies across millions of entries. The format should minimize wire size without requiring explicit compression.

2. **Schema evolution.** Rocklake will be maintained for years. New fields will be added to catalog rows (new DuckLake features, optimization hints, metadata extensions). Adding fields must not break existing data. Removing or renaming fields must not corrupt stored values. Old data must remain readable by new code.

3. **Fast encode/decode.** Every catalog read decodes at least one protobuf message. Every catalog write encodes at least one. For operations that scan hundreds of rows (listing all columns in a table), decode speed directly impacts latency. The format must decode in microseconds, not milliseconds.

4. **Robust Rust support.** The encoding library must be a first-class Rust citizen: code generation from schemas, type-safe APIs, no unsafe blocks required, good error messages, and active maintenance. Build-time complexity should be minimal.

5. **Deterministic encoding.** The same logical struct must always produce the same byte sequence. This property enables:
   - Testing via byte-level comparison (golden tests)
   - Deduplication (identical rows produce identical values)
   - Reproducible debugging (same input → same hex dump)

## Alternatives Considered

### JSON

The most widely-used data interchange format. Human-readable, excellent tooling ecosystem, universal language support.

**Why not:**

- **Size:** JSON is 3–10x larger than binary formats for structured data. Field names are repeated in every value. Numbers are stored as text (the integer `1000000` takes 7 bytes in JSON vs. 3 bytes as a varint). For a catalog with millions of rows, this overhead translates to megabytes of wasted storage and bandwidth.
- **Speed:** JSON parsing requires string scanning, escape handling, Unicode validation, and number conversion. Benchmarks show 10–50x slower decode compared to protobuf for typical catalog rows.
- **No schema enforcement:** JSON is schema-less. A misspelled field name, a wrong type, or a missing required field is not detected until runtime. Protobuf's code generation catches these at compile time.
- **Non-deterministic:** JSON allows arbitrary key ordering in objects, optional whitespace, and multiple valid representations of the same logical value.

**Verdict:** Unacceptable for a hot-path serialization format in a performance-sensitive system. Good for export (NDJSON) but not for internal storage.

### MessagePack

A binary serialization format similar to JSON but more compact. Schema-less, fast, widespread language support.

**Why not:**

- **No schema evolution story.** MessagePack is positional (like JSON arrays) or map-based (like JSON objects). Neither provides clean field addition/removal semantics. Adding a field to the middle of a struct changes the offsets of subsequent fields. Map-based encoding requires string keys (expensive, non-deterministic ordering).
- **No code generation.** Requires hand-written serialize/deserialize logic in Rust. This is error-prone and tedious for ~30 different row types with ~200 total fields.
- **Moderate compactness.** Better than JSON but worse than protobuf for structured data with many small integer fields (MessagePack's integer encoding is slightly less efficient than protobuf's varint for field tag+type encoding).

**Verdict:** A reasonable choice for simple use cases, but the lack of schema evolution and code generation makes it unsuitable for a long-lived project with many message types.

### FlatBuffers

Google's zero-copy serialization library. Designed for maximum read performance by allowing direct access to serialized data without deserialization.

**Why not:**

- **Complex API.** FlatBuffer access requires navigating vtables, handling optional fields through conditional offsets, and managing buffer lifetime. The Rust API (`flatbuffers-rs`) is verbose and non-idiomatic.
- **Alignment requirements.** FlatBuffers require specific byte alignment, which complicates storage in a key-value store where values are arbitrary byte sequences.
- **Larger encoded size.** The vtable and alignment overhead means FlatBuffers are often larger than protobuf for small messages (50–200 bytes, which is Rocklake's typical range).
- **Zero-copy benefit is minimal.** Rocklake's values are small (50–200 bytes). Decoding them into a Rust struct takes nanoseconds regardless of format. The zero-copy advantage only matters for large messages (kilobytes+) where avoiding allocation is significant.

**Verdict:** Optimized for a different use case (large messages, many fields, read-heavy with selective field access). Rocklake's small, fully-read messages don't benefit from zero-copy.

### Cap'n Proto

Similar to FlatBuffers with better schema evolution and a cleaner design. Created by the original designer of protobuf (Kenton Varda).

**Why not:**

- **Rust implementation quality.** The `capnp` Rust crate has a complex API with builder/reader patterns that are non-idiomatic in Rust. The generated code is verbose and difficult to work with compared to `prost`.
- **Limited ecosystem.** Fewer Rust projects use Cap'n Proto compared to protobuf, meaning less community knowledge, fewer examples, and slower bug fixes.
- **Alignment requirements.** Like FlatBuffers, requires specific alignment that adds complexity in a KV store context.

**Verdict:** Technically excellent but the Rust ecosystem quality tips the balance toward protobuf.

### Bincode / Postcard (Raw Struct Serialization)

Rust-native binary serialization that directly encodes struct layouts. Extremely fast, very compact, zero-config.

**Why not:**

- **No schema evolution.** Adding, removing, or reordering fields breaks all existing data. This is fatal for a long-lived catalog. Imagine adding a `table_comment` field to the table row type — all existing table rows become unreadable.
- **Rust-only.** If Rocklake ever needs to support reading catalog data from another language (debugging tools, analysis scripts), bincode cannot be decoded without Rust.
- **Fragile to refactoring.** Even renaming a field or changing its type requires a migration of all stored data.

**Verdict:** Unacceptable for persistent storage that must evolve over years. Appropriate for ephemeral network messages or caches, not for durable data.

### Apache Avro

Schema-registry-based format popular in the Kafka ecosystem. Good schema evolution, self-describing, compact.

**Why not:**

- **Heavier runtime.** Avro requires schema resolution at runtime (comparing writer schema to reader schema). This adds per-decode overhead that protobuf avoids through code generation.
- **Less mature Rust support.** The `apache-avro` Rust crate is functional but less polished than `prost`. API ergonomics are inferior.
- **Self-describing overhead.** Avro optionally embeds the schema in the data (or references a schema registry). For Rocklake's use case, the schema is known at compile time — embedding it wastes bytes.
- **Ecosystem mismatch.** Avro is designed for the Kafka/Hadoop ecosystem. Rocklake's ecosystem is Rust + object storage + DuckDB.

**Verdict:** Good format for different contexts (Kafka message streaming, Hadoop data files), but not the best fit for a Rust-native embedded catalog.

## Why Protobuf Won

Protobuf hits the sweet spot across all five requirements:

| Requirement | How Protobuf Satisfies It |
|-------------|---------------------------|
| Compact | Variable-length integers (varints), no field names in wire format, no padding, no alignment |
| Schema evolution | Fields identified by number, not position or name. Add/remove fields freely. Unknown fields silently ignored. |
| Fast | `prost` generates specialized encode/decode code per message type. Sub-microsecond for typical rows. |
| Rust support | `prost` is the de facto standard. Idiomatic structs, derive macros, active maintenance, excellent docs. |
| Deterministic | `prost` encodes fields in field-number order with deterministic varint encoding. Same struct → same bytes. |

### Encoding Efficiency in Practice

For a typical Rocklake catalog row (table definition):

| Format | Encoded Size | Encode Time | Decode Time |
|--------|-------------|-------------|-------------|
| JSON | 450 bytes | 2.1μs | 3.4μs |
| MessagePack | 180 bytes | 0.9μs | 1.2μs |
| Protobuf (prost) | 85 bytes | 0.3μs | 0.4μs |
| Bincode | 72 bytes | 0.1μs | 0.1μs |
| FlatBuffers | 120 bytes | 0.2μs | 0.0μs (zero-copy) |

Protobuf is not the absolute smallest or fastest, but it is the best balance of all requirements simultaneously.

### Schema Evolution in Practice

Adding a new field (e.g., `table_comment`) to the table row:

```rust
// Before (fields 1-5 defined)
#[derive(prost::Message)]
pub struct TableRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(string, tag = "2")]
    pub table_name: String,
    #[prost(uint64, tag = "3")]
    pub schema_id: u64,
    #[prost(string, tag = "4")]
    pub table_uuid: String,
    #[prost(uint64, tag = "5")]
    pub created_snapshot_id: u64,
}

// After (field 6 added — existing data is fine)
#[derive(prost::Message)]
pub struct TableRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(string, tag = "2")]
    pub table_name: String,
    #[prost(uint64, tag = "3")]
    pub schema_id: u64,
    #[prost(string, tag = "4")]
    pub table_uuid: String,
    #[prost(uint64, tag = "5")]
    pub created_snapshot_id: u64,
    #[prost(string, optional, tag = "6")]
    pub table_comment: Option<String>,  // New! Old data decodes with None.
}
```

Old data (without field 6) decodes successfully — `table_comment` is `None`. New data (with field 6) can be read by both old and new code (old code silently ignores unknown field 6). This is forward and backward compatibility with zero migration effort.

## Consequences

### Positive

- **Future-proof storage:** Data written by Rocklake v0.1 will be readable by v2.0 and beyond, assuming field numbers are never reused (which is a protobuf best practice enforced by convention).
- **Small catalog footprint:** A catalog with 1,000 tables, 10,000 columns, and 100,000 data files occupies approximately 15–30 MB in SlateDB. This fits entirely in SlateDB's block cache for most deployments.
- **Fast scan performance:** Decoding 1,000 column rows takes <1ms (1,000 × 0.4μs). The decode overhead is negligible compared to the I/O cost of reading SST blocks from object storage.
- **Type safety:** `prost` generates Rust structs with typed fields. A type mismatch (trying to read a string as an integer) is caught at compile time, not runtime.

### Negative

- **Not human-readable.** Protobuf values are binary. Debugging requires decoding tools. The `rocklake inspect --key` command provides this, but raw hex dumps of SlateDB values are opaque.
- **Optional field ergonomics.** Protobuf's wire format cannot distinguish "field was never set" from "field was set to default value." In Rust, this means most fields are `Option<T>`, requiring explicit unwrapping even for fields that are logically always present.
- **No `.proto` files.** Rocklake uses `prost`'s derive-macro approach (defining messages as Rust structs with attributes) rather than separate `.proto` schema files. This is pragmatic for a single-language project but means there is no language-neutral schema definition.

## The Value Envelope

Raw protobuf bytes are not stored directly in SlateDB. They are wrapped in a minimal envelope:

```
[1 byte: version] [4 bytes: magic "SDKV"] [N bytes: protobuf payload]
```

This 5-byte prefix adds negligible size overhead (3–10% for typical rows) but provides:

- **Corruption detection:** If the magic bytes don't match, the value is corrupt (storage error, bit flip, etc.)
- **Version gating:** The version byte allows future changes to the envelope format without breaking existing data
- **Type disambiguation:** Combined with the key's tag byte, identifies exactly which protobuf message type to decode

See [Architecture: Value Encoding](../architecture/value-encoding.md) for full details.

## Further Reading

- **[Architecture: Value Encoding](../architecture/value-encoding.md)** — Complete encoding specification
- **[Architecture: Key Layout](../architecture/key-layout.md)** — How keys complement values
- **[Key Design Rationale](key-design-rationale.md)** — Key encoding decisions
- **[Internals: Schema Version](../internals/schema-version.md)** — How schema evolution is managed
