# Reference

This section provides lookup-oriented reference material for Rocklake. Unlike the conceptual documentation (which explains WHY things work the way they do) or the architecture documentation (which explains HOW components fit together), reference pages are designed for quick answers to specific questions. You come here when you need to know what error code 42P01 means, what environment variable controls the cache size, what fields a table row contains, or what SQL patterns DuckDB sends.

The reference section is exhaustive rather than instructive. Every supported SQL statement is listed. Every error code is documented. Every environment variable is described. Every metric is named and explained. This is the section you bookmark and return to repeatedly during development, debugging, and operations.

## How to Use This Section

Each page covers a single reference domain:

<div class="grid cards" markdown>

-   **[Catalog Tables](catalog-tables.md)**

    ---

    Complete list of all 28 catalog table types stored in the Rocklake catalog. For each table: the tag byte, protobuf message fields, key encoding format, sort behavior, and relationship to other tables. This is the authoritative specification of the catalog schema.

-   **[Supported SQL](sql-supported.md)**

    ---

    Exhaustive list of every SQL statement pattern that Rocklake's bounded classifier recognizes. Organized by category (schema operations, table operations, column operations, data file operations, transaction management). Includes the exact SQL format DuckDB sends and what catalog operation it maps to.

-   **[Error Codes](error-codes.md)**

    ---

    All SQLSTATE error codes returned by Rocklake, organized by error class. For each code: name, description, common causes, and recommended handling. Stable across versions — safe for programmatic error handling.

-   **[Environment Variables](environment-vars.md)**

    ---

    Complete configuration reference. Every environment variable recognized by Rocklake: purpose, type, default value, and examples. Includes storage credentials, server settings, TLS, logging, and performance tuning.

-   **[Metrics](metrics.md)**

    ---

    All Prometheus metrics exposed by Rocklake's `/metrics` endpoint. For each metric: type (counter, gauge, histogram), labels, description, and what it tells you about system health and performance.

-   **[Glossary](glossary.md)**

    ---

    Definitions of all terms used throughout the documentation. If you encounter an unfamiliar word in any Rocklake page, look it up here. Alphabetically organized for quick lookup.

</div>

## Quick Links

| I need to know... | Go to... |
|-------------------|---------|
| What fields does a table row have? | [Catalog Tables](catalog-tables.md) |
| What SQL creates a schema? | [Supported SQL](sql-supported.md) |
| What does error 57P04 mean? | [Error Codes](error-codes.md) |
| How do I set the cache size? | [Environment Variables](environment-vars.md) |
| What metrics show read latency? | [Metrics](metrics.md) |
| What is "excision"? | [Glossary](glossary.md) |

## Conventions

Throughout this section:

- **Required** means the system will not start without this value
- **Default** is the value used when nothing is explicitly configured
- **Stable** means the value/behavior will not change without a major version bump
- Types use Rust notation: `u64` (unsigned 64-bit integer), `string` (UTF-8), `bool` (true/false), `Option<T>` (may be absent)
