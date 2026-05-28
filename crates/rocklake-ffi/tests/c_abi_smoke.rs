//! C ABI smoke test — exercises the full FFI lifecycle from Rust, calling the
//! same `pub extern "C"` functions that a real C/C++ caller would invoke.
//!
//! This is the Rust-side companion to `tests/ffi_smoke.c`.  Both cover
//! identical code paths; this file runs under `cargo test -p rocklake-ffi`
//! while the `.c` version is compiled by the CI `c-abi-smoke` job to verify
//! that the generated C header actually links and compiles.

use std::ffi::CString;
use std::ptr;

use rocklake_ffi::{
    rocklake_abi_version, rocklake_close, rocklake_error_code, rocklake_error_free,
    rocklake_error_message, rocklake_list_schemas, rocklake_open, rocklake_schema_list_free,
    RockLakeError, RockLakeSchemaList, ROCKLAKE_ABI_VERSION,
};
use tempfile::TempDir;

// Helper: build a zeroed error struct.
fn blank_err() -> RockLakeError {
    RockLakeError {
        code: 0,
        message: ptr::null_mut(),
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

/// ABI version is a positive, non-zero integer.
#[test]
fn abi_version_is_positive() {
    let v = rocklake_abi_version();
    assert!(v > 0, "ABI version must be > 0, got {v}");
}

/// Runtime ABI version matches the compile-time constant.
#[test]
fn abi_version_matches_constant() {
    assert_eq!(
        rocklake_abi_version(),
        ROCKLAKE_ABI_VERSION,
        "rocklake_abi_version() must equal ROCKLAKE_ABI_VERSION"
    );
}

/// Full open → list_schemas → close lifecycle on a fresh catalog.
#[test]
fn open_list_close_lifecycle() {
    let dir = TempDir::new().expect("tempdir");
    let path = CString::new(dir.path().to_str().expect("utf8 path")).expect("no nul");

    let mut err = blank_err();

    // Open
    let cat = rocklake_open(path.as_ptr(), &mut err);
    assert!(
        !cat.is_null(),
        "rocklake_open must succeed on a fresh directory"
    );
    let code = rocklake_error_code(&err);
    assert_eq!(code, 0, "error code must be ROCKLAKE_OK after open");
    rocklake_error_free(&mut err);

    // list_schemas on empty catalog
    let mut list_err = blank_err();
    let mut schemas: RockLakeSchemaList = rocklake_list_schemas(cat, 0, &mut list_err);
    let list_code = rocklake_error_code(&list_err);
    assert_eq!(list_code, 0, "list_schemas on empty catalog must return OK");
    assert_eq!(schemas.count, 0, "empty catalog must have 0 schemas");
    rocklake_schema_list_free(&mut schemas);
    rocklake_error_free(&mut list_err);

    // Close
    rocklake_close(cat);
}

/// Passing a null URI returns a null handle and a non-zero error code.
#[test]
fn null_uri_returns_null_handle() {
    let mut err = blank_err();
    let cat = rocklake_open(ptr::null(), &mut err);
    assert!(cat.is_null(), "open(NULL) must return a null handle");
    let code = rocklake_error_code(&err);
    assert_ne!(code, 0, "open(NULL) must set a non-zero error code");
    let msg = rocklake_error_message(&err);
    assert!(!msg.is_null(), "open(NULL) must provide an error message");
    rocklake_error_free(&mut err);
}

/// close(NULL) must not crash.
#[test]
fn close_null_is_safe() {
    rocklake_close(ptr::null_mut());
}

/// Double close (calling rocklake_close twice with the same pointer) must not crash.
#[test]
fn double_close_is_safe() {
    let dir = TempDir::new().expect("tempdir");
    let path = CString::new(dir.path().to_str().expect("utf8")).expect("no nul");
    let mut err = blank_err();
    let cat = rocklake_open(path.as_ptr(), &mut err);
    rocklake_error_free(&mut err);
    if !cat.is_null() {
        rocklake_close(cat);
        rocklake_close(cat); // second call must be a no-op
    }
}

/// error_free(NULL) must not crash.
#[test]
fn error_free_null_is_safe() {
    rocklake_error_free(ptr::null_mut());
}
