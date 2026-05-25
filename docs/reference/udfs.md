# User-Defined Functions (WASM)

## Overview

SlateDuck supports user-defined functions (UDFs) written in WebAssembly (WASM).
UDFs extend the IVM SQL surface with custom logic: hash functions, domain-specific
type coercions, scoring models, etc.

## Creating a UDF

```sql
CREATE FUNCTION tokenize(input UTF8) RETURNS UTF8
LANGUAGE WASM AS '<base64-encoded-wasm-module>';
```

## Requirements

- **Deterministic:** All UDFs must be deterministic (`deterministic = true`).
  Non-deterministic UDFs are rejected at `CREATE FUNCTION` time with `SQLSTATE 0A000`.
- **No I/O:** WASM modules are validated against a whitelist of allowed WASI imports
  (currently: none). Modules attempting file I/O, network access, or other syscalls
  are rejected at load time.
- **Arrow-compatible types:** Arguments and return types are limited to:
  `BOOLEAN`, `INT8`, `INT16`, `INT32`, `INT64`, `FLOAT32`, `FLOAT64`,
  `UTF8`, `BINARY`, `DATE32`, `TIMESTAMP`.

## Execution Model

- **Per-batch pooled instance:** A single `wasmtime::Instance` is created per UDF
  per batch and reused across all rows in that batch.
- **Memory limit:** 64 MiB per instance.
- **Fuel limit:** 10M instructions per row. If any single row exhausts its fuel
  slice, the batch is aborted with a clean error naming the row and UDF.
- **Instance lifecycle:** Created at batch start, dropped at batch end.

## Version Management

Views pin to a specific `udf_id` at creation time. To upgrade a view to use a new
UDF version:

```sql
ALTER INCREMENTAL MATERIALIZED VIEW my_view
USING FUNCTION my_udf VERSION 2;
```

This triggers a `REFRESH FULL` to recompute all output with the new UDF version.

## Authoring Guide

### Compiling Rust to WASM

```bash
# Target: wasm32-unknown-unknown (no WASI)
cargo build --target wasm32-unknown-unknown --release

# The output is at:
# target/wasm32-unknown-unknown/release/my_udf.wasm
```

### Example UDF (Rust)

```rust
#[no_mangle]
pub extern "C" fn tokenize(ptr: *const u8, len: u32) -> u64 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    let text = std::str::from_utf8(input).unwrap_or("");
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let result = tokens.join(",");
    // Return pointer and length packed into u64
    let result_ptr = result.as_ptr() as u64;
    let result_len = result.len() as u64;
    std::mem::forget(result);
    (result_ptr << 32) | result_len
}
```

### Determinism Contract

Your UDF must produce identical output for identical input across all invocations.
The following are NOT allowed:
- Reading system time
- Using random number generators
- Accessing files or network
- Maintaining mutable global state between calls

## Error Handling

| Error | Cause | Recovery |
|---|---|---|
| Fuel exhausted | UDF exceeded 10M instructions on one row | View marked `Stale`; `REFRESH FULL` recovers |
| Memory exhausted | UDF allocated > 64 MiB | Same as fuel exhaustion |
| Disallowed import | Module uses `fd_write` or similar | `CREATE FUNCTION` returns `SQLSTATE 0A000` |
| Not deterministic | `deterministic = false` | `CREATE FUNCTION` returns `SQLSTATE 0A000` |

## See Also

- [SQL Reference: IVM DDL](sql-ivm.md)
- [Concepts: Incremental Views](../concepts/incremental-views.md)

## wasmtime Version Policy (v0.17)

SlateDuck pins `wasmtime` to a specific major version (currently **29.x**).

- The fuel API and instance lifecycle model are stable within a major version.
- **Upgrade cadence:** wasmtime major version may be bumped once per SlateDuck
  release cycle. Each bump is a dedicated maintenance PR.
- **Upgrade procedure:**
  1. Update `wasmtime = "N"` in workspace `Cargo.toml`
  2. Update fuel API callsites (if the fuel interface changed)
  3. Re-run the full WASM UDF test suite (Tier 6f)
  4. Verify no sandbox escape or memory limit regressions
- **EOL policy:** Staying on an EOL wasmtime major for more than one release cycle
  is disallowed. WASM sandbox CVEs accumulate and must be addressed promptly.

## Catalog Storage

UDFs are stored in the `matview_udfs` catalog table (tag `0x21`):

| Column | Type | Description |
|---|---|---|
| `udf_id` | u64 | Unique UDF identifier (bumped on ALTER REPLACE) |
| `name` | string | Function name |
| `schema_name` | string | Schema (default: `public`) |
| `wasm_blob` | bytes | Compiled WASM module binary |
| `signature` | json | `{arg_types: [...], return_type: ...}` |
| `deterministic` | bool | Must be `true` for IVM views |
| `created_at_snapshot` | u64 | Catalog snapshot when created |
