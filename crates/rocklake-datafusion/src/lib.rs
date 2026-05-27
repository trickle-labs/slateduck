//! DataFusion integration for RockLake.
//!
//! Implements DataFusion's `CatalogProvider` trait backed by `CatalogStore`,
//! enabling DataFusion users to run SQL against a RockLake-backed lakehouse
//! without DuckDB.

pub mod catalog_provider;
pub mod virtual_catalog;

pub use catalog_provider::RockLakeCatalogProvider;
pub use virtual_catalog::VirtualCatalogContext;
