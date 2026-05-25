//! CatalogStore: the main entry point for catalog operations.

use slatedb::{Db, IsolationLevel};
use object_store::path::Path as ObjectPath;
use slateduck_core::counters::CounterCache;
use slateduck_core::keys;
use slateduck_core::mvcc::SnapshotId;
use slateduck_core::tags::*;
use slateduck_core::values;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::encryption::{AesGcmTransformer, EncryptionConfig};
use crate::error::{CatalogError, CatalogResult};
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
    /// Optional AES-256-GCM encryption for at-rest block data.
    pub encryption: Option<EncryptionConfig>,
}

/// The main catalog store backed by SlateDB.
pub struct CatalogStore {
    db: Db,
    counters: CounterCache,
    writer_epoch: u64,
    schema_version: u64,
    /// In-memory cache of the current `retain-from` floor.
    /// Updated eagerly on `open()` and after every `gc_apply()`.
    /// `read_at()` uses this atomically without holding the mutex.
    retain_from_cache: Arc<AtomicU64>,
}

impl CatalogStore {
    /// Open or create a catalog store.
    /// Uses safe `open_or_create` with serializable transactions.
    /// v0.19: Writer epoch is acquired via CAS — only one writer can hold the epoch.
    pub async fn open(opts: OpenOptions) -> CatalogResult<Self> {
        let db = if let Some(ref enc) = opts.encryption {
            let transformer = Arc::new(AesGcmTransformer::new(enc));
            Db::builder(opts.path, opts.object_store)
                .with_block_transformer(transformer)
                .build()
                .await?
        } else {
            Db::open(opts.path, opts.object_store).await?
        };

        // Initialize or verify
        let counters = init::initialize_catalog(&db).await?;

        // v0.19: CAS-protected writer epoch acquisition.
        // Read the current epoch, validate no other writer holds a non-expired lease,
        // and atomically CAS a new epoch. Fail closed when epoch key is missing after
        // initialization (which means corruption) or when another writer holds it.
        let writer_epoch = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| CatalogError::Internal("system clock before UNIX epoch".to_string()))?
            .as_millis() as u64;

        let epoch_key = keys::key_system(SYSTEM_WRITER_EPOCH);

        // Transactional CAS: read current epoch, verify we can claim it, write new epoch.
        loop {
            let tx = db
                .begin(IsolationLevel::SerializableSnapshot)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

            let current_epoch = match tx
                .get(&epoch_key)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                Some(data) => Some(values::decode_counter(&data)?),
                None => None, // First open — no epoch exists yet
            };

            // If an existing epoch is newer than ours, another writer is active.
            if let Some(existing) = current_epoch {
                if existing > writer_epoch {
                    return Err(CatalogError::WriterEpochMismatch);
                }
            }

            // Write our new epoch atomically.
            tx.put(&epoch_key, values::encode_counter(writer_epoch))
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

            match tx.commit().await {
                Ok(_) => break,
                Err(_) => continue, // CAS conflict — retry
            }
        }

        // Load current schema version (from latest snapshot, or 0)
        let schema_version = Self::load_schema_version(&db, &counters).await;

        // Seed the retain-from cache from SlateDB (single read at startup).
        let retain_from_initial = crate::gc::read_retain_from(&db).await.unwrap_or(0);
        let retain_from_cache = Arc::new(AtomicU64::new(retain_from_initial));

        Ok(Self {
            db,
            counters,
            writer_epoch,
            schema_version,
            retain_from_cache,
        })
    }

    /// Create a reader bound to a specific DuckLake snapshot.
    ///
    /// This is a **synchronous** operation — it checks the in-memory
    /// `retain-from` cache and clones the `Db` handle without any async I/O.
    /// The caller can therefore hold the catalog mutex for this call only and
    /// drop it before performing any async reads on the returned reader.
    ///
    /// Returns `CatalogError::SnapshotOutOfRetention` (SQLSTATE 22023) if
    /// `dl_snapshot_id` falls below the current retain-from floor.
    pub fn read_at(&self, dl_snapshot_id: SnapshotId) -> CatalogResult<CatalogReader> {
        let retain_from = self.retain_from_cache.load(Ordering::Acquire);
        if retain_from > 0 && dl_snapshot_id.as_u64() < retain_from {
            return Err(CatalogError::SnapshotOutOfRetention {
                requested: dl_snapshot_id.as_u64(),
                retain_from,
            });
        }
        Ok(CatalogReader::new(self.db.clone(), dl_snapshot_id))
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

    /// Create a reader for the latest snapshot, reading the counter from SlateDB.
    ///
    /// Unlike `read_latest()` which uses the in-memory counter, this function
    /// reads the snapshot counter directly from SlateDB. Use this for long-lived
    /// read-only processes that need to see snapshots committed by other writers.
    pub async fn read_fresh_latest(&self) -> CatalogResult<CatalogReader> {
        let key = keys::key_counter(slateduck_core::tags::COUNTER_NEXT_SNAPSHOT_ID);
        let latest = match self.db.get(&key).await? {
            Some(data) => {
                let next = values::decode_counter(&data)?;
                next.saturating_sub(1)
            }
            None => 0,
        };
        Ok(CatalogReader::new(self.db.clone(), SnapshotId::new(latest)))
    }

    /// Update the in-memory retain-from cache.
    ///
    /// Must be called after every successful `gc::gc_apply()` so that
    /// subsequent `read_at()` calls see the new floor without re-reading
    /// SlateDB.
    ///
    /// v0.19: Uses `Ordering::Release` so that any thread loading the value
    /// with `Ordering::Acquire` observes all preceding writes.
    pub fn update_retain_from_cache(&self, new_retain_from: u64) {
        self.retain_from_cache
            .store(new_retain_from, Ordering::Release);
    }

    /// Expose the retain-from cache handle so the FFI / CLI can share it
    /// without holding the catalog mutex.
    pub fn retain_from_cache(&self) -> Arc<AtomicU64> {
        self.retain_from_cache.clone()
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
