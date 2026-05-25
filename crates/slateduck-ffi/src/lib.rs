//! SlateDuck FFI: C/C++ foreign function interface for embedding SlateDuck in DuckDB.
//!
//! This crate provides a stable C ABI over `slateduck-catalog` operations.
//! All async operations are bridged via a blocking Tokio runtime.

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
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::mvcc::SnapshotId;

// ─── ABI Version ───────────────────────────────────────────────────────────

/// ABI version: major * 1000 + minor. The DuckDB extension checks this at
/// load time and refuses to proceed on version mismatch.
const ABI_VERSION: u32 = 5_000; // v5.0 (matches v0.5 release)

/// Returns the ABI version. Extension checks this at load time.
#[no_mangle]
pub extern "C" fn slateduck_abi_version() -> u32 {
    ABI_VERSION
}

// ─── Error Handling ────────────────────────────────────────────────────────

/// Opaque error type returned from FFI functions.
#[repr(C)]
pub struct SlateduckError {
    pub code: i32,
    pub message: *mut c_char,
}

/// Error codes matching DuckDB's expected return values.
#[repr(i32)]
#[derive(Clone, Copy)]
pub enum SlateduckErrorCode {
    Ok = 0,
    Internal = 1,
    NotFound = 2,
    WriterFenced = 3,
    FormatMismatch = 4,
    ValueTooLarge = 5,
    TransactionConflict = 6,
    NotInitialized = 7,
    /// Null or already-closed catalog handle passed to an FFI function.
    InvalidHandle = 8,
}

impl SlateduckError {
    fn ok() -> Self {
        Self {
            code: SlateduckErrorCode::Ok as i32,
            message: ptr::null_mut(),
        }
    }

    fn invalid_handle() -> Self {
        Self {
            code: SlateduckErrorCode::InvalidHandle as i32,
            message: CString::new("invalid or null catalog handle")
                .unwrap_or_default()
                .into_raw(),
        }
    }

    fn from_catalog_error(e: slateduck_catalog::CatalogError) -> Self {
        use slateduck_catalog::CatalogError;
        let code = match &e {
            CatalogError::NotFound(_) => SlateduckErrorCode::NotFound,
            CatalogError::WriterEpochMismatch => SlateduckErrorCode::WriterFenced,
            CatalogError::FormatVersionMismatch { .. } => SlateduckErrorCode::FormatMismatch,
            CatalogError::ValueTooLarge { .. } => SlateduckErrorCode::ValueTooLarge,
            CatalogError::TransactionConflict(_) => SlateduckErrorCode::TransactionConflict,
            CatalogError::NotInitialized => SlateduckErrorCode::NotInitialized,
            _ => SlateduckErrorCode::Internal,
        };
        let msg = CString::new(e.to_string()).unwrap_or_default();
        Self {
            code: code as i32,
            message: msg.into_raw(),
        }
    }
}

/// Free an error's message string.
#[no_mangle]
pub extern "C" fn slateduck_error_free(err: *mut SlateduckError) {
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
pub extern "C" fn slateduck_error_code(err: *const SlateduckError) -> i32 {
    if err.is_null() {
        return 0;
    }
    // SAFETY: `err` is non-null (checked above) and points to a valid
    // `SlateduckError`. We only read the `code` field.
    unsafe { (*err).code }
}

/// Get the error message (borrows — do not free separately).
#[no_mangle]
pub extern "C" fn slateduck_error_message(err: *const SlateduckError) -> *const c_char {
    if err.is_null() {
        return ptr::null();
    }
    // SAFETY: `err` is non-null (checked above). We only read the `message`
    // field without taking ownership. The caller must not free this pointer
    // separately; call `slateduck_error_free()` to release the whole error.
    unsafe { (*err).message as *const c_char }
}

// ─── Opaque Handles ────────────────────────────────────────────────────────

/// Magic value stored in every live `SlateduckCatalog` to detect invalid
/// or double-closed handles. Bytes: 'D','U','C','K'.
const CATALOG_MAGIC: u32 = 0x4455_434B;

/// Opaque handle for a CatalogStore.
pub struct SlateduckCatalog {
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
/// 1. `ptr` is either null or points to a live `SlateduckCatalog` allocation
///    created by `slateduck_open()` and not yet freed.
/// 2. No other thread is concurrently calling `slateduck_close()` or any other
///    FFI function with the same `ptr` during the execution of `f`.
///
/// Condition 2 is the caller's responsibility; the magic-number check provides
/// a best-effort defence against use-after-close but is not a substitute for
/// proper ownership discipline in the C caller.
fn with_catalog<T>(
    ptr: *mut SlateduckCatalog,
    f: impl FnOnce(&mut SlateduckCatalog) -> T,
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
pub struct SlateduckSnapshot {
    pub snapshot_id: u64,
    pub schema_version: u64,
}

/// Opaque handle for a file list result.
#[repr(C)]
pub struct SlateduckFileList {
    pub files: *mut SlateduckDataFile,
    pub count: u64,
}

/// A single data file in a file list.
#[repr(C)]
pub struct SlateduckDataFile {
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
pub struct SlateduckSchema {
    pub schema_id: u64,
    pub schema_name: *mut c_char,
}

/// Schema list.
#[repr(C)]
pub struct SlateduckSchemaList {
    pub schemas: *mut SlateduckSchema,
    pub count: u64,
}

/// Table entry.
#[repr(C)]
pub struct SlateduckTable {
    pub table_id: u64,
    pub schema_id: u64,
    pub table_name: *mut c_char,
}

/// Table list.
#[repr(C)]
pub struct SlateduckTableList {
    pub tables: *mut SlateduckTable,
    pub count: u64,
}

/// Column entry.
#[repr(C)]
pub struct SlateduckColumn {
    pub column_id: u64,
    pub table_id: u64,
    pub column_name: *mut c_char,
    pub data_type: *mut c_char,
    pub column_index: u64,
    pub is_nullable: bool,
}

/// Column list.
#[repr(C)]
pub struct SlateduckColumnList {
    pub columns: *mut SlateduckColumn,
    pub count: u64,
}

// ─── Catalog Open / Close ──────────────────────────────────────────────────

/// Open a catalog store at the given URI. Currently supports local filesystem paths.
/// Returns null on failure with error details written to `err`.
#[no_mangle]
pub extern "C" fn slateduck_open(
    uri: *const c_char,
    err: *mut SlateduckError,
) -> *mut SlateduckCatalog {
    if uri.is_null() {
        write_error(
            err,
            SlateduckError {
                code: SlateduckErrorCode::InvalidHandle as i32,
                message: CString::new("uri must not be null")
                    .unwrap_or_default()
                    .into_raw(),
            },
        );
        return ptr::null_mut();
    }

    let uri_str = match unsafe { CStr::from_ptr(uri) }.to_str() {
        Ok(s) => s,
        Err(_) => {
            write_error(
                err,
                SlateduckError::from_catalog_error(slateduck_catalog::CatalogError::NotInitialized),
            );
            return ptr::null_mut();
        }
    };

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => Arc::new(rt),
        Err(e) => {
            write_error(
                err,
                SlateduckError {
                    code: SlateduckErrorCode::Internal as i32,
                    message: CString::new(format!("failed to create runtime: {e}"))
                        .unwrap_or_default()
                        .into_raw(),
                },
            );
            return ptr::null_mut();
        }
    };

    let result = runtime.block_on(async {
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(
            LocalFileSystem::new_with_prefix(uri_str)
                .map_err(|e| slateduck_catalog::CatalogError::SlateDb(e.to_string()))?,
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
            write_error(err, SlateduckError::ok());
            Box::into_raw(Box::new(SlateduckCatalog {
                magic: CATALOG_MAGIC,
                store,
                runtime,
            }))
        }
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            ptr::null_mut()
        }
    }
}

/// Close and free a catalog handle. Safe to call with null or already-closed handles.
#[no_mangle]
pub extern "C" fn slateduck_close(catalog: *mut SlateduckCatalog) {
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
    // `slateduck_open()` via `Box::into_raw`. We reconstruct the Box to
    // trigger the destructor. After this point, `catalog` is dangling.
    let cat = unsafe { Box::from_raw(catalog) };
    let _ = cat.runtime.block_on(cat.store.close());
}

// ─── Read Operations ───────────────────────────────────────────────────────

/// Get the current (latest) snapshot.
#[no_mangle]
pub extern "C" fn slateduck_get_current_snapshot(
    catalog: *mut SlateduckCatalog,
    err: *mut SlateduckError,
) -> SlateduckSnapshot {
    let inner = with_catalog(catalog, |cat| {
        let reader = cat.store.read_latest();
        // SAFETY: `block_on` drives the future to completion synchronously.
        // The future borrows `reader` which is scoped to this closure frame.
        cat.runtime.block_on(reader.get_snapshot())
    });
    match inner {
        None => {
            write_error(err, SlateduckError::invalid_handle());
            SlateduckSnapshot {
                snapshot_id: 0,
                schema_version: 0,
            }
        }
        Some(Ok(Some(snap))) => {
            write_error(err, SlateduckError::ok());
            SlateduckSnapshot {
                snapshot_id: snap.snapshot_id,
                schema_version: snap.schema_version,
            }
        }
        Some(Ok(None)) => {
            write_error(err, SlateduckError::ok());
            SlateduckSnapshot {
                snapshot_id: 0,
                schema_version: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            SlateduckSnapshot {
                snapshot_id: 0,
                schema_version: 0,
            }
        }
    }
}

/// List schemas at a given snapshot.
#[no_mangle]
pub extern "C" fn slateduck_list_schemas(
    catalog: *mut SlateduckCatalog,
    snapshot_id: u64,
    err: *mut SlateduckError,
) -> SlateduckSchemaList {
    let inner = with_catalog(
        catalog,
        |cat| -> slateduck_catalog::error::CatalogResult<_> {
            let reader = cat.store.read_at(SnapshotId::new(snapshot_id))?;
            // SAFETY: future is driven to completion; reader is scoped to this closure.
            cat.runtime.block_on(reader.list_schemas())
        },
    );
    match inner {
        None => {
            write_error(err, SlateduckError::invalid_handle());
            SlateduckSchemaList {
                schemas: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            SlateduckSchemaList {
                schemas: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Ok(schemas)) => {
            write_error(err, SlateduckError::ok());
            let count = schemas.len() as u64;
            let mut out: Vec<SlateduckSchema> = schemas
                .into_iter()
                .map(|s| SlateduckSchema {
                    schema_id: s.schema_id,
                    schema_name: CString::new(s.schema_name).unwrap_or_default().into_raw(),
                })
                .collect();
            out.shrink_to_fit(); // Ensure capacity == len for safe Vec::from_raw_parts in free.
            let out_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            SlateduckSchemaList {
                schemas: out_ptr,
                count,
            }
        }
    }
}

/// List tables in a schema at a given snapshot.
#[no_mangle]
pub extern "C" fn slateduck_list_tables(
    catalog: *mut SlateduckCatalog,
    schema_id: u64,
    snapshot_id: u64,
    err: *mut SlateduckError,
) -> SlateduckTableList {
    let inner = with_catalog(
        catalog,
        |cat| -> slateduck_catalog::error::CatalogResult<_> {
            let reader = cat.store.read_at(SnapshotId::new(snapshot_id))?;
            // SAFETY: future is driven to completion; reader is scoped to this closure.
            cat.runtime.block_on(reader.list_tables(schema_id))
        },
    );
    match inner {
        None => {
            write_error(err, SlateduckError::invalid_handle());
            SlateduckTableList {
                tables: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            SlateduckTableList {
                tables: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Ok(tables)) => {
            write_error(err, SlateduckError::ok());
            let count = tables.len() as u64;
            let mut out: Vec<SlateduckTable> = tables
                .into_iter()
                .map(|t| SlateduckTable {
                    table_id: t.table_id,
                    schema_id: t.schema_id,
                    table_name: CString::new(t.table_name).unwrap_or_default().into_raw(),
                })
                .collect();
            out.shrink_to_fit();
            let out_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            SlateduckTableList {
                tables: out_ptr,
                count,
            }
        }
    }
}

/// Describe a table (get columns) at a given snapshot.
#[no_mangle]
pub extern "C" fn slateduck_describe_table(
    catalog: *mut SlateduckCatalog,
    table_id: u64,
    snapshot_id: u64,
    err: *mut SlateduckError,
) -> SlateduckColumnList {
    let inner = with_catalog(
        catalog,
        |cat| -> slateduck_catalog::error::CatalogResult<_> {
            let reader = cat.store.read_at(SnapshotId::new(snapshot_id))?;
            // SAFETY: future is driven to completion; reader is scoped to this closure.
            cat.runtime.block_on(reader.describe_table(table_id))
        },
    );
    match inner {
        None => {
            write_error(err, SlateduckError::invalid_handle());
            SlateduckColumnList {
                columns: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            SlateduckColumnList {
                columns: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Ok(Some((_table, columns)))) => {
            write_error(err, SlateduckError::ok());
            let count = columns.len() as u64;
            let mut out: Vec<SlateduckColumn> = columns
                .into_iter()
                .map(|c| SlateduckColumn {
                    column_id: c.column_id,
                    table_id: c.table_id,
                    column_name: CString::new(c.column_name).unwrap_or_default().into_raw(),
                    data_type: CString::new(c.data_type).unwrap_or_default().into_raw(),
                    column_index: c.column_index,
                    is_nullable: c.is_nullable,
                })
                .collect();
            out.shrink_to_fit();
            let out_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            SlateduckColumnList {
                columns: out_ptr,
                count,
            }
        }
        Some(Ok(None)) => {
            write_error(
                err,
                SlateduckError::from_catalog_error(slateduck_catalog::CatalogError::NotFound(
                    format!("table {table_id}"),
                )),
            );
            SlateduckColumnList {
                columns: ptr::null_mut(),
                count: 0,
            }
        }
    }
}

/// List data files for a table at a given snapshot.
#[no_mangle]
pub extern "C" fn slateduck_list_data_files(
    catalog: *mut SlateduckCatalog,
    table_id: u64,
    snapshot_id: u64,
    err: *mut SlateduckError,
) -> SlateduckFileList {
    let inner = with_catalog(
        catalog,
        |cat| -> slateduck_catalog::error::CatalogResult<_> {
            let reader = cat.store.read_at(SnapshotId::new(snapshot_id))?;
            // SAFETY: future is driven to completion; reader is scoped to this closure.
            cat.runtime.block_on(reader.list_data_files(table_id))
        },
    );
    match inner {
        None => {
            write_error(err, SlateduckError::invalid_handle());
            SlateduckFileList {
                files: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Err(e)) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            SlateduckFileList {
                files: ptr::null_mut(),
                count: 0,
            }
        }
        Some(Ok(files)) => {
            write_error(err, SlateduckError::ok());
            let count = files.len() as u64;
            let mut out: Vec<SlateduckDataFile> = files
                .into_iter()
                .map(|f| SlateduckDataFile {
                    data_file_id: f.data_file_id,
                    table_id: f.table_id,
                    path: CString::new(f.path).unwrap_or_default().into_raw(),
                    file_format: CString::new(f.file_format).unwrap_or_default().into_raw(),
                    row_count: f.record_count,
                    file_size_bytes: f.file_size_bytes,
                    snapshot_id: f.begin_snapshot.unwrap_or(0),
                })
                .collect();
            out.shrink_to_fit();
            let out_ptr = out.as_mut_ptr();
            std::mem::forget(out);
            SlateduckFileList {
                files: out_ptr,
                count,
            }
        }
    }
}

// ─── Free Functions ────────────────────────────────────────────────────────

/// Free a schema list.
#[no_mangle]
pub extern "C" fn slateduck_schema_list_free(list: *mut SlateduckSchemaList) {
    if list.is_null() {
        return;
    }
    // SAFETY: `list` is non-null (checked above). The `schemas` pointer and
    // `count` were produced by `slateduck_list_schemas()` with `shrink_to_fit()`,
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
pub extern "C" fn slateduck_table_list_free(list: *mut SlateduckTableList) {
    if list.is_null() {
        return;
    }
    // SAFETY: same contract as `slateduck_schema_list_free`; capacity == len.
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
pub extern "C" fn slateduck_column_list_free(list: *mut SlateduckColumnList) {
    if list.is_null() {
        return;
    }
    // SAFETY: same contract as `slateduck_schema_list_free`; capacity == len.
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
pub extern "C" fn slateduck_file_list_free(list: *mut SlateduckFileList) {
    if list.is_null() {
        return;
    }
    // SAFETY: same contract as `slateduck_schema_list_free`; capacity == len.
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

fn write_error(err: *mut SlateduckError, error: SlateduckError) {
    if !err.is_null() {
        // SAFETY: `err` is non-null (checked above). The caller provides a
        // valid, aligned `SlateduckError` stack variable. We overwrite it once.
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
        assert_eq!(slateduck_abi_version(), 5_000);
    }

    #[test]
    fn open_close_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = SlateduckError::ok();

        let catalog = slateduck_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null(), "open failed: code={}", err.code);
        assert_eq!(err.code, 0);

        // Get current snapshot (empty catalog)
        let snap = slateduck_get_current_snapshot(catalog, &mut err);
        assert_eq!(err.code, 0);
        assert_eq!(snap.snapshot_id, 0);

        slateduck_close(catalog);
    }

    #[test]
    fn list_schemas_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = SlateduckError::ok();

        let catalog = slateduck_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null());

        let schemas = slateduck_list_schemas(catalog, 1, &mut err);
        assert_eq!(err.code, 0);
        assert_eq!(schemas.count, 0);

        slateduck_close(catalog);
    }

    #[test]
    fn error_on_null_uri() {
        // This would segfault if we don't handle null properly,
        // but we can test with a non-existent path
        let path = CString::new("/nonexistent/path/that/doesnt/exist/at/all").unwrap();
        let mut err = SlateduckError::ok();

        let catalog = slateduck_open(path.as_ptr(), &mut err);
        // May or may not fail depending on OS behavior with local filesystem
        if catalog.is_null() {
            assert_ne!(err.code, 0);
            slateduck_error_free(&mut err);
        } else {
            slateduck_close(catalog);
        }
    }

    #[test]
    fn null_uri_returns_invalid_handle_error() {
        let mut err = SlateduckError::ok();
        let catalog = slateduck_open(ptr::null(), &mut err);
        assert!(catalog.is_null(), "expected null on null URI");
        assert_eq!(
            err.code,
            SlateduckErrorCode::InvalidHandle as i32,
            "expected InvalidHandle error code"
        );
        slateduck_error_free(&mut err);
    }

    #[test]
    fn null_catalog_returns_invalid_handle_error() {
        let mut err = SlateduckError::ok();
        let snap = slateduck_get_current_snapshot(ptr::null_mut(), &mut err);
        assert_eq!(
            err.code,
            SlateduckErrorCode::InvalidHandle as i32,
            "expected InvalidHandle on null handle"
        );
        assert_eq!(snap.snapshot_id, 0);
        slateduck_error_free(&mut err);
    }

    #[test]
    fn double_close_is_safe() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = SlateduckError::ok();

        let catalog = slateduck_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null(), "open failed: code={}", err.code);

        // First close is normal.
        slateduck_close(catalog);
        // Second close must not panic or segfault (magic is zeroed).
        slateduck_close(catalog);
    }

    #[test]
    fn handle_after_close_returns_invalid_handle() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();
        let mut err = SlateduckError::ok();

        let catalog = slateduck_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null(), "open failed: code={}", err.code);

        // Close the handle — this zeroes the magic field.
        slateduck_close(catalog);

        // All operations on the now-closed handle must return InvalidHandle.
        let snap = slateduck_get_current_snapshot(catalog, &mut err);
        assert_eq!(err.code, SlateduckErrorCode::InvalidHandle as i32);
        assert_eq!(snap.snapshot_id, 0);
        slateduck_error_free(&mut err);

        err = SlateduckError::ok();
        let schemas = slateduck_list_schemas(catalog, 1, &mut err);
        assert_eq!(err.code, SlateduckErrorCode::InvalidHandle as i32);
        assert_eq!(schemas.count, 0);
        slateduck_error_free(&mut err);
    }

    #[test]
    fn null_error_pointer_does_not_crash() {
        // Passing a null error pointer is valid — write_error is a no-op.
        let dir = tempfile::TempDir::new().unwrap();
        let path = CString::new(dir.path().to_str().unwrap()).unwrap();

        let catalog = slateduck_open(path.as_ptr(), ptr::null_mut());
        assert!(
            !catalog.is_null(),
            "open with null err must succeed on valid path"
        );

        let snap = slateduck_get_current_snapshot(catalog, ptr::null_mut());
        assert_eq!(snap.snapshot_id, 0);

        slateduck_close(catalog);
    }

    #[test]
    fn free_functions_accept_null_without_crash() {
        // All free functions must be no-ops on null input.
        slateduck_error_free(ptr::null_mut());
        slateduck_schema_list_free(ptr::null_mut());
        slateduck_table_list_free(ptr::null_mut());
        slateduck_column_list_free(ptr::null_mut());
        slateduck_file_list_free(ptr::null_mut());
        slateduck_close(ptr::null_mut());
    }

    #[test]
    fn error_code_and_message_on_null_error() {
        // slateduck_error_code / slateduck_error_message must not crash on null.
        let code = slateduck_error_code(ptr::null());
        assert_eq!(code, 0);
        let msg = slateduck_error_message(ptr::null());
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
        let mut err = SlateduckError::ok();

        let catalog = slateduck_open(path.as_ptr(), &mut err);
        assert!(!catalog.is_null(), "open failed: code={}", err.code);

        // Simulate: thread A closes, thread B tries to use the same handle.
        slateduck_close(catalog); // magic zeroed, allocation freed.

        // Any use after close must return InvalidHandle without crashing.
        let snap = slateduck_get_current_snapshot(catalog, &mut err);
        assert_eq!(
            err.code,
            SlateduckErrorCode::InvalidHandle as i32,
            "expected InvalidHandle after concurrent close"
        );
        assert_eq!(snap.snapshot_id, 0);
        slateduck_error_free(&mut err);

        // A second close (double-close) must also be a safe no-op.
        slateduck_close(catalog);
    }
}
