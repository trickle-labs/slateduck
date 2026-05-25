//! Output: write the current aggregate state to the catalog as inlined inserts.

use std::collections::HashMap;

use serde_json::Value;
use slateduck_catalog::CatalogWriter;

use crate::worker::IvmError;

/// Write the current IVM output rows to the catalog.
///
/// Each output row is serialised as JSON and written as an inlined insert into
/// the matview's output table.  The caller must call `create_snapshot()` after
/// all rows have been staged.
///
/// `shard_id` is used to namespace the row IDs so that multiple shards writing
/// to the same output table do not collide.  The row ID layout is:
/// `(shard_id as u64) << 24 | row_index_within_shard`.
pub async fn write_output_rows(
    writer: &mut CatalogWriter,
    output_table_id: u64,
    shard_id: u32,
    rows: &[HashMap<String, Value>],
) -> Result<(), IvmError> {
    let shard_offset = (shard_id as u64) << 24;
    for (i, row) in rows.iter().enumerate() {
        let row_id = shard_offset | (i as u64 & 0x00FF_FFFF);
        let payload = serde_json::to_vec(row).map_err(|e| IvmError::Output(e.to_string()))?;
        writer
            .register_inlined_insert(output_table_id, 1, row_id, payload)
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;
    }
    Ok(())
}
