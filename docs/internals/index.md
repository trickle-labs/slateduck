# Internals

Welcome to the engine room. This section documents Rocklake's internal implementation details — the mechanisms, data structures, and algorithms that make the catalog work. These pages are for contributors who need to modify the code, advanced operators who need to debug unusual behavior, and the technically curious who want to understand exactly how the system achieves its guarantees.

You do not need to read this section to use Rocklake effectively. The concepts, architecture, and operations sections cover everything needed for productive use. But if you have ever wondered "how does MVCC actually work at the key-value level?" or "what happens to crash safety when the process dies mid-write?" — this section provides precise, implementation-level answers.

The internals are documented honestly: where the implementation is elegant, we explain why. Where it involves trade-offs or complexity, we explain what drove those choices. The goal is to make the codebase accessible to new contributors — to reduce the time between "I want to contribute" and "I understand enough to make changes."

## Section Philosophy

Each page in this section follows a consistent structure:

1. **What the mechanism does** — the observable behavior from the outside
2. **How it is implemented** — the specific Rust code, data structures, and algorithms
3. **Why it is implemented this way** — the alternatives considered and trade-offs made
4. **Where to find it in the code** — exact crate, module, and function references

This structure means you can read at the level of detail you need. If you just want to understand the behavior, read section 1. If you are about to modify the code, read all four.

## Pages

<div class="grid cards" markdown>

-   **[MVCC Filter](mvcc-filter.md)**

    ---

    How multi-version concurrency control works at the key-value level. Visibility rules, scan-time filtering, interaction with garbage collection, and performance implications of version accumulation.

-   **[Tag Allocation](tag-allocation.md)**

    ---

    How tag bytes are assigned, organized into ranges, and reserved for future expansion. The mapping between tag values and DuckLake catalog tables, internal tables, and system keys.

-   **[Type-Aware Statistics](type-aware-stats.md)**

    ---

    How column statistics (min/max values, null counts) are encoded for different DuckDB data types. The type registry, encoding functions, and how DuckDB uses statistics for partition pruning.

-   **[SQLSTATE Mapping](sqlstate-mapping.md)**

    ---

    How internal Rust error types map to PostgreSQL SQLSTATE codes. Error classification, severity levels, and client-side error handling patterns.

-   **[Wire Corpus](wire-corpus.md)**

    ---

    The test corpus of actual DuckDB wire protocol sessions. How the corpus is captured, maintained, and used to verify SQL classifier compatibility across DuckDB versions.

-   **[Schema Version](schema-version.md)**

    ---

    How catalog format versions are stored, checked, and migrated. What triggers a version bump and what is forward-compatible without one.

-   **[Inlined Data](inlined-data.md)**

    ---

    How small data files are stored directly in the catalog to eliminate extra object storage round-trips. Threshold configuration, reader behavior, and trade-offs.

-   **[Crash Safety](crash-safety.md)**

    ---

    How crash safety is achieved without explicit recovery or WAL replay. The atomic PUT foundation, write path guarantees, and why startup is instantaneous.

</div>

## Navigating the Code

For contributors who want to dive into the source code, here is a map from internal concepts to crate locations:

| Concept | Primary Crate | Key Module |
|---------|--------------|-----------|
| MVCC filter | `rocklake-core` | `src/mvcc.rs` |
| Tag allocation | `rocklake-core` | `src/tags.rs` |
| Key encoding/decoding | `rocklake-core` | `src/keys.rs` |
| Value encoding (protobuf) | `rocklake-core` | `src/values.rs` |
| Type-aware statistics | `rocklake-core` | `src/types.rs` |
| SQL classification | `rocklake-sql` | `src/classifier.rs` |
| SQLSTATE mapping | `rocklake-pgwire` | `src/error.rs` |
| Wire protocol handling | `rocklake-pgwire` | `src/protocol.rs` |
| Catalog operations | `rocklake-catalog` | `src/` |
| Inlined data | `rocklake-catalog` | `src/inline.rs` |

## Prerequisites

These pages assume familiarity with:

- The [Architecture Overview](../architecture/overview.md) (layer separation, request flow)
- The [Key Layout](../architecture/key-layout.md) (how keys are structured)
- The [Value Encoding](../architecture/value-encoding.md) (protobuf envelope format)
- Basic Rust familiarity (ownership, traits, error handling)

If you have not read the architecture section, start there. The internals pages build on architectural concepts without re-explaining them.
