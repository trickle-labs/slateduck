//! SlateDuck Catalog: DuckLake catalog operations backed by SlateDB.

pub mod error;
pub mod init;
pub mod reader;
pub mod store;
pub mod verify;
pub mod writer;

pub use error::{CatalogError, CatalogResult};
pub use reader::CatalogReader;
pub use store::{CatalogStore, OpenOptions};
pub use writer::CatalogWriter;
