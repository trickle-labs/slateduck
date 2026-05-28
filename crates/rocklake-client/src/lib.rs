//! RockLake Client: idiomatic async Rust API for the RockLake catalog.
//!
//! `rocklake-client` wraps `rocklake-catalog`'s `CatalogStore` with an
//! ergonomic, async-first interface that is independent of both DuckDB and
//! the C ABI.  It is the recommended entry point for:
//!
//! - Rust microservices
//! - DataFusion integrations
//! - The sync blocking wrapper used by C extension threads and Python
//!
//! # Quick start
//!
//! ```no_run
//! # tokio::runtime::Runtime::new().unwrap().block_on(async {
//! use rocklake_client::{CatalogClient, CatalogClientBuilder};
//!
//! let client = CatalogClientBuilder::new("file:///tmp/my-catalog")
//!     .build()
//!     .await
//!     .unwrap();
//!
//! let snapshot = client.snapshot_id().await.unwrap();
//! println!("current snapshot: {snapshot}");
//!
//! let schemas = client.list_schemas(snapshot).await.unwrap();
//! println!("schemas: {schemas:?}");
//!
//! client.close().await;
//! # });
//! ```

use std::sync::Arc;

use object_store::local::LocalFileSystem;
use object_store::path::Path as ObjectPath;
use thiserror::Error;

use rocklake_catalog::{CatalogError, CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;

// ─── Error type ────────────────────────────────────────────────────────────

/// Errors returned by `rocklake-client`.
#[derive(Debug, Error)]
pub enum ClientError {
    /// Underlying catalog operation failed.
    #[error("catalog error: {0}")]
    Catalog(#[from] CatalogError),

    /// Bad configuration (e.g., unparseable URI).
    #[error("configuration error: {0}")]
    Config(String),
}

/// Shorthand result type.
pub type ClientResult<T> = Result<T, ClientError>;

// ─── Schema / Table / DataFile value types ─────────────────────────────────

/// A catalog schema returned by [`CatalogClient::list_schemas`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    /// Opaque schema identifier.
    pub schema_id: u64,
    /// Human-readable schema name.
    pub schema_name: String,
}

/// A catalog table returned by [`CatalogClient::list_tables`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// Opaque table identifier.
    pub table_id: u64,
    /// Schema this table belongs to.
    pub schema_id: u64,
    /// Human-readable table name.
    pub table_name: String,
}

/// A data file returned by [`CatalogClient::list_data_files`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataFile {
    /// Opaque data file identifier.
    pub data_file_id: u64,
    /// Table this file belongs to.
    pub table_id: u64,
    /// Object-store path or URL.
    pub path: String,
    /// File format (`"parquet"`, `"csv"`, …).
    pub file_format: String,
    /// Number of rows in the file.
    pub row_count: u64,
    /// Total size in bytes.
    pub file_size_bytes: u64,
    /// Snapshot at which this file was first visible.
    pub snapshot_id: u64,
}

/// A column descriptor returned by [`CatalogClient::get_table`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// Opaque column identifier.
    pub column_id: u64,
    /// Table this column belongs to.
    pub table_id: u64,
    /// Column name.
    pub column_name: String,
    /// SQL data type string.
    pub data_type: String,
    /// Zero-based ordinal position.
    pub column_index: u64,
    /// Whether `NULL` values are permitted.
    pub is_nullable: bool,
}

// ─── CatalogClientBuilder ──────────────────────────────────────────────────

/// Builder for [`CatalogClient`].
///
/// # Examples
///
/// ```no_run
/// # tokio::runtime::Runtime::new().unwrap().block_on(async {
/// use rocklake_client::CatalogClientBuilder;
///
/// let client = CatalogClientBuilder::new("file:///tmp/demo")
///     .build()
///     .await
///     .unwrap();
/// client.close().await;
/// # });
/// ```
pub struct CatalogClientBuilder {
    uri: String,
}

impl CatalogClientBuilder {
    /// Create a new builder with the given catalog URI.
    ///
    /// Supported URI schemes:
    /// - `file:///path/to/catalog` — local filesystem
    /// - Raw local path (no scheme) — also treated as local filesystem
    pub fn new(uri: impl Into<String>) -> Self {
        Self { uri: uri.into() }
    }

    /// Build and open a [`CatalogClient`].
    ///
    /// Connects to the catalog storage backend and returns a ready-to-use
    /// client.  Fails if the URI scheme is unsupported or if the underlying
    /// store cannot be opened.
    pub async fn build(self) -> ClientResult<CatalogClient> {
        let path = self
            .uri
            .strip_prefix("file://")
            .unwrap_or(&self.uri)
            .to_owned();

        let object_store: Arc<dyn object_store::ObjectStore> =
            Arc::new(LocalFileSystem::new_with_prefix(&path).map_err(|e| {
                ClientError::Config(format!("cannot open local store at {path}: {e}"))
            })?);

        let opts = OpenOptions {
            object_store,
            path: ObjectPath::from("catalog"),
            encryption: None,
        };

        let store = CatalogStore::open(opts).await?;
        Ok(CatalogClient {
            store: Arc::new(tokio::sync::Mutex::new(Some(store))),
        })
    }
}

// ─── CatalogClient ─────────────────────────────────────────────────────────

/// Async-first client for RockLake catalog operations.
///
/// All methods are `async` and cancel-safe.  To use from synchronous code
/// (e.g., C extension threads) use [`CatalogClientSync`].
///
/// # Thread safety
///
/// `CatalogClient` is `Send + Sync` and may be shared across tasks via
/// `Arc<CatalogClient>`.  Internally the `CatalogStore` is protected by a
/// `tokio::sync::Mutex`.
pub struct CatalogClient {
    store: Arc<tokio::sync::Mutex<Option<CatalogStore>>>,
}

impl CatalogClient {
    /// Return the current (latest committed) snapshot ID.
    ///
    /// Returns `0` when the catalog has no snapshots yet.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// use rocklake_client::CatalogClientBuilder;
    /// let client = CatalogClientBuilder::new("file:///tmp/demo").build().await.unwrap();
    /// let snap = client.snapshot_id().await.unwrap();
    /// assert_eq!(snap, 0); // fresh catalog
    /// client.close().await;
    /// # });
    /// ```
    pub async fn snapshot_id(&self) -> ClientResult<u64> {
        let guard = self.store.lock().await;
        let store = guard
            .as_ref()
            .ok_or(ClientError::Config("catalog has been closed".to_owned()))?;
        let reader = store.read_latest();
        let snap = reader.get_snapshot().await?;
        Ok(snap.map(|s| s.snapshot_id).unwrap_or(0))
    }

    /// List all schemas visible at `snapshot_id`.
    ///
    /// Pass `0` to read the latest committed snapshot.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// use rocklake_client::CatalogClientBuilder;
    /// let client = CatalogClientBuilder::new("file:///tmp/demo").build().await.unwrap();
    /// let schemas = client.list_schemas(0).await.unwrap();
    /// assert!(schemas.is_empty());
    /// client.close().await;
    /// # });
    /// ```
    pub async fn list_schemas(&self, snapshot_id: u64) -> ClientResult<Vec<Schema>> {
        let guard = self.store.lock().await;
        let store = guard
            .as_ref()
            .ok_or(ClientError::Config("catalog has been closed".to_owned()))?;
        let reader = store.read_at(SnapshotId::new(snapshot_id))?;
        let rows = reader.list_schemas().await?;
        Ok(rows
            .into_iter()
            .map(|r| Schema {
                schema_id: r.schema_id,
                schema_name: r.schema_name,
            })
            .collect())
    }

    /// List all tables in `schema_id` visible at `snapshot_id`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// use rocklake_client::CatalogClientBuilder;
    /// let client = CatalogClientBuilder::new("file:///tmp/demo").build().await.unwrap();
    /// let tables = client.list_tables(1, 0).await.unwrap();
    /// assert!(tables.is_empty());
    /// client.close().await;
    /// # });
    /// ```
    pub async fn list_tables(&self, schema_id: u64, snapshot_id: u64) -> ClientResult<Vec<Table>> {
        let guard = self.store.lock().await;
        let store = guard
            .as_ref()
            .ok_or(ClientError::Config("catalog has been closed".to_owned()))?;
        let reader = store.read_at(SnapshotId::new(snapshot_id))?;
        let rows = reader.list_tables(schema_id).await?;
        Ok(rows
            .into_iter()
            .map(|r| Table {
                table_id: r.table_id,
                schema_id: r.schema_id,
                table_name: r.table_name,
            })
            .collect())
    }

    /// Describe the columns of `table_id` at `snapshot_id`.
    ///
    /// Returns `None` when no table with that ID exists at the given snapshot.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// use rocklake_client::CatalogClientBuilder;
    /// let client = CatalogClientBuilder::new("file:///tmp/demo").build().await.unwrap();
    /// let cols = client.get_table(999, 0).await.unwrap();
    /// assert!(cols.is_none());
    /// client.close().await;
    /// # });
    /// ```
    pub async fn get_table(
        &self,
        table_id: u64,
        snapshot_id: u64,
    ) -> ClientResult<Option<Vec<Column>>> {
        let guard = self.store.lock().await;
        let store = guard
            .as_ref()
            .ok_or(ClientError::Config("catalog has been closed".to_owned()))?;
        let reader = store.read_at(SnapshotId::new(snapshot_id))?;
        let result = reader.describe_table(table_id).await?;
        Ok(result.map(|(_table, cols)| {
            cols.into_iter()
                .map(|c| Column {
                    column_id: c.column_id,
                    table_id: c.table_id,
                    column_name: c.column_name,
                    data_type: c.data_type,
                    column_index: c.column_index,
                    is_nullable: c.is_nullable,
                })
                .collect()
        }))
    }

    /// List data files for `table_id` visible at `snapshot_id`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// use rocklake_client::CatalogClientBuilder;
    /// let client = CatalogClientBuilder::new("file:///tmp/demo").build().await.unwrap();
    /// let files = client.list_data_files(1, 0).await.unwrap();
    /// assert!(files.is_empty());
    /// client.close().await;
    /// # });
    /// ```
    pub async fn list_data_files(
        &self,
        table_id: u64,
        snapshot_id: u64,
    ) -> ClientResult<Vec<DataFile>> {
        let guard = self.store.lock().await;
        let store = guard
            .as_ref()
            .ok_or(ClientError::Config("catalog has been closed".to_owned()))?;
        let reader = store.read_at(SnapshotId::new(snapshot_id))?;
        let rows = reader.list_data_files(table_id).await?;
        Ok(rows
            .into_iter()
            .map(|f| DataFile {
                data_file_id: f.data_file_id,
                table_id: f.table_id,
                path: f.path,
                file_format: f.file_format,
                row_count: f.record_count,
                file_size_bytes: f.file_size_bytes,
                snapshot_id: f.begin_snapshot.unwrap_or(0),
            })
            .collect())
    }

    /// Close the catalog and release all resources.
    ///
    /// After this call all methods return an error.  It is not an error to
    /// call `close()` more than once.
    pub async fn close(self) {
        let mut guard = self.store.lock().await;
        if let Some(store) = guard.take() {
            let _ = store.close().await;
        }
    }
}

// ─── CatalogClientSync ─────────────────────────────────────────────────────

/// Synchronous (blocking) wrapper around [`CatalogClient`].
///
/// Use this from contexts that cannot use `async` Rust:
/// - C extension threads
/// - Python GIL-holding code (before releasing the GIL)
/// - FFI callers outside a Tokio runtime
///
/// # Examples
///
/// ```no_run
/// use rocklake_client::CatalogClientSync;
///
/// let client = CatalogClientSync::open("file:///tmp/demo").unwrap();
/// let snap = client.snapshot_id().unwrap();
/// assert_eq!(snap, 0);
/// client.close();
/// ```
pub struct CatalogClientSync {
    runtime: tokio::runtime::Runtime,
    inner: CatalogClient,
}

impl CatalogClientSync {
    /// Open a catalog synchronously.
    pub fn open(uri: impl Into<String>) -> ClientResult<Self> {
        let runtime = tokio::runtime::Runtime::new()
            .map_err(|e| ClientError::Config(format!("failed to create Tokio runtime: {e}")))?;
        let inner = runtime.block_on(CatalogClientBuilder::new(uri).build())?;
        Ok(Self { runtime, inner })
    }

    /// Return the current snapshot ID.
    pub fn snapshot_id(&self) -> ClientResult<u64> {
        self.runtime.block_on(self.inner.snapshot_id())
    }

    /// List schemas at `snapshot_id`.
    pub fn list_schemas(&self, snapshot_id: u64) -> ClientResult<Vec<Schema>> {
        self.runtime.block_on(self.inner.list_schemas(snapshot_id))
    }

    /// List tables in `schema_id` at `snapshot_id`.
    pub fn list_tables(&self, schema_id: u64, snapshot_id: u64) -> ClientResult<Vec<Table>> {
        self.runtime
            .block_on(self.inner.list_tables(schema_id, snapshot_id))
    }

    /// Describe columns of `table_id` at `snapshot_id`.
    pub fn get_table(&self, table_id: u64, snapshot_id: u64) -> ClientResult<Option<Vec<Column>>> {
        self.runtime
            .block_on(self.inner.get_table(table_id, snapshot_id))
    }

    /// List data files for `table_id` at `snapshot_id`.
    pub fn list_data_files(&self, table_id: u64, snapshot_id: u64) -> ClientResult<Vec<DataFile>> {
        self.runtime
            .block_on(self.inner.list_data_files(table_id, snapshot_id))
    }

    /// Close the catalog.
    pub fn close(self) {
        self.runtime.block_on(async { self.inner.close().await });
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn builder_open_close() {
        let dir = tempfile::TempDir::new().unwrap();
        let uri = format!("file://{}", dir.path().to_str().unwrap());
        let client = CatalogClientBuilder::new(&uri).build().await.unwrap();
        let snap = client.snapshot_id().await.unwrap();
        assert_eq!(snap, 0, "fresh catalog has no snapshots");
        client.close().await;
    }

    #[tokio::test]
    async fn list_schemas_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let uri = format!("file://{}", dir.path().to_str().unwrap());
        let client = CatalogClientBuilder::new(uri).build().await.unwrap();
        let schemas = client.list_schemas(0).await.unwrap();
        assert!(schemas.is_empty(), "fresh catalog has no schemas");
        client.close().await;
    }

    #[tokio::test]
    async fn list_tables_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let uri = format!("file://{}", dir.path().to_str().unwrap());
        let client = CatalogClientBuilder::new(uri).build().await.unwrap();
        let tables = client.list_tables(1, 0).await.unwrap();
        assert!(tables.is_empty(), "fresh catalog has no tables");
        client.close().await;
    }

    #[tokio::test]
    async fn get_table_not_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let uri = format!("file://{}", dir.path().to_str().unwrap());
        let client = CatalogClientBuilder::new(uri).build().await.unwrap();
        let cols = client.get_table(999, 0).await.unwrap();
        assert!(cols.is_none(), "non-existent table returns None");
        client.close().await;
    }

    #[tokio::test]
    async fn list_data_files_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let uri = format!("file://{}", dir.path().to_str().unwrap());
        let client = CatalogClientBuilder::new(uri).build().await.unwrap();
        let files = client.list_data_files(1, 0).await.unwrap();
        assert!(files.is_empty(), "fresh catalog has no data files");
        client.close().await;
    }

    #[test]
    fn sync_client_open_close() {
        let dir = tempfile::TempDir::new().unwrap();
        let uri = format!("file://{}", dir.path().to_str().unwrap());
        let client = CatalogClientSync::open(&uri).unwrap();
        assert_eq!(client.snapshot_id().unwrap(), 0);
        client.close();
    }

    #[test]
    fn sync_client_list_schemas_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let uri = format!("file://{}", dir.path().to_str().unwrap());
        let client = CatalogClientSync::open(uri).unwrap();
        let schemas = client.list_schemas(0).unwrap();
        assert!(schemas.is_empty());
        client.close();
    }

    #[test]
    fn sync_raw_path_without_file_scheme() {
        let dir = tempfile::TempDir::new().unwrap();
        let client = CatalogClientSync::open(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(client.snapshot_id().unwrap(), 0);
        client.close();
    }
}
