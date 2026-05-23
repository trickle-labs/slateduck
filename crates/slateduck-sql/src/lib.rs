//! SlateDuck SQL: bounded SQL dispatcher for DuckLake wire protocol translation.
//!
//! This crate implements exactly the SQL statement shapes observed in the Phase 0
//! wire corpus. Pattern matching is done on `sqlparser-rs` AST nodes — never on
//! raw SQL strings — and parameter values are substituted at dispatch time.
//!
//! Anything outside the bounded set returns `SQLSTATE 0A000` (feature not supported).

pub mod classifier;
pub mod error;
pub mod params;

pub use classifier::{classify_statement, StatementKind};
pub use error::SqlDispatchError;
pub use params::ParamValues;
