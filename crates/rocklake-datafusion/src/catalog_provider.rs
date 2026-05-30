//! DataFusion CatalogProvider implementation backed by RockLake.

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
    /// Construct the bridge, building the Tokio runtime and spawning the
    /// worker thread.  Both operations are now fallible and return
    /// `DataFusionError` on failure instead of panicking.
    fn new() -> Result<Arc<Self>, DataFusionError> {
        Self::with_queue_depth(256)
    }

    /// Construct the bridge with a configurable channel capacity.
    ///
    /// `queue_depth` is the number of tasks that can be queued before
    /// `run_sync()` blocks the caller.  The default (used by `new()`) is 256.
    /// Increase this value if you observe backpressure under high concurrency.
    fn with_queue_depth(queue_depth: usize) -> Result<Arc<Self>, DataFusionError> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| DataFusionError::External(Box::new(e)))?;
        let (sender, receiver) = std::sync::mpsc::sync_channel::<AsyncTask>(queue_depth);
        std::thread::Builder::new()
            .name("rocklake-df-bridge".to_string())
            .spawn(move || {
                while let Ok(task) = receiver.recv() {
                    task(&rt);
                }
                // Sender dropped — exit cleanly.
            })
            .map_err(|e| {
                DataFusionError::External(Box::new(std::io::Error::other(format!(
                    "datafusion async bridge thread spawn failed: {e}"
                ))))
            })?;
        Ok(Arc::new(Self { sender }))
    }

    /// Submit an async closure to the persistent background thread and block
    /// the calling thread until the result is ready.  Returns `Err` if the
    /// bridge channel is disconnected (thread has exited).
    fn run_sync<F, Fut, R>(&self, f: F) -> Result<R, DataFusionError>
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
        self.sender.send(task).map_err(|_| {
            DataFusionError::External(Box::new(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "datafusion async bridge thread disconnected",
            )))
        })?;
        result_rx.recv().map_err(|_| {
            DataFusionError::External(Box::new(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "datafusion async bridge task panicked or disconnected",
            )))
        })
    }

    /// Test helper: create a bridge with an immediately-disconnected channel.
    /// Any call to `run_sync()` on this bridge will return `Err`.
    #[cfg(test)]
    fn new_disconnected() -> Arc<Self> {
        let (sender, _receiver_dropped) = std::sync::mpsc::sync_channel::<AsyncTask>(1);
        // _receiver_dropped is immediately dropped, disconnecting the channel.
        Arc::new(Self { sender })
    }
}

#[cfg(test)]
mod bridge_tests {
    use super::*;

    /// `run_sync()` returns `Err` — not panic — when the worker thread is gone.
    #[test]
    fn bridge_run_sync_returns_err_on_disconnected_channel() {
        let bridge = AsyncBridge::new_disconnected();
        let result: Result<i32, _> = bridge.run_sync(|| async { 42_i32 });
        assert!(
            result.is_err(),
            "run_sync on disconnected channel must return Err, not panic"
        );
    }
}

/// A DataFusion CatalogProvider backed by RockLake's CatalogStore.
/// Provides schema and table discovery from a RockLake catalog.
pub struct RockLakeCatalogProvider {
    store: Arc<RwLock<CatalogStore>>,
    snapshot_id: Option<SnapshotId>,
    /// F-14: stored async bridge capturing the construction-time runtime handle.
    bridge: Arc<AsyncBridge>,
    /// F-15: root path of the object store for resolving data file URLs.
    /// When set, `RockLakeTableProvider::scan()` can read real Parquet files.
    data_root: Option<String>,
}

impl std::fmt::Debug for RockLakeCatalogProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RockLakeCatalogProvider")
            .field("snapshot_id", &self.snapshot_id)
            .finish_non_exhaustive()
    }
}

impl RockLakeCatalogProvider {
    /// Create a new provider from an existing CatalogStore.
    pub fn new(
        store: CatalogStore,
        snapshot_id: Option<SnapshotId>,
    ) -> Result<Self, DataFusionError> {
        Self::new_with_queue_depth(store, snapshot_id, 256)
    }

    /// Create a new provider with a configurable AsyncBridge channel capacity.
    ///
    /// `queue_depth` controls how many concurrent DataFusion queries can be
    /// queued before callers block.  The default (`new()`) is 256.
    pub fn new_with_queue_depth(
        store: CatalogStore,
        snapshot_id: Option<SnapshotId>,
        queue_depth: usize,
    ) -> Result<Self, DataFusionError> {
        Ok(Self {
            store: Arc::new(RwLock::new(store)),
            snapshot_id,
            bridge: AsyncBridge::with_queue_depth(queue_depth)?,
            data_root: None,
        })
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
            bridge: AsyncBridge::new()?,
            data_root,
        })
    }

    /// Open a catalog at the given path and create a provider.
    ///
    /// `data_root` is read from the `data_path` catalog metadata key rather
    /// than from the `ObjectStore` Display string, which was brittle (N-05).
    /// Set the `data_path` metadata key (e.g. via `writer.set_metadata`) to
    /// enable real Parquet scans against a local filesystem catalog.
    pub async fn open(
        object_store: Arc<dyn object_store::ObjectStore>,
        path: ObjectPath,
        snapshot_id: Option<SnapshotId>,
    ) -> Result<Self, DataFusionError> {
        let opts = OpenOptions {
            object_store,
            path,
            encryption: None,
        };
        let store = CatalogStore::open(opts)
            .await
            .map_err(|e| DataFusionError::External(Box::new(e)))?;

        // Resolve data_root from catalog metadata (stable; no Display-string parsing).
        let data_root = {
            let reader = store.read_latest();
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
            store: Arc::new(RwLock::new(store)),
            snapshot_id,
            bridge: AsyncBridge::new()?,
            data_root,
        })
    }
}

impl CatalogProvider for RockLakeCatalogProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema_names(&self) -> Vec<String> {
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;
        let bridge = self.bridge.clone();
        match bridge.run_sync(move || async move {
            let store = store.read().await;
            let reader = match snapshot_id {
                Some(sid) => match store.read_at(sid) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("schema_names: read_at({sid}) failed: {e}");
                        return vec![];
                    }
                },
                None => store.read_latest(),
            };
            match reader.list_schemas().await {
                Ok(schemas) => schemas.into_iter().map(|s| s.schema_name).collect(),
                Err(e) => {
                    tracing::error!("schema_names: list_schemas failed: {e}");
                    vec![]
                }
            }
        }) {
            Ok(names) => names,
            Err(e) => {
                tracing::error!("schema_names: async bridge failure: {e}");
                vec![]
            }
        }
    }

    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        let schema_name = name.to_string();
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;
        let bridge = self.bridge.clone();
        let data_root = self.data_root.clone();

        Some(Arc::new(RockLakeSchemaProvider {
            store,
            schema_name,
            snapshot_id,
            bridge,
            data_root,
        }))
    }
}

/// A DataFusion SchemaProvider backed by RockLake.
pub struct RockLakeSchemaProvider {
    store: Arc<RwLock<CatalogStore>>,
    schema_name: String,
    snapshot_id: Option<SnapshotId>,
    /// F-14: shared async bridge from the parent CatalogProvider.
    bridge: Arc<AsyncBridge>,
    /// F-15: inherited data root for resolving Parquet file paths.
    data_root: Option<String>,
}

impl std::fmt::Debug for RockLakeSchemaProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RockLakeSchemaProvider")
            .field("schema_name", &self.schema_name)
            .field("snapshot_id", &self.snapshot_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SchemaProvider for RockLakeSchemaProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_names(&self) -> Vec<String> {
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;
        let schema_name = self.schema_name.clone();
        let bridge = self.bridge.clone();
        match bridge.run_sync(move || async move {
            let store = store.read().await;
            let reader = match snapshot_id {
                Some(sid) => match store.read_at(sid) {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::error!("table_names: read_at({sid}) failed: {e}");
                        return vec![];
                    }
                },
                None => store.read_latest(),
            };
            let schemas = match reader.list_schemas().await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("table_names: list_schemas failed: {e}");
                    return vec![];
                }
            };
            let schema = schemas.iter().find(|s| s.schema_name == schema_name);
            match schema {
                Some(s) => match reader.list_tables(s.schema_id).await {
                    Ok(tables) => tables.into_iter().map(|t| t.table_name).collect(),
                    Err(e) => {
                        tracing::error!("table_names: list_tables failed: {e}");
                        vec![]
                    }
                },
                None => vec![],
            }
        }) {
            Ok(names) => names,
            Err(e) => {
                tracing::error!("table_names: async bridge failure: {e}");
                vec![]
            }
        }
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
                // Propagate catalog errors rather than silently returning empty results.
                let data_files = reader
                    .list_data_files(table.table_id)
                    .await
                    .map_err(|e| DataFusionError::External(Box::new(e)))?;

                let table_provider = RockLakeTableProvider::new(
                    table.table_name.clone(),
                    table.table_id,
                    columns,
                    data_files,
                    self.data_root.clone(),
                )?;
                Ok(Some(Arc::new(table_provider)))
            }
        }
    }

    fn table_exist(&self, name: &str) -> bool {
        self.table_names().contains(&name.to_string())
    }
}

/// A DataFusion TableProvider that exposes table schema from RockLake catalog
/// and, when data files are present, reads real Parquet data (F-15).
#[derive(Debug)]
pub struct RockLakeTableProvider {
    schema: datafusion::arrow::datatypes::SchemaRef,
    /// F-15: data files registered in the catalog at the active snapshot.
    data_files: Vec<DataFileRow>,
    /// F-15: root path of the object store for constructing absolute file URLs.
    data_root: Option<String>,
}

impl RockLakeTableProvider {
    fn new(
        _table_name: String,
        _table_id: u64,
        columns: Vec<rocklake_core::rows::ColumnRow>,
        data_files: Vec<DataFileRow>,
        data_root: Option<String>,
    ) -> datafusion::error::Result<Self> {
        use datafusion::arrow::datatypes::{Field, Schema};

        let fields = columns
            .iter()
            .map(|col| {
                let dt = Self::map_data_type(&col.data_type)?;
                Ok(Field::new(&col.column_name, dt, col.is_nullable))
            })
            .collect::<datafusion::error::Result<Vec<Field>>>()?;

        let schema = Arc::new(Schema::new(fields));

        Ok(Self {
            schema,
            data_files,
            data_root,
        })
    }

    /// Map a DuckLake column type string to the corresponding Arrow DataType.
    ///
    /// Uses `DuckLakeType::parse()` from `rocklake-core` for all v1.0 scalar
    /// types.  Nested types (list, struct, map) and geometry/variant return
    /// `DataFusionError::NotImplemented` rather than silently falling back to
    /// UTF-8.
    fn map_data_type(
        type_str: &str,
    ) -> datafusion::error::Result<datafusion::arrow::datatypes::DataType> {
        use datafusion::arrow::datatypes::{DataType, IntervalUnit, TimeUnit};
        use rocklake_core::types::DuckLakeType;
        match DuckLakeType::parse(type_str) {
            DuckLakeType::Integer {
                signed: true,
                width_bits: 8,
            } => Ok(DataType::Int8),
            DuckLakeType::Integer {
                signed: true,
                width_bits: 16,
            } => Ok(DataType::Int16),
            DuckLakeType::Integer {
                signed: true,
                width_bits: 32,
            } => Ok(DataType::Int32),
            DuckLakeType::Integer {
                signed: true,
                width_bits: 64,
            } => Ok(DataType::Int64),
            DuckLakeType::Integer {
                signed: true,
                width_bits: 128,
            } => {
                // HUGEINT: Arrow has no Int128; represent as Decimal128(38, 0).
                Ok(DataType::Decimal128(38, 0))
            }
            DuckLakeType::Integer {
                signed: false,
                width_bits: 8,
            } => Ok(DataType::UInt8),
            DuckLakeType::Integer {
                signed: false,
                width_bits: 16,
            } => Ok(DataType::UInt16),
            DuckLakeType::Integer {
                signed: false,
                width_bits: 32,
            } => Ok(DataType::UInt32),
            DuckLakeType::Integer {
                signed: false,
                width_bits: 64,
            } => Ok(DataType::UInt64),
            DuckLakeType::Integer {
                signed: false,
                width_bits: 128,
            } => {
                // UHUGEINT: Arrow has no UInt128; Decimal128(38, 0) is the best approximation.
                Ok(DataType::Decimal128(38, 0))
            }
            DuckLakeType::Integer { .. } => Err(DataFusionError::NotImplemented(format!(
                "integer type not supported in Arrow: {type_str}"
            ))),
            DuckLakeType::Decimal { precision, scale } => {
                Ok(DataType::Decimal128(precision, scale as i8))
            }
            DuckLakeType::Float { width_bits: 32 } => Ok(DataType::Float32),
            DuckLakeType::Float { width_bits: 64 } => Ok(DataType::Float64),
            DuckLakeType::Float { .. } => Err(DataFusionError::NotImplemented(format!(
                "float type not supported: {type_str}"
            ))),
            DuckLakeType::Timestamp {
                with_timezone: false,
                precision,
            } => {
                let tu = match precision {
                    0 => TimeUnit::Second,
                    3 => TimeUnit::Millisecond,
                    9 => TimeUnit::Nanosecond,
                    _ => TimeUnit::Microsecond,
                };
                Ok(DataType::Timestamp(tu, None))
            }
            DuckLakeType::Timestamp {
                with_timezone: true,
                precision,
            } => {
                let tu = match precision {
                    0 => TimeUnit::Second,
                    3 => TimeUnit::Millisecond,
                    9 => TimeUnit::Nanosecond,
                    _ => TimeUnit::Microsecond,
                };
                Ok(DataType::Timestamp(tu, Some("UTC".into())))
            }
            DuckLakeType::Date => Ok(DataType::Date32),
            DuckLakeType::Time { .. } => Ok(DataType::Time64(TimeUnit::Microsecond)),
            DuckLakeType::Interval => Ok(DataType::Interval(IntervalUnit::MonthDayNano)),
            DuckLakeType::Varchar => Ok(DataType::Utf8),
            DuckLakeType::Blob => Ok(DataType::Binary),
            DuckLakeType::Boolean => Ok(DataType::Boolean),
            // UUID: fixed-size 16-byte binary per Arrow UUID extension type.
            DuckLakeType::Uuid => Ok(DataType::FixedSizeBinary(16)),
            // JSON: stored as Utf8 with semantic annotation.
            DuckLakeType::Json => Ok(DataType::Utf8),
            DuckLakeType::Variant => Err(DataFusionError::NotImplemented(format!(
                "unsupported DuckLake type: {type_str}"
            ))),
            DuckLakeType::Geometry => Err(DataFusionError::NotImplemented(format!(
                "unsupported DuckLake type: {type_str}"
            ))),
            DuckLakeType::List(_) | DuckLakeType::Struct(_) | DuckLakeType::Map { .. } => {
                Err(DataFusionError::NotImplemented(format!(
                    "nested type not yet supported in Arrow mapping: {type_str}"
                )))
            }
            DuckLakeType::Unknown(t) => Err(DataFusionError::NotImplemented(format!(
                "unknown DuckLake type: {t}"
            ))),
        }
    }
}

#[async_trait]
impl TableProvider for RockLakeTableProvider {
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

        if parquet_files.is_empty() {
            return Ok(Arc::new(EmptyExec::new(self.schema.clone())));
        }

        if self.data_root.is_none() {
            return Err(DataFusionError::Plan(
                "data_root is not available for this object store type; \
                 cannot scan registered Parquet files — ensure the catalog \
                 metadata key 'data_path' is set"
                    .to_string(),
            ));
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
