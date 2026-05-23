//! SlateDuck SQL — bounded SQL dispatcher for the pgwire sidecar.
//!
//! This crate pattern-matches on sqlparser AST nodes to dispatch
//! DuckLake catalog SQL into typed CatalogStore operations.

pub mod dispatcher;
pub mod gluesql_spike;

pub use dispatcher::{dispatch, CatalogOp, DispatchError};
