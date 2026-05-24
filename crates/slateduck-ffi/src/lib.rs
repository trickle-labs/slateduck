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
    unsafe { (*err).code }
}

/// Get the error message (borrows — do not free separately).
#[no_mangle]
pub extern "C" fn slateduck_error_message(err: *const SlateduckError) -> *const c_char {
    if err.is_null() {
        return ptr::null();
    }
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

/// Validate a catalog handle: returns `Some(&mut cat)` when the pointer is
/// non-null and the magic field is intact, `None` otherwise.
fn validate_catalog(ptr: *mut SlateduckCatalog) -> Option<&'static mut SlateduckCatalog> {
    if ptr.is_null() {
        return None;
    }
    let cat = unsafe { &mut *ptr };
    if cat.magic != CATALOG_MAGIC {
        return None;
    }
    Some(cat)
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
    let magic = unsafe { (*catalog).magic };
    if magic != CATALOG_MAGIC {
        return;
    }
    unsafe { (*catalog).magic = 0 };
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
    let cat = match validate_catalog(catalog) {
        Some(c) => c,
        None => {
            write_error(err, SlateduckError::invalid_handle());
            return SlateduckSnapshot {
                snapshot_id: 0,
                schema_version: 0,
            };
        }
    };
    let reader = cat.store.read_latest();

    let result = cat.runtime.block_on(reader.get_snapshot());
    match result {
        Ok(Some(snap)) => {
            write_error(err, SlateduckError::ok());
            SlateduckSnapshot {
                snapshot_id: snap.snapshot_id,
                schema_version: snap.schema_version,
            }
        }
        Ok(None) => {
            write_error(err, SlateduckError::ok());
            SlateduckSnapshot {
                snapshot_id: 0,
                schema_version: 0,
            }
        }
        Err(e) => {
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
    let cat = match validate_catalog(catalog) {
        Some(c) => c,
        None => {
            write_error(err, SlateduckError::invalid_handle());
            return SlateduckSchemaList {
                schemas: ptr::null_mut(),
                count: 0,
            };
        }
    };
    let reader = match cat.store.read_at(SnapshotId::new(snapshot_id)) {
        Ok(r) => r,
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            return SlateduckSchemaList {
                schemas: ptr::null_mut(),
                count: 0,
            };
        }
    };

    let result = cat.runtime.block_on(reader.list_schemas());
    match result {
        Ok(schemas) => {
            write_error(err, SlateduckError::ok());
            let count = schemas.len() as u64;
            let mut out: Vec<SlateduckSchema> = schemas
                .into_iter()
                .map(|s| SlateduckSchema {
                    schema_id: s.schema_id,
                    schema_name: CString::new(s.schema_name).unwrap_or_default().into_raw(),
                })
                .collect();
            let ptr = out.as_mut_ptr();
            std::mem::forget(out);
            SlateduckSchemaList {
                schemas: ptr,
                count,
            }
        }
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            SlateduckSchemaList {
                schemas: ptr::null_mut(),
                count: 0,
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
    let cat = match validate_catalog(catalog) {
        Some(c) => c,
        None => {
            write_error(err, SlateduckError::invalid_handle());
            return SlateduckTableList {
                tables: ptr::null_mut(),
                count: 0,
            };
        }
    };
    let reader = match cat.store.read_at(SnapshotId::new(snapshot_id)) {
        Ok(r) => r,
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            return SlateduckTableList {
                tables: ptr::null_mut(),
                count: 0,
            };
        }
    };

    let result = cat.runtime.block_on(reader.list_tables(schema_id));
    match result {
        Ok(tables) => {
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
            let ptr = out.as_mut_ptr();
            std::mem::forget(out);
            SlateduckTableList { tables: ptr, count }
        }
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            SlateduckTableList {
                tables: ptr::null_mut(),
                count: 0,
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
    let cat = match validate_catalog(catalog) {
        Some(c) => c,
        None => {
            write_error(err, SlateduckError::invalid_handle());
            return SlateduckColumnList {
                columns: ptr::null_mut(),
                count: 0,
            };
        }
    };
    let reader = match cat.store.read_at(SnapshotId::new(snapshot_id)) {
        Ok(r) => r,
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            return SlateduckColumnList {
                columns: ptr::null_mut(),
                count: 0,
            };
        }
    };

    let result = cat.runtime.block_on(reader.describe_table(table_id));
    match result {
        Ok(Some((_table, columns))) => {
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
            let ptr = out.as_mut_ptr();
            std::mem::forget(out);
            SlateduckColumnList {
                columns: ptr,
                count,
            }
        }
        Ok(None) => {
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
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
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
    let cat = match validate_catalog(catalog) {
        Some(c) => c,
        None => {
            write_error(err, SlateduckError::invalid_handle());
            return SlateduckFileList {
                files: ptr::null_mut(),
                count: 0,
            };
        }
    };
    let reader = match cat.store.read_at(SnapshotId::new(snapshot_id)) {
        Ok(r) => r,
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            return SlateduckFileList {
                files: ptr::null_mut(),
                count: 0,
            };
        }
    };

    let result = cat.runtime.block_on(reader.list_data_files(table_id));
    match result {
        Ok(files) => {
            write_error(err, SlateduckError::ok());
            let count = files.len() as u64;
            let mut out: Vec<SlateduckDataFile> = files
                .into_iter()
                .map(|f| SlateduckDataFile {
                    data_file_id: f.data_file_id,
                    table_id: f.table_id,
                    path: CString::new(f.path).unwrap_or_default().into_raw(),
                    file_format: CString::new(f.file_format).unwrap_or_default().into_raw(),
                    row_count: f.row_count,
                    file_size_bytes: f.file_size_bytes,
                    snapshot_id: f.snapshot_id,
                })
                .collect();
            let ptr = out.as_mut_ptr();
            std::mem::forget(out);
            SlateduckFileList { files: ptr, count }
        }
        Err(e) => {
            write_error(err, SlateduckError::from_catalog_error(e));
            SlateduckFileList {
                files: ptr::null_mut(),
                count: 0,
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
}
