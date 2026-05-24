//! DataFusion CatalogProvider implementation backed by SlateDuck.

use async_trait::async_trait;
use datafusion::catalog::{CatalogProvider, SchemaProvider};
use datafusion::common::DataFusionError;
use datafusion::datasource::TableProvider;
use datafusion::logical_expr::TableType;
use datafusion::physical_plan::ExecutionPlan;
use datafusion::prelude::*;
use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogStore, OpenOptions};
use slateduck_core::mvcc::SnapshotId;
use std::any::Any;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A DataFusion CatalogProvider backed by SlateDuck's CatalogStore.
/// Provides schema and table discovery from a SlateDuck catalog.
pub struct SlateDuckCatalogProvider {
    store: Arc<RwLock<CatalogStore>>,
    snapshot_id: Option<SnapshotId>,
}

impl std::fmt::Debug for SlateDuckCatalogProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlateDuckCatalogProvider")
            .field("snapshot_id", &self.snapshot_id)
            .finish_non_exhaustive()
    }
}

impl SlateDuckCatalogProvider {
    /// Create a new provider from an existing CatalogStore.
    pub fn new(store: CatalogStore, snapshot_id: Option<SnapshotId>) -> Self {
        Self {
            store: Arc::new(RwLock::new(store)),
            snapshot_id,
        }
    }

    /// Open a catalog at the given path and create a provider.
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
        Ok(Self::new(store, snapshot_id))
    }
}

impl CatalogProvider for SlateDuckCatalogProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn schema_names(&self) -> Vec<String> {
        // Use blocking runtime to get schemas synchronously
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;
        let handle = tokio::runtime::Handle::try_current();
        match handle {
            Ok(handle) => {
                let result = std::thread::spawn(move || {
                    handle.block_on(async {
                        let store = store.read().await;
                        let reader = match snapshot_id {
                            Some(sid) => match store.read_at(sid).await {
                                Ok(r) => r,
                                Err(_) => return vec![],
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
                })
                .join()
                .unwrap_or_default();
                result
            }
            Err(_) => vec![],
        }
    }

    fn schema(&self, name: &str) -> Option<Arc<dyn SchemaProvider>> {
        let schema_name = name.to_string();
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;

        Some(Arc::new(SlateDuckSchemaProvider {
            store,
            schema_name,
            snapshot_id,
        }))
    }
}

/// A DataFusion SchemaProvider backed by SlateDuck.
pub struct SlateDuckSchemaProvider {
    store: Arc<RwLock<CatalogStore>>,
    schema_name: String,
    snapshot_id: Option<SnapshotId>,
}

impl std::fmt::Debug for SlateDuckSchemaProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlateDuckSchemaProvider")
            .field("schema_name", &self.schema_name)
            .field("snapshot_id", &self.snapshot_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SchemaProvider for SlateDuckSchemaProvider {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn table_names(&self) -> Vec<String> {
        let store = self.store.clone();
        let snapshot_id = self.snapshot_id;
        let schema_name = self.schema_name.clone();
        let handle = tokio::runtime::Handle::try_current();
        match handle {
            Ok(handle) => {
                let result = std::thread::spawn(move || {
                    handle.block_on(async {
                        let store = store.read().await;
                        let reader = match snapshot_id {
                            Some(sid) => match store.read_at(sid).await {
                                Ok(r) => r,
                                Err(_) => return vec![],
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
                })
                .join()
                .unwrap_or_default();
                result
            }
            Err(_) => vec![],
        }
    }

    async fn table(&self, name: &str) -> datafusion::error::Result<Option<Arc<dyn TableProvider>>> {
        let store = self.store.read().await;
        let reader = match self.snapshot_id {
            Some(sid) => store
                .read_at(sid)
                .await
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
                let table_provider = SlateDuckTableProvider::new(
                    table.table_name.clone(),
                    table.table_id,
                    columns,
                    table.data_path.clone(),
                );
                Ok(Some(Arc::new(table_provider)))
            }
        }
    }

    fn table_exist(&self, name: &str) -> bool {
        self.table_names().contains(&name.to_string())
    }
}

/// A minimal DataFusion TableProvider that exposes table schema from SlateDuck catalog.
#[derive(Debug)]
pub struct SlateDuckTableProvider {
    #[allow(dead_code)]
    table_name: String,
    #[allow(dead_code)]
    table_id: u64,
    schema: datafusion::arrow::datatypes::SchemaRef,
    #[allow(dead_code)]
    data_path: Option<String>,
}

impl SlateDuckTableProvider {
    fn new(
        table_name: String,
        table_id: u64,
        columns: Vec<slateduck_core::rows::ColumnRow>,
        data_path: Option<String>,
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
            table_name,
            table_id,
            schema,
            data_path,
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
impl TableProvider for SlateDuckTableProvider {
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
        _state: &dyn datafusion::catalog::Session,
        _projection: Option<&Vec<usize>>,
        _filters: &[Expr],
        _limit: Option<usize>,
    ) -> datafusion::error::Result<Arc<dyn ExecutionPlan>> {
        // Return an empty execution plan - the catalog provides metadata;
        // actual data reading from Parquet files is handled by the query engine
        // connecting directly to the data path.
        use datafusion::physical_plan::empty::EmptyExec;
        Ok(Arc::new(EmptyExec::new(self.schema.clone())))
    }
}
