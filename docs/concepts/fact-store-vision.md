# The Fact Store Vision

This page describes a direction that Rocklake is committed to exploring in its v2.x roadmap, and it is written honestly as such. The ideas presented here are architectural motivations — they explain why certain current design choices exist and where they lead — but the implementation does not yet exist. Reading this page will help you understand why Rocklake's storage substrate is designed with more generality than DuckLake alone requires, and why certain decisions (like the 1-byte tag namespace, the separation between `rocklake-core` and `rocklake-catalog`, the strict naming conventions) feel over-engineered for a "just a DuckLake catalog" interpretation of the project.

## The Core Observation

The storage substrate that Rocklake uses for DuckLake — append-only keys scoped by a monotonically increasing version identifier, Protobuf values with a versioned header, counter allocation under a dedicated namespace, `retain-from` advancement for visibility gating, and audited excision for physical deletion — is not specific to DuckLake. It is a generic pattern for storing versioned facts in object storage.

Consider what Rocklake's storage layer actually provides:

- **Immutable fact storage.** You can assert facts ("table X exists with name Y at version V") and they persist forever unless explicitly excised.
- **Temporal versioning.** Every fact has a begin-version and an optional end-version, enabling point-in-time queries across the entire history.
- **Atomic multi-fact assertions.** Multiple facts can be asserted in a single atomic transaction — either all become visible at a new version, or none do.
- **Schema-independent key layout.** The 1-byte tag namespace supports 256 independent entity types, each with its own key structure.
- **Audited retraction.** Facts can be retracted (end-version set) or physically excised, with full audit trails.

These properties describe a general-purpose immutable fact log over object storage, not just a DuckLake catalog implementation. Any relational schema — not just DuckLake's 28 tables — could be hosted on this substrate with the same guarantees.

## What a General Fact Store Looks Like

Imagine a `rocklake-factstore` crate that provides a generic API for storing versioned facts:

```rust
// Hypothetical API — not yet implemented
let store = FactStore::open("s3://bucket/my-facts").await?;

// Assert facts at a new version
let txn = store.begin().await?;
txn.assert("user", user_id, &UserFact { name: "Alice", email: "alice@example.com" })?;
txn.assert("order", order_id, &OrderFact { user_id, total: 99.99, status: "pending" })?;
let version = txn.commit().await?;

// Query facts at a specific version
let user = store.as_of(version).get("user", user_id).await?;

// Retract a fact (set end-version)
let txn = store.begin().await?;
txn.retract("order", order_id)?;
txn.commit().await?;

// Query the fact as of the version before retraction — still visible
let order = store.as_of(version).get("order", order_id).await?;
```

This API would provide the same guarantees that DuckLake gets through Rocklake: immutability, time travel, crash safety, horizontal read scale-out. But it would be available for any schema — user profiles, configuration records, audit logs, compliance records, financial transactions, sensor readings — anything that benefits from an append-only fact log with temporal queries.

## Why This Matters for Rocklake's Current Design

The fact store vision explains several design choices in the current codebase that might otherwise seem like unnecessary generality:

**The `rocklake-core` / `rocklake-catalog` separation.** The `rocklake-core` crate defines the key encoding primitives, the SDKV value header, the counter allocation protocol, and the MVCC visibility filter — all generic mechanisms that do not know about DuckLake's specific tables. The `rocklake-catalog` crate maps DuckLake's 28 tables onto these primitives. In the fact store future, additional crates would map additional schemas onto the same `rocklake-core` primitives.

**The 1-byte tag namespace with 256 slots.** DuckLake needs 28 table tags plus 3 system tags (counters, system metadata, inlined data). That is 31 out of 256 possible tags. The remaining 225 slots are reserved for future schemas that might be hosted on the same storage substrate. If Rocklake ever supports multiple independent schemas (each with their own tables), the tag namespace has room.

**The strict naming conventions.** `dl_snapshot_id` (DuckLake-level snapshot) is carefully distinguished from `kv_snapshot` (SlateDB-level read view) because in a multi-schema future, each hosted schema might have its own version counter, and confusing them would be catastrophic.

**The `retain-from` and excision mechanisms.** These are designed to work at the tag level, not at the DuckLake level specifically. A fact store could advance `retain-from` independently per schema, excise independently per entity type, and maintain independent audit trails — all using the mechanisms that already exist.

## What is Speculative

To be transparent about what is committed versus what is exploration:

**Committed (exists in the current design):**
- The storage primitives in `rocklake-core` are schema-independent
- The tag namespace has room for growth
- The MVCC filter, counter allocation, and excision mechanisms are generic

**Under active consideration (v2.x roadmap):**
- A `rocklake-factstore` crate that exposes the generic API
- Multi-schema isolation (one SlateDB `Db` per schema, or tag-range-based isolation within a single `Db`)
- A generic `assert`/`retract`/`as_of` API

**Exploratory (no timeline):**
- A Datalog-style query interface for fact stores
- Cross-schema joins with temporal alignment
- Federation across multiple fact stores in different buckets

The purpose of this page is not to promise features that do not exist. It is to give you the conceptual context that makes certain current design choices feel like deliberate architecture rather than arbitrary decisions. When you see that `rocklake-core` is carefully separated from `rocklake-catalog`, or that the tag namespace is far larger than DuckLake needs, or that the MVCC filter is parameterized rather than hardcoded to DuckLake's column names — now you know why.

## The Philosophical Foundation

At a deeper level, the fact store vision rests on an observation about how organizations relate to their data: most valuable data is factual (it records something that happened or something that is true) and most facts should be preserved rather than discarded. Traditional databases model data as mutable state — a row exists, you update it, the old value is gone. This makes sense for operational systems (the customer's address should be their current address, not their historical address), but it destroys information that turns out to be valuable later (auditing, compliance, analytics, debugging, reproducing past decisions).

An immutable fact store preserves everything by default and provides temporal queries as the natural read mode. The current state is just the most recent version. The historical state is just an older version. Both are equally accessible, equally fast, equally reliable. This is not a new idea — Datomic, Event Store, and various event-sourcing architectures have explored it — but Rocklake's contribution is hosting it on commodity object storage with no infrastructure beyond a bucket, using the same operational model (single-writer, infinite readers, audited excision) that has proven correct and practical for the DuckLake use case.

## Why "Fact Store" Rather Than "Event Store"

The terminology matters. An event store records *events* — things that happened. An event store for an e-commerce system might record `OrderPlaced`, `ItemShipped`, `PaymentReceived`. To find the current state of an order, you replay the events. This is powerful but expensive: replay time grows with history length, and you must model every state transition explicitly as an event type.

A fact store records *facts* — assertions about what is true, valid at a specific version. A fact store entry might be `user:42 has email alice@example.com (begin_version=100, end_version=NULL)`. Finding the current state is a key lookup, not a replay. Finding the historical state is the same key lookup with an older version filter. There is no event replay, no projection logic, no materialized view needed.

Rocklake uses the fact model because catalog data is naturally factual: "table X exists with these columns" is an assertion about a state of affairs, not an event in a stream. The snapshot ID is a version counter, not a timestamp of occurrence. This framing keeps the data model simple, queries fast, and the storage format elegant — there is no event schema to design, no projection to materialize, and no replay budget to worry about.

## Conclusion

The fact store vision is not a product announcement. It is an architectural compass — a direction that explains why certain current decisions are made the way they are, and where the project aspires to go. Today, Rocklake uses this substrate exclusively for DuckLake. Tomorrow, it might support other schemas. The infrastructure is ready. The abstractions are clean. When the use case arrives, the foundation will be there.

If you are building something that would benefit from an immutable, temporally-queryable, object-storage-backed fact store, we would love to hear about it. Open an issue or discussion on GitHub — that conversation shapes the roadmap more than any internal planning document.

## Further Reading

- **[Catalog Immutability](immutability.md)** — The current implementation of the immutability principle for DuckLake
- **[Architecture: Crate Structure](../architecture/crate-structure.md)** — How the crate separation supports future schema independence
- **[Key-Value Mapping](key-value-mapping.md)** — How the 1-byte tag namespace works and why it has room for growth
- **[Roadmap](../roadmap/index.md)** — The project roadmap including fact store milestones

## Comparison with Existing Approaches

Several systems have explored immutable fact storage, each with different trade-offs:

**Datomic** (Cognitect) pioneered the idea of a database as an immutable accumulation of facts with temporal queries. Datomic provides Datalog queries, entity-attribute-value storage, and automatic indexing. However, it requires a JVM runtime, a specific peer library, and either DynamoDB or a specialized storage backend. Rocklake's approach trades Datomic's query sophistication for infrastructure simplicity — commodity object storage via a REST API.

**Event Sourcing** (pattern, many implementations) stores domain events as the primary source of truth and derives current state by replaying events. Event sourcing provides complete auditability and enables arbitrary read-model projections. The downside is operational complexity: event schemas evolve, replay times grow unbounded without snapshots, and each read model requires explicit materialization logic. Rocklake's fact store is simpler — facts are already indexed by entity and version, so "current state" is a single key lookup rather than an event replay.

**Apache Kafka** (with compaction disabled) provides an append-only log of records. Like Rocklake, records are immutable once written. Unlike Rocklake, Kafka does not provide point-in-time queries, entity-level addressing, or schema enforcement at the storage layer. Kafka is optimized for streaming throughput; Rocklake's fact store would be optimized for temporal queries and long-term persistence at low cost.

**Git** stores versioned snapshots of a filesystem as an immutable object graph. Conceptually, this is similar to Rocklake's approach — each commit is a snapshot, history is preserved, any past state is retrievable. Git, however, is optimized for file-tree diffs, not structured data queries. You cannot efficiently ask "what was the value of key X at version 500?" without checking out that version.

The distinctive position of a Rocklake fact store would be: the persistence cost of object storage (pennies per gigabyte-month), the query model of a key-value store with temporal dimensions, the operational simplicity of no infrastructure beyond a bucket, and the consistency guarantees of single-writer MVCC. This combination does not exist in any current system.

## Potential Use Cases

Beyond DuckLake, the fact store substrate could support:

**Configuration management.** Store application configuration as versioned facts. Each deployment creates a new version. Roll back by reading an older version. Audit who changed what and when. Compare configurations across versions without external diff tools.

**Compliance and regulatory records.** Financial regulations often require that records be preserved exactly as they existed at a point in time. An immutable fact store satisfies this requirement inherently — you cannot accidentally modify historical data because the data model does not support modification, only assertion of new versions.

**Feature flag systems.** Track feature flag states over time with perfect correlation to user-visible behavior. When a bug is reported, query the exact flag configuration that was active at the time of the report.

**Multi-tenant SaaS metadata.** Each tenant's metadata (plans, limits, feature entitlements, billing state) stored as versioned facts with per-tenant isolation at the tag level. Time travel enables answering "what plan was this tenant on when they hit the rate limit?"

## Further Reading

- **[Catalog Immutability](immutability.md)** — The current implementation of the immutability principle for DuckLake
- **[Architecture: Crate Structure](../architecture/crate-structure.md)** — How the crate separation supports future schema independence
- **[Roadmap](../roadmap/index.md)** — The project roadmap including fact store milestones
