//! SlateDuck Core: foundational types, key/value encoding, and SlateDB integration.

#![deny(missing_docs)]

pub mod clock;
pub mod counters;
pub mod keys;
pub mod mvcc;
pub mod path;
pub mod rows;
pub mod tags;
pub mod types;
pub mod validation;
pub mod values;
