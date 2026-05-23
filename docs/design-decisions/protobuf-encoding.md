# Protobuf Encoding

All catalog values are Protocol Buffer messages.

## Alternatives Considered

- **Bincode/postcard:** Fast but no schema evolution
- **FlatBuffers:** Zero-copy but weaker evolution, less tooling
- **MessagePack/CBOR:** No schema enforcement at decode time

## Why Protobuf

- Forward/backward compatibility (add/remove fields safely)
- Schema enforcement (wrong wire type = decode error)
- Compact encoding (varints for small integers)
- Excellent tooling (`protoc --decode_raw`)
