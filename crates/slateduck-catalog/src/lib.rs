//! SlateDuck Catalog: DuckLake catalog operations backed by SlateDB.

pub mod audit;
pub mod checkpoint;
pub mod cleanup;
pub mod encryption;
pub mod error;
pub mod excise;
pub mod export;
pub mod gc;
pub mod init;
pub mod inspect;
pub mod metrics;
pub mod partition;
pub mod performance;
pub mod reader;
pub mod repair;
pub mod store;
pub mod verify;
pub mod writer;

pub use audit::{AuditChange, AuditEntry};
pub use error::{CatalogError, CatalogResult};
pub use metrics::CatalogMetrics;
pub use partition::{CatalogRegistry, DatasetEntry, PartitionedWriter};
pub use performance::{BenchmarkReport, HotKeyState, SlateDbTuning};
pub use reader::CatalogReader;
pub use store::{CatalogStore, OpenOptions};
pub use writer::CatalogWriter;
