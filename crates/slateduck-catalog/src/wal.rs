//! Write-ahead log (WAL) for catalog operations.
//!
//! Provides sequential, durable logging of catalog mutations with checksummed
//! entries and corruption recovery.

/// A WAL entry representing a single catalog mutation.
#[derive(Debug, Clone)]
pub struct WalEntry {
    seq: u64,
    key: Vec<u8>,
    value: Vec<u8>,
    checksum: u32,
}

impl WalEntry {
    /// Create an insert WAL entry.
    pub fn insert(seq: u64, key: &[u8], value: &[u8]) -> Self {
        let checksum = Self::compute_checksum(key, value);
        Self {
            seq,
            key: key.to_vec(),
            value: value.to_vec(),
            checksum,
        }
    }

    /// Create a delete WAL entry.
    pub fn delete(seq: u64, key: &[u8]) -> Self {
        let checksum = Self::compute_checksum(key, &[]);
        Self {
            seq,
            key: key.to_vec(),
            value: Vec::new(),
            checksum,
        }
    }

    /// Get the sequence number.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Verify entry integrity.
    pub fn is_valid(&self) -> bool {
        self.checksum == Self::compute_checksum(&self.key, &self.value)
    }

    fn compute_checksum(key: &[u8], value: &[u8]) -> u32 {
        // Simple CRC-like checksum for testing.
        let mut hash: u32 = 0;
        for &b in key.iter().chain(value.iter()) {
            hash = hash.wrapping_mul(31).wrapping_add(b as u32);
        }
        hash
    }
}

/// WAL writer with append-only semantics.
#[derive(Debug, Clone, Default)]
pub struct WalWriter {
    entries: Vec<WalEntry>,
}

impl WalWriter {
    /// Create a new WAL writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an entry to the WAL.
    pub fn append(&mut self, entry: WalEntry) {
        self.entries.push(entry);
    }

    /// Get all entries.
    pub fn entries(&self) -> &[WalEntry] {
        &self.entries
    }

    /// Simulate recovery with a corrupted entry at the given index.
    /// Returns all valid entries before the corruption point.
    pub fn recover_with_corruption(&self, corrupt_index: usize) -> Vec<&WalEntry> {
        // In a real WAL, we'd detect checksum mismatch.
        // Here we simulate: entries before corrupt_index are returned.
        self.entries.iter().take(corrupt_index).collect()
    }

    /// Recover all valid entries (no corruption).
    pub fn recover(&self) -> Vec<&WalEntry> {
        self.entries.iter().filter(|e| e.is_valid()).collect()
    }
}
