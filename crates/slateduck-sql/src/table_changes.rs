//! `table_changes()` SQL table function: exposes row-level CDC from DuckLake snapshots.
//!
//! Returns rows with `rowid`, `change_type`, and user columns for a given snapshot range.
//! Change types: `insert`, `delete`, `update_preimage`, `update_postimage`.
//!
//! When `start_snapshot` has been GC'd, returns SQLSTATE 55000 (snapshot too old).
//!
//! v0.27.1: Real Parquet row scanning via `object_store` + `parquet` crates.
//! `extract_rows_from_parquet()` opens Parquet files from object storage,
//! deserialises every record batch, and emits actual column values as JSON.
//! A `record_count` mismatch warning is emitted and counted when the scanned
//! row count differs from the value stored in catalog metadata.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Global counter for CDC record-count mismatches (N-04).
/// Incremented whenever scanned row count ≠ catalog `record_count` for a file.
/// Readable via `cdc_record_count_mismatch_total()` for inclusion in metrics.
static CDC_RECORD_COUNT_MISMATCHES: AtomicU64 = AtomicU64::new(0);

/// Return the current value of the `slateduck_cdc_record_count_mismatch_total` counter.
pub fn cdc_record_count_mismatch_total() -> u64 {
    CDC_RECORD_COUNT_MISMATCHES.load(Ordering::Relaxed)
}

/// Default batch size for Parquet streaming reads (50 000 rows per batch).
pub const DEFAULT_CDC_BATCH_SIZE: usize = 50_000;

/// Change type enumeration for table_changes output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    Insert,
    Delete,
    UpdatePreimage,
    UpdatePostimage,
}

impl ChangeType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChangeType::Insert => "insert",
            ChangeType::Delete => "delete",
            ChangeType::UpdatePreimage => "update_preimage",
            ChangeType::UpdatePostimage => "update_postimage",
        }
    }
}

impl std::fmt::Display for ChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single row from `table_changes()` output.
#[derive(Debug, Clone)]
pub struct ChangeRecord {
    /// Stable row identifier from Parquet metadata.
    pub rowid: Option<u64>,
    /// Type of change.
    pub change_type: ChangeType,
    /// JSON-encoded column values from the affected row (keyed by column name).
    pub columns_json: String,
}

/// Result of a `table_changes()` call.
#[derive(Debug, Clone)]
pub struct TableChangesResult {
    /// The resolved table name.
    pub table_ref: String,
    /// Start snapshot (inclusive).
    pub start_snapshot: u64,
    /// End snapshot (inclusive).
    pub end_snapshot: u64,
    /// Change records.
    pub records: Vec<ChangeRecord>,
}

/// Error type for table_changes operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableChangesError {
    /// SQLSTATE 55000: start_snapshot has been GC'd.
    SnapshotTooOld { requested: u64, retain_from: u64 },
    /// Table not found.
    TableNotFound(String),
    /// Object storage error while reading a Parquet data file.
    Storage(String),
    /// Generic error.
    Other(String),
}

impl TableChangesError {
    /// Returns the SQLSTATE code for this error.
    pub fn sqlstate(&self) -> &'static str {
        match self {
            TableChangesError::SnapshotTooOld { .. } => "55000",
            TableChangesError::TableNotFound(_) => "42P01",
            TableChangesError::Storage(_) => "58030",
            TableChangesError::Other(_) => "XX000",
        }
    }
}

impl std::fmt::Display for TableChangesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TableChangesError::SnapshotTooOld {
                requested,
                retain_from,
            } => write!(
                f,
                "snapshot {requested} has been garbage collected (retain_from={retain_from})"
            ),
            TableChangesError::TableNotFound(t) => write!(f, "table not found: {t}"),
            TableChangesError::Storage(msg) => write!(f, "object storage error: {msg}"),
            TableChangesError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for TableChangesError {}

/// A row extracted from a Parquet file with actual column values.
#[derive(Debug, Clone)]
pub struct ParquetRowData {
    /// Row ID — sequential within the file, starting at `base_rowid`.
    pub rowid: u64,
    /// Column values keyed by column name, JSON-encoded object string.
    pub columns_json: String,
}

/// Open a Parquet file from `object_store`, scan all record batches, and
/// return one `ParquetRowData` per row with real column values as JSON.
///
/// # Record-count verification (N-04)
/// When `expected_record_count` is `Some(n)` and the scanned row count differs
/// from `n`, a structured `tracing::warn!` is emitted and the global
/// `slateduck_cdc_record_count_mismatch_total` counter is incremented.
/// The scanned count is always used — the catalog value is informational.
///
/// # Batching
/// `batch_size` rows are requested per record-batch from the Parquet reader.
/// Use `DEFAULT_CDC_BATCH_SIZE` (50 000) unless you have specific memory
/// constraints.
///
/// # Errors
/// Returns `TableChangesError::Storage` if the object store returns any error
/// (including `NotFound` for a missing data file path).
pub async fn extract_rows_from_parquet(
    object_store: &Arc<dyn object_store::ObjectStore>,
    file_path: &str,
    base_rowid: u64,
    expected_record_count: Option<u64>,
    batch_size: usize,
) -> Result<Vec<ParquetRowData>, TableChangesError> {
    let path = object_store::path::Path::from(file_path);

    // Fetch all bytes from object storage.  Using the bytes-based path avoids
    // the object_store version conflict that arises with parquet's async reader
    // (parquet 54 depends on object_store 0.11, workspace uses 0.12).
    let bytes = object_store
        .get(&path)
        .await
        .map_err(|e| TableChangesError::Storage(e.to_string()))?
        .bytes()
        .await
        .map_err(|e| TableChangesError::Storage(e.to_string()))?;

    // Use the synchronous reader on the in-memory bytes.
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(bytes)
        .map_err(|e| TableChangesError::Storage(e.to_string()))?
        .with_batch_size(batch_size)
        .build()
        .map_err(|e| TableChangesError::Storage(e.to_string()))?;

    let mut rows = Vec::new();
    let mut rowid = base_rowid;

    for batch_result in reader {
        let batch = batch_result.map_err(|e| TableChangesError::Storage(e.to_string()))?;
        let schema = batch.schema();
        for row_idx in 0..batch.num_rows() {
            let mut col_map = serde_json::Map::new();
            for col_idx in 0..batch.num_columns() {
                let field = schema.field(col_idx);
                let array = batch.column(col_idx);
                let val = arrow_to_json_value(array.as_ref(), row_idx);
                col_map.insert(field.name().clone(), val);
            }
            rows.push(ParquetRowData {
                rowid,
                columns_json: serde_json::Value::Object(col_map).to_string(),
            });
            rowid += 1;
        }
    }

    // N-04: verify record_count against actual scanned rows.
    if let Some(expected) = expected_record_count {
        let actual = rows.len() as u64;
        if actual != expected {
            CDC_RECORD_COUNT_MISMATCHES.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                file_path = file_path,
                expected_record_count = expected,
                actual_record_count = actual,
                counter = "slateduck_cdc_record_count_mismatch_total",
                "CDC record count mismatch: catalog metadata says {} rows but \
                 scanned {}; using scanned count (partial-write recovery path)",
                expected,
                actual,
            );
        }
    }

    Ok(rows)
}

/// Convert a single value in an Arrow array to a `serde_json::Value`.
///
/// Complex or unrecognised types are rendered as a string `"<DataType>"`.
fn arrow_to_json_value(array: &dyn arrow::array::Array, row_idx: usize) -> serde_json::Value {
    use arrow::array::{
        BinaryArray, BooleanArray, Date32Array, Date64Array, Float32Array, Float64Array,
        Int16Array, Int32Array, Int64Array, Int8Array, LargeStringArray, StringArray, UInt16Array,
        UInt32Array, UInt64Array, UInt8Array,
    };
    use arrow::datatypes::DataType;

    if array.is_null(row_idx) {
        return serde_json::Value::Null;
    }

    match array.data_type() {
        DataType::Boolean => {
            let arr = array.as_any().downcast_ref::<BooleanArray>().unwrap();
            serde_json::Value::Bool(arr.value(row_idx))
        }
        DataType::Int8 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<Int8Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::Int16 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<Int16Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::Int32 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::Int64 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::UInt8 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<UInt8Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::UInt16 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<UInt16Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::UInt32 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<UInt32Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::UInt64 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<UInt64Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::Float32 => {
            let v = array
                .as_any()
                .downcast_ref::<Float32Array>()
                .unwrap()
                .value(row_idx);
            if v.is_finite() {
                serde_json::json!(v as f64)
            } else {
                serde_json::Value::Null
            }
        }
        DataType::Float64 => {
            let v = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .value(row_idx);
            if v.is_finite() {
                serde_json::json!(v)
            } else {
                serde_json::Value::Null
            }
        }
        DataType::Utf8 => serde_json::Value::String(
            array
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(row_idx)
                .to_string(),
        ),
        DataType::LargeUtf8 => serde_json::Value::String(
            array
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .unwrap()
                .value(row_idx)
                .to_string(),
        ),
        DataType::Binary => {
            // Encode raw bytes as hex for round-trip safety.
            let v = array
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap()
                .value(row_idx);
            let hex: String = v.iter().map(|b| format!("{b:02x}")).collect();
            serde_json::Value::String(hex)
        }
        DataType::Date32 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<Date32Array>()
                .unwrap()
                .value(row_idx))
        }
        DataType::Date64 => {
            serde_json::json!(array
                .as_any()
                .downcast_ref::<Date64Array>()
                .unwrap()
                .value(row_idx))
        }
        other => {
            // Complex or unsupported types: render type name as a string.
            serde_json::Value::String(format!("<{other}>"))
        }
    }
}

/// Resolve table_changes for a snapshot range using catalog diff.
///
/// v0.19: Full row-level CDC implementation.
/// - Files in `added_rows` produce INSERT records
/// - Files in `removed_rows` produce DELETE records
/// - Rows with matching rowids in both sets produce UPDATE (preimage + postimage) pairs
///
/// The `added_rows` and `removed_rows` parameters contain rows extracted from Parquet
/// files in the diff's added/retired data file lists respectively.
pub fn compute_table_changes(
    table_ref: &str,
    start_snapshot: u64,
    end_snapshot: u64,
    retain_from: u64,
    added_rows: &[ParquetRowData],
    removed_rows: &[ParquetRowData],
) -> Result<TableChangesResult, TableChangesError> {
    // Check GC boundary
    if start_snapshot < retain_from && retain_from > 0 {
        return Err(TableChangesError::SnapshotTooOld {
            requested: start_snapshot,
            retain_from,
        });
    }

    let mut records = Vec::new();

    // Build index of removed rows by rowid for update detection.
    let mut removed_by_rowid: HashMap<u64, &ParquetRowData> = HashMap::new();
    for row in removed_rows {
        removed_by_rowid.insert(row.rowid, row);
    }

    // Build index of added rows by rowid for update detection.
    let mut added_by_rowid: HashMap<u64, &ParquetRowData> = HashMap::new();
    for row in added_rows {
        added_by_rowid.insert(row.rowid, row);
    }

    // Detect updates: rows present in both removed and added with same rowid.
    let mut updated_rowids = std::collections::HashSet::new();
    for rowid in removed_by_rowid.keys() {
        if added_by_rowid.contains_key(rowid) {
            updated_rowids.insert(*rowid);
        }
    }

    // Emit update pre/post images for matching rowids.
    for &rowid in &updated_rowids {
        let preimage = removed_by_rowid[&rowid];
        let postimage = added_by_rowid[&rowid];
        records.push(ChangeRecord {
            rowid: Some(rowid),
            change_type: ChangeType::UpdatePreimage,
            columns_json: preimage.columns_json.clone(),
        });
        records.push(ChangeRecord {
            rowid: Some(rowid),
            change_type: ChangeType::UpdatePostimage,
            columns_json: postimage.columns_json.clone(),
        });
    }

    // Emit INSERTs for added rows that are not updates.
    for row in added_rows {
        if !updated_rowids.contains(&row.rowid) {
            records.push(ChangeRecord {
                rowid: Some(row.rowid),
                change_type: ChangeType::Insert,
                columns_json: row.columns_json.clone(),
            });
        }
    }

    // Emit DELETEs for removed rows that are not updates.
    for row in removed_rows {
        if !updated_rowids.contains(&row.rowid) {
            records.push(ChangeRecord {
                rowid: Some(row.rowid),
                change_type: ChangeType::Delete,
                columns_json: row.columns_json.clone(),
            });
        }
    }

    Ok(TableChangesResult {
        table_ref: table_ref.to_string(),
        start_snapshot,
        end_snapshot,
        records,
    })
}

/// Apply a change stream to a start-snapshot state and verify reconstruction.
///
/// This is a property-test helper: given start-state rows and a change stream,
/// produces the expected end-state rows.
pub fn apply_changes(
    start_state: &[ParquetRowData],
    changes: &[ChangeRecord],
) -> Vec<ParquetRowData> {
    let mut state: HashMap<u64, ParquetRowData> =
        start_state.iter().map(|r| (r.rowid, r.clone())).collect();

    for change in changes {
        let rowid = change.rowid.unwrap_or(0);
        match change.change_type {
            ChangeType::Insert => {
                state.insert(
                    rowid,
                    ParquetRowData {
                        rowid,
                        columns_json: change.columns_json.clone(),
                    },
                );
            }
            ChangeType::Delete => {
                state.remove(&rowid);
            }
            ChangeType::UpdatePostimage => {
                state.insert(
                    rowid,
                    ParquetRowData {
                        rowid,
                        columns_json: change.columns_json.clone(),
                    },
                );
            }
            ChangeType::UpdatePreimage => {
                // Preimage is informational; the postimage applies the state change.
            }
        }
    }

    let mut rows: Vec<_> = state.into_values().collect();
    rows.sort_by_key(|r| r.rowid);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_change_type_display() {
        assert_eq!(ChangeType::Insert.as_str(), "insert");
        assert_eq!(ChangeType::Delete.as_str(), "delete");
        assert_eq!(ChangeType::UpdatePreimage.as_str(), "update_preimage");
        assert_eq!(ChangeType::UpdatePostimage.as_str(), "update_postimage");
    }

    #[test]
    fn test_snapshot_too_old_error() {
        let result = compute_table_changes("public.orders", 5, 10, 8, &[], &[]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.sqlstate(), "55000");
        assert!(err.to_string().contains("garbage collected"));
    }

    #[test]
    fn test_no_gc_boundary() {
        let added = vec![
            ParquetRowData {
                rowid: 0,
                columns_json: r#"{"id":1,"name":"alice"}"#.to_string(),
            },
            ParquetRowData {
                rowid: 1,
                columns_json: r#"{"id":2,"name":"bob"}"#.to_string(),
            },
            ParquetRowData {
                rowid: 2,
                columns_json: r#"{"id":3,"name":"carol"}"#.to_string(),
            },
        ];
        let result = compute_table_changes(
            "public.orders",
            5,
            10,
            0, // retain_from=0 means infinite retention
            &added,
            &[],
        );
        assert!(result.is_ok());
        let changes = result.unwrap();
        assert_eq!(changes.records.len(), 3);
        assert!(changes
            .records
            .iter()
            .all(|r| r.change_type == ChangeType::Insert));
    }

    #[test]
    fn test_insert_and_delete_changes() {
        let added = vec![
            ParquetRowData {
                rowid: 0,
                columns_json: r#"{"id":1}"#.to_string(),
            },
            ParquetRowData {
                rowid: 1,
                columns_json: r#"{"id":2}"#.to_string(),
            },
        ];
        let removed = vec![ParquetRowData {
            rowid: 10,
            columns_json: r#"{"id":99}"#.to_string(),
        }];
        let result = compute_table_changes("public.orders", 5, 10, 0, &added, &removed);
        assert!(result.is_ok());
        let changes = result.unwrap();
        let inserts: Vec<_> = changes
            .records
            .iter()
            .filter(|r| r.change_type == ChangeType::Insert)
            .collect();
        let deletes: Vec<_> = changes
            .records
            .iter()
            .filter(|r| r.change_type == ChangeType::Delete)
            .collect();
        assert_eq!(inserts.len(), 2);
        assert_eq!(deletes.len(), 1);
        // Verify real column values are present
        assert!(inserts[0].columns_json.contains("id"));
        assert!(deletes[0].columns_json.contains("99"));
    }

    #[test]
    fn test_update_detection() {
        // Row with rowid=5 is in both removed (preimage) and added (postimage)
        let added = vec![
            ParquetRowData {
                rowid: 5,
                columns_json: r#"{"id":5,"name":"updated"}"#.to_string(),
            },
            ParquetRowData {
                rowid: 6,
                columns_json: r#"{"id":6,"name":"new"}"#.to_string(),
            },
        ];
        let removed = vec![
            ParquetRowData {
                rowid: 5,
                columns_json: r#"{"id":5,"name":"original"}"#.to_string(),
            },
            ParquetRowData {
                rowid: 7,
                columns_json: r#"{"id":7,"name":"deleted"}"#.to_string(),
            },
        ];
        let result = compute_table_changes("public.orders", 5, 10, 0, &added, &removed);
        assert!(result.is_ok());
        let changes = result.unwrap();

        let preimages: Vec<_> = changes
            .records
            .iter()
            .filter(|r| r.change_type == ChangeType::UpdatePreimage)
            .collect();
        let postimages: Vec<_> = changes
            .records
            .iter()
            .filter(|r| r.change_type == ChangeType::UpdatePostimage)
            .collect();
        let inserts: Vec<_> = changes
            .records
            .iter()
            .filter(|r| r.change_type == ChangeType::Insert)
            .collect();
        let deletes: Vec<_> = changes
            .records
            .iter()
            .filter(|r| r.change_type == ChangeType::Delete)
            .collect();

        assert_eq!(preimages.len(), 1);
        assert_eq!(postimages.len(), 1);
        assert_eq!(inserts.len(), 1); // rowid=6 is new
        assert_eq!(deletes.len(), 1); // rowid=7 is deleted
        assert!(preimages[0].columns_json.contains("original"));
        assert!(postimages[0].columns_json.contains("updated"));
    }

    #[test]
    fn test_apply_changes_reconstructs_end_state() {
        // Start state: rows 1, 2, 3
        let start_state = vec![
            ParquetRowData {
                rowid: 1,
                columns_json: r#"{"v":"a"}"#.to_string(),
            },
            ParquetRowData {
                rowid: 2,
                columns_json: r#"{"v":"b"}"#.to_string(),
            },
            ParquetRowData {
                rowid: 3,
                columns_json: r#"{"v":"c"}"#.to_string(),
            },
        ];

        // Changes: delete row 2, update row 3, insert row 4
        let added = vec![
            ParquetRowData {
                rowid: 3,
                columns_json: r#"{"v":"c_updated"}"#.to_string(),
            },
            ParquetRowData {
                rowid: 4,
                columns_json: r#"{"v":"d"}"#.to_string(),
            },
        ];
        let removed = vec![
            ParquetRowData {
                rowid: 2,
                columns_json: r#"{"v":"b"}"#.to_string(),
            },
            ParquetRowData {
                rowid: 3,
                columns_json: r#"{"v":"c"}"#.to_string(),
            },
        ];

        let result = compute_table_changes("t", 1, 2, 0, &added, &removed).unwrap();

        // Apply changes to start state
        let end_state = apply_changes(&start_state, &result.records);

        // Expected end state: rows 1, 3 (updated), 4
        assert_eq!(end_state.len(), 3);
        assert_eq!(end_state[0].rowid, 1);
        assert_eq!(end_state[0].columns_json, r#"{"v":"a"}"#);
        assert_eq!(end_state[1].rowid, 3);
        assert_eq!(end_state[1].columns_json, r#"{"v":"c_updated"}"#);
        assert_eq!(end_state[2].rowid, 4);
        assert_eq!(end_state[2].columns_json, r#"{"v":"d"}"#);
    }

    #[test]
    fn test_table_changes_error_sqlstate() {
        let err = TableChangesError::SnapshotTooOld {
            requested: 5,
            retain_from: 8,
        };
        assert_eq!(err.sqlstate(), "55000");

        let err = TableChangesError::TableNotFound("orders".to_string());
        assert_eq!(err.sqlstate(), "42P01");

        let err = TableChangesError::Storage("s3 error".to_string());
        assert_eq!(err.sqlstate(), "58030");
    }

    #[test]
    fn test_real_rowid_values() {
        let added = vec![
            ParquetRowData {
                rowid: 42,
                columns_json: r#"{"x":1}"#.to_string(),
            },
            ParquetRowData {
                rowid: 100,
                columns_json: r#"{"x":2}"#.to_string(),
            },
        ];
        let result = compute_table_changes("t", 0, 1, 0, &added, &[]).unwrap();
        assert_eq!(result.records[0].rowid, Some(42));
        assert_eq!(result.records[1].rowid, Some(100));
    }

    #[test]
    fn test_storage_error_display() {
        let err = TableChangesError::Storage("connection refused".to_string());
        assert!(err.to_string().contains("object storage error"));
        assert!(err.to_string().contains("connection refused"));
    }
}
