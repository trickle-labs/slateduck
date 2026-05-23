//! CatalogReader: read catalog state at a specific DuckLake snapshot.

use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::mvcc::{self, SnapshotId};
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::types::{DuckLakeType, PruneResult};
use slateduck_core::values;

use crate::error::{CatalogError, CatalogResult};

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
        let prefix = keys::prefix_for_tag(TAG_TABLE);
        let mut table_row: Option<TableRow> = None;
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
                match &table_row {
                    None => table_row = Some(row),
                    Some(existing) if row.begin_snapshot > existing.begin_snapshot => {
                        table_row = Some(row);
                    }
                    _ => {}
                }
            }
        }

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
}
