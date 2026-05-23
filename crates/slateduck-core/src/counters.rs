//! Counter and ID allocation backed by SlateDB.
//!
//! Counter allocation, counter increment, and the row that consumes the ID
//! must commit in a single SlateDB `DbTransaction`.
//! The in-memory counter value is cached by the writer process.

use crate::keys;
use crate::tags::*;
use crate::values;

/// All DuckLake ID counter domains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CounterDomain {
    /// Next snapshot ID.
    SnapshotId,
    /// Next catalog ID (schemas, tables, views, macros).
    CatalogId,
    /// Next file ID (data files, delete files).
    FileId,
    /// Next column ID for a specific table.
    ColumnId(u64),
}

impl CounterDomain {
    /// Get the SlateDB key for this counter.
    pub fn key(&self) -> Vec<u8> {
        match self {
            Self::SnapshotId => keys::key_counter(COUNTER_NEXT_SNAPSHOT_ID),
            Self::CatalogId => keys::key_counter(COUNTER_NEXT_CATALOG_ID),
            Self::FileId => keys::key_counter(COUNTER_NEXT_FILE_ID),
            Self::ColumnId(table_id) => keys::key_counter_column_id(*table_id),
        }
    }
}

/// In-memory counter cache for the writer process.
#[derive(Debug)]
pub struct CounterCache {
    next_snapshot_id: u64,
    next_catalog_id: u64,
    next_file_id: u64,
}

impl CounterCache {
    /// Create a new counter cache with initial values (loaded from SlateDB on open).
    pub fn new(next_snapshot_id: u64, next_catalog_id: u64, next_file_id: u64) -> Self {
        Self {
            next_snapshot_id,
            next_catalog_id,
            next_file_id,
        }
    }

    /// Allocate the next snapshot ID. Updates the cache.
    pub fn alloc_snapshot_id(&mut self) -> u64 {
        let id = self.next_snapshot_id;
        self.next_snapshot_id += 1;
        id
    }

    /// Allocate the next catalog ID. Updates the cache.
    pub fn alloc_catalog_id(&mut self) -> u64 {
        let id = self.next_catalog_id;
        self.next_catalog_id += 1;
        id
    }

    /// Allocate the next file ID. Updates the cache.
    pub fn alloc_file_id(&mut self) -> u64 {
        let id = self.next_file_id;
        self.next_file_id += 1;
        id
    }

    /// Get the current next snapshot ID without allocating.
    pub fn peek_snapshot_id(&self) -> u64 {
        self.next_snapshot_id
    }

    /// Get the current next catalog ID without allocating.
    pub fn peek_catalog_id(&self) -> u64 {
        self.next_catalog_id
    }

    /// Get the current next file ID without allocating.
    pub fn peek_file_id(&self) -> u64 {
        self.next_file_id
    }

    /// Encode the counter value for writing to SlateDB.
    pub fn encode_snapshot_counter(&self) -> Vec<u8> {
        values::encode_counter(self.next_snapshot_id)
    }

    /// Encode the catalog counter value for writing to SlateDB.
    pub fn encode_catalog_counter(&self) -> Vec<u8> {
        values::encode_counter(self.next_catalog_id)
    }

    /// Encode the file counter value for writing to SlateDB.
    pub fn encode_file_counter(&self) -> Vec<u8> {
        values::encode_counter(self.next_file_id)
    }
}

/// Encode a column counter value for a specific table.
pub fn encode_column_counter(next_column_id: u64) -> Vec<u8> {
    values::encode_counter(next_column_id)
}

/// Decode a counter value from SlateDB bytes.
pub fn decode_counter_value(data: &[u8]) -> Result<u64, values::ValueError> {
    values::decode_counter(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_cache_allocation() {
        let mut cache = CounterCache::new(1, 1, 1);
        assert_eq!(cache.alloc_snapshot_id(), 1);
        assert_eq!(cache.alloc_snapshot_id(), 2);
        assert_eq!(cache.alloc_catalog_id(), 1);
        assert_eq!(cache.alloc_catalog_id(), 2);
        assert_eq!(cache.alloc_file_id(), 1);
        assert_eq!(cache.alloc_file_id(), 2);
    }

    #[test]
    fn counter_domain_keys() {
        let sk = CounterDomain::SnapshotId.key();
        assert_eq!(sk, vec![TAG_COUNTERS, COUNTER_NEXT_SNAPSHOT_ID]);

        let ck = CounterDomain::CatalogId.key();
        assert_eq!(ck, vec![TAG_COUNTERS, COUNTER_NEXT_CATALOG_ID]);

        let fk = CounterDomain::FileId.key();
        assert_eq!(fk, vec![TAG_COUNTERS, COUNTER_NEXT_FILE_ID]);

        let col_k = CounterDomain::ColumnId(42).key();
        assert_eq!(col_k[0], TAG_COUNTERS);
        assert_eq!(col_k[1], COUNTER_NEXT_COLUMN_ID_PREFIX);
    }

    #[test]
    fn counter_round_trip() {
        let cache = CounterCache::new(100, 200, 300);
        let encoded = cache.encode_snapshot_counter();
        let decoded = decode_counter_value(&encoded).unwrap();
        assert_eq!(decoded, 100);
    }

    #[test]
    fn id_monotonicity() {
        let mut cache = CounterCache::new(1, 1, 1);
        let mut prev = 0u64;
        for _ in 0..100 {
            let id = cache.alloc_snapshot_id();
            assert!(id > prev);
            prev = id;
        }
    }
}
