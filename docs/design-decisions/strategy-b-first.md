# Strategy B First

Strategy B (PG-wire sidecar) was built first in v0.3; Strategy C (native extension) followed in v0.5.

## Why Sidecar First

- **Immediate compatibility** with any DuckDB version
- **Debuggability** via standard PG tooling (psql, tcpdump, Wireshark)
- **Faster iteration** — standalone Rust binary, no CMake
- **Multi-client support** — serves any PG-wire client

## Why Native Extension Second

Strategy C eliminates network hop for 2-5x lower latency. Built second because:

1. Catalog logic was already correct from Strategy B testing
2. FFI layer is thin — hard work was in the catalog
3. DuckDB extension API needed time to stabilize
