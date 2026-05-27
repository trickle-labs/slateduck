# FFI Safety

The `rocklake-ffi` crate exposes a C-compatible API over the Rust catalog stack. All FFI functions are `extern "C"` and accept raw pointers. This document describes the pointer ownership model, handle lifecycle, and the safety invariants that callers and the Rust implementation must uphold.

## Pointer Ownership

| Pointer | Owner | Freed by |
|---|---|---|
| `*mut RocklakeCatalog` (from `rocklake_open`) | C caller | `rocklake_close()` |
| `*mut RocklakeSchemaList` (embedded) | C caller | `rocklake_schema_list_free()` |
| `*mut RocklakeTableList` (embedded) | C caller | `rocklake_table_list_free()` |
| `*mut RocklakeColumnList` (embedded) | C caller | `rocklake_column_list_free()` |
| `*mut RocklakeFileList` (embedded) | C caller | `rocklake_file_list_free()` |
| `*mut RocklakeError` (stack) | C caller | `rocklake_error_free()` for the message field |

## Handle Lifecycle

```
rocklake_open()  →  *mut RocklakeCatalog  →  {use}  →  rocklake_close()
                                                              │
                                              magic zeroed ──┘  (prevents double-close)
```

- `rocklake_open()` allocates a `RocklakeCatalog` on the heap via `Box::new` and returns a raw pointer (`Box::into_raw`).
- `rocklake_close()` checks and zeroes the magic field before reconstructing the `Box` and dropping it.
- A second call to `rocklake_close()` with the same pointer is a **safe no-op**: the magic check fails and the function returns early without touching the already-freed memory.
- Any catalog operation after `rocklake_close()` (use-after-free) is caught by the `with_catalog()` magic check and returns `InvalidHandle` without dereferencing stale memory.

## with_catalog Safety Pattern

Internally, all catalog operations go through `with_catalog(ptr, |cat| { … })` instead of the old `validate_catalog()` which returned `Option<&'static mut RocklakeCatalog>`. The new pattern:

1. Returns `None` (→ `InvalidHandle` error) for null pointers.
2. Returns `None` for any pointer where the magic field is not `CATALOG_MAGIC` (covers zeroed-on-close handles and wild pointers).
3. Creates a mutable reference **bounded by the closure frame** — the reference cannot escape to a longer lifetime.

## SAFETY Invariants

Every `unsafe` block in `lib.rs` has a `// SAFETY:` comment stating:

- Which precondition (non-null check, magic check, or both) justifies the dereference.
- Whether ownership is being transferred (`Box::from_raw`) or borrowed (`&mut *ptr`).
- The aliasing constraint: no other thread may concurrently close or mutate the same handle during the call.

## Caller Responsibilities

The following are the C caller's responsibilities (not enforced by Rust):

1. **No concurrent close and use.** If thread A calls `rocklake_close(cat)` while thread B calls any other function with `cat`, the behaviour is undefined at the hardware level even if the magic check would fire. Guard concurrent access with a mutex in the C layer.
2. **No use after close.** After `rocklake_close()` returns, the pointer is dangling. The magic-number guard is a best-effort defence, not a substitute for correct ownership.
3. **Free list types before reuse.** Free `RocklakeSchemaList`, `RocklakeTableList`, etc. with their respective `_free` functions before overwriting the struct.

## Vec Capacity Invariant

All list-builder functions call `.shrink_to_fit()` before `std::mem::forget()`. This ensures `capacity == len` at the time the Vec is forgotten, which is required for `Vec::from_raw_parts(ptr, len, len)` in the `_free` functions to reconstruct the original allocation correctly.

## Sanitizer and Miri CI

A scheduled nightly CI workflow (`.github/workflows/sanitizers.yml`) runs:

- **ASAN**: detects heap use-after-free, double-free, and buffer overflows.
- **UBSAN**: detects undefined behaviour including invalid pointer casts and signed overflow.
- **Miri**: interprets the Rust MIR and catches UB in pure-Rust `unsafe` code.

These jobs are currently `continue-on-error: true` and will be promoted to blocking at v1.0.
