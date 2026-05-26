//! Session state: transaction buffering between BEGIN and COMMIT.
//!
//! Accumulates INSERT/UPDATE statements into a PendingCatalogTxn between
//! BEGIN and COMMIT. ROLLBACK or disconnect drops the pending batch.

use crate::error::SlateDuckError;
use crate::notify::ConnectionSubscriptions;

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
        initial_default: Option<String>,
        default_value_type: Option<String>,
        default_value_dialect: Option<String>,
        parent_column: Option<u64>,
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
        delete_count: u64,
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
    SetTableStats {
        table_id: u64,
        record_count: u64,
        file_size_bytes: u64,
        next_row_id: u64,
    },
    InsertFileColumnStats {
        table_id: u64,
        column_id: u64,
        data_file_id: u64,
        contains_null: bool,
        min_value: Option<String>,
        max_value: Option<String>,
        contains_nan: bool,
    },
    InsertTableColumnStats {
        table_id: u64,
        column_id: u64,
        contains_null: bool,
        contains_nan: Option<bool>,
        min_value: Option<String>,
        max_value: Option<String>,
        extra_stats: Option<String>,
    },
    InsertMetadata {
        key: String,
        value: String,
        scope: Option<String>,
        scope_id: Option<u64>,
    },
    InsertInlinedDataTables {
        table_id: u64,
        table_name: String,
        schema_version: u64,
    },
    InsertSchemaVersions {
        begin_snapshot: u64,
        schema_version: u64,
        table_id: u64,
    },
    InsertInlinedRow {
        table_name: String,
        rows: Vec<Vec<Option<String>>>,
    },
    DeleteInlinedRows {
        table_name: String,
        row_ids: Vec<u64>,
    },
    InsertView {
        schema_id: u64,
        view_name: String,
        sql: String,
        view_uuid: Option<String>,
        dialect: Option<String>,
        column_aliases: Option<String>,
    },
    InsertMacro {
        schema_id: u64,
        macro_name: String,
        macro_type: String,
        macro_uuid: Option<String>,
    },
    InsertMacroImpl {
        macro_id: u64,
        sql: String,
        dialect: Option<String>,
        impl_type: Option<String>,
    },
    InsertMacroParams {
        macro_id: u64,
        impl_id: u64,
        column_id: u64,
        parameter_name: String,
        parameter_type: String,
        default_value: Option<String>,
        default_value_type: Option<String>,
    },
    InsertTableStats {
        table_id: u64,
        record_count: u64,
        file_count: u64,
        file_size_bytes: u64,
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

// ─── COPY FROM STDIN accumulator ─────────────────────────────────────────────

/// Accumulates raw bytes for an in-progress binary COPY FROM STDIN stream.
#[derive(Debug, Default)]
pub struct CopyAccumulator {
    /// Normalised table name (e.g. `"ducklake_snapshot"`).
    pub table: String,
    /// Raw binary COPY bytes buffered across one or more `CopyData` messages.
    pub data: Vec<u8>,
}

/// A schema row parsed from a `ducklake_schema` binary COPY FROM STDIN stream.
#[derive(Debug, Clone)]
pub struct BootstrapSchemaRow {
    pub schema_name: String,
}

/// Bootstrap state collected during the DuckDB `ATTACH` initialisation phase.
///
/// DuckDB bootstraps the catalog by issuing binary `COPY … FROM STDIN` for
/// each `ducklake_*` table, then `COMMIT`.  We accumulate the relevant rows
/// here so that the `COMMIT` handler can persist them to the catalog store.
#[derive(Debug, Default)]
pub struct BootstrapState {
    /// `true` if at least one row was received for `ducklake_snapshot`.
    pub has_snapshot: bool,
    /// Schema rows received from `ducklake_schema`.
    pub schemas: Vec<BootstrapSchemaRow>,
}

/// Per-session state.
#[derive(Debug)]
pub struct SessionState {
    pub in_transaction: bool,
    pub pending_txn: PendingCatalogTxn,
    pub settings: SessionSettings,
    /// Per-connection LISTEN/NOTIFY subscription state.
    pub subscriptions: ConnectionSubscriptions,
    /// Accumulator for the currently active COPY FROM STDIN binary stream.
    pub pending_copy: Option<CopyAccumulator>,
    /// Bootstrap rows received via COPY FROM STDIN during DuckDB ATTACH.
    pub bootstrap: BootstrapState,
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
            subscriptions: ConnectionSubscriptions::new(),
            pending_copy: None,
            bootstrap: BootstrapState::default(),
        }
    }
}

impl SessionState {
    pub fn new() -> Self {
        Self::default()
    }
}
