# Bounded SQL

This page documents the decision to implement a bounded SQL dispatcher rather than a general-purpose SQL engine. It is one of Rocklake's most surprising design choices — a system that speaks the PostgreSQL wire protocol but deliberately rejects most SQL statements. This decision reveals a core philosophy: match the solution's complexity to the problem's complexity, not to the interface's expectations.

For a detailed explanation of what bounded SQL is and how it works at a technical level, see [Architecture: SQL Dispatcher](../architecture/sql-dispatcher.md). This page focuses on the trade-off analysis: why we made this choice, what we considered instead, what we gain, and what we lose.

## The Decision

Rocklake's SQL layer recognizes exactly the SQL statement shapes emitted by DuckDB's `ducklake` extension — approximately 50 patterns — and rejects everything else. It does not support arbitrary queries, joins, subqueries, aggregations, window functions, CTEs, or user-defined functions. When an unrecognized statement arrives, it returns SQLSTATE 42601 (syntax error) with a clear message explaining that only DuckLake catalog SQL is supported.

This is not a temporary limitation waiting to be lifted. It is a deliberate, permanent design choice.

## The Problem Context

To understand why bounded SQL makes sense, consider what Rocklake actually does:

1. DuckDB's `ducklake` extension constructs SQL statements that manage catalog metadata
2. These statements are sent over the PostgreSQL wire protocol to Rocklake
3. Rocklake executes the catalog operation and returns results
4. DuckDB uses the results for query planning and execution

The crucial insight is that DuckDB's ducklake extension produces a **finite, well-defined set of SQL patterns**. It does not generate arbitrary SQL. The extension was written by specific developers who made specific implementation choices. Those choices produce specific SQL text. Rocklake only needs to handle exactly that text.

Here is the complete picture of what DuckDB sends:

- **Schema operations:** ~5 patterns (create, drop, rename, list, get)
- **Table operations:** ~8 patterns (create, drop, rename, alter, list, get, move)
- **Column operations:** ~6 patterns (add, drop, rename, list, get, reorder)
- **File operations:** ~6 patterns (register, deregister, list, get stats, list stats)
- **Transaction operations:** ~4 patterns (begin, commit, rollback, snapshot)
- **View/macro operations:** ~6 patterns (create, drop, list, get)
- **Counter operations:** ~3 patterns (get next, increment, reset)
- **System operations:** ~5 patterns (get epoch, set epoch, get version, etc.)
- **Miscellaneous:** ~7 patterns (sequences, inlined inserts, metadata queries)

Total: approximately 50 distinct SQL patterns. This is a finite, enumerable, testable surface.

## Alternatives Considered

### Option A: Embed a Full SQL Engine

We could embed GlueSQL, DataFusion's SQL planner, or sqlparser-rs with a custom executor to support arbitrary queries against the catalog.

**Pros:**

- Users could query the catalog with any SQL tool (psql, Grafana, etc.)
- Enables ad-hoc catalog exploration without export
- Feels more "natural" for a PostgreSQL-speaking system

**Cons:**

- Massively increases the attack surface (SQL injection, resource exhaustion via expensive queries)
- Requires a query optimizer (cost model, plan enumeration, join ordering)
- Requires memory management for query execution (spill-to-disk, memory limits)
- Creates an expectation of general SQL support that must be maintained forever
- Every DuckDB version update requires testing against the full SQL surface, not just 50 patterns
- Query planning overhead for every operation (even trivial ones like "get epoch")
- Type coercion complexity (hundreds of implicit cast rules)
- Bug surface is proportional to SQL feature count (combinatorial explosion)

**Assessment:** The effort to build and maintain a general SQL engine exceeds the effort to build the entire rest of Rocklake combined. This is not an exaggeration — SQL engines are among the most complex software artifacts in existence.

### Option B: Subset SQL Engine

Support SELECT with WHERE, basic comparison operators, and simple expressions — but not joins, subqueries, or aggregations. A "useful subset."

**Pros:**

- Less complex than full SQL
- Enables some ad-hoc queries

**Cons:**

- Worse than either extreme: complex enough to have optimizer bugs, limited enough to frustrate users who expect more
- The boundary between "supported" and "unsupported" is unclear and confusing
- Users would constantly discover things that look like they should work but don't
- Still requires a type system, expression evaluator, and some form of planner
- Maintenance burden grows as users request "just one more feature"

**Assessment:** The worst of both worlds. The boundary is impossible to document clearly, and user expectations will always exceed reality.

### Option C: No SQL at All — Binary Protocol

Define a custom binary protocol for catalog operations with strongly-typed request/response messages. No SQL parsing, no ambiguity, maximum efficiency.

**Pros:**

- Maximum performance (no parsing overhead)
- Perfectly typed (no string-based SQL)
- Smallest possible implementation surface

**Cons:**

- Requires a custom DuckDB extension that speaks the binary protocol
- Eliminates compatibility with existing DuckLake tooling (ducklake extension uses SQL)
- Cannot use standard PostgreSQL client libraries for custom tooling
- Must design, version, and maintain a custom protocol

**Assessment:** Viable but eliminates the key advantage of speaking PostgreSQL wire protocol — existing client compatibility.

### Option D: What We Actually Chose — Bounded Pattern Matching

Recognize exactly the SQL patterns that DuckDB's ducklake extension emits. Reject everything else. No general SQL engine. No optimizer. No expression evaluator beyond what is needed for the known patterns.

**Pros:**

- Every code path is testable (50 patterns × several variations = hundreds of test cases, not infinite)
- Zero query planning overhead (O(1) pattern classification)
- Security surface is finite and auditable
- Maintenance burden is proportional to DuckDB's ducklake changes (slow-growing)
- Implementation is straightforward (pattern matching, not query planning)

**Cons:**

- Cannot query catalog with arbitrary SQL tools
- Tight coupling to DuckDB's SQL patterns (must update when ducklake changes)
- Custom clients are limited to the recognized patterns
- Looks strange to people who expect PostgreSQL-compatible = general SQL

## Why Bounded SQL Wins

### Finite Surface = Provable Security

With approximately 50 statement kinds, every single code path is testable and auditable. You can enumerate all possible inputs (modulo parameter values) and verify correct behavior for each one. This is not true for a general SQL engine where the input space is combinatorially explosive.

Security-relevant implications:

- No SQL injection risk beyond the recognized patterns (unrecognized patterns are rejected, not partially executed)
- No resource exhaustion via complex queries (there are no complex queries)
- No information leakage through clever query construction (you cannot construct clever queries)
- Audit trail covers 100% of possible operations (they are enumerable)

### Zero Query Planning Overhead

Classification is O(1) pattern matching (a few string comparisons and regex matches). There is no cost model, no optimizer rules, no plan enumeration, no statistics gathering. For a workload that processes thousands of small catalog queries per second, this matters. Each query takes microseconds to classify and dispatch, not milliseconds to plan.

Compare to a general SQL engine where even a simple `SELECT * FROM t WHERE id = 1` requires:

1. Tokenization
2. Parsing (AST construction)
3. Semantic analysis (name resolution, type checking)
4. Query rewriting (predicate pushdown, view expansion)
5. Plan enumeration (consider indexes, join orders)
6. Cost estimation
7. Plan selection
8. Execution

Rocklake skips steps 1–7 entirely. The "plan" is determined by the pattern classification, which takes nanoseconds.

### Maintenance Burden Stays Constant

A general SQL engine requires ongoing investment:

- New SQL standards (LATERAL, RECURSIVE, GROUPING SETS)
- Optimizer regression testing
- Type coercion edge cases
- Performance regression hunting
- Expression evaluation bugs

The bounded dispatcher's maintenance is proportional to the number of supported patterns, which grows at the rate DuckDB adds new ducklake operations: approximately 2–5 new patterns per DuckDB major release (twice a year). Each addition is a discrete, self-contained change that can be implemented and tested in isolation.

### Testing Is Exhaustive

The wire corpus captures actual SQL emitted by each supported DuckDB version and replays it against Rocklake. This provides:

- 100% coverage of the supported pattern space
- Regression detection when DuckDB changes SQL formatting
- Cross-version compatibility validation
- A living specification of Rocklake's SQL surface

With a general SQL engine, achieving this level of coverage is infeasible (the input space is infinite).

## The Cost

### Cannot Use as a General Catalog Query Tool

You cannot run:

```sql
-- These all fail:
SELECT COUNT(*) FROM ducklake_tables;
SELECT t.table_name, COUNT(c.column_id) FROM ducklake_tables t JOIN ducklake_columns c ON ...;
SELECT * FROM ducklake_schemas WHERE schema_name LIKE 'analytics%';
```

**Mitigation:** Export the catalog to NDJSON and query it with DuckDB directly:

```bash
rocklake export --catalog s3://bucket/catalog/ --output catalog.ndjson
duckdb -c "SELECT * FROM read_ndjson('catalog.ndjson') WHERE table = 'ducklake_schemas'"
```

### Tight Coupling to DuckDB's SQL Patterns

If DuckDB changes how it formats a query (reorders columns in a SELECT list, changes a WHERE clause condition, adds a new query type), Rocklake's classifier must be updated.

**Mitigation:** The wire corpus test suite detects these changes immediately when testing against a new DuckDB version. The changes are typically trivial to accommodate (adding a new pattern or adjusting an existing regex).

### Cannot Serve General PostgreSQL Clients

psql, pgAdmin, Grafana, and other PostgreSQL tools will find that most queries fail. Rocklake is not a PostgreSQL replacement.

**Mitigation:** This is clearly documented. Custom clients can use the recognized patterns (see [Custom Clients](../integration/custom-clients.md)). For ad-hoc exploration, use the export + DuckDB pattern.

## The Classification Algorithm

The dispatcher uses a tiered classification approach:

1. **Prefix match:** First few tokens identify the statement category (SELECT, INSERT, UPDATE, DELETE, BEGIN, COMMIT)
2. **Table extraction:** For SELECT/INSERT/UPDATE/DELETE, identify the target table name
3. **Pattern match:** Within each category × table combination, match against known patterns using regex or structural comparison
4. **Dispatch:** Route to the appropriate handler function

This is essentially a hand-coded parser that recognizes specific sentence structures in a language with known grammar. It is closer to a command interpreter than a query engine.

## Historical Validation

After 8 months of development and testing against multiple DuckDB versions (1.1.x, 1.2.x, 1.3.x), the bounded SQL approach has never been the bottleneck for adding new functionality. Every feature request has been implementable by adding 1–3 new patterns to the dispatcher. The maintenance burden is exactly as predicted: small, discrete, and proportional to ducklake's evolution.

## Further Reading

- **[Architecture: SQL Dispatcher](../architecture/sql-dispatcher.md)** — Technical implementation details
- **[DuckDB Compatibility](../integration/duckdb-compatibility.md)** — Complete pattern listing
- **[Internals: Wire Corpus](../internals/wire-corpus.md)** — Testing methodology
- **[What Rocklake Is Not](what-rocklake-is-not.md)** — Related non-goals
