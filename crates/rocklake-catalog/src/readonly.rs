//! Read-only catalog access path (RFC-01, v0.47.0).
//!
//! `ReadOnlyCatalog` opens SlateDB **without** acquiring or incrementing the
//! writer-epoch CAS key.  This means many reader instances can open the same
//! catalog concurrently with zero coordination overhead — ideal for stateless,
//! horizontally-scaled reader fleets.
//!
//! # Guarantees
//!
//! * Zero writes to SlateDB on open or refresh.
//! * `reader()` creates a `CatalogReader` bound to the most recently refreshed
//!   snapshot; it never sees data past a snapshot that was committed *after*
//!   the last `refresh()` call (snapshot isolation).
//! * Concurrent `ReadOnlyCatalog` instances opened against the same catalog
//!   prefix produce **zero** CAS transaction conflicts in the SlateDB write log.
//!
//! # Example
//!
//! ```no_run
//! # tokio::runtime::Runtime::new().expect("runtime").block_on(async {
//! use std::sync::Arc;
//! use object_store::local::LocalFileSystem;
//! use object_store::path::Path as ObjectPath;
//! use rocklake_catalog::{OpenOptions, ReadOnlyCatalog};
//!
//! let dir = tempfile::tempdir().expect("tempdir");
//! let store = Arc::new(LocalFileSystem::new_with_prefix(dir.path()).expect("store"));
//! let mut cat = ReadOnlyCatalog::open(OpenOptions {
//!     object_store: store,
//!     path: ObjectPath::from(""),
//!     encryption: None,
//! }).await.expect("open");
//! let snapshot_id = cat.refresh().await.expect("refresh");
//! let reader = cat.reader();
//! # });
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use slatedb::Db;

use rocklake_core::keys;
use rocklake_core::mvcc::SnapshotId;
use rocklake_core::tags::COUNTER_NEXT_SNAPSHOT_ID;
use rocklake_core::values;

use crate::encryption::AesGcmTransformer;
use crate::error::{CatalogError, CatalogResult};
use crate::reader::CatalogReader;
use crate::store::OpenOptions;

/// A read-only catalog handle.
///
/// Does **not** hold a writer epoch — multiple instances may be opened against
/// the same S3/GCS prefix without contention.
pub struct ReadOnlyCatalog {
    db: Db,
    /// Snapshot ID of the latest committed snapshot at the time of the last
    /// `refresh()` call (or `open()`).
    current_snapshot_id: SnapshotId,
    /// Cached retain-from floor (read from SlateDB on open / refresh).
    retain_from: Arc<AtomicU64>,
    /// Object store held for callers (e.g. data-file reads).
    object_store: Arc<dyn object_store::ObjectStore>,
}

impl ReadOnlyCatalog {
    /// Open a read-only catalog.  No writer epoch is acquired or incremented.
    ///
    /// The catalog is initialized if it does not yet exist so that reader
    /// pods can tolerate racing against a writer that is still creating the
    /// catalog for the first time.
    pub async fn open(opts: OpenOptions) -> CatalogResult<Self> {
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

        // Run key-encoding migration (no-op on already-migrated catalogs).
        crate::key_migration::migrate_key_encoding_if_needed(&db).await?;

        // Read the latest snapshot ID without writing anything.
        let current_snapshot_id = Self::read_latest_snapshot_id(&db).await;

        // Read the retain-from floor.
        let retain_from_initial = crate::gc::read_retain_from(&db).await.unwrap_or(0);

        Ok(Self {
            db,
            current_snapshot_id,
            retain_from: Arc::new(AtomicU64::new(retain_from_initial)),
            object_store: object_store_ref,
        })
    }

    /// Return a reader bound to the snapshot captured at the last `refresh()`.
    ///
    /// Returns `CatalogError::SnapshotOutOfRetention` if the current snapshot
    /// has been GC-retired.
    pub fn reader(&self) -> CatalogResult<CatalogReader> {
        let retain_from = self.retain_from.load(Ordering::Acquire);
        if retain_from > 0 && self.current_snapshot_id.as_u64() < retain_from {
            return Err(CatalogError::SnapshotOutOfRetention {
                requested: self.current_snapshot_id.as_u64(),
                retain_from,
            });
        }
        Ok(CatalogReader::new(
            self.db.clone(),
            self.current_snapshot_id,
        ))
    }

    /// Advance to the latest committed snapshot without writer coordination.
    ///
    /// Re-reads the `next_snapshot_id` counter and the `retain_from` key from
    /// SlateDB.  Returns the newly observed snapshot ID.
    pub async fn refresh(&mut self) -> CatalogResult<SnapshotId> {
        // Refresh snapshot ID.
        self.current_snapshot_id = Self::read_latest_snapshot_id(&self.db).await;

        // Refresh retain-from floor.
        let retain_from = crate::gc::read_retain_from(&self.db).await.unwrap_or(0);
        self.retain_from.store(retain_from, Ordering::Release);

        Ok(self.current_snapshot_id)
    }

    /// Return the snapshot ID observed at the last `refresh()` (or `open()`).
    pub fn current_snapshot_id(&self) -> SnapshotId {
        self.current_snapshot_id
    }

    /// Return the object store backing this catalog.
    pub fn object_store(&self) -> Arc<dyn object_store::ObjectStore> {
        Arc::clone(&self.object_store)
    }

    /// Close the underlying SlateDB handle.
    pub async fn close(self) -> CatalogResult<()> {
        self.db.close().await?;
        Ok(())
    }

    // ── internal ───────────────────────────────────────────────────────────

    async fn read_latest_snapshot_id(db: &Db) -> SnapshotId {
        let key = keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID);
        match db.get(&key).await {
            Ok(Some(data)) => {
                let next = values::decode_counter(&data).unwrap_or(1);
                // next_snapshot_id is always 1 ahead of the committed snapshot.
                SnapshotId::new(if next > 0 { next - 1 } else { 0 })
            }
            _ => SnapshotId::new(0),
        }
    }
}
