//! CatalogWriter: write operations that produce new snapshots.
//!
//! # Staging model (v0.9.1)
//!
//! All MVCC-versioned catalog row mutations are **staged in memory** until
//! `create_snapshot()` is called, which commits every staged write plus all
//! counter updates and the snapshot row in **one atomic SlateDB transaction**.
//! A crash before `create_snapshot()` leaves the catalog unchanged.
//!
//! # Non-MVCC direct writes (v0.19 staged write discipline)
//!
//! The following methods write directly via `self.db.put()` and are intentionally
//! **not** staged through `create_snapshot()`. They are safe to write outside
//! the snapshot boundary because they are ancillary metadata that does not
//! participate in MVCC versioning and does not need crash-atomic commit with
//! the snapshot row:
//!
//! - `update_table_stats()` — aggregate statistics, recomputable from data files
//! - `upsert_file_column_stats()` — per-file zone maps, recomputable from Parquet
//! - `upsert_file_variant_stats()` — variant stats, recomputable
//! - `add_macro_parameter()` — macro metadata, idempotent
//! - `schedule_file_deletion()` — deletion scheduling, advisory
//! - `remove_scheduled_deletion()` — cleanup of scheduling metadata
//!
//! A partial write of any of these is always recoverable: either the data is
//! recomputed on next access, or the operation is idempotent on retry.

use slatedb::{Db, DbTransaction, IsolationLevel};
use slateduck_core::counters::CounterCache;
use slateduck_core::keys;
use slateduck_core::mvcc::SnapshotId;
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::values::{self, MAX_INLINED_VALUE_SIZE};

use crate::error::{CatalogError, CatalogResult};

/// Outcome of a `claim_matview_shard` attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// Lease acquired; the caller now owns the shard.
    Acquired {
        /// New generation value after the CAS.
        generation: u64,
        /// Unix-millisecond timestamp when the lease expires.
        expires_unix_ms: u64,
    },
    /// Another worker holds a non-expired lease.
    Contended { current_owner: String },
    /// This worker already owns the shard.
    AlreadyOwned { generation: u64 },
}

/// Writes to the catalog, producing new snapshots atomically.
///
/// Call `create_snapshot()` to commit all staged mutations in a single atomic
/// SlateDB transaction.  Dropping a `CatalogWriter` without calling
/// `create_snapshot()` discards all staged mutations without touching SlateDB.
pub struct CatalogWriter {
    pub(crate) db: Db,
    pub(crate) counters: CounterCache,
    writer_epoch: u64,
    schema_changed: bool,
    current_schema_version: u64,
    /// Staged (key, value) pairs committed atomically by `create_snapshot()`.
    staged: Vec<(Vec<u8>, Vec<u8>)>,
}

impl CatalogWriter {
    pub(crate) fn new(
        db: Db,
        counters: CounterCache,
        writer_epoch: u64,
        schema_version: u64,
    ) -> Self {
        Self {
            db,
            counters,
            writer_epoch,
            schema_changed: false,
            current_schema_version: schema_version,
            staged: Vec::new(),
        }
    }

    /// Stage a (key, value) write for atomic commit in `create_snapshot()`.
    fn stage(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.staged.push((key, value));
    }

    /// Mark that a schema-mutating operation occurred in this write session.
    pub fn mark_schema_changed(&mut self) {
        self.schema_changed = true;
    }

    pub fn schema_version(&self) -> u64 {
        self.current_schema_version
    }

    pub async fn create_schema(&mut self, schema_name: &str) -> CatalogResult<u64> {
        let schema_id = self.counters.alloc_catalog_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        let row = SchemaRow {
            schema_id,
            schema_name: schema_name.to_string(),
            begin_snapshot: snapshot_id,
            end_snapshot: None,
        };

        let key = keys::key_schema(schema_id);
        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(schema_id)
    }

    pub async fn drop_schema(&mut self, schema_id: u64) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_schema(schema_id);

        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("schema {schema_id}")))?;

        let mut row: SchemaRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(())
    }

    pub async fn create_table(
        &mut self,
        schema_id: u64,
        table_name: &str,
        data_path: Option<&str>,
    ) -> CatalogResult<u64> {
        let table_id = self.counters.alloc_catalog_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        let row = TableRow {
            table_id,
            schema_id,
            table_name: table_name.to_string(),
            begin_snapshot: snapshot_id,
            end_snapshot: None,
            data_path: data_path.map(|s| s.to_string()),
        };

        let key = keys::key_table(schema_id, table_id, snapshot_id);
        self.stage(key, values::encode_value(&row));

        // Secondary index: TAG_TABLE_BY_ID → schema_id for O(1) describe_table lookups.
        let idx_key = keys::key_table_by_id(table_id);
        self.stage(idx_key, values::encode_counter(schema_id));

        self.mark_schema_changed();
        Ok(table_id)
    }

    /// Drop a table by marking its `end_snapshot`.
    ///
    /// The `schema_id` **must** be the correct owning schema; callers that
    /// only have the `table_id` should first call `find_table_schema_id`.
    pub async fn drop_table(
        &mut self,
        schema_id: u64,
        table_id: u64,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_table(schema_id, table_id, begin_snapshot);

        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("table {table_id}")))?;

        let mut row: TableRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(())
    }

    /// Return the `schema_id` that owns `table_id` using the TAG_TABLE_BY_ID
    /// secondary index — O(1) point-lookup instead of O(n) full-table scan.
    ///
    /// Returns `None` if the secondary index entry does not exist (e.g. the
    /// table was created before this index was introduced).  Callers that need
    /// a guaranteed result should fall back to the legacy scan themselves.
    pub async fn find_table_schema_id(&self, table_id: u64) -> CatalogResult<Option<u64>> {
        let idx_key = keys::key_table_by_id(table_id);
        match self.db.get(&idx_key).await? {
            Some(data) => Ok(Some(values::decode_counter(&data)?)),
            None => {
                // Fallback for catalogs created before the secondary index.
                let prefix = keys::prefix_for_tag(TAG_TABLE);
                let mut iter = self
                    .db
                    .scan_prefix(&prefix)
                    .await
                    .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
                while let Some(kv) = iter
                    .next()
                    .await
                    .map_err(|e| CatalogError::SlateDb(e.to_string()))?
                {
                    let row: TableRow = values::decode_value(&kv.value)?;
                    if row.table_id == table_id && row.end_snapshot.is_none() {
                        return Ok(Some(row.schema_id));
                    }
                }
                Ok(None)
            }
        }
    }

    pub async fn add_column(
        &mut self,
        table_id: u64,
        column_name: &str,
        data_type: &str,
        column_index: u64,
        is_nullable: bool,
        default_value: Option<&str>,
    ) -> CatalogResult<u64> {
        let column_id = self.counters.alloc_catalog_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        let row = ColumnRow {
            column_id,
            table_id,
            column_name: column_name.to_string(),
            data_type: data_type.to_string(),
            column_index,
            begin_snapshot: snapshot_id,
            end_snapshot: None,
            default_value: default_value.map(|s| s.to_string()),
            is_nullable,
        };

        let key = keys::key_column(table_id, column_id, snapshot_id);
        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(column_id)
    }

    /// Drop a column by marking its `end_snapshot`.
    ///
    /// The `table_id` **must** be the correct owning table; callers that only
    /// have the `column_id` should first call `find_column_table_id`.
    pub async fn drop_column(
        &mut self,
        table_id: u64,
        column_id: u64,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_column(table_id, column_id, begin_snapshot);

        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("column {column_id}")))?;

        let mut row: ColumnRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(())
    }

    /// Scan all column rows to find the `table_id` that owns `column_id`.
    ///
    /// Returns `None` if no live (end_snapshot IS NULL) column row is found.
    /// Used by the PG-Wire executor to resolve `UPDATE end_snapshot` on
    /// `ducklake_column` without using entity_id for both table_id and column_id.
    pub async fn find_column_table_id(&self, column_id: u64) -> CatalogResult<Option<u64>> {
        let prefix = keys::prefix_for_tag(TAG_COLUMN);
        let mut iter = self
            .db
            .scan_prefix(&prefix)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: ColumnRow = values::decode_value(&kv.value)?;
            if row.column_id == column_id && row.end_snapshot.is_none() {
                return Ok(Some(row.table_id));
            }
        }
        Ok(None)
    }

    pub async fn register_data_file(
        &mut self,
        table_id: u64,
        path: &str,
        file_format: &str,
        row_count: u64,
        file_size_bytes: u64,
    ) -> CatalogResult<u64> {
        let data_file_id = self.counters.alloc_file_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        let row = DataFileRow {
            data_file_id,
            table_id,
            path: path.to_string(),
            file_format: file_format.to_string(),
            row_count,
            file_size_bytes,
            snapshot_id,
            footer_size: None,
            encryption_key: None,
            begin_snapshot: Some(snapshot_id),
            end_snapshot: None,
        };

        let key = keys::key_data_file(table_id, data_file_id);
        self.stage(key, values::encode_value(&row));
        Ok(data_file_id)
    }

    pub async fn register_delete_file(
        &mut self,
        data_file_id: u64,
        path: &str,
        row_count: u64,
        file_size_bytes: u64,
    ) -> CatalogResult<u64> {
        let delete_file_id = self.counters.alloc_file_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        let row = DeleteFileRow {
            delete_file_id,
            data_file_id,
            path: path.to_string(),
            row_count,
            file_size_bytes,
            snapshot_id,
        };

        let key = keys::key_delete_file(data_file_id, delete_file_id);
        self.stage(key, values::encode_value(&row));
        Ok(delete_file_id)
    }

    pub async fn register_inlined_insert(
        &mut self,
        table_id: u64,
        schema_version: u64,
        row_id: u64,
        payload: Vec<u8>,
    ) -> CatalogResult<()> {
        let encoded_size = payload.len() + 100;
        if encoded_size > MAX_INLINED_VALUE_SIZE {
            return Err(CatalogError::ValueTooLarge { size: encoded_size });
        }

        let snapshot_id = self.counters.peek_snapshot_id();
        let row = InlinedInsertRow {
            table_id,
            schema_version,
            row_id,
            payload,
            begin_snapshot: snapshot_id,
            end_snapshot: None,
        };

        let key = keys::key_inlined_insert(table_id, schema_version, row_id);
        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    pub async fn mark_inlined_insert_deleted(
        &mut self,
        table_id: u64,
        schema_version: u64,
        row_id: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_inlined_insert(table_id, schema_version, row_id);

        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("inlined row {row_id}")))?;

        let mut row: InlinedInsertRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    pub async fn register_inlined_delete(
        &mut self,
        table_id: u64,
        data_file_id: u64,
        row_id: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let row = InlinedDeleteRow {
            table_id,
            data_file_id,
            row_id,
            begin_snapshot: snapshot_id,
        };

        let key = keys::key_inlined_delete(table_id, data_file_id, row_id);
        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    pub async fn update_table_stats(
        &mut self,
        table_id: u64,
        row_count: u64,
        file_count: u64,
        total_size_bytes: u64,
    ) -> CatalogResult<()> {
        let row = TableStatsRow {
            table_id,
            row_count,
            file_count,
            total_size_bytes,
        };
        let key = keys::key_table_stats(table_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_file_column_stats(
        &mut self,
        table_id: u64,
        column_id: u64,
        data_file_id: u64,
        has_null: bool,
        min_value: Option<&str>,
        max_value: Option<&str>,
        contains_nan: bool,
    ) -> CatalogResult<()> {
        let row = FileColumnStatsRow {
            table_id,
            column_id,
            data_file_id,
            has_null,
            min_value: min_value.map(|s| s.to_string()),
            max_value: max_value.map(|s| s.to_string()),
            contains_nan,
        };
        let key = keys::key_file_column_stats(table_id, column_id, data_file_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    /// Commit all staged mutations, counter updates, and the snapshot row in a
    /// single atomic SlateDB transaction.
    ///
    /// This is the **only** method that writes MVCC-versioned rows to SlateDB.
    /// Every staging method (`create_schema`, `create_table`, `add_column`,
    /// etc.) merely buffers the write; `create_snapshot()` is the sole commit
    /// boundary.
    #[tracing::instrument(skip(self, author, message))]
    pub async fn create_snapshot(
        &mut self,
        author: Option<&str>,
        message: Option<&str>,
    ) -> CatalogResult<SnapshotId> {
        let snapshot_id = self.counters.alloc_snapshot_id();

        if self.schema_changed {
            self.current_schema_version += 1;
            self.schema_changed = false;
        }

        let row = SnapshotRow {
            snapshot_id,
            schema_version: self.current_schema_version,
            snapshot_time: chrono::Utc::now().to_rfc3339(),
            author: author.map(|s| s.to_string()),
            message: message.map(|s| s.to_string()),
        };

        // Drain staged mutations — these become part of the atomic commit.
        let staged = std::mem::take(&mut self.staged);

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

        Ok(SnapshotId::new(snapshot_id))
    }

    // ─── Phase 6: Views ────────────────────────────────────────────────────

    pub async fn create_view(
        &mut self,
        schema_id: u64,
        view_name: &str,
        sql: &str,
    ) -> CatalogResult<u64> {
        let view_id = self.counters.alloc_catalog_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        let row = ViewRow {
            view_id,
            schema_id,
            view_name: view_name.to_string(),
            sql: sql.to_string(),
            begin_snapshot: snapshot_id,
            end_snapshot: None,
        };

        let key = keys::key_view(schema_id, view_id, snapshot_id);
        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(view_id)
    }

    pub async fn drop_view(
        &mut self,
        schema_id: u64,
        view_id: u64,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_view(schema_id, view_id, begin_snapshot);

        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("view {view_id}")))?;

        let mut row: ViewRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(())
    }

    // ─── Phase 6: Macros ────────────────────────────────────────────────────

    pub async fn create_macro(
        &mut self,
        schema_id: u64,
        macro_name: &str,
        macro_type: &str,
    ) -> CatalogResult<u64> {
        let macro_id = self.counters.alloc_catalog_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        let row = MacroRow {
            macro_id,
            schema_id,
            macro_name: macro_name.to_string(),
            macro_type: macro_type.to_string(),
            begin_snapshot: snapshot_id,
            end_snapshot: None,
        };

        let key = keys::key_macro(schema_id, macro_id, snapshot_id);
        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(macro_id)
    }

    pub async fn drop_macro(
        &mut self,
        schema_id: u64,
        macro_id: u64,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_macro(schema_id, macro_id, begin_snapshot);

        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("macro {macro_id}")))?;

        let mut row: MacroRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(())
    }

    pub async fn add_macro_impl(&mut self, macro_id: u64, definition: &str) -> CatalogResult<u64> {
        let impl_id = self.counters.alloc_catalog_id();

        let row = MacroImplRow {
            impl_id,
            macro_id,
            definition: definition.to_string(),
        };

        let key = keys::key_macro_impl(macro_id, impl_id);
        self.stage(key, values::encode_value(&row));
        Ok(impl_id)
    }

    pub async fn add_macro_parameter(
        &mut self,
        macro_id: u64,
        impl_id: u64,
        column_id: u64,
        parameter_name: &str,
        parameter_type: &str,
        default_value: Option<&str>,
    ) -> CatalogResult<()> {
        let row = MacroParametersRow {
            macro_id,
            impl_id,
            column_id,
            parameter_name: parameter_name.to_string(),
            parameter_type: parameter_type.to_string(),
            default_value: default_value.map(|s| s.to_string()),
        };

        let key = keys::key_macro_parameters(macro_id, impl_id, column_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    // ─── Phase 6: Tags ──────────────────────────────────────────────────────

    pub async fn set_tag(
        &mut self,
        object_id: u64,
        tag_key: &str,
        tag_value: &str,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let tag_key_hash = hash_tag_key(tag_key);

        let row = TagRow {
            object_id,
            tag_key: tag_key.to_string(),
            tag_value: tag_value.to_string(),
            begin_snapshot: snapshot_id,
            end_snapshot: None,
        };

        let key = keys::key_tag(object_id, tag_key_hash, snapshot_id);
        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    pub async fn remove_tag(
        &mut self,
        object_id: u64,
        tag_key: &str,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let tag_key_hash = hash_tag_key(tag_key);
        let key = keys::key_tag(object_id, tag_key_hash, begin_snapshot);

        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("tag {tag_key} on {object_id}")))?;

        let mut row: TagRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    pub async fn set_column_tag(
        &mut self,
        table_id: u64,
        column_id: u64,
        tag_key: &str,
        tag_value: &str,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let tag_key_hash = hash_tag_key(tag_key);

        let row = ColumnTagRow {
            table_id,
            column_id,
            tag_key: tag_key.to_string(),
            tag_value: tag_value.to_string(),
            begin_snapshot: snapshot_id,
            end_snapshot: None,
        };

        let key = keys::key_column_tag(table_id, column_id, tag_key_hash, snapshot_id);
        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    pub async fn remove_column_tag(
        &mut self,
        table_id: u64,
        column_id: u64,
        tag_key: &str,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let tag_key_hash = hash_tag_key(tag_key);
        let key = keys::key_column_tag(table_id, column_id, tag_key_hash, begin_snapshot);

        let existing = self.db.get(&key).await?.ok_or_else(|| {
            CatalogError::NotFound(format!("column tag {tag_key} on {table_id}.{column_id}"))
        })?;

        let mut row: ColumnTagRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    // ─── Phase 6: File Variant Stats ────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_file_variant_stats(
        &mut self,
        table_id: u64,
        column_id: u64,
        variant_path: &str,
        data_file_id: u64,
        min_value: Option<&str>,
        max_value: Option<&str>,
    ) -> CatalogResult<()> {
        let variant_path_hash = hash_tag_key(variant_path);
        let row = FileVariantStatsRow {
            table_id,
            column_id,
            variant_path_hash,
            data_file_id,
            variant_path: variant_path.to_string(),
            min_value: min_value.map(|s| s.to_string()),
            max_value: max_value.map(|s| s.to_string()),
        };
        let key =
            keys::key_file_variant_stats(table_id, column_id, variant_path_hash, data_file_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    // ─── Phase 6: Files Scheduled for Deletion ──────────────────────────────

    pub async fn schedule_file_deletion(
        &mut self,
        data_file_id: u64,
        path: &str,
        file_type: &str,
    ) -> CatalogResult<()> {
        let schedule_start = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let row = FilesScheduledForDeletionRow {
            data_file_id,
            schedule_start,
            path: path.to_string(),
            file_type: file_type.to_string(),
        };

        let key = keys::key_files_scheduled_for_deletion(schedule_start, data_file_id);
        self.db.put(&key, values::encode_value(&row)).await?;
        Ok(())
    }

    pub async fn remove_scheduled_deletion(
        &mut self,
        schedule_start: u64,
        data_file_id: u64,
    ) -> CatalogResult<()> {
        let key = keys::key_files_scheduled_for_deletion(schedule_start, data_file_id);
        self.db.delete(&key).await?;
        Ok(())
    }

    // ─── v0.10: Application Metadata / Streaming Ingest ────────────────────

    /// Write a metadata key/value pair, staged for atomic commit with the next
    /// snapshot.  For the `Global` scope with application-namespace keys
    /// (`{app}.{instance}.{key}` format), the key is validated before staging.
    ///
    /// # Application Metadata Namespace
    ///
    /// Non-DuckDB application state stored in `ducklake_metadata` **must** use
    /// the dotted-prefix convention:
    /// ```text
    /// {application}.{instance}.{key}
    /// e.g. pg_tide.orders-to-lake.offset  →  "4782"
    /// ```
    /// Keys that contain a dot are treated as application-namespace keys and
    /// must contain **at least two dots** (three dot-separated parts).  Plain
    /// DuckDB system keys (no dots, e.g. `data_path`) are accepted without
    /// restriction.
    pub fn set_metadata(
        &mut self,
        scope: slateduck_core::keys::MetadataScope,
        scope_id: u64,
        key: &str,
        value: &str,
    ) -> CatalogResult<()> {
        validate_app_metadata_key(key)?;
        let row = MetadataRow {
            key: key.to_string(),
            value: value.to_string(),
        };
        let k = keys::key_metadata(scope, scope_id, key);
        self.stage(k, values::encode_value(&row));
        Ok(())
    }

    // ─── v0.11 IVM Writer Methods ──────────────────────────────────────────

    /// Create an incremental materialized view entry in the catalog.
    ///
    /// Stages a `MatviewRow`, a `MatviewDepRow` for each `base_table_ids` entry,
    /// and allocates IDs. Caller must call `create_snapshot()` to commit.
    /// Returns the new `matview_id`.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_matview(
        &mut self,
        schema_name: &str,
        name: &str,
        view_sql: &str,
        output_table_id: u64,
        shard_count: u32,
        freshness_target_ms: u32,
        base_table_ids: &[u64],
    ) -> CatalogResult<u64> {
        let matview_id = self.counters.alloc_catalog_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        // Reject duplicate name within the same schema.
        let name_check_key = keys::key_matview(matview_id, snapshot_id);
        // We stage with begin_snapshot = snapshot_id (set during create_snapshot).
        let row = MatviewRow {
            matview_id,
            name: name.to_string(),
            schema_name: schema_name.to_string(),
            view_sql: view_sql.to_string(),
            output_table_id,
            shard_count,
            freshness_target_ms,
            state_uri: String::new(),
            shard_key_column: String::new(),
            created_at_snapshot: snapshot_id,
            begin_snapshot: snapshot_id,
            end_snapshot: 0,
            status: MatviewStatus::Active as u32,
            encoding_version: 1,
            output_mode: 0,
            circuit_compilation_version: 0,
        };
        self.stage(name_check_key, values::encode_value(&row));

        // Stage dependency rows.
        for &base_table_id in base_table_ids {
            let dep = MatviewDepRow {
                matview_id,
                base_table_id,
                columns: Vec::new(),
                is_broadcast: false,
                begin_snapshot: snapshot_id,
                encoding_version: 1,
            };
            self.stage(
                keys::key_matview_dep(matview_id, base_table_id),
                values::encode_value(&dep),
            );
        }

        self.mark_schema_changed();
        Ok(matview_id)
    }

    /// Logically drop a matview by setting `end_snapshot` on its row.
    /// Stages the updated row; caller must call `create_snapshot()`.
    pub async fn drop_matview(
        &mut self,
        matview_id: u64,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_matview(matview_id, begin_snapshot);
        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("matview {matview_id}")))?;
        let mut row: MatviewRow = values::decode_value(&existing)?;
        row.end_snapshot = snapshot_id;
        row.status = MatviewStatus::Dropped as u32;
        self.stage(key, values::encode_value(&row));
        self.mark_schema_changed();
        Ok(())
    }

    /// Update the status of a matview (e.g. Active → Stale).
    /// Reads the current row and stages an updated copy.
    pub async fn set_matview_status(
        &mut self,
        matview_id: u64,
        begin_snapshot: u64,
        status: MatviewStatus,
    ) -> CatalogResult<()> {
        let key = keys::key_matview(matview_id, begin_snapshot);
        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("matview {matview_id}")))?;
        let mut row: MatviewRow = values::decode_value(&existing)?;
        row.status = status as u32;
        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    /// Append a checkpoint watermark for a (matview_id, shard_id) pair.
    /// `seq` must be strictly greater than any existing seq for this pair.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_matview_checkpoint(
        &mut self,
        matview_id: u64,
        shard_id: u32,
        seq: u64,
        last_input_snapshot: u64,
        last_output_snapshot: u64,
        frontier_time: u64,
        worker_id: &str,
    ) -> CatalogResult<()> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let row = MatviewCheckpointRow {
            matview_id,
            shard_id,
            seq,
            last_input_snapshot,
            last_output_snapshot,
            frontier_time,
            durable_at_unix_ms: now_ms,
            worker_id: worker_id.to_string(),
            encoding_version: 1,
        };
        self.stage(
            keys::key_matview_checkpoint(matview_id, shard_id, seq),
            values::encode_value(&row),
        );
        Ok(())
    }

    /// Claim a matview shard for exclusive processing via CAS.
    ///
    /// On success the shard row is updated with `owner_worker` and
    /// `lease_expires_unix_ms`.  The `generation` field is incremented.
    /// Returns [`ClaimOutcome`] to let the caller distinguish the three cases.
    pub async fn claim_matview_shard(
        &mut self,
        matview_id: u64,
        shard_id: u32,
        worker_id: &str,
        lease_duration_ms: u64,
        now_unix_ms: u64,
    ) -> CatalogResult<ClaimOutcome> {
        let key = keys::key_matview_shard(matview_id, shard_id);

        loop {
            let tx = self.begin_tx().await?;
            let current_val = tx
                .get(&key)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

            let (current_row, is_new) = if let Some(bytes) = &current_val {
                (values::decode_value::<MatviewShardRow>(bytes)?, false)
            } else {
                (
                    MatviewShardRow {
                        matview_id,
                        shard_id,
                        owner_worker: String::new(),
                        lease_expires_unix_ms: 0,
                        key_range_lo: Vec::new(),
                        key_range_hi: Vec::new(),
                        generation: 0,
                        encoding_version: 1,
                        last_input_snapshot: 0,
                    },
                    true,
                )
            };

            // Already owned by this worker?
            if current_row.owner_worker == worker_id {
                return Ok(ClaimOutcome::AlreadyOwned {
                    generation: current_row.generation,
                });
            }

            // Owned by another worker with an active lease?
            if !current_row.owner_worker.is_empty()
                && current_row.lease_expires_unix_ms > now_unix_ms
            {
                return Ok(ClaimOutcome::Contended {
                    current_owner: current_row.owner_worker.clone(),
                });
            }

            // Expired or unowned — try to acquire.
            let new_generation = current_row.generation + 1;
            let new_row = MatviewShardRow {
                matview_id,
                shard_id,
                owner_worker: worker_id.to_string(),
                lease_expires_unix_ms: now_unix_ms + lease_duration_ms,
                key_range_lo: current_row.key_range_lo.clone(),
                key_range_hi: current_row.key_range_hi.clone(),
                generation: new_generation,
                encoding_version: 1,
                last_input_snapshot: current_row.last_input_snapshot,
            };

            tx.put(&key, values::encode_value(&new_row))
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

            match tx.commit().await {
                Ok(_) => {
                    if !is_new {
                        // Sync the staged copy so the in-memory writer reflects reality.
                        self.stage(key.clone(), values::encode_value(&new_row));
                    }
                    return Ok(ClaimOutcome::Acquired {
                        generation: new_generation,
                        expires_unix_ms: now_unix_ms + lease_duration_ms,
                    });
                }
                Err(_) => {
                    // CAS conflict — loop and retry.
                    continue;
                }
            }
        }
    }

    /// Extend a shard lease using optimistic CAS.
    ///
    /// The `expected_generation` must match the stored value; if it does not
    /// the lease extension fails with [`CatalogError::GenerationMismatch`].
    pub async fn extend_matview_lease(
        &mut self,
        matview_id: u64,
        shard_id: u32,
        worker_id: &str,
        expected_generation: u64,
        new_expires_unix_ms: u64,
    ) -> CatalogResult<u64> {
        let key = keys::key_matview_shard(matview_id, shard_id);
        loop {
            let tx = self.begin_tx().await?;
            let bytes = tx
                .get(&key)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
                .ok_or_else(|| {
                    CatalogError::NotFound(format!("shard ({matview_id},{shard_id})"))
                })?;
            let mut row: MatviewShardRow = values::decode_value(&bytes)?;

            if row.generation != expected_generation {
                return Err(CatalogError::GenerationMismatch {
                    expected: expected_generation,
                    actual: row.generation,
                });
            }
            if row.owner_worker != worker_id {
                return Err(CatalogError::NotFound(format!(
                    "shard ({matview_id},{shard_id}) not owned by {worker_id}"
                )));
            }

            row.generation += 1;
            row.lease_expires_unix_ms = new_expires_unix_ms;
            tx.put(&key, values::encode_value(&row))
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            match tx.commit().await {
                Ok(_) => return Ok(row.generation),
                Err(_) => continue,
            }
        }
    }

    /// Release a matview shard lease idempotently.
    ///
    /// If the shard is not owned by `worker_id` (or does not exist) this is a no-op.
    pub async fn release_matview_lease(
        &mut self,
        matview_id: u64,
        shard_id: u32,
        worker_id: &str,
    ) -> CatalogResult<()> {
        let key = keys::key_matview_shard(matview_id, shard_id);
        loop {
            let tx = self.begin_tx().await?;
            let current = tx
                .get(&key)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            let Some(bytes) = current else {
                return Ok(());
            };
            let mut row: MatviewShardRow = values::decode_value(&bytes)?;
            if row.owner_worker != worker_id {
                return Ok(()); // idempotent
            }
            row.generation += 1;
            row.owner_worker = String::new();
            row.lease_expires_unix_ms = 0;
            tx.put(&key, values::encode_value(&row))
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            match tx.commit().await {
                Ok(_) => return Ok(()),
                Err(_) => continue,
            }
        }
    }

    // ─── v0.12 IVM Scale-Out Methods ───────────────────────────────────────

    /// Update a matview's `output_mode` field.
    ///
    /// 0 = Consistent (default): output snapshot waits for all shards.
    /// 1 = PerShard: shards publish independently.
    pub async fn set_matview_output_mode(
        &mut self,
        matview_id: u64,
        begin_snapshot: u64,
        output_mode: u32,
    ) -> CatalogResult<()> {
        let key = keys::key_matview(matview_id, begin_snapshot);
        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("matview {matview_id}")))?;
        let mut row: MatviewRow = values::decode_value(&existing)?;
        row.output_mode = output_mode;
        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    /// Update a matview's `circuit_compilation_version`.
    ///
    /// Bump this value when the view SQL changes in a backward-incompatible way.
    /// Workers that see a mismatch between the persisted state version and the
    /// current value will trigger a full rebuild of their shard.
    pub async fn bump_circuit_compilation_version(
        &mut self,
        matview_id: u64,
        begin_snapshot: u64,
    ) -> CatalogResult<u64> {
        let key = keys::key_matview(matview_id, begin_snapshot);
        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("matview {matview_id}")))?;
        let mut row: MatviewRow = values::decode_value(&existing)?;
        row.circuit_compilation_version += 1;
        let version = row.circuit_compilation_version;
        self.stage(key, values::encode_value(&row));
        Ok(version)
    }

    /// Initiate a re-sharding operation by staging a new `MatviewRow` with
    /// `shard_count = new_shard_count` and `status = Rebuilding`.
    ///
    /// The old row (at `begin_snapshot`) has its `end_snapshot` set so that
    /// readers see the view as entering the `Rebuilding` state.  New shard rows
    /// are populated by the IVM worker once this snapshot commits.
    /// Caller must call `create_snapshot()` to commit.
    pub async fn re_shard_matview(
        &mut self,
        matview_id: u64,
        begin_snapshot: u64,
        new_shard_count: u32,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_matview(matview_id, begin_snapshot);
        let existing = self
            .db
            .get(&key)
            .await?
            .ok_or_else(|| CatalogError::NotFound(format!("matview {matview_id}")))?;
        let mut row: MatviewRow = values::decode_value(&existing)?;
        // Close the old row.
        row.end_snapshot = snapshot_id;
        self.stage(key, values::encode_value(&row));

        // Open a new row with the updated shard count and Rebuilding status.
        let new_row = MatviewRow {
            shard_count: new_shard_count,
            status: MatviewStatus::Rebuilding as u32,
            begin_snapshot: snapshot_id,
            end_snapshot: 0,
            circuit_compilation_version: row.circuit_compilation_version,
            ..row
        };
        self.stage(
            keys::key_matview(matview_id, snapshot_id),
            values::encode_value(&new_row),
        );
        self.mark_schema_changed();
        Ok(())
    }

    /// Update the `last_input_snapshot` field on a shard row (no-CAS version,
    /// for heartbeat/checkpoint updates where the worker already holds the lease).
    pub async fn update_shard_last_input_snapshot(
        &mut self,
        matview_id: u64,
        shard_id: u32,
        last_input_snapshot: u64,
    ) -> CatalogResult<()> {
        let key = keys::key_matview_shard(matview_id, shard_id);
        loop {
            let tx = self.begin_tx().await?;
            let bytes = tx
                .get(&key)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
                .ok_or_else(|| {
                    CatalogError::NotFound(format!("shard ({matview_id},{shard_id})"))
                })?;
            let mut row: MatviewShardRow = values::decode_value(&bytes)?;
            row.last_input_snapshot = last_input_snapshot;
            tx.put(&key, values::encode_value(&row))
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            match tx.commit().await {
                Ok(_) => return Ok(()),
                Err(_) => continue,
            }
        }
    }

    // ─── v0.18: Rowid Range Allocation ────────────────────────────────────

    /// Allocate a range of row IDs for a table.
    ///
    /// Returns `(start_rowid, end_rowid)` where the range is `[start, end)`.
    /// The counter at key `0xFE | 0x11 | table_id` is atomically advanced.
    ///
    /// v0.19: Uses `checked_add`; rejects `count == 0` and overflow.
    pub async fn next_rowid_range(&self, table_id: u64, count: u64) -> CatalogResult<(u64, u64)> {
        if count == 0 {
            return Err(CatalogError::InvalidInput(
                "rowid range count must be > 0".to_string(),
            ));
        }
        let key = keys::key_counter_rowid(table_id);
        loop {
            let tx = self.begin_tx().await?;
            let current = match tx
                .get(&key)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                Some(data) => values::decode_counter(&data)?,
                None => 0,
            };
            let start = current;
            let end = current.checked_add(count).ok_or_else(|| {
                CatalogError::InvalidInput(format!(
                    "rowid overflow: {current} + {count} exceeds u64::MAX"
                ))
            })?;
            tx.put(&key, values::encode_counter(end))
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            match tx.commit().await {
                Ok(_) => return Ok((start, end)),
                Err(_) => continue, // Retry on contention
            }
        }
    }

    // ─── Internal Helpers ───────────────────────────────────────────────────

    async fn begin_tx(&self) -> CatalogResult<DbTransaction> {
        self.db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))
    }

    async fn check_epoch(&self, tx: &DbTransaction) -> CatalogResult<()> {
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
            // v0.19: Missing epoch key means corruption or concurrent deletion — fail closed.
            None => {
                return Err(CatalogError::WriterEpochMismatch);
            }
        }
        Ok(())
    }
}

/// Hash a tag key string to u64 for key encoding.
fn hash_tag_key(key: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

/// Validate an application metadata key.
///
/// Keys without dots (DuckDB system keys like `data_path`) are always accepted.
/// Keys with dots must follow the `{app}.{instance}.{key}` convention — at least
/// three dot-separated, non-empty parts — and must not be empty.
pub fn validate_app_metadata_key(key: &str) -> CatalogResult<()> {
    if key.is_empty() {
        return Err(CatalogError::InvalidInput(
            "metadata key must not be empty".to_string(),
        ));
    }
    if key.contains('.') {
        let parts: Vec<&str> = key.splitn(4, '.').collect();
        if parts.len() < 3 || parts.iter().any(|p| p.is_empty()) {
            return Err(CatalogError::InvalidInput(format!(
                "application metadata key must follow {{app}}.{{instance}}.{{key}} \
                 convention (at least 3 non-empty dot-separated parts); got {:?}",
                key
            )));
        }
    }
    Ok(())
}

/// Allocate a range of row IDs for a table (standalone version).
///
/// Returns `(start_rowid, end_rowid)` where the range is `[start, end)`.
/// Uses serializable snapshot transactions for atomicity.
///
/// v0.19: Uses `checked_add`; rejects `count == 0` and overflow.
pub async fn next_rowid_range(db: &Db, table_id: u64, count: u64) -> CatalogResult<(u64, u64)> {
    if count == 0 {
        return Err(CatalogError::InvalidInput(
            "rowid range count must be > 0".to_string(),
        ));
    }
    let key = keys::key_counter_rowid(table_id);
    loop {
        let tx = db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        let current = match tx
            .get(&key)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            Some(data) => values::decode_counter(&data)?,
            None => 0,
        };
        let start = current;
        let end = current.checked_add(count).ok_or_else(|| {
            CatalogError::InvalidInput(format!(
                "rowid overflow: {current} + {count} exceeds u64::MAX"
            ))
        })?;
        tx.put(&key, values::encode_counter(end))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        match tx.commit().await {
            Ok(_) => return Ok((start, end)),
            Err(_) => continue,
        }
    }
}
