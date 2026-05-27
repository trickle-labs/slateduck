//! DataFusion CatalogProvider implementation backed by Rocklake.

use async_trait::async_trait;
use datafusion::catalog::{CatalogProvider, SchemaProvider};
use datafusion::common::DataFusionError;
use datafusion::datasource::TableProvider;
use datafusion::logical_expr::TableType;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::prelude::*;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::keys::MetadataScope;
use rocklake_core::mvcc::SnapshotId;
use rocklake_core::rows::DataFileRow;
use std::any::Any;
use std::sync::Arc;
use tokio::sync::RwLock;

/// N-05: Thread-safe bridge between DataFusion's sync trait methods and async
/// catalog I/O.
///
/// A single background OS thread is started at construction time and kept alive
/// for the lifetime of the bridge.  All async work is submitted via a bounded
/// channel and executed on that thread's `current_thread` Tokio runtime.  This
/// avoids the per-call `std::thread::spawn` overhead of the previous design and
/// eliminates the risk of exhausting the OS thread pool under concurrent
/// DataFusion queries.
///
/// When constructed inside an existing Tokio runtime the same architecture is
/// used: a dedicated worker thread is started so that `block_on` is never
/// called from within an async task (which would panic).
#[derive(Debug)]
struct AsyncBridge {
    sender: std::sync::mpsc::SyncSender<AsyncTask>,
}

type AsyncTask = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send>;

impl AsyncBridge {
    fn new() -> Arc<Self> {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<AsyncTask>(64);
        std::thread::Builder::new()
            .name("rocklake-df-bridge".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("datafusion async bridge runtime build failed");
                while let Ok(task) = receiver.recv() {
                    task(&rt);
                }
                // Sender dropped — exit cleanly.
            })
            .expect("datafusion async bridge thread spawn failed");
        Arc::new(Self { sender })
    }

    /// Submit an async closure to the persistent background thread and block
    /// the calling thread until the result is ready.
    fn run_sync<F, Fut, R>(&self, f: F) -> R
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        let (result_tx, result_rx) = std::sync::mpsc::sync_channel::<R>(1);
        let task: AsyncTask = Box::new(move |rt| {
            let result = rt.block_on(f());
            let _ = result_tx.send(result);
        });
        self.sender
            .send(task)
            .expect("datafusion async bridge thread disconnected");
        result_rx
            .recv()
            .expect("datafusion async bridge task panicked or disconnected")
    }
}

/// A DataFusion CatalogProvider backed by Rocklake's CatalogStore.
/// Provides schema and table discovery from a Rocklake catalog.
pub struct RocklakeCatalogProvider {
    store: Arc<RwLock<CatalogStore>>,
    snapshot_id: Option<SnapshotId>,
    /// F-14: stored async bridge capturing the construction-time runtime handle.
    bridge: Arc<AsyncBridge>,
    /// F-15: root path of the object store for resolving data file URLs.
    /// When set, `RocklakeTableProvider::scan()` can read real Parquet files.
    data_root: Option<String>,
}

impl std::fmt::Debug for RocklakeCatalogProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocklakeCatalogProvider")
            .field("snapshot_id", &self.snapshot_id)
            .finish_non_exhaustive()
    }
}

impl RocklakeCatalogProvider {
    /// Create a new provider from an existing CatalogStore.
    pub fn new(store: CatalogStore, snapshot_id: Option<SnapshotId>) -> Self {
        Self {
            store: Arc::new(RwLock::new(store)),
            snapshot_id,
            bridge: AsyncBridge::new(),
            data_root: None,
        }
    }

    /// N-02: Create a provider from a pre-opened `CatalogStore`, automatically
    /// resolving `data_root` from the `ducklake_metadata` `data_path` entry.
    ///
    /// This constructor is useful when the catalog has already been opened (e.g.
    /// by the PG-Wire server) and the DataFusion layer should reuse the same
    /// store without re-opening it.  The `data_path` metadata key is read once
    /// at construction time; later metadata changes are not tracked.
    pub async fn from_catalog_store(
        store: Arc<RwLock<CatalogStore>>,
        snapshot_id: Option<SnapshotId>,
    ) -> Result<Self, DataFusionError> {
        let data_root = {
            let store_guard = store.read().await;
            let reader = store_guard.read_latest();
            match reader
                .get_metadata(MetadataScope::Global, 0, "data_path")
                .await
                .map_err(|e| DataFusionError::External(Box::new(e)))?
            {
                Some(row) if !row.value.is_empty() => Some(row.value),
                _ => None,
            }
        };
        Ok(Self {
            store,
            snapshot_id,
            bridge: AsyncBridge::new(),
            data_root,
        })
    }

    /// Open a catalog at the given path and create a provider.
    ///
    /// When `object_store` is a local filesystem with a known prefix,
    /// the provider will resolve data file paths against that prefix, enabling
    /// real Parquet scans via DataFusion (F-15).
    pub async fn open(
        object_store: Arc<dyn object_store::ObjectStore>,
        path: ObjectPath,
        snapshot_id: Option<SnapshotId>,
    ) -> Result<Self, DataFusionError> {
        // Extract local root from the object store's URL if it is a local
        // filesystem.  The `Display` of `LocalFileSystem` yields the root path.
        // object_store 0.11 format: "LocalFileSystem(file:///path/)" (URL form)
        // older format:             "LocalFileSystem(root=/path)"
        let data_root = {
            let display = format!("{object_store}");
            // Strip "LocalFileSystem(" prefix and ")" suffix.
            let inner = display
                .strip_prefix("LocalFileSystem(")
                .and_then(|s| s.strip_suffix(')'));
            inner.and_then(|s| {
                // Strip optional "root=" key from older format.
                let s = s.strip_prefix("root=").unwrap_or(s);
                // Strip "file://" scheme to get a plain OS path.
                let os_path = s.strip_prefix("file://").unwrap_or(s);
                if os_path.starts_with('/') {
                    Some(os_path.trim_end_matches('/').to_string())
                } else {
                    None
                }
            })
        };

        let opts = OpenOptions {
            object_store,
            path,
            encryption: None,
        };
        let store = CatalogStore::open(opts)
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
        Ok(Self {
            store: Arc::new(RwLock::new(store)),
            snapshot_id,
            bridge: AsyncBridge::new(),
            data_root,
        })
    }
}

impl CatalogProvider for RocklakeCatalogProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema_names(&self) -> Vec<String> {
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;
        let bridge = self.bridge.clone();
        bridge.run_sync(move || async move {
            let store = store.read().await;
            let reader = match snapshot_id {
                Some(sid) => match store.read_at(sid) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("read_at failed in schema_names: {e}");
                        return vec![];
                    }
                },
                None => store.read_latest(),
            };
            reader
                .list_schemas()
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.schema_name)
                .collect::<Vec<_>>()
        })
    }

    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        let schema_name = name.to_string();
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;
        let bridge = self.bridge.clone();
        let data_root = self.data_root.clone();

        Some(Arc::new(RocklakeSchemaProvider {
            store,
            schema_name,
            snapshot_id,
            bridge,
            data_root,
        }))
    }
}

/// A DataFusion SchemaProvider backed by Rocklake.
pub struct RocklakeSchemaProvider {
    store: Arc<RwLock<CatalogStore>>,
    schema_name: String,
    snapshot_id: Option<SnapshotId>,
    /// F-14: shared async bridge from the parent CatalogProvider.
    bridge: Arc<AsyncBridge>,
    /// F-15: inherited data root for resolving Parquet file paths.
    data_root: Option<String>,
}

impl std::fmt::Debug for RocklakeSchemaProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RocklakeSchemaProvider")
            .field("schema_name", &self.schema_name)
            .field("snapshot_id", &self.snapshot_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SchemaProvider for RocklakeSchemaProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_names(&self) -> Vec<String> {
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;
        let schema_name = self.schema_name.clone();
        let bridge = self.bridge.clone();
        bridge.run_sync(move || async move {
            let store = store.read().await;
            let reader = match snapshot_id {
                Some(sid) => match store.read_at(sid) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("read_at failed in table_names: {e}");
                        return vec![];
                    }
                },
                None => store.read_latest(),
            };
            let schemas = reader.list_schemas().await.unwrap_or_default();
            let schema = schemas.iter().find(|s| s.schema_name == schema_name);
            match schema {
                Some(s) => reader
                    .list_tables(s.schema_id)
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|t| t.table_name)
                    .collect::<Vec<_>>(),
                None => vec![],
            }
        })
    }

    async fn table(&self, name: &str) -> datafusion::error::Result<Option<Arc<dyn TableProvider>>> {
        let store = self.store.read().await;
        let reader = match self.snapshot_id {
            Some(sid) => store
                .read_at(sid)
                .map_err(|e| DataFusionError::External(Box::new(e)))?,
            None => store.read_latest(),
        };

        let schemas = reader
            .list_schemas()
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
        let schema = schemas.iter().find(|s| s.schema_name == self.schema_name);
        let schema = match schema {
            Some(s) => s,
            None => return Ok(None),
        };

        let tables = reader
            .list_tables(schema.schema_id)
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
        let table = tables.iter().find(|t| t.table_name == name);
        let table = match table {
            Some(t) => t,
            None => return Ok(None),
        };

        let desc = reader
            .describe_table(table.table_id)
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))?;

        match desc {
            None => Ok(None),
            Some((_table_row, columns)) => {
                // F-15: fetch data files for real Parquet scan support.
                let data_files = reader
                    .list_data_files(table.table_id)
                    .await
                    .unwrap_or_default();

                let table_provider = RocklakeTableProvider::new(
                    table.table_name.clone(),
                    table.table_id,
                    columns,
                    data_files,
                    self.data_root.clone(),
                );
                Ok(Some(Arc::new(table_provider)))
            }
        }
    }

    fn table_exist(&self, name: &str) -> bool {
        self.table_names().contains(&name.to_string())
    }
}

/// A DataFusion TableProvider that exposes table schema from Rocklake catalog
/// and, when data files are present, reads real Parquet data (F-15).
#[derive(Debug)]
pub struct RocklakeTableProvider {
    schema: datafusion::arrow::datatypes::SchemaRef,
    /// F-15: data files registered in the catalog at the active snapshot.
    data_files: Vec<DataFileRow>,
    /// F-15: root path of the object store for constructing absolute file URLs.
    data_root: Option<String>,
}

impl RocklakeTableProvider {
    fn new(
        _table_name: String,
        _table_id: u64,
        columns: Vec<rocklake_core::rows::ColumnRow>,
        data_files: Vec<DataFileRow>,
        data_root: Option<String>,
    ) -> Self {
        use datafusion::arrow::datatypes::{Field, Schema};

        let fields: Vec<Field> = columns
            .iter()
            .map(|col| {
                let dt = Self::map_data_type(&col.data_type);
                Field::new(&col.column_name, dt, col.is_nullable)
            })
            .collect();

        let schema = Arc::new(Schema::new(fields));

        Self {
            schema,
            data_files,
            data_root,
        }
    }

    fn map_data_type(type_str: &str) -> datafusion::arrow::datatypes::DataType {
        use datafusion::arrow::datatypes::DataType;
        match type_str.to_uppercase().as_str() {
            "INTEGER" | "INT" | "INT32" => DataType::Int32,
            "BIGINT" | "INT64" | "LONG" => DataType::Int64,
            "SMALLINT" | "INT16" => DataType::Int16,
            "TINYINT" | "INT8" => DataType::Int8,
            "FLOAT" | "FLOAT32" | "REAL" => DataType::Float32,
            "DOUBLE" | "FLOAT64" => DataType::Float64,
            "BOOLEAN" | "BOOL" => DataType::Boolean,
            "VARCHAR" | "TEXT" | "STRING" => DataType::Utf8,
            "BLOB" | "BYTEA" | "BINARY" => DataType::Binary,
            "DATE" => DataType::Date32,
            "TIMESTAMP" => {
                DataType::Timestamp(datafusion::arrow::datatypes::TimeUnit::Microsecond, None)
            }
            _ => DataType::Utf8, // fallback
        }
    }
}

#[async_trait]
impl TableProvider for RocklakeTableProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema(&self) -> datafusion::arrow::datatypes::SchemaRef {
        self.schema.clone()
    }

    fn table_type(&self) -> TableType {
        TableType::Base
    }

    async fn scan(
        &self,
        state: &dyn datafusion::catalog::Session,
        projection: Option<&Vec<usize>>,
        filters: &[Expr],
        limit: Option<usize>,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        use datafusion::physical_plan::empty::EmptyExec;

        // F-15: if there are Parquet data files and a known data root, use the
        // real DataFusion Parquet reader.  Fall back to EmptyExec when either
        // the data root is not set (non-local stores) or no files are registered.
        let parquet_files: Vec<&DataFileRow> = self
            .data_files
            .iter()
            .filter(|f| f.file_format.to_lowercase() == "parquet")
            .collect();

        if parquet_files.is_empty() || self.data_root.is_none() {
            return Ok(Arc::new(EmptyExec::new(self.schema.clone())));
        }

        let root = self.data_root.as_deref().unwrap();

        use datafusion::datasource::file_format::parquet::ParquetFormat;
        use datafusion::datasource::listing::{
            ListingOptions, ListingTable, ListingTableConfig, ListingTableUrl,
        };

        let urls: Result<Vec<ListingTableUrl>, _> = parquet_files
            .iter()
            .map(|f| {
                let abs = format!("{}/{}", root.trim_end_matches('/'), f.path);
                // abs is an absolute OS path starting with '/'; prepend "file://"
                // to get a valid file:// URL (three slashes total).
                ListingTableUrl::parse(format!("file://{abs}"))
            })
            .collect();
        let urls = urls?;

        let file_format = Arc::new(ParquetFormat::default());
        let listing_options = ListingOptions::new(file_format).with_file_extension(".parquet");

        let config = ListingTableConfig::new_with_multi_paths(urls)
            .with_listing_options(listing_options)
            .with_schema(self.schema.clone());

        let listing_table = ListingTable::try_new(config)?;
        listing_table.scan(state, projection, filters, limit).await
    }
}
