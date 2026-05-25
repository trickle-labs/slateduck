//! Protobuf-encoded row types for all 28 DuckLake v1.0 tables.
//!
//! These types use prost derive macros for protobuf encoding/decoding.

/// Metadata row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct MetadataRow {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(string, tag = "2")]
    pub value: String,
}

/// Snapshot row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SnapshotRow {
    #[prost(uint64, tag = "1")]
    pub snapshot_id: u64,
    #[prost(uint64, tag = "2")]
    pub schema_version: u64,
    #[prost(string, tag = "3")]
    pub snapshot_time: String,
    #[prost(string, optional, tag = "4")]
    pub author: Option<String>,
    #[prost(string, optional, tag = "5")]
    pub message: Option<String>,
    /// v0.24: next_catalog_id at commit time (spec: ducklake_snapshot).
    #[prost(uint64, optional, tag = "6")]
    pub next_catalog_id: Option<u64>,
    /// v0.24: next_file_id at commit time (spec: ducklake_snapshot).
    #[prost(uint64, optional, tag = "7")]
    pub next_file_id: Option<u64>,
}

/// Snapshot changes row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SnapshotChangesRow {
    #[prost(uint64, tag = "1")]
    pub snapshot_id: u64,
    #[prost(string, tag = "2")]
    pub change_type: String,
    #[prost(string, optional, tag = "3")]
    pub change_info: Option<String>,
    #[prost(uint64, optional, tag = "4")]
    pub schema_id: Option<u64>,
    #[prost(uint64, optional, tag = "5")]
    pub table_id: Option<u64>,
    /// v0.24: author of the snapshot (spec: ducklake_snapshot_changes).
    #[prost(string, optional, tag = "6")]
    pub author: Option<String>,
    /// v0.24: commit message (spec: ducklake_snapshot_changes).
    #[prost(string, optional, tag = "7")]
    pub commit_message: Option<String>,
    /// v0.24: extra commit info JSON (spec: ducklake_snapshot_changes).
    #[prost(string, optional, tag = "8")]
    pub commit_extra_info: Option<String>,
    /// v0.24: human-readable summary of changes made (spec: ducklake_snapshot_changes).
    #[prost(string, optional, tag = "9")]
    pub changes_made: Option<String>,
}

/// Schema row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SchemaRow {
    #[prost(uint64, tag = "1")]
    pub schema_id: u64,
    #[prost(string, tag = "2")]
    pub schema_name: String,
    #[prost(uint64, tag = "3")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "4")]
    pub end_snapshot: Option<u64>,
}

/// Table row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct TableRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub schema_id: u64,
    #[prost(string, tag = "3")]
    pub table_name: String,
    #[prost(uint64, tag = "4")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "5")]
    pub end_snapshot: Option<u64>,
    #[prost(string, optional, tag = "6")]
    pub data_path: Option<String>,
}

/// Column row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ColumnRow {
    #[prost(uint64, tag = "1")]
    pub column_id: u64,
    #[prost(uint64, tag = "2")]
    pub table_id: u64,
    #[prost(string, tag = "3")]
    pub column_name: String,
    #[prost(string, tag = "4")]
    pub data_type: String,
    #[prost(uint64, tag = "5")]
    pub column_index: u64,
    #[prost(uint64, tag = "6")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "7")]
    pub end_snapshot: Option<u64>,
    #[prost(string, optional, tag = "8")]
    pub default_value: Option<String>,
    #[prost(bool, tag = "9")]
    pub is_nullable: bool,
}

/// View row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ViewRow {
    #[prost(uint64, tag = "1")]
    pub view_id: u64,
    #[prost(uint64, tag = "2")]
    pub schema_id: u64,
    #[prost(string, tag = "3")]
    pub view_name: String,
    #[prost(string, tag = "4")]
    pub sql: String,
    #[prost(uint64, tag = "5")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "6")]
    pub end_snapshot: Option<u64>,
}

/// Macro row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct MacroRow {
    #[prost(uint64, tag = "1")]
    pub macro_id: u64,
    #[prost(uint64, tag = "2")]
    pub schema_id: u64,
    #[prost(string, tag = "3")]
    pub macro_name: String,
    #[prost(string, tag = "4")]
    pub macro_type: String,
    #[prost(uint64, tag = "5")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "6")]
    pub end_snapshot: Option<u64>,
}

/// Macro implementation row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct MacroImplRow {
    #[prost(uint64, tag = "1")]
    pub impl_id: u64,
    #[prost(uint64, tag = "2")]
    pub macro_id: u64,
    #[prost(string, tag = "3")]
    pub definition: String,
}

/// Macro parameters row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct MacroParametersRow {
    #[prost(uint64, tag = "1")]
    pub macro_id: u64,
    #[prost(uint64, tag = "2")]
    pub impl_id: u64,
    #[prost(uint64, tag = "3")]
    pub column_id: u64,
    #[prost(string, tag = "4")]
    pub parameter_name: String,
    #[prost(string, tag = "5")]
    pub parameter_type: String,
    #[prost(string, optional, tag = "6")]
    pub default_value: Option<String>,
}

/// Data file row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct DataFileRow {
    #[prost(uint64, tag = "1")]
    pub data_file_id: u64,
    #[prost(uint64, tag = "2")]
    pub table_id: u64,
    #[prost(string, tag = "3")]
    pub path: String,
    #[prost(string, tag = "4")]
    pub file_format: String,
    /// v0.24: renamed from row_count — spec field is record_count.
    #[prost(uint64, tag = "5")]
    pub record_count: u64,
    #[prost(uint64, tag = "6")]
    pub file_size_bytes: u64,
    // tag 7 was legacy snapshot_id (removed in v0.24; begin_snapshot is canonical)
    /// v0.24: footer_size in bytes (BIGINT semantics).
    #[prost(int64, optional, tag = "8")]
    pub footer_size: Option<i64>,
    /// v0.18: Per-file Parquet encryption key (pass-through, opaque bytes hex-encoded).
    #[prost(string, optional, tag = "9")]
    pub encryption_key: Option<String>,
    /// v0.19: Snapshot at which this file was added (begin of MVCC window).
    #[prost(uint64, optional, tag = "10")]
    pub begin_snapshot: Option<u64>,
    /// v0.19: Snapshot at which this file was logically deleted/replaced (end of MVCC window).
    /// `None` means the file is still active.
    #[prost(uint64, optional, tag = "11")]
    pub end_snapshot: Option<u64>,
    /// v0.24: monotonically increasing file order within a table.
    #[prost(uint64, optional, tag = "12")]
    pub file_order: Option<u64>,
    /// v0.24: true if path is relative to the table data root.
    #[prost(bool, optional, tag = "13")]
    pub path_is_relative: Option<bool>,
    /// v0.24: first row ID assigned from the table's next_row_id counter.
    #[prost(uint64, optional, tag = "14")]
    pub row_id_start: Option<u64>,
    /// v0.24: partition ID for this file (references ducklake_partition_info).
    #[prost(uint64, optional, tag = "15")]
    pub partition_id: Option<u64>,
    /// v0.24: column mapping ID for this file.
    #[prost(uint64, optional, tag = "16")]
    pub mapping_id: Option<u64>,
    /// v0.24: partial_max upper-bound for zone-map pruning.
    #[prost(string, optional, tag = "17")]
    pub partial_max: Option<String>,
}

/// Delete file row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct DeleteFileRow {
    #[prost(uint64, tag = "1")]
    pub delete_file_id: u64,
    #[prost(uint64, tag = "2")]
    pub data_file_id: u64,
    #[prost(string, tag = "3")]
    pub path: String,
    /// v0.24: renamed from row_count — spec field is delete_count.
    #[prost(uint64, tag = "4")]
    pub delete_count: u64,
    #[prost(uint64, tag = "5")]
    pub file_size_bytes: u64,
    #[prost(uint64, tag = "6")]
    pub snapshot_id: u64,
    /// v0.24: owning table ID (spec: ducklake_delete_file).
    #[prost(uint64, optional, tag = "7")]
    pub table_id: Option<u64>,
    /// v0.24: snapshot at which this delete file was added.
    #[prost(uint64, optional, tag = "8")]
    pub begin_snapshot: Option<u64>,
    /// v0.24: snapshot at which this delete file was retired.
    #[prost(uint64, optional, tag = "9")]
    pub end_snapshot: Option<u64>,
    /// v0.24: true if path is relative to the table data root.
    #[prost(bool, optional, tag = "10")]
    pub path_is_relative: Option<bool>,
    /// v0.24: delete file format (e.g. "parquet").
    #[prost(string, optional, tag = "11")]
    pub format: Option<String>,
    /// v0.24: footer size in bytes.
    #[prost(int64, optional, tag = "12")]
    pub footer_size: Option<i64>,
    /// v0.24: partial_max upper-bound for zone-map pruning.
    #[prost(string, optional, tag = "13")]
    pub partial_max: Option<String>,
}

/// Files scheduled for deletion row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct FilesScheduledForDeletionRow {
    #[prost(uint64, tag = "1")]
    pub data_file_id: u64,
    #[prost(uint64, tag = "2")]
    pub schedule_start: u64,
    #[prost(string, tag = "3")]
    pub path: String,
    #[prost(string, tag = "4")]
    pub file_type: String,
}

/// Inlined data tables row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct InlinedDataTablesRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub schema_version: u64,
    #[prost(string, tag = "3")]
    pub sql: String,
}

/// Column mapping row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ColumnMappingRow {
    #[prost(uint64, tag = "1")]
    pub mapping_id: u64,
    #[prost(uint64, tag = "2")]
    pub table_id: u64,
    #[prost(string, tag = "3")]
    pub file_column_name: String,
    #[prost(uint64, tag = "4")]
    pub column_id: u64,
}

/// Name mapping row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct NameMappingRow {
    #[prost(uint64, tag = "1")]
    pub mapping_id: u64,
    #[prost(uint64, tag = "2")]
    pub column_id: u64,
    #[prost(string, tag = "3")]
    pub source_name: String,
    #[prost(uint64, tag = "4")]
    pub source_name_hash: u64,
}

/// Table stats row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct TableStatsRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    /// v0.24: renamed from row_count — spec field is record_count.
    #[prost(uint64, tag = "2")]
    pub record_count: u64,
    #[prost(uint64, tag = "3")]
    pub file_count: u64,
    /// v0.24: renamed from total_size_bytes — spec field is file_size_bytes.
    #[prost(uint64, tag = "4")]
    pub file_size_bytes: u64,
    /// v0.24: next row ID to assign (tracks row ID allocation per table).
    #[prost(uint64, optional, tag = "5")]
    pub next_row_id: Option<u64>,
}

/// Table column stats row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct TableColumnStatsRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub column_id: u64,
    #[prost(bool, tag = "3")]
    pub has_null: bool,
    #[prost(string, optional, tag = "4")]
    pub min_value: Option<String>,
    #[prost(string, optional, tag = "5")]
    pub max_value: Option<String>,
}

/// File column stats row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct FileColumnStatsRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub column_id: u64,
    #[prost(uint64, tag = "3")]
    pub data_file_id: u64,
    #[prost(bool, tag = "4")]
    pub has_null: bool,
    #[prost(string, optional, tag = "5")]
    pub min_value: Option<String>,
    #[prost(string, optional, tag = "6")]
    pub max_value: Option<String>,
    #[prost(bool, tag = "7")]
    pub contains_nan: bool,
}

/// File variant stats row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct FileVariantStatsRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub column_id: u64,
    #[prost(uint64, tag = "3")]
    pub variant_path_hash: u64,
    #[prost(uint64, tag = "4")]
    pub data_file_id: u64,
    #[prost(string, tag = "5")]
    pub variant_path: String,
    #[prost(string, optional, tag = "6")]
    pub min_value: Option<String>,
    #[prost(string, optional, tag = "7")]
    pub max_value: Option<String>,
}

/// Partition info row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct PartitionInfoRow {
    #[prost(uint64, tag = "1")]
    pub partition_id: u64,
    #[prost(uint64, tag = "2")]
    pub table_id: u64,
    #[prost(uint64, tag = "3")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "4")]
    pub end_snapshot: Option<u64>,
}

/// Partition column row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct PartitionColumnRow {
    #[prost(uint64, tag = "1")]
    pub partition_id: u64,
    #[prost(uint64, tag = "2")]
    pub partition_key_index: u64,
    #[prost(uint64, tag = "3")]
    pub column_id: u64,
    #[prost(string, optional, tag = "4")]
    pub transform: Option<String>,
}

/// File partition value row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct FilePartitionValueRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub partition_key_index: u64,
    #[prost(uint64, tag = "3")]
    pub data_file_id: u64,
    #[prost(string, optional, tag = "4")]
    pub value: Option<String>,
}

/// Sort info row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SortInfoRow {
    #[prost(uint64, tag = "1")]
    pub sort_id: u64,
    #[prost(uint64, tag = "2")]
    pub table_id: u64,
    #[prost(uint64, tag = "3")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "4")]
    pub end_snapshot: Option<u64>,
}

/// Sort expression row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SortExpressionRow {
    #[prost(uint64, tag = "1")]
    pub sort_id: u64,
    #[prost(uint64, tag = "2")]
    pub sort_key_index: u64,
    #[prost(uint64, tag = "3")]
    pub column_id: u64,
    #[prost(bool, tag = "4")]
    pub ascending: bool,
    #[prost(bool, tag = "5")]
    pub nulls_first: bool,
}

/// Tag row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct TagRow {
    #[prost(uint64, tag = "1")]
    pub object_id: u64,
    #[prost(string, tag = "2")]
    pub tag_key: String,
    #[prost(string, tag = "3")]
    pub tag_value: String,
    #[prost(uint64, tag = "4")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "5")]
    pub end_snapshot: Option<u64>,
}

/// Column tag row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ColumnTagRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub column_id: u64,
    #[prost(string, tag = "3")]
    pub tag_key: String,
    #[prost(string, tag = "4")]
    pub tag_value: String,
    #[prost(uint64, tag = "5")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "6")]
    pub end_snapshot: Option<u64>,
}

/// Schema versions row value.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SchemaVersionsRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub begin_snapshot: u64,
    #[prost(uint64, tag = "3")]
    pub schema_version: u64,
}

/// Inlined insert row value (stored under 0xFD | 0x01).
#[derive(Clone, PartialEq, prost::Message)]
pub struct InlinedInsertRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub schema_version: u64,
    #[prost(uint64, tag = "3")]
    pub row_id: u64,
    #[prost(bytes = "vec", tag = "4")]
    pub payload: Vec<u8>,
    #[prost(uint64, tag = "5")]
    pub begin_snapshot: u64,
    #[prost(uint64, optional, tag = "6")]
    pub end_snapshot: Option<u64>,
}

/// Inlined delete marker value (stored under 0xFD | 0x02).
#[derive(Clone, PartialEq, prost::Message)]
pub struct InlinedDeleteRow {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub data_file_id: u64,
    #[prost(uint64, tag = "3")]
    pub row_id: u64,
    #[prost(uint64, tag = "4")]
    pub begin_snapshot: u64,
}

/// Excision audit record (stored under 0xFF | "excised" | timestamp).
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExcisionRecord {
    #[prost(string, tag = "1")]
    pub timestamp: String,
    #[prost(uint64, tag = "2")]
    pub before_snapshot: u64,
    #[prost(uint64, tag = "3")]
    pub keys_removed: u64,
    #[prost(string, optional, tag = "4")]
    pub operator: Option<String>,
}

/// Hot-key value: persists current snapshot ID and per-table file counts
/// under a single system key for cold-start optimization.
#[derive(Clone, PartialEq, prost::Message)]
pub struct HotKeyValue {
    #[prost(uint64, tag = "1")]
    pub current_snapshot_id: u64,
    #[prost(message, repeated, tag = "2")]
    pub table_file_counts: Vec<TableFileCount>,
}

/// Per-table file count entry for hot-key.
#[derive(Clone, PartialEq, prost::Message)]
pub struct TableFileCount {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(uint64, tag = "2")]
    pub file_count: u64,
}

/// Packed table metadata: all per-table metadata in one composite value.
/// Enables single-read planning queries.
#[derive(Clone, PartialEq, prost::Message)]
pub struct PackedTableMetadata {
    #[prost(uint64, tag = "1")]
    pub table_id: u64,
    #[prost(message, repeated, tag = "2")]
    pub columns: Vec<ColumnRow>,
    #[prost(message, repeated, tag = "3")]
    pub partition_info: Vec<PartitionInfoRow>,
    #[prost(message, repeated, tag = "4")]
    pub sort_info: Vec<SortInfoRow>,
    #[prost(message, optional, tag = "5")]
    pub table_stats: Option<TableStatsRow>,
    #[prost(uint64, tag = "6")]
    pub schema_version: u64,
}

/// Secondary index entry value (minimal; the key carries the semantics).
#[derive(Clone, PartialEq, prost::Message)]
pub struct SecondaryIndexEntry {
    #[prost(uint64, tag = "1")]
    pub data_file_id: u64,
    #[prost(string, tag = "2")]
    pub path: String,
}

// ─── v0.18: Snapshot Lease ─────────────────────────────────────────────────

/// Snapshot lease row (tag 0x22, MutableSingleton per consumer_id).
/// Prevents GC from advancing past `min_snapshot_id` until TTL expires or lease is released.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SnapshotLeaseRow {
    /// Consumer identifier (e.g., "pgtrickle:stream_1").
    #[prost(string, tag = "1")]
    pub consumer_id: String,
    /// Minimum snapshot ID that must be retained.
    #[prost(uint64, tag = "2")]
    pub min_snapshot_id: u64,
    /// Unix-millisecond timestamp when the lease expires.
    #[prost(uint64, tag = "3")]
    pub expires_at_unix_ms: u64,
}

// ─── v0.18: Extension Schema ───────────────────────────────────────────────

/// Extension schema row (tag 0x23). Stores application-defined metadata.
/// Used by pg-trickle and other DuckLake-compatible systems to persist
/// their own tables within the catalog.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ExtensionSchemaRow {
    /// Extension identifier (e.g., 0x01 for pgtrickle).
    #[prost(uint32, tag = "1")]
    pub extension_id: u32,
    /// Table name within the extension schema.
    #[prost(string, tag = "2")]
    pub table_name: String,
    /// Row ID within the extension table.
    #[prost(uint64, tag = "3")]
    pub row_id: u64,
    /// JSON-encoded column values.
    #[prost(string, tag = "4")]
    pub data_json: String,
}
