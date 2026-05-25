//! Input source: read inlined insert rows from the catalog since a given snapshot.

use slateduck_catalog::CatalogStore;
use slateduck_core::mvcc::SnapshotId;

use crate::worker::IvmError;

/// Reads incremental input rows for a materialized view shard from the catalog.
pub struct MatviewInputSource {
    pub store: CatalogStore,
    pub matview_id: u64,
    pub base_table_id: u64,
    pub shard_id: u32,
    /// Last snapshot that has already been consumed.
    pub last_snapshot: u64,
}

impl MatviewInputSource {
    /// Create a new input source.
    pub fn new(
        store: CatalogStore,
        matview_id: u64,
        base_table_id: u64,
        shard_id: u32,
        last_snapshot: u64,
    ) -> Self {
        Self {
            store,
            matview_id,
            base_table_id,
            shard_id,
            last_snapshot,
        }
    }

    /// Poll for new rows since `last_snapshot` and return them as JSON maps.
    ///
    /// Advances `last_snapshot` to `up_to_snapshot` on success.
    pub async fn poll(&mut self, up_to_snapshot: u64) -> Result<Vec<serde_json::Value>, IvmError> {
        if up_to_snapshot <= self.last_snapshot {
            return Ok(Vec::new());
        }

        let reader = self
            .store
            .read_at(SnapshotId::new(up_to_snapshot))
            .map_err(|e| IvmError::Catalog(e.to_string()))?;

        let rows = reader
            .list_inlined_inserts(self.base_table_id)
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;

        self.last_snapshot = up_to_snapshot;

        let result = rows
            .into_iter()
            .filter_map(|row| serde_json::from_slice::<serde_json::Value>(&row.payload).ok())
            .collect();

        Ok(result)
    }
}
