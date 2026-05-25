//! Extension schema: first-class catalog objects for application-defined metadata.
//!
//! Registered extensions (e.g., `pgtrickle`) can create, read, and delete rows
//! in their own tables within the SlateDuck catalog. This avoids the need for
//! a separate SQLite sidecar and guarantees crash-consistent storage.

use prost::Message;
use slatedb::Db;
use slateduck_core::keys;
use slateduck_core::rows::ExtensionSchemaRow;

use crate::error::{CatalogError, CatalogResult};

/// Known extension schemas and their IDs.
pub const EXTENSION_PGTRICKLE: u8 = 0x01;

/// Map an extension schema name to its ID, if registered.
pub fn resolve_extension_id(schema_name: &str) -> Option<u8> {
    match schema_name.to_lowercase().as_str() {
        "pgtrickle" => Some(EXTENSION_PGTRICKLE),
        _ => None,
    }
}

/// Check if an extension schema is registered.
pub fn is_registered_extension(schema_name: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|s| s.eq_ignore_ascii_case(schema_name))
}

/// Create an extension table (idempotent — CREATE TABLE IF NOT EXISTS semantics).
/// Returns true if the table was newly created, false if it already existed.
pub async fn create_extension_table(
    db: &Db,
    extension_id: u8,
    table_name: &str,
) -> CatalogResult<bool> {
    // Check if any row exists for this table already (marker row with row_id=0).
    let marker_key = keys::key_extension_schema(extension_id, table_name, 0);
    if db.get(&marker_key).await?.is_some() {
        return Ok(false); // Already exists
    }

    // Write a marker row (row_id=0) to indicate the table exists.
    let marker = ExtensionSchemaRow {
        extension_id: extension_id as u32,
        table_name: table_name.to_string(),
        row_id: 0,
        data_json: "{}".to_string(),
    };
    db.put(&marker_key, &marker.encode_to_vec()).await?;
    Ok(true)
}

/// Insert a row into an extension table. Returns the assigned row_id.
pub async fn insert_extension_row(
    db: &Db,
    extension_id: u8,
    table_name: &str,
    data_json: &str,
) -> CatalogResult<u64> {
    // Use a simple counter to assign row IDs.
    let counter_key = keys::key_extension_schema(extension_id, table_name, 0);
    let next_id = match db.get(&counter_key).await? {
        Some(data) => {
            if let Ok(marker) = ExtensionSchemaRow::decode(data.as_ref()) {
                // Parse the next_id from the marker's data_json field.
                let next: u64 = marker
                    .data_json
                    .strip_prefix("{\"next_id\":")
                    .and_then(|s| s.strip_suffix('}'))
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
                next
            } else {
                1
            }
        }
        None => 1,
    };

    // Write the data row.
    let row = ExtensionSchemaRow {
        extension_id: extension_id as u32,
        table_name: table_name.to_string(),
        row_id: next_id,
        data_json: data_json.to_string(),
    };
    let key = keys::key_extension_schema(extension_id, table_name, next_id);
    db.put(&key, &row.encode_to_vec()).await?;

    // Update the marker with the next ID.
    let marker = ExtensionSchemaRow {
        extension_id: extension_id as u32,
        table_name: table_name.to_string(),
        row_id: 0,
        data_json: format!("{{\"next_id\":{}}}", next_id + 1),
    };
    db.put(&counter_key, &marker.encode_to_vec()).await?;

    Ok(next_id)
}

/// Select all rows from an extension table.
pub async fn select_extension_rows(
    db: &Db,
    extension_id: u8,
    table_name: &str,
) -> CatalogResult<Vec<ExtensionSchemaRow>> {
    let prefix = keys::prefix_extension_table(extension_id, table_name);
    let mut rows = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;

    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        if let Ok(row) = ExtensionSchemaRow::decode(kv.value.as_ref()) {
            // Skip marker rows (row_id == 0).
            if row.row_id > 0 {
                rows.push(row);
            }
        }
    }

    Ok(rows)
}

/// Delete all rows from an extension table (but keep the table marker).
pub async fn delete_extension_rows(
    db: &Db,
    extension_id: u8,
    table_name: &str,
) -> CatalogResult<u64> {
    let prefix = keys::prefix_extension_table(extension_id, table_name);
    let mut keys_to_delete = Vec::new();
    let mut iter = db.scan_prefix(&prefix).await?;

    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| CatalogError::SlateDb(e.to_string()))?
    {
        if let Ok(row) = ExtensionSchemaRow::decode(kv.value.as_ref()) {
            if row.row_id > 0 {
                keys_to_delete.push(kv.key.to_vec());
            }
        }
    }

    let count = keys_to_delete.len() as u64;
    for key in keys_to_delete {
        db.delete(&key).await?;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_extension_id() {
        assert_eq!(resolve_extension_id("pgtrickle"), Some(EXTENSION_PGTRICKLE));
        assert_eq!(resolve_extension_id("PgTrickle"), Some(EXTENSION_PGTRICKLE));
        assert_eq!(resolve_extension_id("unknown"), None);
    }

    #[test]
    fn test_is_registered_extension() {
        let allowed = vec!["pgtrickle".to_string()];
        assert!(is_registered_extension("pgtrickle", &allowed));
        assert!(!is_registered_extension("unknown", &allowed));
    }
}
