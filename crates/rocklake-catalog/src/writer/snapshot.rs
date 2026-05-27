//! Snapshot commit: create_snapshot and transaction helpers.

use slatedb::{DbTransaction, IsolationLevel};
use rocklake_core::keys;
use rocklake_core::mvcc::SnapshotId;
use rocklake_core::rows::{SnapshotChangesRow, SnapshotRow};
use rocklake_core::tags::{
    COUNTER_NEXT_CATALOG_ID, COUNTER_NEXT_FILE_ID, COUNTER_NEXT_SNAPSHOT_ID, SYSTEM_WRITER_EPOCH,
};
use rocklake_core::values;

use crate::error::{CatalogError, CatalogResult};

use super::CatalogWriter;

/// The result of a successful `create_snapshot()` call.
///
/// **Must** be passed to `CatalogStore::commit_writer(result)` to update the
/// store's in-memory counter cache.  The `#[must_use]` attribute causes the
/// compiler to emit an error (in `--deny warnings` CI builds) if the value is
/// silently discarded, making it impossible to forget the `commit_writer` call.
///
/// The `snapshot_id` field gives callers access to the new snapshot ID without
/// having to call into the store again.  `CommitResult` also implements
/// `Deref<Target = SnapshotId>` for ergonomic use as a `SnapshotId` in calls
/// that do not yet need the full result.
#[must_use = "CommitResult must be passed to CatalogStore::commit_writer(result)"]
#[derive(Clone, Copy)]
pub struct CommitResult {
    /// The snapshot ID that was committed.
    pub snapshot_id: SnapshotId,
    pub(crate) next_snapshot_id: u64,
    pub(crate) next_catalog_id: u64,
    pub(crate) next_file_id: u64,
    pub(crate) schema_version: u64,
}

impl std::fmt::Debug for CommitResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommitResult")
            .field("snapshot_id", &self.snapshot_id)
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for CommitResult {
    type Target = SnapshotId;
    fn deref(&self) -> &SnapshotId {
        &self.snapshot_id
    }
}

impl From<CommitResult> for SnapshotId {
    fn from(c: CommitResult) -> SnapshotId {
        c.snapshot_id
    }
}

impl CatalogWriter {
    /// Commit all staged mutations, counter updates, and the snapshot row in a
    /// single atomic SlateDB transaction.
    ///
    /// This is the **only** method that writes MVCC-versioned rows to SlateDB.
    /// Every staging method (`create_schema`, `create_table`, `add_column`,
    /// etc.) merely buffers the write; `create_snapshot()` is the sole commit
    /// boundary.
    ///
    /// The returned [`CommitResult`] **must** be passed to
    /// `CatalogStore::commit_writer(result)` to keep the store's in-memory
    /// counter cache in sync.
    #[tracing::instrument(skip(self, author, message))]
    pub async fn create_snapshot(
        &mut self,
        author: Option<&str>,
        message: Option<&str>,
    ) -> CatalogResult<CommitResult> {
        let snapshot_id = self.counters.alloc_snapshot_id();

        if self.schema_changed {
            self.current_schema_version += 1;
            self.schema_changed = false;
        }

        // v0.27.8: author and message are persisted in ducklake_snapshot_changes
        // (DuckLake v1.0 spec), not in ducklake_snapshot.  SnapshotRow retains
        // the fields for backward-compatible decoding of existing rows but new
        // rows no longer populate them.
        let row = SnapshotRow {
            snapshot_id,
            schema_version: self.current_schema_version,
            snapshot_time: chrono::Utc::now().to_rfc3339(),
            author: None,
            message: None,
            next_catalog_id: Some(self.counters.peek_catalog_id()),
            next_file_id: Some(self.counters.peek_file_id()),
        };

        // Drain staged mutations — these become part of the atomic commit.
        let staged = std::mem::take(&mut self.staged);

        // Drain and finalize the pending snapshot-changes row (if any), populating
        // author / commit_message from the InsertSnapshot op.
        let pending_changes = {
            let mut row = self
                .pending_snapshot_changes
                .take()
                .unwrap_or(SnapshotChangesRow {
                    snapshot_id,
                    change_type: String::new(),
                    change_info: None,
                    schema_id: None,
                    table_id: None,
                    author: None,
                    commit_message: None,
                    commit_extra_info: None,
                    changes_made: None,
                });
            row.snapshot_id = snapshot_id;
            if row.author.is_none() {
                row.author = author.map(|s| s.to_string());
            }
            if row.commit_message.is_none() {
                row.commit_message = message.map(|s| s.to_string());
            }
            // Only persist if there is something meaningful to store.
            if row.author.is_some()
                || row.commit_message.is_some()
                || row.changes_made.is_some()
                || !row.change_type.is_empty()
            {
                Some(row)
            } else {
                None
            }
        };

        // One serializable transaction for everything.
        let tx = self.begin_tx().await?;

        // Verify writer-epoch fencing before touching any data.
        self.check_epoch(&tx).await?;

        // Write all staged catalog rows.
        for (key, value) in &staged {
            tx.put(key, value)
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        }

        // Write the snapshot row.
        let snapshot_key = keys::key_snapshot(snapshot_id);
        tx.put(&snapshot_key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        // Write the snapshot-changes row (if any changes were recorded).
        if let Some(changes_row) = &pending_changes {
            let changes_key = keys::key_snapshot_changes(snapshot_id);
            tx.put(&changes_key, values::encode_value(changes_row))
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        }

        // Persist all counter values atomically with the snapshot.
        tx.put(
            keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID),
            self.counters.encode_snapshot_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        tx.put(
            keys::key_counter(COUNTER_NEXT_CATALOG_ID),
            self.counters.encode_catalog_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        tx.put(
            keys::key_counter(COUNTER_NEXT_FILE_ID),
            self.counters.encode_file_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;

        Ok(CommitResult {
            snapshot_id: SnapshotId::new(snapshot_id),
            next_snapshot_id: self.counters.peek_snapshot_id(),
            next_catalog_id: self.counters.peek_catalog_id(),
            next_file_id: self.counters.peek_file_id(),
            schema_version: self.current_schema_version,
        })
    }

    pub(super) async fn begin_tx(&self) -> CatalogResult<DbTransaction> {
        self.db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))
    }

    pub(super) async fn check_epoch(&self, tx: &DbTransaction) -> CatalogResult<()> {
        let epoch_key = keys::key_system(SYSTEM_WRITER_EPOCH);
        match tx
            .get(&epoch_key)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            Some(data) => {
                let stored_epoch = values::decode_counter(&data)?;
                if stored_epoch != self.writer_epoch {
                    return Err(CatalogError::WriterEpochMismatch);
                }
            }
            None => {
                return Err(CatalogError::WriterEpochMismatch);
            }
        }
        Ok(())
    }
}
