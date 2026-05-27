//! CatalogHarness: lightweight catalog write/read helper for integration tests.
//!
//! Provides a thin wrapper around `CatalogStore` for tests that need to verify
//! catalog round-trips (write → read) without spinning up a full IVM worker.
//!
//! ## Usage
//! ```ignore
//! let harness = CatalogHarness::in_memory().await;
//! let table_id = harness.create_table("orders", &["id", "amount"]).await;
//! harness.insert_rows(table_id, vec![row!{"id" => 1, "amount" => 100}]).await;
//! let rows = harness.read_all(table_id).await;
//! assert_eq!(rows.len(), 1);
//! ```

use std::sync::Arc;

use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;

use rocklake_catalog::{CatalogError, CatalogStore, OpenOptions};
use rocklake_core::rows::InlinedInsertRow;

/// Lightweight catalog harness for Tier 2+ integration tests.
pub struct CatalogHarness {
    pub store: CatalogStore,
    opts: OpenOptions,
}

impl CatalogHarness {
    /// Create a harness backed by an in-memory object store.
    pub async fn in_memory() -> Result<Self, CatalogError> {
        let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
        let opts = OpenOptions {
            object_store,
            path: ObjectPath::from("test-catalog"),
            encryption: None,
        };
        let store = CatalogStore::open(opts.clone()).await?;
        Ok(Self { store, opts })
    }

    /// Create a harness backed by a specific object store (e.g., MinIO).
    pub async fn with_object_store(
        object_store: Arc<dyn object_store::ObjectStore>,
        path: &str,
    ) -> Result<Self, CatalogError> {
        let opts = OpenOptions {
            object_store,
            path: ObjectPath::from(path),
            encryption: None,
        };
        let store = CatalogStore::open(opts.clone()).await?;
        Ok(Self { store, opts })
    }

    /// Reopen the catalog (simulates process restart).
    pub async fn reopen(&mut self) -> Result<(), CatalogError> {
        self.store = CatalogStore::open(self.opts.clone()).await?;
        Ok(())
    }

    /// Get a reference to the underlying CatalogStore.
    pub fn store(&self) -> &CatalogStore {
        &self.store
    }

    /// Get a mutable reference to the underlying CatalogStore.
    pub fn store_mut(&mut self) -> &mut CatalogStore {
        &mut self.store
    }

    /// Read back all inline inserts for a given table.
    pub async fn read_inlined_inserts(
        &self,
        table_id: u64,
    ) -> Result<Vec<InlinedInsertRow>, CatalogError> {
        let reader = self.store.read_latest();
        reader.list_inlined_inserts(table_id).await
    }

    /// Assert the catalog can be reopened without error (durability check).
    pub async fn assert_durable(&self) -> Result<(), CatalogError> {
        let _reopened = CatalogStore::open(self.opts.clone()).await?;
        Ok(())
    }
}
