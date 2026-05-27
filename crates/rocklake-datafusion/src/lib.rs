//! DataFusion integration for Rocklake.
//!
//! Implements DataFusion's `CatalogProvider` trait backed by `CatalogStore`,
//! enabling DataFusion users to run SQL against a Rocklake-backed lakehouse
//! without DuckDB.

pub mod catalog_provider;

pub use catalog_provider::RocklakeCatalogProvider;
