//! SlateDuck PG-Wire — PostgreSQL wire protocol sidecar.
//!
//! This crate implements the Strategy B sidecar serving DuckDB via
//! the standard `postgres` extension.

pub mod error_mapping;
pub mod executor;
pub mod handler;
pub mod pg_types;
pub mod session;

pub use handler::SlateDuckHandler;
