# Design Decisions

Every system is the product of trade-offs. This section documents the major architectural choices made in Rocklake, the alternatives that were considered, the reasoning behind each decision, and the consequences — both positive and negative — that follow. These pages are written with honest acknowledgment of costs, not just benefits. Understanding what you give up is as important as understanding what you get.

Software architecture is not about finding perfect solutions. It is about finding the best trade-offs for a specific context: the workloads you expect, the operational constraints you face, the team size you have, and the timeline you operate under. Rocklake's decisions were made in the context of building a serverless lakehouse catalog that is simple to deploy, correct by construction, and operationally boring. If your context differs significantly — if you need multi-writer concurrency, real-time streaming, or general-purpose SQL — some of these decisions would be wrong for you, and that is fine.

Reading these pages will help you:

- **Evaluate fit:** Understand whether Rocklake is appropriate for your use case by understanding what it prioritizes and what it sacrifices.
- **Predict behavior:** Anticipate how Rocklake will behave in edge cases by understanding the principles behind its design.
- **Contribute effectively:** If you want to contribute to Rocklake, understanding the "why" behind existing decisions prevents proposing changes that conflict with core design values.
- **Learn from trade-offs:** Even if you never use Rocklake, the trade-off analysis patterns here apply to any system design.

## Decision Pages

- **[Why SlateDB?](why-slatedb.md)** — The choice of persistence engine and what it means for durability, performance, and operational model. Why an LSM-tree on object storage beats local-disk databases, distributed key-value stores, and managed cloud services for this use case.

- **[Strategy B First](strategy-b-first.md)** — Why the PG-wire sidecar was built before the native extension, and what that prioritization reveals about design values. The tension between correctness and performance in early development.

- **[Bounded SQL](bounded-sql.md)** — The decision to support only a finite set of SQL statements rather than implementing a general query engine. Why less is more when your client surface is well-defined.

- **[Protobuf Encoding](protobuf-encoding.md)** — Why Protocol Buffers for value serialization instead of JSON, MessagePack, FlatBuffers, or raw structs. The interplay between compactness, evolution, speed, and tooling.

- **[Immutability Trade-offs](immutability-tradeoffs.md)** — The costs of never modifying data in place and how they are managed. An honest accounting of storage growth, read amplification, and operational overhead.

- **[Single-Writer Model](single-writer.md)** — The choice of serialized writes and its implications for throughput, availability, and operational simplicity. Why consensus is overkill for metadata catalogs.

- **[Key Design Rationale](key-design-rationale.md)** — The reasoning behind the specific binary key encoding scheme. Why tag-first, why big-endian, why fixed-width, and why each detail matters for scan performance.

- **[What Rocklake Is Not](what-rocklake-is-not.md)** — Explicit non-goals that shaped the architecture. What we deliberately refuse to build, and why saying "no" is a design strategy.

## Design Philosophy

Several principles recur across these decisions:

**Simplicity over capability.** When choosing between a feature-rich complex solution and a simple limited one, we choose simple. Complexity has ongoing maintenance costs that compound over time. Simplicity has opportunity costs that are bounded and well-understood.

**Correctness over performance.** When choosing between a fast-but-tricky implementation and a correct-but-slower one, we choose correct. Performance can be added later (caching, batching, better storage tier). Correctness bugs in a metadata catalog can corrupt the view of an entire data lake.

**Explicit over implicit.** When choosing between a system that "just works" via magic and one that requires explicit operator action, we choose explicit. Magic fails silently and unpredictably. Explicit operations fail loudly and can be debugged.

**Cloud-native over portable.** When choosing between supporting every possible deployment target and optimizing for cloud object storage, we choose cloud. Rocklake exists because cloud object storage is cheap, durable, and ubiquitous. Optimizing for local-disk deployments would compromise the cloud-native design.

**Single-purpose over general-purpose.** Rocklake does one thing: serve DuckLake catalog metadata from object storage. Every feature is evaluated against this mission. Features that serve other purposes are refused, no matter how individually compelling they might be.
