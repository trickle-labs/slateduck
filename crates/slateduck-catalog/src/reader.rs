//! CatalogReader: read catalog state at a specific DuckLake snapshot.

use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::mvcc::{self, SnapshotId};
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::types::{DuckLakeType, PruneResult};
use slateduck_core::values;

use crate::error::{CatalogError, CatalogResult};

// ─── v0.10: Snapshot Diff ──────────────────────────────────────────────────

/// Structured diff between two DuckLake snapshots.
///
/// Contains the sets of catalog facts that were added or retired in the
/// transition from `from_snapshot` to `to_snapshot`.  This is the primary
/// primitive for CDC export: every committed snapshot is a natural change
/// stream.
#[derive(Debug, Clone)]
pub struct SnapshotDiff {
    pub from_snapshot: SnapshotId,
    pub to_snapshot: SnapshotId,
    /// Schema rows first written at `to_snapshot`.
    pub added_schemas: Vec<SchemaRow>,
    /// Schema rows retired at `to_snapshot`.
    pub retired_schemas: Vec<SchemaRow>,
    /// Table rows first written at `to_snapshot`.
    pub added_tables: Vec<TableRow>,
    /// Table rows retired at `to_snapshot`.
    pub retired_tables: Vec<TableRow>,
    /// Column rows first written at `to_snapshot`.
    pub added_columns: Vec<ColumnRow>,
    /// Column rows retired at `to_snapshot`.
    pub retired_columns: Vec<ColumnRow>,
    /// Data files registered in the `(from_snapshot, to_snapshot]` window.
    pub added_data_files: Vec<DataFileRow>,
    /// Data files logically deleted/replaced in the `(from_snapshot, to_snapshot]` window.
    pub retired_data_files: Vec<DataFileRow>,
}

impl SnapshotDiff {
    /// Returns true if there are no changes between the two snapshots.
    pub fn is_empty(&self) -> bool {
        self.added_schemas.is_empty()
            && self.retired_schemas.is_empty()
            && self.added_tables.is_empty()
            && self.retired_tables.is_empty()
            && self.added_columns.is_empty()
            && self.retired_columns.is_empty()
            && self.added_data_files.is_empty()
            && self.retired_data_files.is_empty()
    }

    /// Total number of changed facts.
    pub fn change_count(&self) -> usize {
        self.added_schemas.len()
            + self.retired_schemas.len()
            + self.added_tables.len()
            + self.retired_tables.len()
            + self.added_columns.len()
            + self.retired_columns.len()
            + self.added_data_files.len()
            + self.retired_data_files.len()
    }
}

/// Reads catalog state at a specific DuckLake snapshot ID.
pub struct CatalogReader {
    db: Db,
    dl_snapshot_id: SnapshotId,
}

impl CatalogReader {
    pub(crate) fn new(db: Db, dl_snapshot_id: SnapshotId) -> Self {
        Self { db, dl_snapshot_id }
    }

    pub fn snapshot_id(&self) -> SnapshotId {
        self.dl_snapshot_id
    }

    pub async fn get_snapshot(&self) -> CatalogResult<Option<SnapshotRow>> {
        let key = keys::key_snapshot(self.dl_snapshot_id.as_u64());
        match self.db.get(&key).await? {
            None => Ok(None),
            Some(data) => Ok(Some(values::decode_value::<SnapshotRow>(&data)?)),
        }
    }

    pub async fn list_schemas(&self) -> CatalogResult<Vec<SchemaRow>> {
        let prefix = keys::prefix_for_tag(TAG_SCHEMA);
        let mut schemas = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: SchemaRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                schemas.push(row);
            }
        }
        Ok(schemas)
    }

    pub async fn list_tables(&self, schema_id: u64) -> CatalogResult<Vec<TableRow>> {
        let prefix = keys::prefix_tables_for_schema(schema_id);
        let mut tables = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: TableRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                tables.push(row);
            }
        }
        Ok(tables)
    }

    pub async fn describe_table(
        &self,
        table_id: u64,
    ) -> CatalogResult<Option<(TableRow, Vec<ColumnRow>)>> {
        // O(1) secondary-index lookup: TAG_TABLE_BY_ID → schema_id.
        let idx_key = keys::key_table_by_id(table_id);
        let schema_id_opt = match self.db.get(&idx_key).await? {
            Some(data) => Some(values::decode_counter(&data)?),
            None => None,
        };

        let table_row: Option<TableRow> = if let Some(schema_id) = schema_id_opt {
            // Use the narrow schema+table prefix — O(log n) in practice.
            let prefix = keys::prefix_tables_for_schema_table(schema_id, table_id);
            let mut best: Option<TableRow> = None;
            let mut iter = self.db.scan_prefix(&prefix).await?;
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let row: TableRow = values::decode_value(&kv.value)?;
                if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                    match &best {
                        None => best = Some(row),
                        Some(existing) if row.begin_snapshot > existing.begin_snapshot => {
                            best = Some(row);
                        }
                        _ => {}
                    }
                }
            }
            best
        } else {
            // Fallback: full scan for catalogs predating the secondary index.
            let prefix = keys::prefix_for_tag(TAG_TABLE);
            let mut best: Option<TableRow> = None;
            let mut iter = self.db.scan_prefix(&prefix).await?;
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let row: TableRow = values::decode_value(&kv.value)?;
                if row.table_id == table_id
                    && mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id)
                {
                    match &best {
                        None => best = Some(row),
                        Some(existing) if row.begin_snapshot > existing.begin_snapshot => {
                            best = Some(row);
                        }
                        _ => {}
                    }
                }
            }
            best
        };

        let table = match table_row {
            None => return Ok(None),
            Some(t) => t,
        };

        let col_prefix = keys::prefix_columns_for_table(table_id);
        let mut columns = Vec::new();
        let mut iter = self.db.scan_prefix(&col_prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: ColumnRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                columns.push(row);
            }
        }

        columns.sort_by(|a, b| {
            a.column_id
                .cmp(&b.column_id)
                .then(b.begin_snapshot.cmp(&a.begin_snapshot))
        });
        columns.dedup_by_key(|c| c.column_id);
        // v0.26: sort into a column tree — top-level columns by column_index,
        // then child columns (parent_column IS NOT NULL) following their parent.
        sort_columns_tree(&mut columns);

        Ok(Some((table, columns)))
    }

    pub async fn list_data_files(&self, table_id: u64) -> CatalogResult<Vec<DataFileRow>> {
        // Use the secondary index TAG_DATA_FILE_BY_SNAPSHOT (0x21) for an
        // O(log N) range scan bounded by read_snapshot instead of scanning all
        // data files for the table and filtering in memory.
        let prefix = keys::prefix_data_files_by_snapshot_for_table(table_id);
        let upper =
            keys::prefix_data_files_by_snapshot_upper(table_id, self.dl_snapshot_id.as_u64());

        let mut files = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            // Stop once we exceed the upper bound (snapshot_id > read_snapshot).
            if let Some(ref upper_key) = upper {
                if kv.key.as_ref() >= upper_key.as_slice() {
                    break;
                }
            }
            let row: DataFileRow = values::decode_value(&kv.value)?;
            // v0.24: filter out rows retired at or before the requested snapshot.
            if let Some(end) = row.end_snapshot {
                if end <= self.dl_snapshot_id.as_u64() {
                    continue;
                }
            }
            files.push(row);
        }
        // v0.24: order results by file_order (spec requirement).
        files.sort_by_key(|f| f.file_order.unwrap_or(f.data_file_id));
        Ok(files)
    }

    /// List delete files visible at the current snapshot.
    ///
    /// v0.24: implements spec MVCC visibility: `begin_snapshot ≤ snapshot_id`
    /// and (`end_snapshot IS NULL` or `end_snapshot > snapshot_id`).
    pub async fn list_delete_files(&self, table_id: u64) -> CatalogResult<Vec<DeleteFileRow>> {
        use slateduck_core::tags::TAG_DELETE_FILE;
        let prefix = keys::prefix_for_tag(TAG_DELETE_FILE);
        let snap = self.dl_snapshot_id.as_u64();
        let mut files = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: DeleteFileRow = values::decode_value(&kv.value)?;
            // Filter by table_id if populated; fall back to data_file_id key for legacy rows.
            if let Some(tid) = row.table_id {
                if tid != table_id {
                    continue;
                }
            }
            // MVCC visibility using begin_snapshot / end_snapshot if present,
            // falling back to legacy snapshot_id.
            let begin = row.begin_snapshot.unwrap_or(row.snapshot_id);
            if begin > snap {
                continue;
            }
            if let Some(end) = row.end_snapshot {
                if end <= snap {
                    continue;
                }
            }
            files.push(row);
        }
        Ok(files)
    }

    pub async fn get_table_stats(&self, table_id: u64) -> CatalogResult<Option<TableStatsRow>> {
        let key = keys::key_table_stats(table_id);
        match self.db.get(&key).await? {
            Some(data) => Ok(Some(values::decode_value(&data)?)),
            None => Ok(None),
        }
    }

    pub async fn prune_files(
        &self,
        table_id: u64,
        column_id: u64,
        predicate_value: &str,
        col_type: &DuckLakeType,
    ) -> CatalogResult<Vec<u64>> {
        use slateduck_core::types::{prune_file, type_aware_compare};

        let mut buf = Vec::with_capacity(17);
        buf.push(TAG_FILE_COLUMN_STATS);
        buf.extend_from_slice(&keys::encode_u64(table_id));
        buf.extend_from_slice(&keys::encode_u64(column_id));

        let mut kept_file_ids = Vec::new();
        let mut iter = self.db.scan_prefix(&buf).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: FileColumnStatsRow = values::decode_value(&kv.value)?;
            let result = prune_file(
                predicate_value,
                row.min_value.as_deref(),
                row.max_value.as_deref(),
                row.contains_nan,
                col_type,
            )?;
            if result == PruneResult::Keep {
                kept_file_ids.push(row.data_file_id);
            }
        }

        // v0.26: partial_max pruning shortcut.
        // If a data file has partial_max IS NOT NULL and predicate > partial_max,
        // the file cannot contain matching rows — prune it.
        if !kept_file_ids.is_empty() {
            let data_files = self.list_data_files(table_id).await?;
            let partial_map: std::collections::HashMap<u64, &str> = data_files
                .iter()
                .filter_map(|f| f.partial_max.as_deref().map(|pm| (f.data_file_id, pm)))
                .collect();
            kept_file_ids.retain(|&file_id| {
                if let Some(partial_max) = partial_map.get(&file_id) {
                    // If predicate > partial_max, the file can be pruned.
                    match type_aware_compare(predicate_value, partial_max, col_type) {
                        Ok(std::cmp::Ordering::Greater) => false, // prune
                        _ => true,                                // keep
                    }
                } else {
                    true // no partial_max — keep
                }
            });
        }

        Ok(kept_file_ids)
    }

    pub async fn get_metadata(
        &self,
        scope: slateduck_core::keys::MetadataScope,
        scope_id: u64,
        key: &str,
    ) -> CatalogResult<Option<MetadataRow>> {
        let k = keys::key_metadata(scope, scope_id, key);
        match self.db.get(&k).await? {
            None => Ok(None),
            Some(data) => Ok(Some(values::decode_value::<MetadataRow>(&data)?)),
        }
    }

    pub async fn list_inlined_inserts(
        &self,
        table_id: u64,
    ) -> CatalogResult<Vec<InlinedInsertRow>> {
        let prefix = keys::prefix_inlined_inserts_for_table(table_id);
        let mut rows = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: InlinedInsertRow = values::decode_value(&kv.value)?;
            if mvcc::is_inlined_insert_visible(
                row.begin_snapshot,
                row.end_snapshot,
                self.dl_snapshot_id,
            ) {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    pub async fn list_inlined_deletes(
        &self,
        table_id: u64,
    ) -> CatalogResult<Vec<InlinedDeleteRow>> {
        let prefix = keys::prefix_inlined_deletes_for_table(table_id);
        let mut rows = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: InlinedDeleteRow = values::decode_value(&kv.value)?;
            if mvcc::is_inlined_delete_visible(row.begin_snapshot, self.dl_snapshot_id) {
                rows.push(row);
            }
        }
        Ok(rows)
    }

    // ─── Phase 6: Views ────────────────────────────────────────────────────

    pub async fn list_views(&self, schema_id: u64) -> CatalogResult<Vec<ViewRow>> {
        let prefix = keys::prefix_views_for_schema(schema_id);
        let mut views = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: ViewRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                views.push(row);
            }
        }
        Ok(views)
    }

    /// v0.25: List all visible views across all schemas.
    pub async fn list_all_views(&self) -> CatalogResult<Vec<ViewRow>> {
        let prefix = keys::prefix_for_tag(slateduck_core::tags::TAG_VIEW);
        let mut views = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: ViewRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                views.push(row);
            }
        }
        Ok(views)
    }

    // ─── Phase 6: Macros ────────────────────────────────────────────────────

    pub async fn list_macros(&self, schema_id: u64) -> CatalogResult<Vec<MacroRow>> {
        let prefix = keys::prefix_macros_for_schema(schema_id);
        let mut macros = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: MacroRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                macros.push(row);
            }
        }
        Ok(macros)
    }

    /// v0.25: List all visible macros across all schemas.
    pub async fn list_all_macros(&self) -> CatalogResult<Vec<MacroRow>> {
        let prefix = keys::prefix_for_tag(slateduck_core::tags::TAG_MACRO);
        let mut macros = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: MacroRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                macros.push(row);
            }
        }
        Ok(macros)
    }

    /// v0.25: List all metadata entries (all scopes) for the SQL facade.
    pub async fn list_all_metadata(&self) -> CatalogResult<Vec<MetadataRow>> {
        let prefix = keys::prefix_all_metadata();
        let mut rows = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: MetadataRow = values::decode_value(&kv.value)?;
            rows.push(row);
        }
        Ok(rows)
    }

    pub async fn list_macro_impls(&self, macro_id: u64) -> CatalogResult<Vec<MacroImplRow>> {
        let prefix = keys::prefix_macro_impls(macro_id);
        let mut impls = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: MacroImplRow = values::decode_value(&kv.value)?;
            impls.push(row);
        }
        Ok(impls)
    }

    pub async fn list_macro_parameters(
        &self,
        macro_id: u64,
        impl_id: u64,
    ) -> CatalogResult<Vec<MacroParametersRow>> {
        let prefix = keys::prefix_macro_params(macro_id, impl_id);
        let mut params = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: MacroParametersRow = values::decode_value(&kv.value)?;
            params.push(row);
        }
        Ok(params)
    }

    // ─── Phase 6: Tags ──────────────────────────────────────────────────────

    pub async fn list_tags(&self, object_id: u64) -> CatalogResult<Vec<TagRow>> {
        let prefix = keys::prefix_tags_for_object(object_id);
        let mut tags = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: TagRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                tags.push(row);
            }
        }
        Ok(tags)
    }

    pub async fn list_column_tags(
        &self,
        table_id: u64,
        column_id: u64,
    ) -> CatalogResult<Vec<ColumnTagRow>> {
        let prefix = keys::prefix_column_tags(table_id, column_id);
        let mut tags = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: ColumnTagRow = values::decode_value(&kv.value)?;
            if mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id) {
                tags.push(row);
            }
        }
        Ok(tags)
    }

    // ─── Phase 6: File Variant Stats ────────────────────────────────────────

    pub async fn list_file_variant_stats(
        &self,
        table_id: u64,
        column_id: u64,
    ) -> CatalogResult<Vec<FileVariantStatsRow>> {
        let mut buf = Vec::with_capacity(17);
        buf.push(TAG_FILE_VARIANT_STATS);
        buf.extend_from_slice(&keys::encode_u64(table_id));
        buf.extend_from_slice(&keys::encode_u64(column_id));

        let mut stats = Vec::new();
        let mut iter = self.db.scan_prefix(&buf).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: FileVariantStatsRow = values::decode_value(&kv.value)?;
            stats.push(row);
        }
        Ok(stats)
    }

    // ─── Phase 6: Files Scheduled for Deletion ──────────────────────────────

    pub async fn list_files_scheduled_for_deletion(
        &self,
    ) -> CatalogResult<Vec<FilesScheduledForDeletionRow>> {
        let prefix = keys::prefix_for_tag(TAG_FILES_SCHEDULED_FOR_DELETION);
        let mut rows = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: FilesScheduledForDeletionRow = values::decode_value(&kv.value)?;
            rows.push(row);
        }
        Ok(rows)
    }

    // ─── v0.10: Snapshot Diff (CDC Output Primitive) ────────────────────────

    /// Compute the diff between two snapshots.
    ///
    /// Returns the set of catalog facts that changed between `from_snapshot`
    /// and `to_snapshot` — specifically the rows whose `begin_snapshot` equals
    /// `to_snapshot` (newly added) and rows whose `end_snapshot` equals
    /// `to_snapshot` (retired at that snapshot).
    ///
    /// This is the foundational primitive for CDC output: every committed
    /// snapshot is a natural change stream for rows that carry begin/end
    /// versioning.
    pub async fn snapshot_diff(
        &self,
        from_snapshot: SnapshotId,
        to_snapshot: SnapshotId,
    ) -> CatalogResult<SnapshotDiff> {
        let to = to_snapshot.as_u64();
        let from = from_snapshot.as_u64();

        let mut added_schemas: Vec<SchemaRow> = Vec::new();
        let mut retired_schemas: Vec<SchemaRow> = Vec::new();
        let mut added_tables: Vec<TableRow> = Vec::new();
        let mut retired_tables: Vec<TableRow> = Vec::new();
        let mut added_columns: Vec<ColumnRow> = Vec::new();
        let mut retired_columns: Vec<ColumnRow> = Vec::new();
        let mut added_data_files: Vec<DataFileRow> = Vec::new();
        let mut retired_data_files: Vec<DataFileRow> = Vec::new();

        // ── schemas ──────────────────────────────────────────────────────────
        {
            let prefix = keys::prefix_for_tag(TAG_SCHEMA);
            let mut iter = self.db.scan_prefix(&prefix).await?;
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let row: SchemaRow = values::decode_value(&kv.value)?;
                if row.begin_snapshot > from && row.begin_snapshot <= to {
                    added_schemas.push(row.clone());
                }
                if let Some(end) = row.end_snapshot {
                    if end > from && end <= to {
                        retired_schemas.push(row);
                    }
                }
            }
        }

        // ── tables ───────────────────────────────────────────────────────────
        {
            let prefix = keys::prefix_for_tag(TAG_TABLE);
            let mut iter = self.db.scan_prefix(&prefix).await?;
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let row: TableRow = values::decode_value(&kv.value)?;
                if row.begin_snapshot > from && row.begin_snapshot <= to {
                    added_tables.push(row.clone());
                }
                if let Some(end) = row.end_snapshot {
                    if end > from && end <= to {
                        retired_tables.push(row);
                    }
                }
            }
        }

        // ── columns ──────────────────────────────────────────────────────────
        {
            let prefix = keys::prefix_for_tag(TAG_COLUMN);
            let mut iter = self.db.scan_prefix(&prefix).await?;
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let row: ColumnRow = values::decode_value(&kv.value)?;
                if row.begin_snapshot > from && row.begin_snapshot <= to {
                    added_columns.push(row.clone());
                }
                if let Some(end) = row.end_snapshot {
                    if end > from && end <= to {
                        retired_columns.push(row);
                    }
                }
            }
        }

        // ── data files ───────────────────────────────────────────────────────
        // v0.24: use begin_snapshot/end_snapshot exclusively; snapshot_id was removed.
        {
            let prefix = keys::prefix_for_tag(TAG_DATA_FILE);
            let mut iter = self.db.scan_prefix(&prefix).await?;
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let row: DataFileRow = values::decode_value(&kv.value)?;
                let begin = row.begin_snapshot.unwrap_or(0);
                if begin > from && begin <= to {
                    added_data_files.push(row.clone());
                }
                if let Some(end) = row.end_snapshot {
                    if end > from && end <= to {
                        retired_data_files.push(row);
                    }
                }
            }
        }

        Ok(SnapshotDiff {
            from_snapshot,
            to_snapshot,
            added_schemas,
            retired_schemas,
            added_tables,
            retired_tables,
            added_columns,
            retired_columns,
            added_data_files,
            retired_data_files,
        })
    }

    /// v0.26: Look up the column type string for a given (table_id, column_id).
    ///
    /// Returns the `column_type` field from the most-recent visible `ColumnRow`,
    /// or `None` if the column is not found at this snapshot.
    pub async fn get_column_type(
        &self,
        table_id: u64,
        column_id: u64,
    ) -> CatalogResult<Option<String>> {
        let col_prefix = keys::prefix_columns_for_table(table_id);
        let mut best: Option<ColumnRow> = None;
        let mut iter = self.db.scan_prefix(&col_prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: ColumnRow = values::decode_value(&kv.value)?;
            if row.column_id == column_id
                && mvcc::is_visible(row.begin_snapshot, row.end_snapshot, self.dl_snapshot_id)
            {
                match &best {
                    None => best = Some(row),
                    Some(existing) if row.begin_snapshot > existing.begin_snapshot => {
                        best = Some(row);
                    }
                    _ => {}
                }
            }
        }
        Ok(best.map(|r| r.data_type))
    }
}

// ─── Column Tree Sort ─────────────────────────────────────────────────────

/// Sort a flat list of column rows into column-tree order.
///
/// Top-level columns (parent_column IS NULL) are ordered by `column_index`.
/// Child columns follow their parent, ordered by `column_index` within the
/// same parent.  This handles arbitrarily nested struct columns.
fn sort_columns_tree(columns: &mut Vec<ColumnRow>) {
    // Separate top-level from nested columns.
    let mut top_level: Vec<ColumnRow> = std::mem::take(columns);

    // Sort everything by column_index first (stable order within each level).
    top_level.sort_by_key(|c| c.column_index);

    // Build a map from column_id to children.
    let mut children: std::collections::HashMap<u64, Vec<ColumnRow>> =
        std::collections::HashMap::new();
    let mut roots: Vec<ColumnRow> = Vec::new();
    for col in top_level {
        if let Some(parent_id) = col.parent_column {
            children.entry(parent_id).or_default().push(col);
        } else {
            roots.push(col);
        }
    }

    // Recursively expand each root into the output list.
    fn expand(
        col: ColumnRow,
        children: &mut std::collections::HashMap<u64, Vec<ColumnRow>>,
        out: &mut Vec<ColumnRow>,
    ) {
        let col_id = col.column_id;
        out.push(col);
        if let Some(mut kids) = children.remove(&col_id) {
            kids.sort_by_key(|c| c.column_index);
            for kid in kids {
                expand(kid, children, out);
            }
        }
    }

    for root in roots {
        expand(root, &mut children, columns);
    }
    // Append any orphaned children that had an unknown parent (shouldn't happen normally).
    for (_, orphans) in children {
        for orphan in orphans {
            columns.push(orphan);
        }
    }
}
