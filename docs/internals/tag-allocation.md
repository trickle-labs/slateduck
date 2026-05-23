# Tag Allocation

## Ranges

- `0x01`-`0x1C`: DuckLake catalog tables (28 allocated)
- `0x1D`-`0xFB`: Reserved for future use
- `0xFC`: Secondary indexes
- `0xFD`: Dynamic inlined rows
- `0xFE`: Counters
- `0xFF`: System keys

## Rules

1. Every tag pre-allocated before encoders are written
2. Tags never reassigned
3. Status tracked in `crates/slateduck-core/src/tags.rs`: `Live`, `Deferred(phase)`, `Reserved`
