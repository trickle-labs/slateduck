//! CatalogStore: the main entry point for catalog operations.

use object_store::path::Path as ObjectPath;
use rocklake_core::counters::CounterCache;
use rocklake_core::keys;
use rocklake_core::mvcc::SnapshotId;
use rocklake_core::tags::*;
use rocklake_core::values;
use slatedb::{Db, IsolationLevel};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use uuid::Uuid;

use crate::encryption::{AesGcmTransformer, EncryptionConfig};
use crate::error::{CatalogError, CatalogResult};
use crate::init;
use crate::reader::CatalogReader;
use crate::writer::{snapshot::CommitResult, CatalogWriter};

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
    /// v0.28.0: UUID nonce generated at open time, stored alongside the epoch so
    /// two writers with the same counter value can be distinguished at commit.
    writer_nonce: String,
    schema_version: u64,
    /// In-memory cache of the current `retain-from` floor.
    /// Updated eagerly on `open()` and after every `gc_apply()`.
    /// `read_at()` uses this atomically without holding the mutex.
    retain_from_cache: Arc<AtomicU64>,
    /// The object store used to back the SlateDB database.
    /// Exposed so callers (e.g. the PG-Wire executor) can read data files from
    /// the same storage root without requiring a separate configuration parameter.
    object_store: Arc<dyn object_store::ObjectStore>,
}

impl CatalogStore {
    /// Open or create a catalog store.
    /// Uses safe `open_or_create` with serializable transactions.
    /// v0.19: Writer epoch is acquired via CAS — only one writer can hold the epoch.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// use std::sync::Arc;
    /// use object_store::local::LocalFileSystem;
    /// use object_store::path::Path as ObjectPath;
    /// use rocklake_catalog::{CatalogStore, OpenOptions};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    /// let catalog = CatalogStore::open(OpenOptions {
    ///     object_store: store,
    ///     path: ObjectPath::from(""),
    ///     encryption: None,
    /// }).await.unwrap();
    /// # });
    /// ```
    pub async fn open(opts: OpenOptions) -> CatalogResult<Self> {
        // Clone before moving into Db::builder / Db::open so we can keep a
        // reference for the `object_store` field added in v0.27.1.
        let object_store_ref = Arc::clone(&opts.object_store);
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

        // v0.20: Automatically migrate hash-encoded lease/extension keys to
        // length-prefixed encoding. This is a no-op on already-migrated catalogs.
        crate::key_migration::migrate_key_encoding_if_needed(&db).await?;

        // v0.28.0: CAS-protected monotonic writer epoch acquisition.
        // The epoch is a persisted counter incremented by each successful open.
        // A UUID nonce is stored alongside the epoch so two writers that race
        // to the same counter value can still be distinguished at commit time.
        let epoch_key = keys::key_system(SYSTEM_WRITER_EPOCH);
        let nonce_key = keys::key_system(SYSTEM_WRITER_NONCE);
        let writer_nonce = Uuid::new_v4().to_string();

        let writer_epoch: u64 = loop {
            let tx = db
                .begin(IsolationLevel::SerializableSnapshot)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

            let current_epoch: u64 = match tx
                .get(&epoch_key)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                Some(data) => values::decode_counter(&data)?,
                None => 0, // First open — counter starts at zero
            };

            let new_epoch = current_epoch.checked_add(1).ok_or_else(|| {
                CatalogError::Internal("writer epoch counter overflow".to_string())
            })?;

            // Write the new epoch and nonce atomically.
            tx.put(&epoch_key, values::encode_counter(new_epoch))
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            tx.put(&nonce_key, writer_nonce.as_bytes())
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

            match tx.commit().await {
                Ok(_) => break new_epoch,
                Err(_) => continue, // CAS conflict — another writer beat us; retry
            }
        };

        // Load current schema version (from latest snapshot, or 0)
        let schema_version = Self::load_schema_version(&db, &counters).await;

        // Seed the retain-from cache from SlateDB (single read at startup).
        let retain_from_initial = crate::gc::read_retain_from(&db).await.unwrap_or(0);
        let retain_from_cache = Arc::new(AtomicU64::new(retain_from_initial));

        Ok(Self {
            db,
            counters,
            writer_epoch,
            writer_nonce,
            schema_version,
            retain_from_cache,
            object_store: object_store_ref,
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
    ///
    /// Accepts any type that converts to `SnapshotId`, including `CommitResult`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # tokio::runtime::Runtime::new().unwrap().block_on(async {
    /// use std::sync::Arc;
    /// use object_store::local::LocalFileSystem;
    /// use object_store::path::Path as ObjectPath;
    /// use rocklake_catalog::{CatalogStore, OpenOptions};
    ///
    /// let dir = tempfile::tempdir().unwrap();
    /// let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).unwrap());
    /// let catalog = CatalogStore::open(OpenOptions { object_store: store, path: ObjectPath::from(""), encryption: None }).await.unwrap();
    /// let reader = catalog.read_at(rocklake_core::mvcc::SnapshotId::new(0)).unwrap();
    /// let schemas = reader.list_schemas().await.unwrap();
    /// assert!(schemas.is_empty());
    /// # });
    /// ```
    pub fn read_at(&self, dl_snapshot_id: impl Into<SnapshotId>) -> CatalogResult<CatalogReader> {
        let dl_snapshot_id = dl_snapshot_id.into();
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
        let key = keys::key_counter(rocklake_core::tags::COUNTER_NEXT_SNAPSHOT_ID);
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
            self.writer_nonce.clone(),
            self.schema_version,
        )
    }

    /// Synchronise the store's in-memory counters from a successfully committed
    /// snapshot.  Must be called after every successful `create_snapshot()` so
    /// that subsequent `begin_write()` and `read_latest()` calls reflect the
    /// newly committed state.
    ///
    /// The `result` argument is the [`CommitResult`] returned by
    /// `CatalogWriter::create_snapshot()`.  It is `#[must_use]` so the compiler
    /// will reject code that discards it without calling this method.
    pub fn commit_writer(&mut self, result: CommitResult) {
        self.counters.sync_from(
            result.next_snapshot_id,
            result.next_catalog_id,
            result.next_file_id,
        );
        self.schema_version = result.schema_version;
    }

    /// Close the catalog store.
    pub async fn close(self) -> CatalogResult<()> {
        self.db.close().await?;
        Ok(())
    }

    /// Open a catalog store **without** acquiring the writer epoch.
    ///
    /// This is the correct path for read-only sidecar replicas.  Skipping the
    /// CAS epoch means any number of reader pods can open the same catalog
    /// prefix concurrently with zero write conflicts.
    ///
    /// The returned `CatalogStore` has `writer_epoch = 0`.  Attempting to call
    /// `begin_write()` / `commit_writer()` on it will succeed at the in-process
    /// level, but the SQL handler is expected to reject writes based on the
    /// serving mode set by `cmd_serve`.
    ///
    /// Prefer [`crate::readonly::ReadOnlyCatalog`] for true read-only access
    /// (no write path at all).  This method exists so the PG-Wire server can
    /// reuse the existing `Arc<Mutex<CatalogStore>>` infrastructure while still
    /// avoiding the epoch contention.
    pub async fn open_without_epoch(opts: OpenOptions) -> CatalogResult<Self> {
        let object_store_ref = Arc::clone(&opts.object_store);
        let db = if let Some(ref enc) = opts.encryption {
            let transformer = Arc::new(AesGcmTransformer::new(enc));
            Db::builder(opts.path, opts.object_store)
                .with_block_transformer(transformer)
                .build()
                .await?
        } else {
            Db::open(opts.path, opts.object_store).await?
        };

        crate::key_migration::migrate_key_encoding_if_needed(&db).await?;
        let counters = init::initialize_catalog(&db).await?;
        let schema_version = Self::load_schema_version(&db, &counters).await;
        let retain_from_initial = crate::gc::read_retain_from(&db).await.unwrap_or(0);

        Ok(Self {
            db,
            counters,
            writer_epoch: 0, // no epoch acquired — reader pod
            writer_nonce: "reader".to_string(),
            schema_version,
            retain_from_cache: Arc::new(AtomicU64::new(retain_from_initial)),
            object_store: object_store_ref,
        })
    }

    /// Open a **read-only** catalog: no writer epoch is acquired or incremented.
    ///
    /// Convenience wrapper for [`ReadOnlyCatalog::open()`].  Multiple
    /// simultaneous calls with the same `opts` produce zero CAS conflicts.
    ///
    /// See [`crate::readonly::ReadOnlyCatalog`] for the full API.
    pub async fn open_readonly(
        opts: OpenOptions,
    ) -> CatalogResult<crate::readonly::ReadOnlyCatalog> {
        crate::readonly::ReadOnlyCatalog::open(opts).await
    }

    /// Get the underlying database reference (for verification/testing).
    pub fn db(&self) -> &Db {
        &self.db
    }

    /// Return the object store backing this catalog.
    ///
    /// The PG-Wire CDC executor uses this to resolve Parquet data file paths
    /// relative to the same object storage root as the catalog.
    pub fn object_store(&self) -> Arc<dyn object_store::ObjectStore> {
        Arc::clone(&self.object_store)
    }

    /// Return the current catalog schema version (used for ducklake_schema_version facade).
    pub fn schema_version(&self) -> u64 {
        self.schema_version
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
                match values::decode_value::<rocklake_core::rows::SnapshotRow>(&data) {
                    Ok(row) => row.schema_version,
                    Err(_) => 0,
                }
            }
            _ => 0,
        }
    }
}
