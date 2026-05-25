//! Shard key range computation.
//!
//! Given `shard_count` shards, this module divides the hash space
//! `[0, u64::MAX]` evenly so that each shard owns a contiguous
//! non-overlapping interval of key-hash values.
//!
//! ## Encoding
//! Key range bounds are stored as big-endian u64 bytes so that they are
//! byte-comparable in the same order as the hash values.
//!
//! ## Auto-detection
//! If no explicit `shard_key` is provided via `WITH (shard_key = '...')` the
//! IVM plan extracts the first GROUP BY column name.  If there is no GROUP BY
//! (e.g. a simple SELECT with no aggregation) shard 0 owns the full range.

/// A shard's key range: `[lo, hi)` over hash-space bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardKeyRange {
    pub lo: Vec<u8>,
    pub hi: Vec<u8>,
}

impl ShardKeyRange {
    /// Full range: shard 0 when `shard_count = 1` or when there is no shard key.
    pub fn full() -> Self {
        ShardKeyRange {
            lo: 0u64.to_be_bytes().to_vec(),
            hi: u64::MAX.to_be_bytes().to_vec(),
        }
    }

    /// Returns true if this range covers the entire hash space.
    pub fn is_full(&self) -> bool {
        self.lo == 0u64.to_be_bytes() && self.hi == u64::MAX.to_be_bytes()
    }

    /// Returns true if `hash_value` falls within `[lo, hi)`.
    pub fn contains(&self, hash_value: u64) -> bool {
        let lo = u64::from_be_bytes(self.lo.as_slice().try_into().unwrap_or([0u8; 8]));
        let hi = u64::from_be_bytes(self.hi.as_slice().try_into().unwrap_or([0u8; 8]));
        hash_value >= lo && hash_value < hi
    }
}

/// Compute the `shard_count` non-overlapping key ranges that cover
/// `[0, u64::MAX]` deterministically.
///
/// Uses integer arithmetic to avoid floating-point rounding:
/// `step = u64::MAX / shard_count` with the last shard absorbing any remainder.
pub fn compute_key_ranges(shard_count: u32) -> Vec<ShardKeyRange> {
    assert!(shard_count > 0, "shard_count must be > 0");
    if shard_count == 1 {
        return vec![ShardKeyRange::full()];
    }
    let n = shard_count as u64;
    let step = u64::MAX / n;
    (0..n)
        .map(|i| {
            let lo = i * step;
            let hi = if i == n - 1 { u64::MAX } else { (i + 1) * step };
            ShardKeyRange {
                lo: lo.to_be_bytes().to_vec(),
                hi: hi.to_be_bytes().to_vec(),
            }
        })
        .collect()
}

/// Hash a shard-key value (as a JSON string) to a u64 hash.
///
/// Uses a FNV-1a (64-bit) variant for determinism and speed.
/// This is NOT a cryptographic hash; it is only used for sharding.
pub fn hash_key_value(value: &str) -> u64 {
    fnv1a_64(value.as_bytes())
}

fn fnv1a_64(data: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Determine which shard index (0..shard_count) owns a given key value.
pub fn shard_index_for(value: &str, ranges: &[ShardKeyRange]) -> usize {
    let hash = hash_key_value(value);
    for (i, range) in ranges.iter().enumerate() {
        if range.contains(hash) {
            return i;
        }
    }
    // Fallback: last shard (handles u64::MAX edge case)
    ranges.len().saturating_sub(1)
}

/// Auto-detect the shard key column from a list of GROUP BY column names.
///
/// Returns the first column name if available, or `None` if there are no
/// GROUP BY columns (in which case the view is unshardable — use shard 0).
pub fn auto_detect_shard_key(group_by_columns: &[String]) -> Option<&str> {
    group_by_columns.first().map(|s| s.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_shard_is_full_range() {
        let ranges = compute_key_ranges(1);
        assert_eq!(ranges.len(), 1);
        assert!(ranges[0].is_full());
    }

    #[test]
    fn ranges_are_non_overlapping_and_cover_space() {
        for n in [2u32, 4, 8, 16] {
            let ranges = compute_key_ranges(n);
            assert_eq!(ranges.len(), n as usize);
            // First shard starts at 0.
            assert_eq!(ranges[0].lo, 0u64.to_be_bytes());
            // Last shard ends at u64::MAX.
            assert_eq!(ranges[n as usize - 1].hi, u64::MAX.to_be_bytes());
            // Adjacent shards are contiguous.
            for i in 1..ranges.len() {
                assert_eq!(ranges[i - 1].hi, ranges[i].lo);
            }
        }
    }

    #[test]
    fn shard_index_is_stable() {
        let ranges = compute_key_ranges(8);
        // Same input always maps to same shard.
        assert_eq!(
            shard_index_for("order_id=42", &ranges),
            shard_index_for("order_id=42", &ranges)
        );
    }

    #[test]
    fn all_shards_reachable() {
        let ranges = compute_key_ranges(8);
        // Use pre-verified keys known to cover all 8 shards of the FNV-1a
        // u64 / shard_count partition.  Sequential short strings like "key=N"
        // can alias into the same region of hash-space; these keys were
        // selected by scanning "row-{N}" until every shard was reached.
        let keys = [
            "row-0",    // shard 0
            "row-1800", // shard 1
            "row-10",   // shard 2
            "row-200",  // shard 3
            "row-100",  // shard 4
            "row-400",  // shard 5
            "row-800",  // shard 6
            "row-2500", // shard 7
        ];
        let mut seen = std::collections::HashSet::new();
        for key in &keys {
            seen.insert(shard_index_for(key, &ranges));
        }
        assert_eq!(
            seen.len(),
            8,
            "expected all 8 shards reachable, got {seen:?}"
        );
    }
}
