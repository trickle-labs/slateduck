//! CatalogWriter: write operations that produce new snapshots.
//!
//! # Staging model (v0.9.1)
//!
//! All MVCC-versioned catalog row mutations are **staged in memory** until
//! `create_snapshot()` is called, which commits every staged write plus all
//! counter updates and the snapshot row in **one atomic SlateDB transaction**.
//! A crash before `create_snapshot()` leaves the catalog unchanged.
//!
//! Non-MVCC ancillary writes (`update_table_stats`, `upsert_file_column_stats`,
//! `upsert_file_variant_stats`, `add_macro_parameter`,
//! `schedule_file_deletion`, `remove_scheduled_deletion`) are written directly
//! and are safe to write outside the snapshot boundary.

use slatedb::{Db, DbTransaction, IsolationLevel};
use slateduck_core::counters::CounterCache;
use slateduck_core::keys;
use slateduck_core::mvcc::SnapshotId;
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::values::{self, MAX_INLINED_VALUE_SIZE};

use crate::error::{CatalogError, CatalogResult};

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

    /// Scan all table rows to find the `schema_id` that owns `table_id`.
    ///
    /// Returns `None` if no live (end_snapshot IS NULL) table row is found.
    /// Used by the PG-Wire executor to resolve `UPDATE end_snapshot` on
    /// `ducklake_table` without a hard-coded `schema_id = 0`.
    pub async fn find_table_schema_id(&self, table_id: u64) -> CatalogResult<Option<u64>> {
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

    // ─── Internal Helpers ───────────────────────────────────────────────────

    async fn begin_tx(&self) -> CatalogResult<DbTransaction> {
        self.db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))
    }

    async fn check_epoch(&self, tx: &DbTransaction) -> CatalogResult<()> {
        let epoch_key = keys::key_system(SYSTEM_WRITER_EPOCH);
        if let Some(data) = tx
            .get(&epoch_key)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let stored_epoch = values::decode_counter(&data)?;
            if stored_epoch != self.writer_epoch {
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
