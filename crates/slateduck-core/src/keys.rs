//! Binary key encoding and decoding for the SlateDuck catalog.
//!
//! All keys are big-endian encoded for correct lexicographic ordering.
//! The first byte is always the table tag.

use crate::tags::*;

/// Errors that can occur during key encoding/decoding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("unknown tag byte 0x{0:02X}")]
    UnknownTag(u8),
    #[error("key too short: expected at least {expected} bytes, got {actual}")]
    TooShort { expected: usize, actual: usize },
    #[error("invalid UTF-8 in key: {0}")]
    InvalidUtf8(String),
}

/// Encode a u64 as 8 big-endian bytes.
#[inline]
pub fn encode_u64(val: u64) -> [u8; 8] {
    val.to_be_bytes()
}

/// Decode a u64 from 8 big-endian bytes.
#[inline]
pub fn decode_u64(bytes: &[u8]) -> Result<u64, KeyError> {
    if bytes.len() < 8 {
        return Err(KeyError::TooShort {
            expected: 8,
            actual: bytes.len(),
        });
    }
    Ok(u64::from_be_bytes(bytes[..8].try_into().unwrap()))
}

/// Decode a u32 from 4 big-endian bytes.
#[inline]
pub fn decode_u32(bytes: &[u8]) -> Result<u32, KeyError> {
    if bytes.len() < 4 {
        return Err(KeyError::TooShort {
            expected: 4,
            actual: bytes.len(),
        });
    }
    Ok(u32::from_be_bytes(bytes[..4].try_into().unwrap()))
}

/// Encode a u32 as 4 big-endian bytes.
#[inline]
pub fn encode_u32(val: u32) -> [u8; 4] {
    val.to_be_bytes()
}

/// Metadata scope enum values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MetadataScope {
    Global = 0x00,
    Schema = 0x01,
    Table = 0x02,
}

impl MetadataScope {
    pub fn from_byte(b: u8) -> Result<Self, KeyError> {
        match b {
            0x00 => Ok(Self::Global),
            0x01 => Ok(Self::Schema),
            0x02 => Ok(Self::Table),
            _ => Err(KeyError::UnknownTag(b)),
        }
    }
}

// ─── Key Builders ──────────────────────────────────────────────────────────

/// Build a key for `ducklake_metadata`: `0x01 | scope | scope_id(u64) | len(u16) | key_bytes`.
pub fn key_metadata(scope: MetadataScope, scope_id: u64, key: &str) -> Vec<u8> {
    let key_bytes = key.as_bytes();
    let mut buf = Vec::with_capacity(1 + 1 + 8 + 2 + key_bytes.len());
    buf.push(TAG_METADATA);
    buf.push(scope as u8);
    buf.extend_from_slice(&encode_u64(scope_id));
    buf.extend_from_slice(&(key_bytes.len() as u16).to_be_bytes());
    buf.extend_from_slice(key_bytes);
    buf
}

/// Build a key for `ducklake_snapshot`: `0x02 | snapshot_id(u64)`.
pub fn key_snapshot(snapshot_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_SNAPSHOT);
    buf.extend_from_slice(&encode_u64(snapshot_id));
    buf
}

/// Build a key for `ducklake_snapshot_changes`: `0x03 | snapshot_id(u64)`.
pub fn key_snapshot_changes(snapshot_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_SNAPSHOT_CHANGES);
    buf.extend_from_slice(&encode_u64(snapshot_id));
    buf
}

/// Build a key for `ducklake_schema`: `0x04 | schema_id(u64)`.
pub fn key_schema(schema_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_SCHEMA);
    buf.extend_from_slice(&encode_u64(schema_id));
    buf
}

/// Build a key for `ducklake_table`: `0x05 | schema_id(u64) | table_id(u64) | begin_snapshot(u64)`.
pub fn key_table(schema_id: u64, table_id: u64, begin_snapshot: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_TABLE);
    buf.extend_from_slice(&encode_u64(schema_id));
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

/// Build a key for `ducklake_column`: `0x06 | table_id(u64) | column_id(u64) | begin_snapshot(u64)`.
pub fn key_column(table_id: u64, column_id: u64, begin_snapshot: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_COLUMN);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(column_id));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

/// Build a key for `ducklake_view`: `0x07 | schema_id(u64) | view_id(u64) | begin_snapshot(u64)`.
pub fn key_view(schema_id: u64, view_id: u64, begin_snapshot: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_VIEW);
    buf.extend_from_slice(&encode_u64(schema_id));
    buf.extend_from_slice(&encode_u64(view_id));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

/// Build a key for `ducklake_macro`: `0x08 | schema_id(u64) | macro_id(u64) | begin_snapshot(u64)`.
pub fn key_macro(schema_id: u64, macro_id: u64, begin_snapshot: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_MACRO);
    buf.extend_from_slice(&encode_u64(schema_id));
    buf.extend_from_slice(&encode_u64(macro_id));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

/// Build a key for `ducklake_macro_impl`: `0x09 | macro_id(u64) | impl_id(u64)`.
pub fn key_macro_impl(macro_id: u64, impl_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_MACRO_IMPL);
    buf.extend_from_slice(&encode_u64(macro_id));
    buf.extend_from_slice(&encode_u64(impl_id));
    buf
}

/// Build a key for `ducklake_macro_parameters`: `0x0A | macro_id(u64) | impl_id(u64) | column_id(u64)`.
pub fn key_macro_parameters(macro_id: u64, impl_id: u64, column_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_MACRO_PARAMETERS);
    buf.extend_from_slice(&encode_u64(macro_id));
    buf.extend_from_slice(&encode_u64(impl_id));
    buf.extend_from_slice(&encode_u64(column_id));
    buf
}

/// Build a key for `ducklake_data_file`: `0x0B | table_id(u64) | data_file_id(u64)`.
pub fn key_data_file(table_id: u64, data_file_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_DATA_FILE);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(data_file_id));
    buf
}

/// Build a key for `ducklake_delete_file`: `0x0C | data_file_id(u64) | delete_file_id(u64)`.
pub fn key_delete_file(data_file_id: u64, delete_file_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_DELETE_FILE);
    buf.extend_from_slice(&encode_u64(data_file_id));
    buf.extend_from_slice(&encode_u64(delete_file_id));
    buf
}

/// Build a key for `ducklake_files_scheduled_for_deletion`: `0x0D | schedule_start(u64) | data_file_id(u64)`.
pub fn key_files_scheduled_for_deletion(schedule_start: u64, data_file_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_FILES_SCHEDULED_FOR_DELETION);
    buf.extend_from_slice(&encode_u64(schedule_start));
    buf.extend_from_slice(&encode_u64(data_file_id));
    buf
}

/// Build a key for `ducklake_inlined_data_tables`: `0x0E | table_id(u64) | schema_version(u64)`.
pub fn key_inlined_data_tables(table_id: u64, schema_version: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_INLINED_DATA_TABLES);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(schema_version));
    buf
}

/// Build a key for `ducklake_column_mapping`: `0x0F | table_id(u64) | mapping_id(u64)`.
pub fn key_column_mapping(table_id: u64, mapping_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_COLUMN_MAPPING);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(mapping_id));
    buf
}

/// Build a key for `ducklake_name_mapping`: `0x10 | mapping_id(u64) | column_id(u64) | source_name_hash(u64)`.
pub fn key_name_mapping(mapping_id: u64, column_id: u64, source_name_hash: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_NAME_MAPPING);
    buf.extend_from_slice(&encode_u64(mapping_id));
    buf.extend_from_slice(&encode_u64(column_id));
    buf.extend_from_slice(&encode_u64(source_name_hash));
    buf
}

/// Build a key for `ducklake_table_stats`: `0x11 | table_id(u64)`.
pub fn key_table_stats(table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_TABLE_STATS);
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

/// Build a key for `ducklake_table_column_stats`: `0x12 | table_id(u64) | column_id(u64)`.
pub fn key_table_column_stats(table_id: u64, column_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_TABLE_COLUMN_STATS);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(column_id));
    buf
}

/// Build a key for `ducklake_file_column_stats`: `0x13 | table_id(u64) | column_id(u64) | data_file_id(u64)`.
pub fn key_file_column_stats(table_id: u64, column_id: u64, data_file_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_FILE_COLUMN_STATS);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(column_id));
    buf.extend_from_slice(&encode_u64(data_file_id));
    buf
}

/// Build a key for `ducklake_file_variant_stats`: `0x14 | table_id(u64) | column_id(u64) | variant_path_hash(u64) | data_file_id(u64)`.
pub fn key_file_variant_stats(
    table_id: u64,
    column_id: u64,
    variant_path_hash: u64,
    data_file_id: u64,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(33);
    buf.push(TAG_FILE_VARIANT_STATS);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(column_id));
    buf.extend_from_slice(&encode_u64(variant_path_hash));
    buf.extend_from_slice(&encode_u64(data_file_id));
    buf
}

/// Build a key for `ducklake_partition_info`: `0x15 | table_id(u64) | partition_id(u64) | begin_snapshot(u64)`.
pub fn key_partition_info(table_id: u64, partition_id: u64, begin_snapshot: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_PARTITION_INFO);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(partition_id));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

/// Build a key for `ducklake_partition_column`: `0x16 | partition_id(u64) | partition_key_index(u64)`.
pub fn key_partition_column(partition_id: u64, partition_key_index: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_PARTITION_COLUMN);
    buf.extend_from_slice(&encode_u64(partition_id));
    buf.extend_from_slice(&encode_u64(partition_key_index));
    buf
}

/// Build a key for `ducklake_file_partition_value`: `0x17 | table_id(u64) | partition_key_index(u64) | data_file_id(u64)`.
pub fn key_file_partition_value(
    table_id: u64,
    partition_key_index: u64,
    data_file_id: u64,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_FILE_PARTITION_VALUE);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(partition_key_index));
    buf.extend_from_slice(&encode_u64(data_file_id));
    buf
}

/// Build a key for `ducklake_sort_info`: `0x18 | table_id(u64) | sort_id(u64) | begin_snapshot(u64)`.
pub fn key_sort_info(table_id: u64, sort_id: u64, begin_snapshot: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_SORT_INFO);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(sort_id));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

/// Build a key for `ducklake_sort_expression`: `0x19 | sort_id(u64) | sort_key_index(u64)`.
pub fn key_sort_expression(sort_id: u64, sort_key_index: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_SORT_EXPRESSION);
    buf.extend_from_slice(&encode_u64(sort_id));
    buf.extend_from_slice(&encode_u64(sort_key_index));
    buf
}

/// Build a key for `ducklake_tag`: `0x1A | object_id(u64) | tag_key_hash(u64) | begin_snapshot(u64)`.
pub fn key_tag(object_id: u64, tag_key_hash: u64, begin_snapshot: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_TAG);
    buf.extend_from_slice(&encode_u64(object_id));
    buf.extend_from_slice(&encode_u64(tag_key_hash));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

/// Build a key for `ducklake_column_tag`: `0x1B | table_id(u64) | column_id(u64) | tag_key_hash(u64) | begin_snapshot(u64)`.
pub fn key_column_tag(
    table_id: u64,
    column_id: u64,
    tag_key_hash: u64,
    begin_snapshot: u64,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(33);
    buf.push(TAG_COLUMN_TAG);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(column_id));
    buf.extend_from_slice(&encode_u64(tag_key_hash));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

/// Build a key for `ducklake_schema_versions`: `0x1C | table_id(u64) | begin_snapshot(u64)`.
pub fn key_schema_versions(table_id: u64, begin_snapshot: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_SCHEMA_VERSIONS);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(begin_snapshot));
    buf
}

// ─── Inlined Row Keys ──────────────────────────────────────────────────────

/// Build a key for inlined insert row: `0xFD | 0x01 | table_id(u64) | schema_version(u64) | row_id(u64)`.
pub fn key_inlined_insert(table_id: u64, schema_version: u64, row_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(26);
    buf.push(TAG_INLINED_ROWS);
    buf.push(INLINED_SUBTYPE_INSERT);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(schema_version));
    buf.extend_from_slice(&encode_u64(row_id));
    buf
}

/// Build a key for inlined delete marker: `0xFD | 0x02 | table_id(u64) | data_file_id(u64) | row_id(u64)`.
pub fn key_inlined_delete(table_id: u64, data_file_id: u64, row_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(26);
    buf.push(TAG_INLINED_ROWS);
    buf.push(INLINED_SUBTYPE_DELETE);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(data_file_id));
    buf.extend_from_slice(&encode_u64(row_id));
    buf
}

// ─── Counter Keys ──────────────────────────────────────────────────────────

/// Build a key for a global counter: `0xFE | counter_id`.
pub fn key_counter(counter_id: u8) -> Vec<u8> {
    vec![TAG_COUNTERS, counter_id]
}

/// Build a key for a per-table column counter: `0xFE | 0x10 | table_id(u64)`.
pub fn key_counter_column_id(table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(10);
    buf.push(TAG_COUNTERS);
    buf.push(COUNTER_NEXT_COLUMN_ID_PREFIX);
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

// ─── System Keys ───────────────────────────────────────────────────────────

/// Build a system key: `0xFF | suffix`.
pub fn key_system(suffix: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(1 + suffix.len());
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(suffix);
    buf
}

/// Build an audit log key: `0xFF | "audit" | snapshot_id(u64)`.
pub fn key_audit(snapshot_id: u64) -> Vec<u8> {
    let prefix = b"audit";
    let mut buf = Vec::with_capacity(1 + prefix.len() + 8);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(prefix);
    buf.extend_from_slice(&encode_u64(snapshot_id));
    buf
}

/// Build the scan prefix for all audit log entries: `0xFF | "audit"`.
pub fn audit_prefix() -> Vec<u8> {
    let mut buf = Vec::with_capacity(6);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(b"audit");
    buf
}

// ─── Prefix Helpers ────────────────────────────────────────────────────────

/// Build a scan prefix for all entries of a given table tag.
pub fn prefix_for_tag(tag: u8) -> Vec<u8> {
    vec![tag]
}

/// Build a scan prefix for data files of a specific table: `0x0B | table_id`.
pub fn prefix_data_files_for_table(table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_DATA_FILE);
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

/// Build a scan prefix for columns of a specific table: `0x06 | table_id`.
pub fn prefix_columns_for_table(table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_COLUMN);
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

/// Build a scan prefix for tables in a schema: `0x05 | schema_id`.
pub fn prefix_tables_for_schema(schema_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_TABLE);
    buf.extend_from_slice(&encode_u64(schema_id));
    buf
}

/// Build a scan prefix for a specific table in a schema: `0x05 | schema_id | table_id`.
pub fn prefix_tables_for_schema_table(schema_id: u64, table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_TABLE);
    buf.extend_from_slice(&encode_u64(schema_id));
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

/// Build a secondary-index key for O(1) table→schema lookup: `0xFC | table_id(u64 BE)`.
pub fn key_table_by_id(table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_TABLE_BY_ID);
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

/// Build a scan prefix for inlined inserts of a table: `0xFD | 0x01 | table_id`.
pub fn prefix_inlined_inserts_for_table(table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(10);
    buf.push(TAG_INLINED_ROWS);
    buf.push(INLINED_SUBTYPE_INSERT);
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

/// Build a scan prefix for inlined deletes of a table: `0xFD | 0x02 | table_id`.
pub fn prefix_inlined_deletes_for_table(table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(10);
    buf.push(TAG_INLINED_ROWS);
    buf.push(INLINED_SUBTYPE_DELETE);
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

/// Build a scan prefix for views in a schema: `0x07 | schema_id`.
pub fn prefix_views_for_schema(schema_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_VIEW);
    buf.extend_from_slice(&encode_u64(schema_id));
    buf
}

/// Build a scan prefix for macros in a schema: `0x08 | schema_id`.
pub fn prefix_macros_for_schema(schema_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_MACRO);
    buf.extend_from_slice(&encode_u64(schema_id));
    buf
}

/// Build a scan prefix for macro implementations: `0x09 | macro_id`.
pub fn prefix_macro_impls(macro_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_MACRO_IMPL);
    buf.extend_from_slice(&encode_u64(macro_id));
    buf
}

/// Build a scan prefix for macro parameters: `0x0A | macro_id | impl_id`.
pub fn prefix_macro_params(macro_id: u64, impl_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_MACRO_PARAMETERS);
    buf.extend_from_slice(&encode_u64(macro_id));
    buf.extend_from_slice(&encode_u64(impl_id));
    buf
}

/// Build a scan prefix for tags on an object: `0x1A | object_id`.
pub fn prefix_tags_for_object(object_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9);
    buf.push(TAG_TAG);
    buf.extend_from_slice(&encode_u64(object_id));
    buf
}

/// Build a scan prefix for column tags: `0x1B | table_id | column_id`.
pub fn prefix_column_tags(table_id: u64, column_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_COLUMN_TAG);
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(column_id));
    buf
}

/// Extract the tag byte from a raw key. Returns error if key is empty.
pub fn extract_tag(key: &[u8]) -> Result<u8, KeyError> {
    if key.is_empty() {
        return Err(KeyError::TooShort {
            expected: 1,
            actual: 0,
        });
    }
    let tag = key[0];
    if !is_known_tag(tag) {
        return Err(KeyError::UnknownTag(tag));
    }
    Ok(tag)
}

// ─── Hot Key ───────────────────────────────────────────────────────────────

/// Build the hot-key system key: `0xFF | "hot-key"`.
/// Stores current snapshot ID and per-table file counts for cold-start optimization.
pub fn key_hot() -> Vec<u8> {
    key_system(SYSTEM_HOT_KEY)
}

// ─── Secondary Index Keys ──────────────────────────────────────────────────

/// Build a secondary index key for snapshot-scoped file lookups:
/// `0xFC | snapshot_id(u64) | table_id(u64) | data_file_id(u64)`.
pub fn key_secondary_index(snapshot_id: u64, table_id: u64, data_file_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.push(TAG_SECONDARY_INDEX);
    buf.extend_from_slice(&encode_u64(snapshot_id));
    buf.extend_from_slice(&encode_u64(table_id));
    buf.extend_from_slice(&encode_u64(data_file_id));
    buf
}

/// Build a scan prefix for all secondary index entries of a given snapshot and table:
/// `0xFC | snapshot_id(u64) | table_id(u64)`.
pub fn prefix_secondary_index(snapshot_id: u64, table_id: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(17);
    buf.push(TAG_SECONDARY_INDEX);
    buf.extend_from_slice(&encode_u64(snapshot_id));
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

// ─── Packed Table Metadata Key ─────────────────────────────────────────────

/// Build a key for packed table metadata: `0xFF | "packed-meta" | table_id(u64)`.
/// Stores all per-table metadata (columns, partitions, sort info) as one composite value.
pub fn key_packed_table_metadata(table_id: u64) -> Vec<u8> {
    let prefix = b"packed-meta";
    let mut buf = Vec::with_capacity(1 + prefix.len() + 8);
    buf.push(TAG_SYSTEM);
    buf.extend_from_slice(prefix);
    buf.extend_from_slice(&encode_u64(table_id));
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_snapshot_ordering() {
        let k1 = key_snapshot(5);
        let k2 = key_snapshot(6);
        assert!(k1 < k2);
    }

    #[test]
    fn key_table_ordering() {
        let k1 = key_table(1, 1, 1);
        let k2 = key_table(1, 1, 2);
        let k3 = key_table(1, 2, 1);
        let k4 = key_table(2, 1, 1);
        assert!(k1 < k2);
        assert!(k2 < k3);
        assert!(k3 < k4);
    }

    #[test]
    fn prefix_isolation() {
        let data_prefix = prefix_data_files_for_table(42);
        let col_prefix = prefix_columns_for_table(42);
        // Different tags → no overlap
        assert_ne!(data_prefix[0], col_prefix[0]);
    }

    #[test]
    fn metadata_key_structure() {
        let k = key_metadata(MetadataScope::Global, 0, "data_path");
        assert_eq!(k[0], TAG_METADATA);
        assert_eq!(k[1], 0x00); // Global scope
    }

    #[test]
    fn counter_key_structure() {
        let k = key_counter(COUNTER_NEXT_SNAPSHOT_ID);
        assert_eq!(k, vec![0xFE, 0x01]);
    }

    #[test]
    fn system_key_structure() {
        let k = key_system(SYSTEM_WRITER_EPOCH);
        assert_eq!(k[0], 0xFF);
        assert_eq!(&k[1..], b"writer-epoch");
    }

    #[test]
    fn inlined_insert_key_structure() {
        let k = key_inlined_insert(1, 1, 100);
        assert_eq!(k[0], TAG_INLINED_ROWS);
        assert_eq!(k[1], INLINED_SUBTYPE_INSERT);
    }

    #[test]
    fn inlined_delete_key_structure() {
        let k = key_inlined_delete(1, 5, 200);
        assert_eq!(k[0], TAG_INLINED_ROWS);
        assert_eq!(k[1], INLINED_SUBTYPE_DELETE);
    }

    #[test]
    fn extract_tag_valid() {
        let k = key_snapshot(1);
        assert_eq!(extract_tag(&k).unwrap(), TAG_SNAPSHOT);
    }

    #[test]
    fn extract_tag_unknown() {
        assert!(extract_tag(&[0x50]).is_err());
    }

    #[test]
    fn extract_tag_empty() {
        assert!(extract_tag(&[]).is_err());
    }
}
