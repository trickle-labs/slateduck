# Concepts

The Concepts section is the intellectual backbone of the Rocklake documentation. Where the Getting Started section shows you what Rocklake does, this section explains why it works the way it does — the principles, the constraints, the deliberate trade-offs that shaped every aspect of the system's design. These pages are written as flowing technical essays, not reference lists. They build from first principles (what is a lakehouse? what is a catalog?) through to the distinctive architectural properties that make Rocklake unique (immutability, time travel, horizontal read scale-out, writer fencing).

You do not need to read these pages to operate Rocklake. The Getting Started and Deployment sections give you everything you need to deploy and run the system. But you will find that understanding the concepts makes the rest of the documentation clearer — error messages will make more sense, configuration choices will feel less arbitrary, and the design decisions will feel like logical conclusions rather than unexplained preferences.

## Reading Order

The pages in this section are ordered from most foundational to most advanced. If you are new to the lakehouse concept entirely, start at the top and work down. If you already understand DuckLake and SlateDB, skip to the pages that interest you.

1. **[The Lakehouse Model](lakehouse-primer.md)** — What is a lakehouse, what is a catalog, and why does the catalog turn out to be the hard part? This page makes the documentation self-contained for readers who are new to the space.

2. **[The DuckLake Format](ducklake.md)** — The format Rocklake implements. 28 catalog tables, snapshot-based versioning, a bounded SQL query set, and separation between catalog plane and data plane. Understanding DuckLake is prerequisite to understanding Rocklake.

3. **[The SlateDB Storage Engine](slatedb.md)** — The embedded key-value store that provides Rocklake's durability and transaction guarantees. LSM trees, atomic write batches, single-writer enforcement, and the object-store-native persistence model.

4. **[Catalog Immutability](immutability.md)** — The most distinctive architectural commitment. Why committed catalog facts are never physically deleted by normal operation, and how that single decision enables time travel, read scale-out, and crash safety simultaneously.

5. **[MVCC and Snapshot Isolation](mvcc.md)** — How DuckLake's versioning model maps to SlateDB's key-value layout. The `begin_snapshot` / `end_snapshot` visibility filter, the difference between `dl_snapshot_id` and SlateDB's internal read views, and what snapshot isolation means in practice.

6. **[Time Travel](snapshots.md)** — Not a feature layered on top, but the natural consequence of the storage model. How to query historical states, how retention policies limit query depth, and how time travel interacts with garbage collection.

7. **[Horizontal Read Scale-Out](single-writer-many-readers.md)** — Why immutability enables unlimited concurrent readers with zero coordination, and how this is fundamentally different from traditional read replicas.

8. **[Writer Fencing](writer-fencing.md)** — The single-writer constraint, why it simplifies consistency, how the fencing protocol works when a writer fails over, and the recovery latency you should expect.

9. **[The Fact Store Vision](fact-store-vision.md)** — A forward-looking page about where Rocklake is headed. The storage substrate is not specific to DuckLake — it can host any relational schema as an immutable fact log.

## Key Themes

Several themes recur across these pages. Watch for them — they connect the individual concepts into a coherent whole:

**Constraints that enable.** The single-writer constraint sounds like a limitation, but it is what enables unlimited reader scale-out. The immutability constraint sounds like it would consume unbounded storage, but it is what enables time travel as a zero-cost feature. Many of Rocklake's most powerful properties are direct consequences of constraints that initially seem restrictive.

**Layer separation.** Rocklake carefully separates concerns across layers: DuckLake defines the catalog model, SlateDB provides the storage engine, Rocklake maps one onto the other. Understanding which layer owns which responsibility prevents confusion when diagnosing problems or reasoning about behavior.

**Honest trade-offs.** No architectural decision is free. Every page in this section discusses both the benefits and the costs of the choices Rocklake makes. If you want a deeper dive into the reasoning behind specific decisions (including the alternatives that were considered), the [Design Decisions](../design-decisions/index.md) section provides that analysis.
