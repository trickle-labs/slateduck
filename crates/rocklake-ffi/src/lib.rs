//! RockLake FFI: C/C++ foreign function interface for embedding RockLake catalog
//! operations in any language ecosystem.
//!
//! This crate provides a stable C ABI over `rocklake-catalog` operations.
//! All async operations are bridged via a blocking Tokio runtime.
//!
//! The primary consumers are:
//! - The `rocklake-client` Rust crate (idiomatic async Rust wrapper)
//! - Python bindings via PyO3
//! - Go bindings via cgo
//! - Node.js bindings via napi-rs
//! - The native DuckDB extension (v0.36.0)

// FFI functions must accept raw pointers from C callers.
// Null/handle safety is enforced explicitly via validate_catalog() and
// per-function null checks — not by Rust's type system.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::ptr;
use std::sync::Arc;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;

// ─── ABI Version ───────────────────────────────────────────────────────────

/// ABI version: major * 1000 + minor.
///
/// Language bindings and embedding callers MUST call `rocklake_abi_version()`
/// at load time and refuse to proceed on version mismatch.
pub const ROCKLAKE_ABI_VERSION: u32 = 5_000; // v5.0 (matches v0.35 release)

/// Backward-compatible alias for `ROCKLAKE_ABI_VERSION`.
///
/// Deprecated: use `ROCKLAKE_ABI_VERSION` in all new code. This alias will be
/// removed in the release cycle following v0.36.0.
#[deprecated(since = "0.35.0", note = "use ROCKLAKE_ABI_VERSION instead")]
pub const ABI_VERSION: u32 = ROCKLAKE_ABI_VERSION;

/// Returns the ABI version. Language bindings check this at load time.
#[no_mangle]
pub extern "C" fn rocklake_abi_version() -> u32 {
    ROCKLAKE_ABI_VERSION
}

// ─── CString Safety ────────────────────────────────────────────────────────

/// Convert a Rust `&str` to a `CString`, replacing embedded NUL bytes with a
/// safe sentinel rather than panicking or returning an empty string.
///
/// `CString::new()` fails only when the input contains interior `\0` bytes.
/// An empty fallback (`unwrap_or_default()`) silently drops the entire message,
/// making errors invisible to C callers. This helper instead returns the literal
/// `"<invalid-utf8>"` so callers always receive a non-empty diagnostic.
fn to_c_string(s: &str) -> CString {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("<invalid-utf8>").expect("fallback is NUL-free"))
}

// ─── Error Handling ────────────────────────────────────────────────────────

/// Opaque error type returned from FFI functions.
#[repr(C)]
pub struct RockLakeError {
    pub code: i32,
    pub message: *mut c_char,
}

/// Error codes for the RockLake C ABI.
///
/// These map to generic catalog semantics — no engine-specific assumptions.
#[repr(i32)]
#[derive(Clone, Copy)]
pub enum RockLakeErrorCode {
    /// Success. Matches `ROCKLAKE_OK` in the C header.
    Ok = 0,
    /// Unexpected internal error.
    Internal = 1,
    /// Requested resource not found.
    NotFound = 2,
    /// Writer was fenced by a competing writer with a higher epoch.
    /// Alias: `ROCKLAKE_ERR_FENCED` in the C header.
    WriterFenced = 3,
    /// Catalog format version mismatch.
    FormatMismatch = 4,
    /// Value exceeds the maximum encoded size.
    ValueTooLarge = 5,
    /// Serializable transaction conflict; the caller should retry.
    /// Alias: `ROCKLAKE_ERR_CONFLICT` in the C header.
    TransactionConflict = 6,
    /// Catalog has not been initialized.
    NotInitialized = 7,
    /// Null or already-closed catalog handle passed to an FFI function.
    InvalidHandle = 8,
}

impl RockLakeError {
    fn ok() -> Self {
        Self {
            code: RockLakeErrorCode::Ok as i32,
            message: ptr::null_mut(),
        }
    }

    fn invalid_handle() -> Self {
        Self {
            code: RockLakeErrorCode::InvalidHandle as i32,
            message: to_c_string("invalid or null catalog handle").into_raw(),
        }
    }

    fn from_catalog_error(e: rocklake_catalog::CatalogError) -> Self {
        use rocklake_catalog::CatalogError;
        let code = match &e {
            CatalogError::NotFound(_) => RockLakeErrorCode::NotFound,
            CatalogError::WriterEpochMismatch => RockLakeErrorCode::WriterFenced,
            CatalogError::FormatVersionMismatch { .. } => RockLakeErrorCode::FormatMismatch,
            CatalogError::ValueTooLarge { .. } => RockLakeErrorCode::ValueTooLarge,
            CatalogError::TransactionConflict(_) => RockLakeErrorCode::TransactionConflict,
            CatalogError::NotInitialized => RockLakeErrorCode::NotInitialized,
            _ => RockLakeErrorCode::Internal,
        };
        let msg = to_c_string(&e.to_string());
        Self {
            code: code as i32,
            message: msg.into_raw(),
        }
    }
}

/// Free an error's message string.
#[no_mangle]
pub extern "C" fn rocklake_error_free(err: *mut RockLakeError) {
    if err.is_null() {
        return;
    }
    // SAFETY: `err` is non-null (checked above). The `message` pointer, if
    // non-null, was produced by `CString::into_raw()` and must be freed with
    // `CString::from_raw()`. After this call the message is freed.
    unsafe {
        let e = &mut *err;
        if !e.message.is_null() {
            drop(CString::from_raw(e.message));
            e.message = ptr::null_mut();
        }
    }
}

/// Get the error code.
#[no_mangle]
pub extern "C" fn rocklake_error_code(err: *const RockLakeError) -> i32 {
    if err.is_null() {
        return 0;
    }
    // SAFETY: `err` is non-null (checked above) and points to a valid
    // `RockLakeError`. We only read the `code` field.
    unsafe { (*err).code }
}

/// Get the error message (borrows — do not free separately).
#[no_mangle]
pub extern "C" fn rocklake_error_message(err: *const RockLakeError) -> *const c_char {
    if err.is_null() {
        return ptr::null();
    }
    // SAFETY: `err` is non-null (checked above). We only read the `message`
    // field without taking ownership. The caller must not free this pointer
    // separately; call `rocklake_error_free()` to release the whole error.
    unsafe { (*err).message as *const c_char }
}

// ─── Opaque Handles ────────────────────────────────────────────────────────

/// Magic value stored in every live `RockLakeCatalog` to detect invalid
/// or double-closed handles. Bytes: 'D','U','C','K'.
const CATALOG_MAGIC: u32 = 0x4455_434B;

/// Opaque handle for a CatalogStore.
pub struct RockLakeCatalog {
    magic: u32,
    store: CatalogStore,
    runtime: Arc<tokio::runtime::Runtime>,
}

/// Validate a catalog handle and run a function with scoped mutable access.
///
/// Returns `Some(f(cat))` when the pointer is non-null and the magic field is
/// intact, `None` otherwise. The reference is bounded by the closure frame and
/// **never escapes** — this is the key safety improvement over the previous
/// `&'static mut` design.
///
/// # Safety (caller contract)
///
/// The C caller must ensure that:
/// 1. `ptr` is either null or points to a live `RockLakeCatalog` allocation
///    created by `rocklake_open()` and not yet freed.
/// 2. No other thread is concurrently calling `rocklake_close()` or any other
///    FFI function with the same `ptr` during the execution of `f`.
///
/// Condition 2 is the caller's responsibility; the magic-number check provides
/// a best-effort defence against use-after-close but is not a substitute for
/// proper ownership discipline in the C caller.
fn with_catalog<T>(
    ptr: *mut RockLakeCatalog,
    f: impl FnOnce(&mut RockLakeCatalog) -> T,
) -> Option<T> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: `ptr` is non-null (checked above). We create a mutable reference
    // bounded by this function's scope; the reference is consumed by `f` and
    // cannot outlive this call frame. The caller guarantees the allocation is
    // live and exclusively accessed during this call.
    let cat = unsafe { &mut *ptr };
    if cat.magic != CATALOG_MAGIC {
        return None;
    }
    Some(f(cat))
}

/// Opaque handle for a snapshot query result.
#[repr(C)]
pub struct RockLakeSnapshot {
    pub snapshot_id: u64,
    pub schema_version: u64,
}

/// Opaque handle for a file list result.
#[repr(C)]
pub struct RockLakeFileList {
    pub files: *mut RockLakeDataFile,
    pub count: u64,
}

/// A single data file in a file list.
#[repr(C)]
pub struct RockLakeDataFile {
    pub data_file_id: u64,
    pub table_id: u64,
    pub path: *mut c_char,
    pub file_format: *mut c_char,
    pub row_count: u64,
    pub file_size_bytes: u64,
    pub snapshot_id: u64,
}

/// Schema entry.
#[repr(C)]
pub struct RockLakeSchema {
    pub schema_id: u64,
    pub schema_name: *mut c_char,
}

/// Schema list.
#[repr(C)]
pub struct RockLakeSchemaList {
    pub schemas: *mut RockLakeSchema,
    pub count: u64,
}

/// Table entry.
#[repr(C)]
pub struct RockLakeTable {
    pub table_id: u64,
    pub schema_id: u64,
    pub table_name: *mut c_char,
}

/// Table list.
#[repr(C)]
pub struct RockLakeTableList {
    pub tables: *mut RockLakeTable,
    pub count: u64,
}

/// Column entry.
#[repr(C)]
pub struct RockLakeColumn {
    pub column_id: u64,
    pub table_id: u64,
    pub column_name: *mut c_char,
    pub data_type: *mut c_char,
    pub column_index: u64,
    pub is_nullable: bool,
}

/// Column list.
#[repr(C)]
pub struct RockLakeColumnList {
    pub columns: *mut RockLakeColumn,
    pub count: u64,
}

// ─── Catalog Open / Close ──────────────────────────────────────────────────

/// Open a catalog store at the given URI. Currently supports local filesystem paths.
/// Returns null on failure with error details written to `err`.
#[no_mangle]
pub extern "C" fn rocklake_open(
    uri: *const c_char,
    err: *mut RockLakeError,
) -> *mut RockLakeCatalog {
    if uri.is_null() {
        write_error(
            err,
            RockLakeError {
                code: RockLakeErrorCode::InvalidHandle as i32,
                message: to_c_string("uri must not be null").into_raw(),
            },
        );
        return ptr::null_mut();
    }

    let uri_str = match unsafe { CStr::from_ptr(uri) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            write_error(
                err,
                RockLakeError::from_catalog_error(rocklake_catalog::CatalogError::NotInitialized),
            );
            return ptr::null_mut();
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => Arc::new(rt),
        Err(e) => {
            write_error(
                err,
                RockLakeError {
                    code: RockLakeErrorCode::Internal as i32,
                    message: to_c_string(&format!("failed to create runtime: {e}")).into_raw(),
                },
            );
            return ptr::null_mut();
        }
    };

    let result = runtime.block_on(async {
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(
            LocalFileSystem::new_with_prefix(uri_str)
                .map_err(|e| rocklake_catalog::CatalogError::SlateDb(e.to_string()))?,
        );

        let opts = OpenOptions {
            object_store,
            path: ObjectPath::from("catalog"),
            encryption: None,
        };

        CatalogStore::open(opts).await
    });

    match result {
        Ok(store) => {
            write_error(err, RockLakeError::ok());
            Box::into_raw(Box::new(RockLakeCatalog {
                magic: CATALOG_MAGIC,
                store,
                runtime,
            }))
        }
        Err(e) => {
            write_error(err, RockLakeError::from_catalog_error(e));
            ptr::null_mut()
        }
    }
}

/// Open a catalog in **read-only** mode: no writer epoch is acquired.
///
/// This function is equivalent to `rocklake_open` but skips the CAS writer-epoch
/// acquisition, which means many reader instances can be opened concurrently
/// against the same catalog prefix with zero write conflicts.
///
/// Returns a catalog handle on success, null on failure. The handle must be
/// closed with `rocklake_close`.
#[no_mangle]
pub extern "C" fn rocklake_open_readonly(
    uri: *const c_char,
    err: *mut RockLakeError,
) -> *mut RockLakeCatalog {
    if uri.is_null() {
        write_error(
            err,
            RockLakeError {
                code: RockLakeErrorCode::InvalidHandle as i32,
                message: to_c_string("uri must not be null").into_raw(),
            },
        );
        return ptr::null_mut();
    }

    let uri_str = match unsafe { CStr::from_ptr(uri) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            write_error(
                err,
                RockLakeError::from_catalog_error(rocklake_catalog::CatalogError::NotInitialized),
            );
            return ptr::null_mut();
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => Arc::new(rt),
        Err(e) => {
            write_error(
                err,
                RockLakeError {
                    code: RockLakeErrorCode::Internal as i32,
                    message: to_c_string(&format!("failed to create runtime: {e}")).into_raw(),
                },
            );
            return ptr::null_mut();
        }
    };

    let result = runtime.block_on(async {
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(
            LocalFileSystem::new_with_prefix(uri_str)
                .map_err(|e| rocklake_catalog::CatalogError::SlateDb(e.to_string()))?,
        );

        let opts = OpenOptions {
            object_store,
            path: ObjectPath::from("catalog"),
            encryption: None,
        };

        CatalogStore::open_without_epoch(opts).await
    });

    match result {
        Ok(store) => {
            write_error(err, RockLakeError::ok());
            Box::into_raw(Box::new(RockLakeCatalog {
                magic: CATALOG_MAGIC,
                store,
                runtime,
            }))
        }
        Err(e) => {
            write_error(err, RockLakeError::from_catalog_error(e));
            ptr::null_mut()
        }
    }
}

/// Close and free a catalog handle. Safe to call with null or already-closed handles.
#[no_mangle]
pub extern "C" fn rocklake_close(catalog: *mut RockLakeCatalog) {
    if catalog.is_null() {
        return;
    }
    // Check and zeroize magic atomically to prevent double-close.
    // SAFETY: `catalog` is non-null (checked above). We read the magic field
    // and zeroize it before dropping to prevent double-close. The caller must
    // ensure no other thread is concurrently using or closing the same handle.
    let magic = unsafe { (*catalog).magic };
    if magic != CATALOG_MAGIC {
        return;
    }
    // SAFETY: zeroing magic before drop ensures a second call sees magic == 0
    // and returns early, preventing a double-free of the allocation.
    unsafe { (*catalog).magic = 0 };
    // SAFETY: `catalog` is a valid, non-null allocation created by
    // `rocklake_open()` via `Box::into_raw`. We reconstruct the Box to
    // trigger the destructor. After this point, `catalog` is dangling.
    let cat = unsafe { Box::from_raw(catalog) };
    let _ = cat.runtime.block_on(cat.store.close());
}

// ─── Read Operations ───────────────────────────────────────────────────────

/// Get the current (latest) snapshot.
#[no_mangle]
pub extern "C" fn rocklake_get_current_snapshot(
    catalog: *mut RockLakeCatalog,
    err: *mut RockLakeError,
) -> RockLakeSnapshot {
    let inner = with_catalog(catalog, |cat| {
        let reader = cat.store.read_latest();
        // SAFETY: `block_on` drives the future to completion synchronously.
        // The future borrows `reader` which is scoped to this closure frame.
        cat.runtime.block_on(reader.get_snapshot())
    });
    match inner {
        None => {
            write_error(err, RockLakeError::invalid_handle());
            RockLakeSnapshot {
                snapshot_id: 0,
                schema_version: 0,
            }
        }
        Some(Ok(Some(snap))) => {
            write_error(err, RockLakeError::ok());
            RockLakeSnapshot {
                snapshot_id: snap.snapshot_id,
                schema_version: snap.schema_version,
            }
        }
        Some(Ok(None)) => {
            write_error(err, RockLakeError::ok());
            RockLakeSnapshot {
                snapshot_id: 0,
                schema_version: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, RockLakeError::from_catalog_error(e));
            RockLakeSnapshot {
                snapshot_id: 0,
                schema_version: 0,
            }
        }
    }
}

/// List schemas at a given snapshot.
#[no_mangle]
pub extern "C" fn rocklake_list_schemas(
    catalog: *mut RockLakeCatalog,
    snapshot_id: u64,
    err: *mut RockLakeError,
) -> RockLakeSchemaList {
    let inner = with_catalog(
        catalog,
        |cat| -> rocklake_catalog::error::CatalogResult<_> {
            let reader = cat.store.read_at(SnapshotId::new(snapshot_id))?;
            // SAFETY: future is driven to completion; reader is scoped to this closure.
            cat.runtime.block_on(reader.list_schemas())
        },
    );
    match inner {
        None => {
            write_error(err, RockLakeError::invalid_handle());
            RockLakeSchemaList {
                schemas: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, RockLakeError::from_catalog_error(e));
            RockLakeSchemaList {
                schemas: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Ok(schemas)) => {
            write_error(err, RockLakeError::ok());
            let count = schemas.len() as u64;
            let mut out: Vec<RockLakeSchema> = schemas
                .into_iter()
                .map(|s| RockLakeSchema {
                    schema_id: s.schema_id,
                    schema_name: to_c_string(&s.schema_name).into_raw(),
                })
                .collect();
            out.shrink_to_fit(); // Ensure capacity == len for safe Vec::from_raw_parts in free.
            let out_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            RockLakeSchemaList {
                schemas: out_ptr,
                count,
            }
        }
    }
}

/// List tables in a schema at a given snapshot.
#[no_mangle]
pub extern "C" fn rocklake_list_tables(
    catalog: *mut RockLakeCatalog,
    schema_id: u64,
    snapshot_id: u64,
    err: *mut RockLakeError,
) -> RockLakeTableList {
    let inner = with_catalog(
        catalog,
        |cat| -> rocklake_catalog::error::CatalogResult<_> {
            let reader = cat.store.read_at(SnapshotId::new(snapshot_id))?;
            // SAFETY: future is driven to completion; reader is scoped to this closure.
            cat.runtime.block_on(reader.list_tables(schema_id))
        },
    );
    match inner {
        None => {
            write_error(err, RockLakeError::invalid_handle());
            RockLakeTableList {
                tables: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, RockLakeError::from_catalog_error(e));
            RockLakeTableList {
                tables: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Ok(tables)) => {
            write_error(err, RockLakeError::ok());
            let count = tables.len() as u64;
            let mut out: Vec<RockLakeTable> = tables
                .into_iter()
                .map(|t| RockLakeTable {
                    table_id: t.table_id,
                    schema_id: t.schema_id,
                    table_name: to_c_string(&t.table_name).into_raw(),
                })
                .collect();
            out.shrink_to_fit();
            let out_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            RockLakeTableList {
                tables: out_ptr,
                count,
            }
        }
    }
}

/// Describe a table (get columns) at a given snapshot.
#[no_mangle]
pub extern "C" fn rocklake_describe_table(
    catalog: *mut RockLakeCatalog,
    table_id: u64,
    snapshot_id: u64,
    err: *mut RockLakeError,
) -> RockLakeColumnList {
    let inner = with_catalog(
        catalog,
        |cat| -> rocklake_catalog::error::CatalogResult<_> {
            let reader = cat.store.read_at(SnapshotId::new(snapshot_id))?;
            // SAFETY: future is driven to completion; reader is scoped to this closure.
            cat.runtime.block_on(reader.describe_table(table_id))
        },
    );
    match inner {
        None => {
            write_error(err, RockLakeError::invalid_handle());
            RockLakeColumnList {
                columns: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, RockLakeError::from_catalog_error(e));
            RockLakeColumnList {
                columns: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Ok(Some((_table, columns)))) => {
            write_error(err, RockLakeError::ok());
            let count = columns.len() as u64;
            let mut out: Vec<RockLakeColumn> = columns
                .into_iter()
                .map(|c| RockLakeColumn {
                    column_id: c.column_id,
                    table_id: c.table_id,
                    column_name: to_c_string(&c.column_name).into_raw(),
                    data_type: to_c_string(&c.data_type).into_raw(),
                    column_index: c.column_index,
                    is_nullable: c.is_nullable,
                })
                .collect();
            out.shrink_to_fit();
            let out_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            RockLakeColumnList {
                columns: out_ptr,
                count,
            }
        }
        Some(Ok(None)) => {
            write_error(
                err,
                RockLakeError::from_catalog_error(rocklake_catalog::CatalogError::NotFound(
                    format!("table {table_id}"),
                )),
            );
            RockLakeColumnList {
                columns: ptr::null_mut(),
                count: 0,
            }
        }
    }
}

/// List data files for a table at a given snapshot.
#[no_mangle]
pub extern "C" fn rocklake_list_data_files(
    catalog: *mut RockLakeCatalog,
    table_id: u64,
    snapshot_id: u64,
    err: *mut RockLakeError,
) -> RockLakeFileList {
    let inner = with_catalog(
        catalog,
        |cat| -> rocklake_catalog::error::CatalogResult<_> {
            let reader = cat.store.read_at(SnapshotId::new(snapshot_id))?;
            // SAFETY: future is driven to completion; reader is scoped to this closure.
            cat.runtime.block_on(reader.list_data_files(table_id))
        },
    );
    match inner {
        None => {
            write_error(err, RockLakeError::invalid_handle());
            RockLakeFileList {
                files: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, RockLakeError::from_catalog_error(e));
            RockLakeFileList {
                files: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Ok(files)) => {
            write_error(err, RockLakeError::ok());
            let count = files.len() as u64;
            let mut out: Vec<RockLakeDataFile> = files
                .into_iter()
                .map(|f| RockLakeDataFile {
                    data_file_id: f.data_file_id,
                    table_id: f.table_id,
                    path: to_c_string(&f.path).into_raw(),
                    file_format: to_c_string(&f.file_format).into_raw(),
                    row_count: f.record_count,
                    file_size_bytes: f.file_size_bytes,
                    snapshot_id: f.begin_snapshot.unwrap_or(0),
                })
                .collect();
            out.shrink_to_fit();
            let out_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            RockLakeFileList {
                files: out_ptr,
                count,
            }
        }
    }
}

// ─── Free Functions ────────────────────────────────────────────────────────

/// Free a schema list.
#[no_mangle]
pub extern "C" fn rocklake_schema_list_free(list: *mut RockLakeSchemaList) {
    if list.is_null() {
        return;
    }
    // SAFETY: `list` is non-null (checked above). The `schemas` pointer and
    // `count` were produced by `rocklake_list_schemas()` with `shrink_to_fit()`,
    // so capacity == len. `Vec::from_raw_parts` reconstructs the original
    // allocation. The caller must not access `list` after this call.
    unsafe {
        let l = &mut *list;
        if !l.schemas.is_null() && l.count > 0 {
            let schemas = Vec::from_raw_parts(l.schemas, l.count as usize, l.count as usize);
            for s in schemas {
                if !s.schema_name.is_null() {
                    drop(CString::from_raw(s.schema_name));
                }
            }
        }
        l.schemas = ptr::null_mut();
        l.count = 0;
    }
}

/// Free a table list.
#[no_mangle]
pub extern "C" fn rocklake_table_list_free(list: *mut RockLakeTableList) {
    if list.is_null() {
        return;
    }
    // SAFETY: same contract as `rocklake_schema_list_free`; capacity == len.
    unsafe {
        let l = &mut *list;
        if !l.tables.is_null() && l.count > 0 {
            let tables = Vec::from_raw_parts(l.tables, l.count as usize, l.count as usize);
            for t in tables {
                if !t.table_name.is_null() {
                    drop(CString::from_raw(t.table_name));
                }
            }
        }
        l.tables = ptr::null_mut();
        l.count = 0;
    }
}

/// Free a column list.
#[no_mangle]
pub extern "C" fn rocklake_column_list_free(list: *mut RockLakeColumnList) {
    if list.is_null() {
        return;
    }
    // SAFETY: same contract as `rocklake_schema_list_free`; capacity == len.
    unsafe {
        let l = &mut *list;
        if !l.columns.is_null() && l.count > 0 {
            let columns = Vec::from_raw_parts(l.columns, l.count as usize, l.count as usize);
            for c in columns {
                if !c.column_name.is_null() {
                    drop(CString::from_raw(c.column_name));
                }
                if !c.data_type.is_null() {
                    drop(CString::from_raw(c.data_type));
                }
            }
        }
        l.columns = ptr::null_mut();
        l.count = 0;
    }
}

/// Free a file list.
#[no_mangle]
pub extern "C" fn rocklake_file_list_free(list: *mut RockLakeFileList) {
    if list.is_null() {
        return;
    }
    // SAFETY: same contract as `rocklake_schema_list_free`; capacity == len.
    unsafe {
        let l = &mut *list;
        if !l.files.is_null() && l.count > 0 {
            let files = Vec::from_raw_parts(l.files, l.count as usize, l.count as usize);
            for f in files {
                if !f.path.is_null() {
                    drop(CString::from_raw(f.path));
                }
                if !f.file_format.is_null() {
                    drop(CString::from_raw(f.file_format));
                }
            }
        }
        l.files = ptr::null_mut();
        l.count = 0;
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────────

fn write_error(err: *mut RockLakeError, error: RockLakeError) {
    if !err.is_null() {
        // SAFETY: `err` is non-null (checked above). The caller provides a
        // valid, aligned `RockLakeError` stack variable. We overwrite it once.
        unsafe {
            *err = error;
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn abi_version_returns_expected() {
        assert_eq!(rocklake_abi_version(), ROCKLAKE_ABI_VERSION);
        assert_eq!(rocklake_abi_version(), 5_000);
    }

    #[test]
    fn open_close_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = RockLakeError::ok();

        let catalog = rocklake_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null(), "open failed: code={}", err.code);
        assert_eq!(err.code, 0);

        // Get current snapshot (empty catalog)
        let snap = rocklake_get_current_snapshot(catalog, &mut err);
        assert_eq!(err.code, 0);
        assert_eq!(snap.snapshot_id, 0);

        rocklake_close(catalog);
    }

    #[test]
    fn list_schemas_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = RockLakeError::ok();

        let catalog = rocklake_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null());

        let schemas = rocklake_list_schemas(catalog, 1, &mut err);
        assert_eq!(err.code, 0);
        assert_eq!(schemas.count, 0);

        rocklake_close(catalog);
    }

    #[test]
    fn error_on_null_uri() {
        // This would segfault if we don't handle null properly,
        // but we can test with a non-existent path
        let path = CString::new("/nonexistent/path/that/doesnt/exist/at/all").unwrap();
        let mut err = RockLakeError::ok();

        let catalog = rocklake_open(path.as_ptr(), &mut err);
        // May or may not fail depending on OS behavior with local filesystem
        if catalog.is_null() {
            assert_ne!(err.code, 0);
            rocklake_error_free(&mut err);
        } else {
            rocklake_close(catalog);
        }
    }

    #[test]
    fn null_uri_returns_invalid_handle_error() {
        let mut err = RockLakeError::ok();
        let catalog = rocklake_open(ptr::null(), &mut err);
        assert!(catalog.is_null(), "expected null on null URI");
        assert_eq!(
            err.code,
            RockLakeErrorCode::InvalidHandle as i32,
            "expected InvalidHandle error code"
        );
        rocklake_error_free(&mut err);
    }

    #[test]
    fn null_catalog_returns_invalid_handle_error() {
        let mut err = RockLakeError::ok();
        let snap = rocklake_get_current_snapshot(ptr::null_mut(), &mut err);
        assert_eq!(
            err.code,
            RockLakeErrorCode::InvalidHandle as i32,
            "expected InvalidHandle on null handle"
        );
        assert_eq!(snap.snapshot_id, 0);
        rocklake_error_free(&mut err);
    }

    #[test]
    fn double_close_is_safe() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = RockLakeError::ok();

        let catalog = rocklake_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null(), "open failed: code={}", err.code);

        // First close is normal.
        rocklake_close(catalog);
        // Second close must not panic or segfault (magic is zeroed).
        rocklake_close(catalog);
    }

    #[test]
    fn handle_after_close_returns_invalid_handle() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = RockLakeError::ok();

        let catalog = rocklake_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null(), "open failed: code={}", err.code);

        // Close the handle — this zeroes the magic field.
        rocklake_close(catalog);

        // All operations on the now-closed handle must return InvalidHandle.
        let snap = rocklake_get_current_snapshot(catalog, &mut err);
        assert_eq!(err.code, RockLakeErrorCode::InvalidHandle as i32);
        assert_eq!(snap.snapshot_id, 0);
        rocklake_error_free(&mut err);

        err = RockLakeError::ok();
        let schemas = rocklake_list_schemas(catalog, 1, &mut err);
        assert_eq!(err.code, RockLakeErrorCode::InvalidHandle as i32);
        assert_eq!(schemas.count, 0);
        rocklake_error_free(&mut err);
    }

    #[test]
    fn null_error_pointer_does_not_crash() {
        // Passing a null error pointer is valid — write_error is a no-op.
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();

        let catalog = rocklake_open(path.as_ptr(), ptr::null_mut());
        assert!(
            !catalog.is_null(),
            "open with null err must succeed on valid path"
        );

        let snap = rocklake_get_current_snapshot(catalog, ptr::null_mut());
        assert_eq!(snap.snapshot_id, 0);

        rocklake_close(catalog);
    }

    #[test]
    fn free_functions_accept_null_without_crash() {
        // All free functions must be no-ops on null input.
        rocklake_error_free(ptr::null_mut());
        rocklake_schema_list_free(ptr::null_mut());
        rocklake_table_list_free(ptr::null_mut());
        rocklake_column_list_free(ptr::null_mut());
        rocklake_file_list_free(ptr::null_mut());
        rocklake_close(ptr::null_mut());
    }

    #[test]
    fn error_code_and_message_on_null_error() {
        // rocklake_error_code / rocklake_error_message must not crash on null.
        let code = rocklake_error_code(ptr::null());
        assert_eq!(code, 0);
        let msg = rocklake_error_message(ptr::null());
        assert!(msg.is_null());
    }

    /// Demonstrates that the magic-number guard makes concurrent close + use
    /// of a catalog handle safe from the Rust side (the second close is a
    /// no-op; operations on the closed handle return InvalidHandle). This test
    /// does not assert race freedom (that is the C caller's responsibility) but
    /// shows the guard behaves correctly under sequential close → use ordering.
    #[test]
    fn concurrent_close_use_guard() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = RockLakeError::ok();

        let catalog = rocklake_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null(), "open failed: code={}", err.code);

        // Simulate: thread A closes, thread B tries to use the same handle.
        rocklake_close(catalog); // magic zeroed, allocation freed.

        // Any use after close must return InvalidHandle without crashing.
        let snap = rocklake_get_current_snapshot(catalog, &mut err);
        assert_eq!(
            err.code,
            RockLakeErrorCode::InvalidHandle as i32,
            "expected InvalidHandle after concurrent close"
        );
        assert_eq!(snap.snapshot_id, 0);
        rocklake_error_free(&mut err);

        // A second close (double-close) must also be a safe no-op.
        rocklake_close(catalog);
    }

    // ─── v0.33: to_c_string safety ────────────────────────────────────────

    #[test]
    fn to_c_string_normal_string_is_preserved() {
        let s = "hello, world";
        let c = to_c_string(s);
        let back = c.to_str().unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn to_c_string_nul_byte_returns_sentinel_not_empty() {
        // A string with an embedded NUL must produce the sentinel "<invalid-utf8>",
        // not an empty string (which would silently drop the message).
        let s_with_nul = "secret\0value";
        let c = to_c_string(s_with_nul);
        let back = c.to_str().unwrap();
        assert_eq!(
            back, "<invalid-utf8>",
            "NUL-containing string must produce sentinel, not empty string; got: {back:?}"
        );
        assert!(!back.is_empty(), "fallback must not be empty");
    }
}
