//! CatalogWriter: write operations that produce new snapshots.
//!
//! This module is decomposed into sub-modules by concern:
//! - `stats`: file column stats, file variant stats, table stats
//! - `snapshot`: create_snapshot and transaction helpers
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

pub mod snapshot;
pub mod stats;

use slatedb::{Db, IsolationLevel};
use slateduck_core::counters::CounterCache;
use slateduck_core::keys;
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
            schema_uuid: Some(uuid::Uuid::new_v4().to_string()),
            path: None,
            path_is_relative: None,
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
            path: data_path.map(|s| s.to_string()),
            table_uuid: Some(uuid::Uuid::new_v4().to_string()),
            path_is_relative: None,
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
    ///
    /// v0.24: CASCADE retirement — retires all dependent spec rows (columns,
    /// column tags, data files, delete files, tags, partition info) in the
    /// same snapshot transaction.
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

        // CASCADE: retire live columns for this table.
        {
            let prefix = keys::prefix_columns_for_table(table_id);
            let mut iter = self
                .db
                .scan_prefix(&prefix)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            let mut to_retire: Vec<(Vec<u8>, ColumnRow)> = Vec::new();
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let col_row: ColumnRow = values::decode_value(&kv.value)?;
                if col_row.end_snapshot.is_none() {
                    to_retire.push((kv.key.to_vec(), col_row));
                }
            }
            for (col_key, mut col_row) in to_retire {
                col_row.end_snapshot = Some(snapshot_id);
                self.stage(col_key, values::encode_value(&col_row));
            }
        }

        // CASCADE: retire live data files for this table.
        {
            let prefix = keys::prefix_for_tag(slateduck_core::tags::TAG_DATA_FILE);
            let mut iter = self
                .db
                .scan_prefix(&prefix)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            let mut to_retire: Vec<(Vec<u8>, DataFileRow)> = Vec::new();
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let df_row: DataFileRow = values::decode_value(&kv.value)?;
                if df_row.table_id == table_id && df_row.end_snapshot.is_none() {
                    to_retire.push((kv.key.to_vec(), df_row));
                }
            }
            for (df_key, mut df_row) in to_retire {
                let file_begin_snap = df_row.begin_snapshot.unwrap_or(0);
                let file_id = df_row.data_file_id;
                df_row.end_snapshot = Some(snapshot_id);
                let encoded = values::encode_value(&df_row);
                // Update canonical key.
                self.stage(df_key, encoded.clone());
                // Also update the secondary index (TAG_DATA_FILE_BY_SNAPSHOT) so
                // list_data_files() sees the retirement via its secondary-index scan.
                let idx_key = keys::key_data_file_by_snapshot(table_id, file_begin_snap, file_id);
                self.stage(idx_key, encoded);
            }
        }

        // CASCADE: retire live partition info for this table.
        {
            let prefix = keys::prefix_for_tag(slateduck_core::tags::TAG_PARTITION_INFO);
            let mut iter = self
                .db
                .scan_prefix(&prefix)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            let mut to_retire: Vec<(Vec<u8>, PartitionInfoRow)> = Vec::new();
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let pi_row: PartitionInfoRow = values::decode_value(&kv.value)?;
                if pi_row.table_id == table_id && pi_row.end_snapshot.is_none() {
                    to_retire.push((kv.key.to_vec(), pi_row));
                }
            }
            for (pi_key, mut pi_row) in to_retire {
                pi_row.end_snapshot = Some(snapshot_id);
                self.stage(pi_key, values::encode_value(&pi_row));
            }
        }

        // CASCADE: retire live tags for this table.
        {
            let prefix = keys::prefix_for_tag(slateduck_core::tags::TAG_TAG);
            let mut iter = self
                .db
                .scan_prefix(&prefix)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            let mut to_retire: Vec<(Vec<u8>, TagRow)> = Vec::new();
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let tag_row: TagRow = values::decode_value(&kv.value)?;
                if tag_row.object_id == table_id && tag_row.end_snapshot.is_none() {
                    to_retire.push((kv.key.to_vec(), tag_row));
                }
            }
            for (tag_key, mut tag_row) in to_retire {
                tag_row.end_snapshot = Some(snapshot_id);
                self.stage(tag_key, values::encode_value(&tag_row));
            }
        }

        // CASCADE: retire live column tags for this table.
        {
            let prefix = keys::prefix_for_tag(slateduck_core::tags::TAG_COLUMN_TAG);
            let mut iter = self
                .db
                .scan_prefix(&prefix)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            let mut to_retire: Vec<(Vec<u8>, ColumnTagRow)> = Vec::new();
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let ct_row: ColumnTagRow = values::decode_value(&kv.value)?;
                if ct_row.table_id == table_id && ct_row.end_snapshot.is_none() {
                    to_retire.push((kv.key.to_vec(), ct_row));
                }
            }
            for (ct_key, mut ct_row) in to_retire {
                ct_row.end_snapshot = Some(snapshot_id);
                self.stage(ct_key, values::encode_value(&ct_row));
            }
        }

        // CASCADE: retire live sort info for this table (v0.27).
        {
            let prefix = keys::prefix_for_tag(slateduck_core::tags::TAG_SORT_INFO);
            let mut iter = self
                .db
                .scan_prefix(&prefix)
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
            let mut to_retire: Vec<(Vec<u8>, SortInfoRow)> = Vec::new();
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let si_row: SortInfoRow = values::decode_value(&kv.value)?;
                if si_row.table_id == table_id && si_row.end_snapshot.is_none() {
                    to_retire.push((kv.key.to_vec(), si_row));
                }
            }
            for (si_key, mut si_row) in to_retire {
                si_row.end_snapshot = Some(snapshot_id);
                self.stage(si_key, values::encode_value(&si_row));
            }
        }

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
        self.add_column_with_opts(
            table_id,
            column_name,
            data_type,
            column_index,
            is_nullable,
            default_value,
            None,
            None,
            None,
            None,
        )
        .await
    }

    /// Extended add_column supporting v0.25 nested column model and default type fields.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_column_with_opts(
        &mut self,
        table_id: u64,
        column_name: &str,
        data_type: &str,
        column_index: u64,
        is_nullable: bool,
        default_value: Option<&str>,
        initial_default: Option<&str>,
        default_value_type: Option<&str>,
        default_value_dialect: Option<&str>,
        parent_column: Option<u64>,
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
            initial_default: initial_default.map(|s| s.to_string()),
            default_value_type: default_value_type.map(|s| s.to_string()),
            default_value_dialect: default_value_dialect.map(|s| s.to_string()),
            parent_column,
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
        record_count: u64,
        file_size_bytes: u64,
    ) -> CatalogResult<u64> {
        let data_file_id = self.counters.alloc_file_id();
        let snapshot_id = self.counters.peek_snapshot_id();
        // v0.24: assign file_order as monotonically increasing within table.
        let file_order = data_file_id;
        // v0.24: row_id_start from pre-increment of table next_row_id.
        let row_id_start = {
            let key = keys::key_table_stats(table_id);
            match self.db.get(&key).await? {
                Some(data) => {
                    let existing: slateduck_core::rows::TableStatsRow =
                        slateduck_core::values::decode_value(&data).unwrap_or_default();
                    existing.next_row_id.unwrap_or(0)
                }
                None => 0,
            }
        };

        let row = DataFileRow {
            data_file_id,
            table_id,
            path: path.to_string(),
            file_format: file_format.to_string(),
            record_count,
            file_size_bytes,
            footer_size: None,
            encryption_key: None,
            begin_snapshot: Some(snapshot_id),
            end_snapshot: None,
            file_order: Some(file_order),
            path_is_relative: Some(false),
            row_id_start: Some(row_id_start),
            partition_id: None,
            mapping_id: None,
            partial_max: None,
        };

        let key = keys::key_data_file(table_id, data_file_id);
        let encoded = values::encode_value(&row);
        // Also write the secondary index entry for O(log N) snapshot-bounded scans.
        let idx_key = keys::key_data_file_by_snapshot(table_id, snapshot_id, data_file_id);
        self.stage(key, encoded.clone());
        self.stage(idx_key, encoded);
        Ok(data_file_id)
    }

    /// v0.26: Register a data file with a `partial_max` upper-bound value.
    ///
    /// Used for partial files (e.g., in-flight appends) where the max value in the
    /// column is known at write time and can be used for zone-map pruning.
    pub async fn register_data_file_partial(
        &mut self,
        table_id: u64,
        path: &str,
        file_format: &str,
        record_count: u64,
        file_size_bytes: u64,
        partial_max: Option<&str>,
    ) -> CatalogResult<u64> {
        let data_file_id = self.counters.alloc_file_id();
        let snapshot_id = self.counters.peek_snapshot_id();
        let file_order = data_file_id;
        let row_id_start = {
            let key = keys::key_table_stats(table_id);
            match self.db.get(&key).await? {
                Some(data) => {
                    let existing: slateduck_core::rows::TableStatsRow =
                        slateduck_core::values::decode_value(&data).unwrap_or_default();
                    existing.next_row_id.unwrap_or(0)
                }
                None => 0,
            }
        };

        let row = DataFileRow {
            data_file_id,
            table_id,
            path: path.to_string(),
            file_format: file_format.to_string(),
            record_count,
            file_size_bytes,
            footer_size: None,
            encryption_key: None,
            begin_snapshot: Some(snapshot_id),
            end_snapshot: None,
            file_order: Some(file_order),
            path_is_relative: Some(false),
            row_id_start: Some(row_id_start),
            partition_id: None,
            mapping_id: None,
            partial_max: partial_max.map(|s| s.to_string()),
        };

        let key = keys::key_data_file(table_id, data_file_id);
        let encoded = values::encode_value(&row);
        let idx_key = keys::key_data_file_by_snapshot(table_id, snapshot_id, data_file_id);
        self.stage(key, encoded.clone());
        self.stage(idx_key, encoded);
        Ok(data_file_id)
    }

    pub async fn register_delete_file(
        &mut self,
        data_file_id: u64,
        path: &str,
        delete_count: u64,
        file_size_bytes: u64,
    ) -> CatalogResult<u64> {
        let delete_file_id = self.counters.alloc_file_id();
        let snapshot_id = self.counters.peek_snapshot_id();

        let row = DeleteFileRow {
            delete_file_id,
            data_file_id,
            path: path.to_string(),
            delete_count,
            file_size_bytes,
            snapshot_id,
            table_id: None,
            begin_snapshot: Some(snapshot_id),
            end_snapshot: None,
            path_is_relative: Some(false),
            format: Some("parquet".to_string()),
            footer_size: None,
            partial_max: None,
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

    // ─── Phase 6: Views ────────────────────────────────────────────────────

    pub async fn create_view(
        &mut self,
        schema_id: u64,
        view_name: &str,
        sql: &str,
    ) -> CatalogResult<u64> {
        self.create_view_with_opts(schema_id, view_name, sql, None, None, None)
            .await
    }

    /// Extended create_view supporting v0.25 UUID and dialect fields.
    pub async fn create_view_with_opts(
        &mut self,
        schema_id: u64,
        view_name: &str,
        sql: &str,
        view_uuid: Option<&str>,
        dialect: Option<&str>,
        column_aliases: Option<&str>,
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
            view_uuid: view_uuid
                .map(|s| s.to_string())
                .or_else(|| Some(uuid::Uuid::new_v4().to_string())),
            dialect: dialect.map(|s| s.to_string()),
            column_aliases: column_aliases.map(|s| s.to_string()),
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
            macro_uuid: Some(uuid::Uuid::new_v4().to_string()),
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
        self.add_macro_impl_with_opts(macro_id, definition, None, None)
            .await
    }

    /// Extended add_macro_impl supporting v0.25 dialect and type fields.
    pub async fn add_macro_impl_with_opts(
        &mut self,
        macro_id: u64,
        sql: &str,
        dialect: Option<&str>,
        impl_type: Option<&str>,
    ) -> CatalogResult<u64> {
        let impl_id = self.counters.alloc_catalog_id();

        let row = MacroImplRow {
            impl_id,
            macro_id,
            sql: sql.to_string(),
            dialect: dialect.map(|s| s.to_string()),
            impl_type: impl_type.map(|s| s.to_string()),
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
        self.add_macro_parameter_with_opts(
            macro_id,
            impl_id,
            column_id,
            parameter_name,
            parameter_type,
            default_value,
            None,
        )
        .await
    }

    /// Extended add_macro_parameter supporting v0.25 default_value_type.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_macro_parameter_with_opts(
        &mut self,
        macro_id: u64,
        impl_id: u64,
        column_id: u64,
        parameter_name: &str,
        parameter_type: &str,
        default_value: Option<&str>,
        default_value_type: Option<&str>,
    ) -> CatalogResult<()> {
        let row = MacroParametersRow {
            macro_id,
            impl_id,
            column_id,
            parameter_name: parameter_name.to_string(),
            parameter_type: parameter_type.to_string(),
            default_value: default_value.map(|s| s.to_string()),
            default_value_type: default_value_type.map(|s| s.to_string()),
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

    // ─── v0.27: Sort Info ────────────────────────────────────────────────────

    /// Write a `ducklake_sort_info` row for this table.
    ///
    /// Stores an individual MVCC entry under `key_sort_info`, which `list_all_sort_info`
    /// can scan.  The sort_id must be caller-assigned and unique within the table.
    pub async fn add_sort_info(&mut self, table_id: u64, sort_id: u64) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let row = SortInfoRow {
            sort_id,
            table_id,
            begin_snapshot: snapshot_id,
            end_snapshot: None,
        };
        let key = keys::key_sort_info(table_id, sort_id, snapshot_id);
        self.stage(key, values::encode_value(&row));
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
            file_type: if file_type.is_empty() {
                None
            } else {
                Some(file_type.to_string())
            },
            path_is_relative: None,
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
        let scope_str = match scope {
            slateduck_core::keys::MetadataScope::Global => "global",
            slateduck_core::keys::MetadataScope::Schema => "schema",
            slateduck_core::keys::MetadataScope::Table => "table",
        };
        let row = MetadataRow {
            key: key.to_string(),
            value: value.to_string(),
            scope: Some(scope_str.to_string()),
            scope_id: Some(scope_id),
        };
        let k = keys::key_metadata(scope, scope_id, key);
        self.stage(k, values::encode_value(&row));
        Ok(())
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

    // ─── v0.24: SnapshotChanges and delta stats ────────────────────────────

    /// Stage a `SnapshotChangesRow` for this transaction.
    ///
    /// The key is `key_snapshot_changes(pending_snapshot_id)`. The row is
    /// committed atomically with the snapshot in `create_snapshot()`.
    pub async fn add_snapshot_changes(
        &mut self,
        change_type: String,
        change_info: Option<String>,
        schema_id: Option<u64>,
        table_id: Option<u64>,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let row = SnapshotChangesRow {
            snapshot_id,
            change_type,
            change_info,
            schema_id,
            table_id,
            author: None,
            commit_message: None,
            commit_extra_info: None,
            changes_made: None,
        };
        let key = keys::key_snapshot_changes(snapshot_id);
        self.stage(key, values::encode_value(&row));
        Ok(())
    }

    /// Apply a row-count delta to existing table stats.
    ///
    /// Reads the current stats, adds `row_count_delta`, and writes back.
    /// Used by `UpdateTableStats` PgWire op (delete operations).
    pub async fn apply_table_stats_delta(
        &mut self,
        table_id: u64,
        row_count_delta: i64,
    ) -> CatalogResult<()> {
        let key = keys::key_table_stats(table_id);
        let existing = match self.db.get(&key).await? {
            Some(data) => slateduck_core::values::decode_value::<TableStatsRow>(&data).unwrap_or(
                TableStatsRow {
                    table_id,
                    record_count: 0,
                    file_count: 0,
                    file_size_bytes: 0,
                    next_row_id: None,
                },
            ),
            None => TableStatsRow {
                table_id,
                record_count: 0,
                file_count: 0,
                file_size_bytes: 0,
                next_row_id: None,
            },
        };
        let new_count = if row_count_delta < 0 {
            existing
                .record_count
                .saturating_sub((-row_count_delta) as u64)
        } else {
            existing.record_count.saturating_add(row_count_delta as u64)
        };
        let updated = TableStatsRow {
            table_id,
            record_count: new_count,
            file_count: existing.file_count,
            file_size_bytes: existing.file_size_bytes,
            next_row_id: existing.next_row_id,
        };
        self.db.put(&key, values::encode_value(&updated)).await?;
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
