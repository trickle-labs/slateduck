//! CatalogStore: the main entry point for catalog operations.

use object_store::path::Path as ObjectPath;
use slatedb::Db;
use slateduck_core::counters::CounterCache;
use slateduck_core::keys;
use slateduck_core::mvcc::SnapshotId;
use slateduck_core::tags::*;
use slateduck_core::values;
use std::sync::Arc;

use crate::error::CatalogResult;
use crate::init;
use crate::reader::CatalogReader;
use crate::writer::CatalogWriter;

/// Options for opening a CatalogStore.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    /// Object store instance.
    pub object_store: Arc<dyn object_store::ObjectStore>,
    /// Path within the object store for the SlateDB database.
    pub path: ObjectPath,
}

/// The main catalog store backed by SlateDB.
pub struct CatalogStore {
    db: Db,
    counters: CounterCache,
    writer_epoch: u64,
    schema_version: u64,
}

impl CatalogStore {
    /// Open or create a catalog store.
    /// Uses safe `open_or_create` with serializable transactions.
    pub async fn open(opts: OpenOptions) -> CatalogResult<Self> {
        let db = Db::open(opts.path, opts.object_store).await?;

        // Initialize or verify
        let counters = init::initialize_catalog(&db).await?;

        // Register writer epoch
        let writer_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let epoch_key = keys::key_system(SYSTEM_WRITER_EPOCH);
        db.put(&epoch_key, &values::encode_counter(writer_epoch))
            .await?;

        // Load current schema version (from latest snapshot, or 0)
        let schema_version = Self::load_schema_version(&db, &counters).await;

        Ok(Self {
            db,
            counters,
            writer_epoch,
            schema_version,
        })
    }

    /// Create a reader bound to a specific DuckLake snapshot.
    pub fn read_at(&self, dl_snapshot_id: SnapshotId) -> CatalogReader {
        CatalogReader::new(self.db.clone(), dl_snapshot_id)
    }

    /// Create a reader for the latest snapshot.
    pub fn read_latest(&self) -> CatalogReader {
        let latest = if self.counters.peek_snapshot_id() > 1 {
            self.counters.peek_snapshot_id() - 1
        } else {
            0
        };
        CatalogReader::new(self.db.clone(), SnapshotId::new(latest))
    }

    /// Begin a write session.
    pub fn begin_write(&mut self) -> CatalogWriter {
        CatalogWriter::new(
            self.db.clone(),
            CounterCache::new(
                self.counters.peek_snapshot_id(),
                self.counters.peek_catalog_id(),
                self.counters.peek_file_id(),
            ),
            self.writer_epoch,
            self.schema_version,
        )
    }

    /// Synchronise the store's in-memory counters from a successfully committed
    /// writer.  Must be called after every successful `create_snapshot()` so
    /// that subsequent `begin_write()` and `read_latest()` calls reflect the
    /// newly committed state.
    pub fn commit_writer(&mut self, writer: &CatalogWriter) {
        self.counters.sync_from(
            writer.counters.peek_snapshot_id(),
            writer.counters.peek_catalog_id(),
            writer.counters.peek_file_id(),
        );
        self.schema_version = writer.schema_version();
    }

    /// Close the catalog store.
    pub async fn close(self) -> CatalogResult<()> {
        self.db.close().await?;
        Ok(())
    }

    /// Get the underlying database reference (for verification/testing).
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Load schema version from the latest snapshot.
    async fn load_schema_version(db: &Db, counters: &CounterCache) -> u64 {
        let latest_id = if counters.peek_snapshot_id() > 1 {
            counters.peek_snapshot_id() - 1
        } else {
            return 0;
        };

        let key = keys::key_snapshot(latest_id);
        match db.get(&key).await {
            Ok(Some(data)) => {
                match values::decode_value::<slateduck_core::rows::SnapshotRow>(&data) {
                    Ok(row) => row.schema_version,
                    Err(_) => 0,
                }
            }
            _ => 0,
        }
    }
}
