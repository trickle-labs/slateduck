# Value Encoding: FlatBuffers Evaluation

Rocklake uses Protocol Buffers (protobuf) for all catalog value encoding.
See [Protobuf Encoding](protobuf-encoding.md) for the full rationale.

During v0.9 planning, FlatBuffers was evaluated as a potential replacement
or complement to protobuf. This document records the evaluation and the
decision reached.

## Motivation

FlatBuffers differs from protobuf in one key property: values are
**zero-copy deserializable** — the in-memory representation is the same as
the wire representation, so deserialization is a pointer cast rather than a
decode loop.  For read-heavy catalog scans this could reduce CPU overhead.

## Benchmark results

We ran a microbenchmark (`benches/encoding.rs`) that encodes and decodes a
`CatalogRow` (schema + 8 columns + 4 data files) 1 000 000 times on an
Apple M2 and an AWS Graviton3 instance.

| Format      | Encode (ns/op) | Decode (ns/op) | Wire size (bytes) |
|-------------|----------------|----------------|-------------------|
| Protobuf    | 220            | 185            | 312               |
| FlatBuffers | 340            | 42             | 448               |

Key observations:

1. **Decode is 4× faster** with FlatBuffers, as expected.
2. **Encode is 55% slower** with FlatBuffers (builder API overhead).
3. **Wire size is 44% larger** because FlatBuffers stores vtable offsets and
   padding bytes for alignment.

## Decision: keep protobuf; defer FlatBuffers

The decode speedup (185 ns → 42 ns) translates to **≈ 143 μs saved per
1 000-row scan**.  For the p95 query in the TPC-H SF10 benchmark, this is
well below measurement noise (< 1% of query latency).  The savings do not
justify:

- A mandatory build-tool dependency (`flatc` schema compiler)
- A format migration (`rocklake migrate`) for all existing catalogs
- Increased wire size (negates the object-storage cost advantage of protobuf)
- Learning curve for contributors unfamiliar with the FlatBuffers builder API

FlatBuffers remains a viable option for a future version if:

- Catalog row counts exceed ~10M (where decode CPU becomes a bottleneck)
- Rocklake adds a columnar layout that benefits from random field access
- The FlatBuffers Rust crate matures to offer safe zero-copy with lifetimes

## Schema evolution compatibility

FlatBuffers supports table field addition via vtable evolution, which is
comparable to protobuf's field number stability guarantee.  There is no
blocker to adopting FlatBuffers for schema evolution reasons.

## See also

- [Protobuf Encoding](protobuf-encoding.md) — the original encoding design
  decision.
- [Zone-Map Readiness](../performance/pruning.md) — another performance
  optimisation deferred from v0.9.
