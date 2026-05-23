//! Catalog path canonicalization.
//!
//! Never use raw string concatenation for object-store paths anywhere.

/// Mode for data path storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataPathMode {
    /// Absolute object-store URI (e.g., `s3://bucket/data/warehouse-a/`).
    Absolute,
    /// Relative to the data prefix, with explicit `path_is_relative` flag.
    RelativeToDataPrefix,
}

/// Encapsulates all path components for a SlateDuck catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogPath {
    /// Root of the object store (e.g., `s3://bucket/`).
    pub object_store_root: String,
    /// Prefix under which catalog (SlateDB) data is stored.
    pub catalog_prefix: String,
    /// Prefix under which Parquet data files are stored.
    pub data_prefix: String,
    /// How data paths are stored in the catalog.
    pub data_path_mode: DataPathMode,
}

impl CatalogPath {
    /// Create a new CatalogPath with all components.
    pub fn new(
        object_store_root: impl Into<String>,
        catalog_prefix: impl Into<String>,
        data_prefix: impl Into<String>,
        data_path_mode: DataPathMode,
    ) -> Self {
        let root = normalize_trailing_slash(object_store_root.into());
        let catalog = normalize_trailing_slash(catalog_prefix.into());
        let data = normalize_trailing_slash(data_prefix.into());
        Self {
            object_store_root: root,
            catalog_prefix: catalog,
            data_prefix: data,
            data_path_mode,
        }
    }

    /// Resolve a data file path to its full object-store URI.
    pub fn resolve_data_path(&self, stored_path: &str) -> String {
        match self.data_path_mode {
            DataPathMode::Absolute => stored_path.to_string(),
            DataPathMode::RelativeToDataPrefix => {
                format!(
                    "{}{}",
                    self.data_prefix,
                    stored_path.trim_start_matches('/')
                )
            }
        }
    }

    /// Convert an absolute data path to its stored form.
    pub fn to_stored_path(&self, absolute_path: &str) -> String {
        match self.data_path_mode {
            DataPathMode::Absolute => absolute_path.to_string(),
            DataPathMode::RelativeToDataPrefix => absolute_path
                .strip_prefix(&self.data_prefix)
                .unwrap_or(absolute_path)
                .to_string(),
        }
    }

    /// Full path to the catalog (SlateDB) directory.
    pub fn catalog_full_path(&self) -> String {
        format!(
            "{}{}",
            self.object_store_root,
            self.catalog_prefix.trim_start_matches('/')
        )
    }

    /// Full path to the data directory.
    pub fn data_full_path(&self) -> String {
        format!(
            "{}{}",
            self.object_store_root,
            self.data_prefix.trim_start_matches('/')
        )
    }
}

/// Ensure a path ends with a slash.
fn normalize_trailing_slash(mut s: String) -> String {
    if !s.is_empty() && !s.ends_with('/') {
        s.push('/');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_mode_resolve() {
        let cp = CatalogPath::new(
            "s3://mybucket",
            "catalogs/main",
            "data/warehouse",
            DataPathMode::Absolute,
        );
        let path = "s3://mybucket/data/warehouse/table1/file.parquet";
        assert_eq!(cp.resolve_data_path(path), path);
    }

    #[test]
    fn relative_mode_resolve() {
        let cp = CatalogPath::new(
            "s3://mybucket",
            "catalogs/main",
            "s3://mybucket/data/warehouse",
            DataPathMode::RelativeToDataPrefix,
        );
        assert_eq!(
            cp.resolve_data_path("table1/file.parquet"),
            "s3://mybucket/data/warehouse/table1/file.parquet"
        );
    }

    #[test]
    fn to_stored_path_relative() {
        let cp = CatalogPath::new(
            "s3://mybucket",
            "catalogs/main",
            "s3://mybucket/data/warehouse",
            DataPathMode::RelativeToDataPrefix,
        );
        assert_eq!(
            cp.to_stored_path("s3://mybucket/data/warehouse/table1/file.parquet"),
            "table1/file.parquet"
        );
    }

    #[test]
    fn trailing_slash_normalized() {
        let cp = CatalogPath::new("s3://bucket", "catalog", "data", DataPathMode::Absolute);
        assert!(cp.object_store_root.ends_with('/'));
        assert!(cp.catalog_prefix.ends_with('/'));
        assert!(cp.data_prefix.ends_with('/'));
    }

    #[test]
    fn catalog_full_path() {
        let cp = CatalogPath::new(
            "s3://mybucket",
            "catalogs/main",
            "data/warehouse",
            DataPathMode::Absolute,
        );
        assert_eq!(cp.catalog_full_path(), "s3://mybucket/catalogs/main/");
    }
}
