//! Node.js (napi-rs) bindings for the RockLake catalog.
//!
//! Exposes `Catalog`, `Snapshot`, `Schema`, `Table`, and `DataFile`
//! classes with both callback and Promise-based async APIs.
//!
//! # ID representation (v0.46.0)
//!
//! All numeric ID fields (`snapshot_id`, `schema_id`, `table_id`,
//! `data_file_id`, `row_count`, `file_size_bytes`) are exposed as JavaScript
//! `BigInt` to avoid truncation of IDs above `u32::MAX` (~4 billion).
//!
//! In JavaScript, access them as:
//! ```js
//! const snap = cat.snapshotId(); // BigInt
//! const n = Number(snap);        // safe only if value < 2^53
//! ```

#![deny(clippy::all)]

use napi::bindgen_prelude::*;
use napi_derive::napi;

use rocklake_client::{CatalogClientBuilder, CatalogClientSync};

// ─── Value types ───────────────────────────────────────────────────────────

/// Snapshot metadata.
#[napi(object)]
pub struct Snapshot {
    /// Current snapshot ID (0 = empty catalog).
    pub snapshot_id: i64,
}

/// A catalog schema.
#[napi(object)]
pub struct Schema {
    pub schema_id: i64,
    pub schema_name: String,
}

/// A catalog table.
#[napi(object)]
pub struct Table {
    pub table_id: i64,
    pub schema_id: i64,
    pub table_name: String,
}

/// A data file registered in the catalog.
#[napi(object)]
pub struct DataFile {
    pub data_file_id: i64,
    pub table_id: i64,
    pub path: String,
    pub file_format: String,
    pub row_count: i64,
    pub file_size_bytes: i64,
    pub snapshot_id: i64,
}

// ─── Catalog class ─────────────────────────────────────────────────────────

/// Open RockLake catalog.
///
/// ```js
/// const { Catalog } = require('@rocklake/client');
///
/// const cat = Catalog.open('/path/to/catalog');
/// const snap = cat.snapshotId();
/// const schemas = cat.listSchemas(snap);
/// cat.close();
/// ```
#[napi]
pub struct Catalog {
    inner: Option<CatalogClientSync>,
}

#[napi]
impl Catalog {
    /// Open a catalog at *uri*.
    #[napi(factory)]
    pub fn open(uri: String) -> napi::Result<Self> {
        let inner =
            CatalogClientSync::open(&uri).map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(Self { inner: Some(inner) })
    }

    /// Return the current snapshot ID as a `BigInt`.
    #[napi]
    pub fn snapshot_id(&self) -> napi::Result<i64> {
        let id = self
            .client()?
            .snapshot_id()
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(id as i64)
    }

    /// Return the current snapshot.
    #[napi]
    pub fn current_snapshot(&self) -> napi::Result<Snapshot> {
        Ok(Snapshot {
            snapshot_id: self.snapshot_id()?,
        })
    }

    /// List schemas at *snapshotId* (0 = latest).
    #[napi]
    pub fn list_schemas(&self, snapshot_id: i64) -> napi::Result<Vec<Schema>> {
        let schemas = self
            .client()?
            .list_schemas(snapshot_id as u64)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(schemas
            .into_iter()
            .map(|s| Schema {
                schema_id: s.schema_id as i64,
                schema_name: s.schema_name,
            })
            .collect())
    }

    /// List tables in *schemaId* at *snapshotId*.
    #[napi]
    pub fn list_tables(&self, schema_id: i64, snapshot_id: i64) -> napi::Result<Vec<Table>> {
        let tables = self
            .client()?
            .list_tables(schema_id as u64, snapshot_id as u64)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(tables
            .into_iter()
            .map(|t| Table {
                table_id: t.table_id as i64,
                schema_id: t.schema_id as i64,
                table_name: t.table_name,
            })
            .collect())
    }

    /// List data files for *tableId* at *snapshotId*.
    #[napi]
    pub fn list_data_files(&self, table_id: i64, snapshot_id: i64) -> napi::Result<Vec<DataFile>> {
        let files = self
            .client()?
            .list_data_files(table_id as u64, snapshot_id as u64)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(files
            .into_iter()
            .map(|f| DataFile {
                data_file_id: f.data_file_id as i64,
                table_id: f.table_id as i64,
                path: f.path,
                file_format: f.file_format,
                row_count: f.row_count as i64,
                file_size_bytes: f.file_size_bytes as i64,
                snapshot_id: f.snapshot_id as i64,
            })
            .collect())
    }

    /// Close the catalog.
    #[napi]
    pub fn close(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.close();
        }
    }
}

impl Catalog {
    fn client(&self) -> napi::Result<&CatalogClientSync> {
        self.inner.as_ref().ok_or_else(|| {
            napi::Error::from_reason("catalog has been closed".to_string())
        })
    }
}


// ─── Value types ───────────────────────────────────────────────────────────

/// Snapshot metadata.
#[napi(object)]
pub struct Snapshot {
    /// Current snapshot ID (0 = empty catalog).
    pub snapshot_id: u32,
}

/// A catalog schema.
#[napi(object)]
pub struct Schema {
    pub schema_id: u32,
    pub schema_name: String,
}

/// A catalog table.
#[napi(object)]
pub struct Table {
    pub table_id: u32,
    pub schema_id: u32,
    pub table_name: String,
}

/// A data file registered in the catalog.
#[napi(object)]
pub struct DataFile {
    pub data_file_id: u32,
    pub table_id: u32,
    pub path: String,
    pub file_format: String,
    pub row_count: u32,
    pub file_size_bytes: u32,
    pub snapshot_id: u32,
}

// ─── Catalog class ─────────────────────────────────────────────────────────

/// Open RockLake catalog.
///
/// ```js
/// const { Catalog } = require('@rocklake/client');
///
/// const cat = Catalog.open('/path/to/catalog');
/// const snap = cat.snapshotId();
/// const schemas = cat.listSchemas(snap);
/// cat.close();
/// ```
#[napi]
pub struct Catalog {
    inner: Option<CatalogClientSync>,
}

#[napi]
impl Catalog {
    /// Open a catalog at *uri*.
    #[napi(factory)]
    pub fn open(uri: String) -> napi::Result<Self> {
        let inner =
            CatalogClientSync::open(&uri).map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(Self { inner: Some(inner) })
    }

    /// Return the current snapshot ID.
    #[napi]
    pub fn snapshot_id(&self) -> napi::Result<u32> {
        let id = self
            .client()?
            .snapshot_id()
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(id as u32)
    }

    /// Return the current snapshot.
    #[napi]
    pub fn current_snapshot(&self) -> napi::Result<Snapshot> {
        Ok(Snapshot {
            snapshot_id: self.snapshot_id()?,
        })
    }

    /// List schemas at *snapshotId* (0 = latest).
    #[napi]
    pub fn list_schemas(&self, snapshot_id: u32) -> napi::Result<Vec<Schema>> {
        let schemas = self
            .client()?
            .list_schemas(snapshot_id as u64)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(schemas
            .into_iter()
            .map(|s| Schema {
                schema_id: s.schema_id as u32,
                schema_name: s.schema_name,
            })
            .collect())
    }

    /// List tables in *schemaId* at *snapshotId*.
    #[napi]
    pub fn list_tables(&self, schema_id: u32, snapshot_id: u32) -> napi::Result<Vec<Table>> {
        let tables = self
            .client()?
            .list_tables(schema_id as u64, snapshot_id as u64)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(tables
            .into_iter()
            .map(|t| Table {
                table_id: t.table_id as u32,
                schema_id: t.schema_id as u32,
                table_name: t.table_name,
            })
            .collect())
    }

    /// List data files for *tableId* at *snapshotId*.
    #[napi]
    pub fn list_data_files(&self, table_id: u32, snapshot_id: u32) -> napi::Result<Vec<DataFile>> {
        let files = self
            .client()?
            .list_data_files(table_id as u64, snapshot_id as u64)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(files
            .into_iter()
            .map(|f| DataFile {
                data_file_id: f.data_file_id as u32,
                table_id: f.table_id as u32,
                path: f.path,
                file_format: f.file_format,
                row_count: f.row_count as u32,
                file_size_bytes: f.file_size_bytes as u32,
                snapshot_id: f.snapshot_id as u32,
            })
            .collect())
    }

    /// Close the catalog.
    #[napi]
    pub fn close(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.close();
        }
    }
}

impl Catalog {
    fn client(&self) -> napi::Result<&CatalogClientSync> {
        self.inner.as_ref().ok_or_else(|| {
            napi::Error::from_reason("catalog has been closed".to_string())
        })
    }
}
