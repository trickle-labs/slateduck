# Strategy B First

Rocklake supports three deployment strategies: Strategy B (PG-wire sidecar), Strategy C (native DuckDB extension via FFI), and DataFusion integration. The project chose to build and stabilize Strategy B first, even though Strategy C offers better raw performance. This decision reveals deep priorities about how we think about system development: correctness before speed, observability before optimization, and stability before flexibility.

This page explains the reasoning, examines the alternatives, and documents the long-term consequences of this prioritization.

## The Three Strategies

### Strategy B — PG-Wire Sidecar

A standalone process that speaks the PostgreSQL wire protocol. DuckDB connects to it over TCP like any PostgreSQL server. The sidecar maintains its own process, its own memory space, its own lifecycle.

```
DuckDB ──TCP──→ Rocklake Process ──→ SlateDB ──→ S3
```

**Characteristics:** Clear process boundary. Independent lifecycle. Language-agnostic protocol. Observable via standard network tools. Deployable independently.

### Strategy C — Native Extension

A shared library (`.so`/`.dylib`/`.dll`) loaded into DuckDB's process. Catalog operations are in-process function calls through a C-compatible FFI boundary.

```
DuckDB Process [DuckDB Core + Rocklake Extension] ──→ SlateDB ──→ S3
```

**Characteristics:** No network overhead. In-process calls. Shared memory space. Coupled lifecycle. Maximum performance.

### DataFusion Integration

A Rust library implementing DataFusion's `CatalogProvider` trait. For Rust applications using DataFusion directly, without DuckDB.

```
Rust Application [DataFusion + Rocklake Library] ──→ SlateDB ──→ S3
```

**Characteristics:** Library integration. Rust-only. Read-only (currently). No DuckDB dependency.

## Why Strategy B First?

The decision was driven by five factors, each reinforcing the others:

### 1. Debugging and Observability

A standalone process with its own logging, metrics, and lifecycle is dramatically easier to debug than code running inside another process. During development, being able to:

- Attach a debugger (gdb/lldb) to Rocklake independently
- Inspect its memory without DuckDB's allocator interfering
- Restart it without affecting DuckDB (and vice versa)
- Run it under Valgrind or ASAN without DuckDB's memory patterns obscuring issues
- Log every PG-wire message received and sent
- Profile it in isolation (CPU, memory, I/O)

...was invaluable for finding and fixing bugs quickly. In-process debugging (Strategy C) requires reproducing issues inside DuckDB's process, which adds layers of indirection and noise.

When a customer reports a bug, asking them to "send us the Rocklake logs" is feasible. Asking them to "rebuild DuckDB with debug symbols and attach a debugger to your production process" is not.

### 2. Protocol Validation

Strategy B forced us to implement the complete PostgreSQL wire protocol interaction correctly. This meant:

- Parsing startup messages (with all their quirky legacy options)
- Handling SSL negotiation
- Implementing the simple query flow (Query → RowDescription → DataRow → CommandComplete)
- Implementing the extended query flow (Parse → Bind → Describe → Execute)
- Returning proper error responses with SQLSTATE codes
- Handling graceful shutdown and connection draining

This protocol implementation was needed regardless — pg-tide-relay, custom clients, monitoring tools, and future integrations all depend on correct PG-wire behavior. Building it first meant we validated the protocol semantics thoroughly, discovered edge cases, and established a comprehensive wire corpus test suite before adding the complexity of FFI.

If we had built Strategy C first, the PG-wire implementation would have been written later with less attention, possibly introducing subtle compatibility issues that would be harder to diagnose in a network context than in an in-process context.

### 3. DuckDB Version Independence

A sidecar communicates over a stable protocol. The PostgreSQL wire protocol has been backward-compatible since 2003 (protocol version 3.0). It does not change when DuckDB releases a new version. Rocklake can be upgraded independently of DuckDB, and DuckDB can be upgraded independently of Rocklake.

Strategy C, by contrast, must match DuckDB's extension ABI exactly. This is a brittle coupling:

| Event | Strategy B Impact | Strategy C Impact |
|-------|------------------|-------------------|
| DuckDB minor release | None | Recompile required |
| DuckDB ABI change | None | Code changes required |
| Rocklake bug fix | Deploy independently | Must rebuild + redistribute extension |
| DuckDB deprecates API | None | Must adapt or break |

In the early days of a project, when both DuckDB's ducklake extension and Rocklake are evolving rapidly, this independence is crucial. It allows both projects to iterate without coordinating releases.

### 4. Deployment Flexibility

A sidecar is operationally flexible in ways an in-process extension cannot be:

- **Runs anywhere:** Containers, VMs, serverless functions, different machines than DuckDB, different clouds
- **Independent scaling:** Can run on a more powerful machine than DuckDB if needed
- **Independent monitoring:** Has its own health check endpoint, metrics port, log stream
- **Serves multiple clients:** One Rocklake instance can serve hundreds of DuckDB connections
- **Survives client crashes:** If DuckDB segfaults, Rocklake continues running
- **Independent security boundary:** Can run with different IAM permissions than DuckDB

Strategy C is locked to DuckDB's process. If DuckDB crashes, the extension crashes with it. If DuckDB runs in a restrictive sandbox, the extension is constrained by that sandbox. If you want to serve multiple DuckDB instances, each needs its own extension instance.

### 5. Correctness Before Performance

This is perhaps the most philosophical reason. Strategy B has higher latency per operation (network round-trip adds 1–5ms), but identical correctness requirements. Every catalog operation must produce the same result regardless of which strategy delivers it.

By building the slower path first and making it correct — with comprehensive tests, golden file comparisons, and wire corpus validation — we established a reference implementation. Strategy C can then be validated against Strategy B: any difference in behavior between the two is a bug in C.

This "build the simple version first, then optimize" pattern is a well-established engineering principle. The simple version serves as both a working implementation and a correctness oracle for the optimized version.

## The Cost of Strategy B First

### Performance Overhead

Strategy B adds network latency to every catalog operation:

| Scenario | Network Overhead | Total Catalog Time | % Overhead |
|----------|-----------------|-------------------|------------|
| Localhost (same machine) | 0.1–0.5ms × 5 calls | 15–30ms | 2–10% |
| Same AZ (1ms latency) | 1ms × 5 calls | 25–35ms | 15–25% |
| Cross-AZ (2–3ms) | 2.5ms × 5 calls | 32–45ms | 30–40% |

For most workloads (batch analytics, ETL pipelines), this overhead is negligible. For interactive dashboards requiring sub-100ms response times, it is noticeable.

### Deployment Complexity

Running a sidecar means running two processes instead of one. This adds:

- Container configuration (two containers in a pod, or a sidecar container)
- Networking (port configuration, service discovery)
- Health checks (must monitor both processes)
- Startup ordering (Rocklake must be ready before DuckDB connects)

### Resource Usage

Two processes mean two memory footprints, two sets of file descriptors, two thread pools. For resource-constrained environments (Lambda, edge devices), this overhead matters.

## Why Not Both Simultaneously?

Building both strategies in parallel would have doubled the initial development effort without doubling the learning. The wire protocol implementation informed the FFI design (we understood exactly which operations were needed). The test suite built for Strategy B directly validates Strategy C (run the same scenarios through both paths).

Sequential development also avoided the temptation to paper over protocol bugs with in-process shortcuts. When Strategy B is the only option, every protocol edge case must be handled correctly — there is no "just call the internal function directly" escape hatch.

## Strategy C Status Today

Strategy C (the native extension via `rocklake-ffi`) is implemented and functional. It provides the same catalog operations as Strategy B without network overhead. The implementation was significantly easier because:

1. The catalog logic was already correct and well-tested (via Strategy B)
2. The operation semantics were precisely defined (by the wire corpus)
3. Error handling patterns were established (SQLSTATE codes, error categories)
4. The FFI boundary only needed to wrap existing functionality, not implement new logic

Strategy C is appropriate for deployments where latency is critical, DuckDB and Rocklake share a lifecycle, and the operational simplicity of a single process outweighs the debugging benefits of separation.

## Lessons Learned

The Strategy B-first approach validated several principles:

- **Protocols reveal requirements.** Implementing a wire protocol forces you to define exact semantics for every operation. This discipline would not have been necessary for in-process calls (where you can "just pass the struct directly").

- **Network boundaries improve design.** The requirement to serialize everything over the wire naturally leads to clean, well-defined interfaces. These interfaces translate directly to good FFI boundaries.

- **Observability pays for itself.** The ability to `tcpdump` traffic between DuckDB and Rocklake caught several bugs that would have been invisible in an in-process integration.

- **Independence enables velocity.** Decoupled releases allowed faster iteration on both the DuckDB extension and Rocklake server without coordination overhead.

## Analogy: Why Web APIs Before SDKs

The Strategy B-first decision parallels a common pattern in software platform development: build the HTTP API first, then build client SDKs that wrap it. The HTTP API (like Strategy B) is:

- Testable with generic tools (curl, Postman / psql, pgcli)
- Observable with network diagnostics (tcpdump, Wireshark)
- Language-independent (any HTTP client / any PostgreSQL driver)
- Self-documenting (the protocol IS the specification)

The SDK (like Strategy C) provides a better developer experience — type safety, autocompletion, no serialization overhead — but it is built on top of the API, not instead of it. Teams that build SDKs first often discover their internal APIs are poorly defined, because the SDK could always "reach inside" and bypass the abstraction. Teams that build the API first are forced to define clean boundaries from day one.

Rocklake's architecture follows this same pattern. The PostgreSQL wire protocol is the "API" — well-defined, testable, observable. The native extension is the "SDK" — higher performance, better integration, but built on top of the same well-defined operation semantics.

## The Decision Framework

For other projects considering a similar choice, the heuristic is:

1. **If correctness is hard to verify:** Build the observable (networked/separate-process) version first. You need all the debugging tools you can get.
2. **If performance is critical from day one:** Build both simultaneously, using the observable version as the correctness oracle.
3. **If the protocol is well-established:** Build the high-performance version directly. PostgreSQL wire protocol is established, but Rocklake's *use* of it (which SQL patterns, which response formats) was not — so we needed the exploration phase.

Rocklake chose option 1 because DuckLake was a new protocol with evolving requirements. The correct set of SQL patterns to support was not known upfront — it was discovered empirically through wire corpus capture. This discovery process required a running server that could be prodded, inspected, and debugged independently.

## Further Reading

- **[Native Extension](../integration/native-extension.md)** — Strategy C usage and implementation
- **[DuckDB Integration](../integration/duckdb.md)** — Strategy B usage
- **[Architecture: PG Wire Protocol](../architecture/pg-wire-protocol.md)** — The protocol implementation
- **[Performance: Latency Model](../performance/latency-model.md)** — Quantified comparison of B vs. C
