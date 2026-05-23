//! Multi-writer via catalog partitioning.
//!
//! SlateDB is single-writer per database, and DuckLake is single-writer per catalog.
//! This module implements a pattern of "one SlateDB catalog per dataset" with a thin
//! global registry, exploiting SlateDB's cheap database creation.
//!
//! Architecture:
//! - Global registry catalog: maps logical dataset names to their catalog paths
//! - Each dataset gets its own isolated SlateDB-backed catalog
//! - Writers shard across datasets with no cross-dataset contention
//! - The global registry itself is a SlateDuck catalog, providing a queryable inventory

use object_store::path::Path as ObjectPath;
use slateduck_core::keys;
use slateduck_core::rows::MetadataRow;
use slateduck_core::values;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{CatalogError, CatalogResult};
use crate::store::{CatalogStore, OpenOptions};

/// A dataset entry in the global registry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DatasetEntry {
    /// Logical name of the dataset.
    pub name: String,
    /// Object store path to the dataset's catalog.
    pub catalog_path: String,
    /// Optional description.
    pub description: Option<String>,
    /// Creation timestamp (RFC 3339).
    pub created_at: String,
}

/// The global registry that maps dataset names to their catalog paths.
/// This is itself a SlateDuck catalog, enabling queryable inventory.
pub struct CatalogRegistry {
    store: CatalogStore,
}

impl CatalogRegistry {
    /// Open or create the global registry catalog.
    pub async fn open(opts: OpenOptions) -> CatalogResult<Self> {
        let store = CatalogStore::open(opts).await?;
        Ok(Self { store })
    }

    /// Register a new dataset in the registry.
    pub async fn register_dataset(&mut self, entry: &DatasetEntry) -> CatalogResult<()> {
        let mut writer = self.store.begin_write();

        // Store dataset as metadata under Global scope
        let value =
            serde_json::to_string(entry).map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        let key = keys::key_metadata(
            keys::MetadataScope::Global,
            0,
            &format!("dataset:{}", entry.name),
        );

        let row = MetadataRow {
            key: format!("dataset:{}", entry.name),
            value,
        };

        self.store
            .db()
            .put(&key, values::encode_value(&row))
            .await?;

        writer
            .create_snapshot(
                Some("registry"),
                Some(&format!("register dataset: {}", entry.name)),
            )
            .await?;
        Ok(())
    }

    /// Remove a dataset from the registry.
    pub async fn unregister_dataset(&mut self, name: &str) -> CatalogResult<()> {
        let key = keys::key_metadata(keys::MetadataScope::Global, 0, &format!("dataset:{}", name));

        // Write empty to mark as removed
        let row = MetadataRow {
            key: format!("dataset:{}", name),
            value: String::new(),
        };

        self.store
            .db()
            .put(&key, values::encode_value(&row))
            .await?;

        let mut writer = self.store.begin_write();
        writer
            .create_snapshot(
                Some("registry"),
                Some(&format!("unregister dataset: {}", name)),
            )
            .await?;
        Ok(())
    }

    /// List all registered datasets.
    pub async fn list_datasets(&self) -> CatalogResult<Vec<DatasetEntry>> {
        // Scan all global metadata entries and filter for dataset: prefix.
        // We scan the full metadata tag + Global scope + scope_id prefix
        // since the length-prefixed key encoding prevents direct prefix matching.
        use slateduck_core::tags::TAG_METADATA;
        let mut scan_prefix = Vec::with_capacity(10);
        scan_prefix.push(TAG_METADATA);
        scan_prefix.push(keys::MetadataScope::Global as u8);
        scan_prefix.extend_from_slice(&keys::encode_u64(0));

        let mut datasets = Vec::new();
        let mut iter = self.store.db().scan_prefix(&scan_prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            if let Ok(row) = values::decode_value::<MetadataRow>(&kv.value) {
                if row.key.starts_with("dataset:") && !row.value.is_empty() {
                    if let Ok(entry) = serde_json::from_str::<DatasetEntry>(&row.value) {
                        datasets.push(entry);
                    }
                }
            }
        }
        Ok(datasets)
    }

    /// Get a specific dataset entry by name.
    pub async fn get_dataset(&self, name: &str) -> CatalogResult<Option<DatasetEntry>> {
        let key = keys::key_metadata(keys::MetadataScope::Global, 0, &format!("dataset:{}", name));
        match self.store.db().get(&key).await? {
            None => Ok(None),
            Some(data) => {
                let row: MetadataRow = values::decode_value(&data)?;
                if row.value.is_empty() {
                    return Ok(None);
                }
                let entry: DatasetEntry = serde_json::from_str(&row.value)
                    .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
                Ok(Some(entry))
            }
        }
    }

    /// Close the registry.
    pub async fn close(self) -> CatalogResult<()> {
        self.store.close().await
    }
}

/// A partitioned writer that manages multiple dataset catalogs.
/// Each dataset has its own isolated SlateDB catalog with no cross-dataset contention.
pub struct PartitionedWriter {
    object_store: Arc<dyn object_store::ObjectStore>,
    base_path: ObjectPath,
    catalogs: HashMap<String, CatalogStore>,
}

impl PartitionedWriter {
    /// Create a new partitioned writer.
    pub fn new(object_store: Arc<dyn object_store::ObjectStore>, base_path: ObjectPath) -> Self {
        Self {
            object_store,
            base_path,
            catalogs: HashMap::new(),
        }
    }

    /// Open or create a dataset-specific catalog.
    pub async fn open_dataset(&mut self, dataset_name: &str) -> CatalogResult<&mut CatalogStore> {
        if !self.catalogs.contains_key(dataset_name) {
            let path = ObjectPath::from(format!(
                "{}/datasets/{}",
                self.base_path.as_ref(),
                dataset_name
            ));
            let opts = OpenOptions {
                object_store: self.object_store.clone(),
                path,
            };
            let store = CatalogStore::open(opts).await?;
            self.catalogs.insert(dataset_name.to_string(), store);
        }
        Ok(self.catalogs.get_mut(dataset_name).unwrap())
    }

    /// Close all dataset catalogs.
    pub async fn close_all(self) -> CatalogResult<()> {
        for (_, store) in self.catalogs {
            store.close().await?;
        }
        Ok(())
    }

    /// List currently open datasets.
    pub fn open_datasets(&self) -> Vec<&str> {
        self.catalogs.keys().map(|k| k.as_str()).collect()
    }
}
