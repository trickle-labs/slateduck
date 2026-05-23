//! Session state: transaction buffering between BEGIN and COMMIT.
//!
//! Accumulates INSERT/UPDATE statements into a PendingCatalogTxn between
//! BEGIN and COMMIT. ROLLBACK or disconnect drops the pending batch.

use crate::error::SlateDuckError;

/// Maximum pending transaction batch size (64 MiB).
const MAX_BATCH_SIZE: usize = 64 * 1024 * 1024;

/// A buffered operation in a pending transaction.
#[derive(Debug, Clone)]
pub enum BufferedOp {
    InsertSchema {
        schema_name: String,
    },
    InsertTable {
        schema_id: u64,
        table_name: String,
        data_path: Option<String>,
    },
    InsertColumn {
        table_id: u64,
        column_name: String,
        data_type: String,
        column_index: u64,
        is_nullable: bool,
        default_value: Option<String>,
    },
    InsertDataFile {
        table_id: u64,
        path: String,
        file_format: String,
        row_count: u64,
        file_size_bytes: u64,
    },
    InsertDeleteFile {
        data_file_id: u64,
        path: String,
        row_count: u64,
        file_size_bytes: u64,
    },
    InsertSnapshot {
        author: Option<String>,
        message: Option<String>,
    },
    InsertSnapshotChanges {
        change_type: String,
        change_info: Option<String>,
        schema_id: Option<u64>,
        table_id: Option<u64>,
    },
    UpdateEndSnapshot {
        table_name: String,
        entity_id: u64,
        begin_snapshot: u64,
        end_snapshot: u64,
    },
    UpdateTableStats {
        table_id: u64,
        row_count_delta: i64,
    },
    InsertFileColumnStats {
        table_id: u64,
        column_id: u64,
        data_file_id: u64,
        has_null: bool,
        min_value: Option<String>,
        max_value: Option<String>,
        contains_nan: bool,
    },
    InsertMetadata {
        key: String,
        value: String,
    },
    InsertInlinedDataTables {
        table_id: u64,
        schema_version: u64,
        sql: String,
    },
    InsertView {
        schema_id: u64,
        view_name: String,
        sql: String,
    },
    InsertMacro {
        schema_id: u64,
        macro_name: String,
        macro_type: String,
    },
    InsertTableStats {
        table_id: u64,
        row_count: u64,
        file_count: u64,
        total_size_bytes: u64,
    },
}

/// Pending catalog transaction buffer.
#[derive(Debug, Default)]
pub struct PendingCatalogTxn {
    ops: Vec<BufferedOp>,
    estimated_size: usize,
}

impl PendingCatalogTxn {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an operation to the pending transaction.
    pub fn push(&mut self, op: BufferedOp) -> Result<(), SlateDuckError> {
        let op_size = std::mem::size_of_val(&op) + 128; // rough estimate
        if self.estimated_size + op_size > MAX_BATCH_SIZE {
            return Err(SlateDuckError::BatchTooLarge);
        }
        self.estimated_size += op_size;
        self.ops.push(op);
        Ok(())
    }

    /// Take all buffered operations, leaving the buffer empty.
    pub fn take(&mut self) -> Vec<BufferedOp> {
        self.estimated_size = 0;
        std::mem::take(&mut self.ops)
    }

    /// Discard all buffered operations.
    pub fn clear(&mut self) {
        self.ops.clear();
        self.estimated_size = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }
}

/// Per-session state.
#[derive(Debug)]
pub struct SessionState {
    pub in_transaction: bool,
    pub pending_txn: PendingCatalogTxn,
    pub settings: SessionSettings,
}

/// Session-level settings (SET/SHOW).
#[derive(Debug, Clone)]
pub struct SessionSettings {
    pub timezone: String,
    pub client_encoding: String,
    pub date_style: String,
    pub application_name: String,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            timezone: "UTC".to_string(),
            client_encoding: "UTF8".to_string(),
            date_style: "ISO, MDY".to_string(),
            application_name: String::new(),
        }
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            in_transaction: false,
            pending_txn: PendingCatalogTxn::new(),
            settings: SessionSettings::default(),
        }
    }
}

impl SessionState {
    pub fn new() -> Self {
        Self::default()
    }
}
