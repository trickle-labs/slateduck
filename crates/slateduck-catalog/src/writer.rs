//! CatalogWriter: write operations that produce new snapshots.

use slatedb::{Db, DbTransaction, IsolationLevel};
use slateduck_core::counters::CounterCache;
use slateduck_core::keys;
use slateduck_core::mvcc::SnapshotId;
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::values::{self, MAX_INLINED_VALUE_SIZE};

use crate::error::{CatalogError, CatalogResult};

/// Writes to the catalog, producing new snapshots atomically.
pub struct CatalogWriter {
    db: Db,
    counters: CounterCache,
    writer_epoch: u64,
    schema_changed: bool,
    current_schema_version: u64,
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
        }
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let key = keys::key_schema(schema_id);
        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        tx.put(
            keys::key_counter(COUNTER_NEXT_CATALOG_ID),
            self.counters.encode_catalog_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
        self.mark_schema_changed();
        Ok(schema_id)
    }

    pub async fn drop_schema(&mut self, schema_id: u64) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_schema(schema_id);

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let existing = tx
            .get(&key)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            .ok_or_else(|| CatalogError::NotFound(format!("schema {schema_id}")))?;

        let mut row: SchemaRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let key = keys::key_table(schema_id, table_id, snapshot_id);
        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        tx.put(
            keys::key_counter(COUNTER_NEXT_CATALOG_ID),
            self.counters.encode_catalog_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
        self.mark_schema_changed();
        Ok(table_id)
    }

    pub async fn drop_table(
        &mut self,
        schema_id: u64,
        table_id: u64,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_table(schema_id, table_id, begin_snapshot);

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let existing = tx
            .get(&key)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            .ok_or_else(|| CatalogError::NotFound(format!("table {table_id}")))?;

        let mut row: TableRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
        self.mark_schema_changed();
        Ok(())
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let key = keys::key_column(table_id, column_id, snapshot_id);
        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        tx.put(
            keys::key_counter(COUNTER_NEXT_CATALOG_ID),
            self.counters.encode_catalog_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
        self.mark_schema_changed();
        Ok(column_id)
    }

    pub async fn drop_column(
        &mut self,
        table_id: u64,
        column_id: u64,
        begin_snapshot: u64,
    ) -> CatalogResult<()> {
        let snapshot_id = self.counters.peek_snapshot_id();
        let key = keys::key_column(table_id, column_id, begin_snapshot);

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let existing = tx
            .get(&key)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            .ok_or_else(|| CatalogError::NotFound(format!("column {column_id}")))?;

        let mut row: ColumnRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
        self.mark_schema_changed();
        Ok(())
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let key = keys::key_data_file(table_id, data_file_id);
        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        tx.put(
            keys::key_counter(COUNTER_NEXT_FILE_ID),
            self.counters.encode_file_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let key = keys::key_delete_file(data_file_id, delete_file_id);
        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        tx.put(
            keys::key_counter(COUNTER_NEXT_FILE_ID),
            self.counters.encode_file_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let key = keys::key_inlined_insert(table_id, schema_version, row_id);
        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let existing = tx
            .get(&key)
            .await
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?
            .ok_or_else(|| CatalogError::NotFound(format!("inlined row {row_id}")))?;

        let mut row: InlinedInsertRow = values::decode_value(&existing)?;
        row.end_snapshot = Some(snapshot_id);

        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let key = keys::key_inlined_delete(table_id, data_file_id, row_id);
        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
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

    /// Create a new snapshot. Increments `schema_version` iff `mark_schema_changed()` was called.
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

        let tx = self.begin_tx().await?;
        self.check_epoch(&tx).await?;

        let key = keys::key_snapshot(snapshot_id);
        tx.put(&key, values::encode_value(&row))
            .map_err(|e| CatalogError::SlateDb(e.to_string()))?;
        tx.put(
            keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID),
            self.counters.encode_snapshot_counter(),
        )
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| CatalogError::TransactionConflict(e.to_string()))?;
        Ok(SnapshotId::new(snapshot_id))
    }

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
