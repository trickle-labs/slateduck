//! SlateDuck PG-Wire: PostgreSQL wire protocol sidecar for DuckLake catalogs.
//!
//! Implements Strategy B: a sidecar process that speaks the PostgreSQL wire protocol
//! and translates DuckLake catalog SQL into CatalogStore operations.

pub mod error;
pub mod executor;
pub mod handler;
pub mod notify;
pub mod server;
pub mod session;
pub mod types;

pub use error::SlateDuckError;
pub use notify::{ConnectionSubscriptions, Notification, NotifyManager};
pub use server::{run_server_with_shutdown, AuthConfig, ServerConfig, TlsConfig};
