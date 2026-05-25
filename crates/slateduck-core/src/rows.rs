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
    #[prost(uint64, tag = "5")]
    pub row_count: u64,
    #[prost(uint64, tag = "6")]
    pub file_size_bytes: u64,
    #[prost(uint64, tag = "7")]
    pub snapshot_id: u64,
    #[prost(string, optional, tag = "8")]
    pub footer_size: Option<String>,
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
    #[prost(uint64, tag = "4")]
    pub row_count: u64,
    #[prost(uint64, tag = "5")]
    pub file_size_bytes: u64,
    #[prost(uint64, tag = "6")]
    pub snapshot_id: u64,
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
    #[prost(uint64, tag = "2")]
    pub row_count: u64,
    #[prost(uint64, tag = "3")]
    pub file_count: u64,
    #[prost(uint64, tag = "4")]
    pub total_size_bytes: u64,
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

// ─── v0.11 IVM Row Types ───────────────────────────────────────────────────

/// Matview status values (encoding_version = 1).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum MatviewStatus {
    Active = 0,
    Stale = 1,
    Rebuilding = 2,
    Dropped = 3,
}

impl MatviewStatus {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Stale,
            2 => Self::Rebuilding,
            3 => Self::Dropped,
            _ => Self::Active,
        }
    }
}

/// Incremental materialized view definition row (tag 0x1D, Versioned).
/// encoding_version = 1.
#[derive(Clone, PartialEq, prost::Message)]
pub struct MatviewRow {
    #[prost(uint64, tag = "1")]
    pub matview_id: u64,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub schema_name: String,
    #[prost(string, tag = "4")]
    pub view_sql: String,
    #[prost(uint64, tag = "5")]
    pub output_table_id: u64,
    #[prost(uint32, tag = "6")]
    pub shard_count: u32,
    #[prost(uint32, tag = "7")]
    pub freshness_target_ms: u32,
    #[prost(string, tag = "8")]
    pub state_uri: String,
    /// Empty = auto-detected from GROUP BY.
    #[prost(string, tag = "9")]
    pub shard_key_column: String,
    #[prost(uint64, tag = "10")]
    pub created_at_snapshot: u64,
    #[prost(uint64, tag = "11")]
    pub begin_snapshot: u64,
    /// 0 = open (still active).
    #[prost(uint64, tag = "12")]
    pub end_snapshot: u64,
    /// MatviewStatus as u32: 0=Active, 1=Stale, 2=Rebuilding, 3=Dropped.
    #[prost(uint32, tag = "13")]
    pub status: u32,
    #[prost(uint32, tag = "14")]
    pub encoding_version: u32,
    /// OutputMode as u32: 0=Consistent (default), 1=PerShard.
    #[prost(uint32, tag = "15")]
    pub output_mode: u32,
    /// Monotonically increasing counter for DBSP circuit schema changes.
    /// A mismatch between persisted state and this value triggers a rebuild.
    #[prost(uint64, tag = "16")]
    pub circuit_compilation_version: u64,
}

/// Output publication mode for a materialized view.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
#[repr(u32)]
pub enum OutputMode {
    /// Default: output snapshot waits for all shards (globally consistent).
    #[default]
    Consistent = 0,
    /// Each shard publishes independently; reader merges at query time.
    PerShard = 1,
}

impl OutputMode {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::PerShard,
            _ => Self::Consistent,
        }
    }
}

/// Matview dependency row (tag 0x1E, AppendOnly).
/// One row per (matview_id, base_table_id) pair.
#[derive(Clone, PartialEq, prost::Message)]
pub struct MatviewDepRow {
    #[prost(uint64, tag = "1")]
    pub matview_id: u64,
    #[prost(uint64, tag = "2")]
    pub base_table_id: u64,
    #[prost(string, repeated, tag = "3")]
    pub columns: Vec<String>,
    /// True if this input is broadcast to every shard.
    #[prost(bool, tag = "4")]
    pub is_broadcast: bool,
    #[prost(uint64, tag = "5")]
    pub begin_snapshot: u64,
    #[prost(uint32, tag = "6")]
    pub encoding_version: u32,
}

/// Per-shard checkpoint watermark row (tag 0x1F, AppendOnly).
/// Monotonically advancing `seq` per (matview_id, shard_id).
#[derive(Clone, PartialEq, prost::Message)]
pub struct MatviewCheckpointRow {
    #[prost(uint64, tag = "1")]
    pub matview_id: u64,
    #[prost(uint32, tag = "2")]
    pub shard_id: u32,
    #[prost(uint64, tag = "3")]
    pub seq: u64,
    #[prost(uint64, tag = "4")]
    pub last_input_snapshot: u64,
    #[prost(uint64, tag = "5")]
    pub last_output_snapshot: u64,
    #[prost(uint64, tag = "6")]
    pub frontier_time: u64,
    #[prost(uint64, tag = "7")]
    pub durable_at_unix_ms: u64,
    #[prost(string, tag = "8")]
    pub worker_id: String,
    #[prost(uint32, tag = "9")]
    pub encoding_version: u32,
}

/// Per-shard lease state row (tag 0x20, MutableSingleton per (matview_id, shard_id)).
/// Updated atomically via CAS; `generation` is bumped on every successful update.
#[derive(Clone, PartialEq, prost::Message)]
pub struct MatviewShardRow {
    #[prost(uint64, tag = "1")]
    pub matview_id: u64,
    #[prost(uint32, tag = "2")]
    pub shard_id: u32,
    /// Empty string = unowned.
    #[prost(string, tag = "3")]
    pub owner_worker: String,
    #[prost(uint64, tag = "4")]
    pub lease_expires_unix_ms: u64,
    /// Inclusive lower bound of the shard's key range (raw bytes).
    #[prost(bytes = "vec", tag = "5")]
    pub key_range_lo: Vec<u8>,
    /// Exclusive upper bound of the shard's key range (raw bytes).
    #[prost(bytes = "vec", tag = "6")]
    pub key_range_hi: Vec<u8>,
    /// Bumped on every CAS update; used for optimistic locking.
    #[prost(uint64, tag = "7")]
    pub generation: u64,
    #[prost(uint32, tag = "8")]
    pub encoding_version: u32,
    /// Most recently consumed input snapshot by this shard.
    #[prost(uint64, tag = "9")]
    pub last_input_snapshot: u64,
}
