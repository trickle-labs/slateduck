//! Python bindings for RockLake catalog (PyO3).
//!
//! Exposes `RockLakeCatalog`, `RockLakeSnapshot`, `RockLakeSchema`,
//! `RockLakeTable`, and `RockLakeDataFile` as Python classes.
//!
//! Usage from Python:
//! ```python
//! from rocklake import RockLakeCatalog
//!
//! cat = RockLakeCatalog.open("file:///path/to/catalog")
//! snap = cat.snapshot_id()
//! schemas = cat.list_schemas(snap)
//! cat.close()
//! ```

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;

use rocklake_client::{CatalogClientSync, ClientError};

fn to_py(e: ClientError) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}

// ─── Value types ───────────────────────────────────────────────────────────

/// Current snapshot metadata.
#[pyclass]
#[derive(Clone)]
pub struct RockLakeSnapshot {
    /// Monotonic snapshot ID (0 = no snapshots yet).
    #[pyo3(get)]
    pub snapshot_id: u64,
}

#[pymethods]
impl RockLakeSnapshot {
    fn __repr__(&self) -> String {
        format!("RockLakeSnapshot(snapshot_id={})", self.snapshot_id)
    }
}

/// A catalog schema.
#[pyclass]
#[derive(Clone)]
pub struct RockLakeSchema {
    #[pyo3(get)]
    pub schema_id: u64,
    #[pyo3(get)]
    pub schema_name: String,
}

#[pymethods]
impl RockLakeSchema {
    fn __repr__(&self) -> String {
        format!(
            "RockLakeSchema(schema_id={}, schema_name={:?})",
            self.schema_id, self.schema_name
        )
    }
}

/// A catalog table.
#[pyclass]
#[derive(Clone)]
pub struct RockLakeTable {
    #[pyo3(get)]
    pub table_id: u64,
    #[pyo3(get)]
    pub schema_id: u64,
    #[pyo3(get)]
    pub table_name: String,
}

#[pymethods]
impl RockLakeTable {
    fn __repr__(&self) -> String {
        format!(
            "RockLakeTable(table_id={}, table_name={:?})",
            self.table_id, self.table_name
        )
    }
}

/// A data file registered in the catalog.
#[pyclass]
#[derive(Clone)]
pub struct RockLakeDataFile {
    #[pyo3(get)]
    pub data_file_id: u64,
    #[pyo3(get)]
    pub table_id: u64,
    #[pyo3(get)]
    pub path: String,
    #[pyo3(get)]
    pub file_format: String,
    #[pyo3(get)]
    pub row_count: u64,
    #[pyo3(get)]
    pub file_size_bytes: u64,
    #[pyo3(get)]
    pub snapshot_id: u64,
}

#[pymethods]
impl RockLakeDataFile {
    /// Return a dict compatible with pandas/polars DataFrame construction.
    pub fn to_dict(&self) -> std::collections::HashMap<&'static str, PyObject> {
        Python::with_gil(|py| {
            let mut m = std::collections::HashMap::new();
            m.insert("data_file_id", self.data_file_id.to_object(py));
            m.insert("table_id", self.table_id.to_object(py));
            m.insert("path", self.path.to_object(py));
            m.insert("file_format", self.file_format.to_object(py));
            m.insert("row_count", self.row_count.to_object(py));
            m.insert("file_size_bytes", self.file_size_bytes.to_object(py));
            m.insert("snapshot_id", self.snapshot_id.to_object(py));
            m
        })
    }

    fn __repr__(&self) -> String {
        format!(
            "RockLakeDataFile(data_file_id={}, path={:?})",
            self.data_file_id, self.path
        )
    }
}

// ─── RockLakeCatalog ───────────────────────────────────────────────────────

/// Open RockLake catalog handle.
///
/// Example::
///
///     from rocklake import RockLakeCatalog
///
///     cat = RockLakeCatalog.open("file:///tmp/my-catalog")
///     snap = cat.snapshot_id()
///     schemas = cat.list_schemas(snap)
///     print(schemas)
///     cat.close()
#[pyclass]
pub struct RockLakeCatalog {
    inner: Option<CatalogClientSync>,
}

#[pymethods]
impl RockLakeCatalog {
    /// Open a catalog at the given URI.
    ///
    /// :param uri: Catalog URI. Local paths and ``file://`` URIs are supported.
    /// :raises RuntimeError: if the catalog cannot be opened.
    #[staticmethod]
    pub fn open(uri: &str) -> PyResult<Self> {
        let inner = CatalogClientSync::open(uri).map_err(to_py)?;
        Ok(Self { inner: Some(inner) })
    }

    /// Open a catalog in read-only mode (no writer epoch acquired).
    ///
    /// Use this for stateless reader replicas and analytics sidecars.
    /// Many simultaneous ``open_readonly`` calls against the same catalog
    /// path produce zero write conflicts.
    ///
    /// :param uri: Catalog URI. Local paths and ``file://`` URIs are supported.
    /// :raises RuntimeError: if the catalog cannot be opened.
    #[staticmethod]
    pub fn open_readonly(uri: &str) -> PyResult<Self> {
        let inner = CatalogClientSync::open_readonly(uri).map_err(to_py)?;
        Ok(Self { inner: Some(inner) })
    }

    /// Return the current snapshot ID (0 = empty catalog).
    pub fn snapshot_id(&self) -> PyResult<u64> {
        self.client()?.snapshot_id().map_err(to_py)
    }

    /// Return the current snapshot as a :class:`RockLakeSnapshot`.
    pub fn current_snapshot(&self) -> PyResult<RockLakeSnapshot> {
        let id = self.snapshot_id()?;
        Ok(RockLakeSnapshot { snapshot_id: id })
    }

    /// List schemas visible at *snapshot_id* (0 = latest).
    pub fn list_schemas(&self, snapshot_id: u64) -> PyResult<Vec<RockLakeSchema>> {
        let schemas = self.client()?.list_schemas(snapshot_id).map_err(to_py)?;
        Ok(schemas
            .into_iter()
            .map(|s| RockLakeSchema {
                schema_id: s.schema_id,
                schema_name: s.schema_name,
            })
            .collect())
    }

    /// List tables in *schema_id* at *snapshot_id*.
    pub fn list_tables(&self, schema_id: u64, snapshot_id: u64) -> PyResult<Vec<RockLakeTable>> {
        let tables = self
            .client()?
            .list_tables(schema_id, snapshot_id)
            .map_err(to_py)?;
        Ok(tables
            .into_iter()
            .map(|t| RockLakeTable {
                table_id: t.table_id,
                schema_id: t.schema_id,
                table_name: t.table_name,
            })
            .collect())
    }

    /// List data files for *table_id* at *snapshot_id*.
    ///
    /// Returns a list of :class:`RockLakeDataFile` objects. To create a
    /// ``polars.DataFrame``::
    ///
    ///     import polars as pl
    ///     files = cat.list_data_files(table_id, 0)
    ///     df = pl.from_dicts([f.to_dict() for f in files])
    pub fn list_data_files(
        &self,
        table_id: u64,
        snapshot_id: u64,
    ) -> PyResult<Vec<RockLakeDataFile>> {
        let files = self
            .client()?
            .list_data_files(table_id, snapshot_id)
            .map_err(to_py)?;
        Ok(files
            .into_iter()
            .map(|f| RockLakeDataFile {
                data_file_id: f.data_file_id,
                table_id: f.table_id,
                path: f.path,
                file_format: f.file_format,
                row_count: f.row_count,
                file_size_bytes: f.file_size_bytes,
                snapshot_id: f.snapshot_id,
            })
            .collect())
    }

    /// Close the catalog and release all resources.
    pub fn close(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.close();
        }
    }

    fn __repr__(&self) -> &'static str {
        "RockLakeCatalog"
    }
}

impl RockLakeCatalog {
    fn client(&self) -> PyResult<&CatalogClientSync> {
        self.inner
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("catalog has been closed"))
    }
}

// ─── Module ────────────────────────────────────────────────────────────────

/// RockLake Python extension module.
#[pymodule]
fn _rocklake(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_class::<RockLakeCatalog>()?;
    m.add_class::<RockLakeSnapshot>()?;
    m.add_class::<RockLakeSchema>()?;
    m.add_class::<RockLakeTable>()?;
    m.add_class::<RockLakeDataFile>()?;
    Ok(())
}
