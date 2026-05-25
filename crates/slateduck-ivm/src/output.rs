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
pub async fn write_output_rows(
    writer: &mut CatalogWriter,
    output_table_id: u64,
    rows: &[HashMap<String, Value>],
) -> Result<(), IvmError> {
    for (i, row) in rows.iter().enumerate() {
        let payload = serde_json::to_vec(row).map_err(|e| IvmError::Output(e.to_string()))?;
        writer
            .register_inlined_insert(output_table_id, 1, i as u64, payload)
            .await
            .map_err(|e| IvmError::Catalog(e.to_string()))?;
    }
    Ok(())
}
