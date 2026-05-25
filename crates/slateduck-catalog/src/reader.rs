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
    /// Data files registered at `to_snapshot`.
    pub added_data_files: Vec<DataFileRow>,
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
        columns.sort_by_key(|c| c.column_index);

        Ok(Some((table, columns)))
    }

    pub async fn list_data_files(&self, table_id: u64) -> CatalogResult<Vec<DataFileRow>> {
        let prefix = keys::prefix_data_files_for_table(table_id);
        let mut files = Vec::new();
        let mut iter = self.db.scan_prefix(&prefix).await?;
        while let Some(kv) = iter
            .next()
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
        {
            let row: DataFileRow = values::decode_value(&kv.value)?;
            if row.snapshot_id <= self.dl_snapshot_id.as_u64() {
                files.push(row);
            }
        }
        Ok(files)
    }

    pub async fn prune_files(
        &self,
        table_id: u64,
        column_id: u64,
        predicate_value: &str,
        col_type: &DuckLakeType,
    ) -> CatalogResult<Vec<u64>> {
        use slateduck_core::types::prune_file;

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
        let _from = from_snapshot.as_u64();

        let mut added_schemas: Vec<SchemaRow> = Vec::new();
        let mut retired_schemas: Vec<SchemaRow> = Vec::new();
        let mut added_tables: Vec<TableRow> = Vec::new();
        let mut retired_tables: Vec<TableRow> = Vec::new();
        let mut added_columns: Vec<ColumnRow> = Vec::new();
        let mut retired_columns: Vec<ColumnRow> = Vec::new();
        let mut added_data_files: Vec<DataFileRow> = Vec::new();

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
                if row.begin_snapshot == to {
                    added_schemas.push(row.clone());
                }
                if row.end_snapshot == Some(to) {
                    retired_schemas.push(row);
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
                if row.begin_snapshot == to {
                    added_tables.push(row.clone());
                }
                if row.end_snapshot == Some(to) {
                    retired_tables.push(row);
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
                if row.begin_snapshot == to {
                    added_columns.push(row.clone());
                }
                if row.end_snapshot == Some(to) {
                    retired_columns.push(row);
                }
            }
        }

        // ── data files ───────────────────────────────────────────────────────
        // DataFileRow uses `snapshot_id` (the snapshot at which the file was
        // registered) rather than begin/end versioning.
        {
            let prefix = keys::prefix_for_tag(TAG_DATA_FILE);
            let mut iter = self.db.scan_prefix(&prefix).await?;
            while let Some(kv) = iter
                .next()
                .await
                .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            {
                let row: DataFileRow = values::decode_value(&kv.value)?;
                if row.snapshot_id == to {
                    added_data_files.push(row);
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
        })
    }
}
