//! MVCC (Multi-Version Concurrency Control) filter logic.
//!
//! Naming conventions (strict):
//! - `dl_snapshot_id` / `catalog_version`: DuckLake snapshot identifier
//! - `kv_read_view` / `kv_snapshot`: SlateDB-level read view

/// A DuckLake snapshot identifier used for MVCC filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnapshotId(pub u64);

impl SnapshotId {
    /// Create a new `SnapshotId` from a raw u64.
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    /// Return the raw u64 snapshot identifier.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "snapshot:{}", self.0)
    }
}

/// MVCC visibility check for versioned rows.
///
/// A row is visible at `dl_snapshot_id` iff:
/// `begin_snapshot <= dl_snapshot_id AND (end_snapshot IS NULL OR dl_snapshot_id < end_snapshot)`
#[inline]
pub fn is_visible(
    begin_snapshot: u64,
    end_snapshot: Option<u64>,
    dl_snapshot_id: SnapshotId,
) -> bool {
    if begin_snapshot > dl_snapshot_id.0 {
        return false;
    }
    match end_snapshot {
        None => true,
        Some(end) => dl_snapshot_id.0 < end,
    }
}

/// Check if an inlined insert row is visible at a given snapshot.
#[inline]
pub fn is_inlined_insert_visible(
    begin_snapshot: u64,
    end_snapshot: Option<u64>,
    dl_snapshot_id: SnapshotId,
) -> bool {
    is_visible(begin_snapshot, end_snapshot, dl_snapshot_id)
}

/// Check if an inlined delete marker is visible at a given snapshot.
/// Delete markers are visible from their begin_snapshot onward (no end).
#[inline]
pub fn is_inlined_delete_visible(begin_snapshot: u64, dl_snapshot_id: SnapshotId) -> bool {
    begin_snapshot <= dl_snapshot_id.0
}

/// Physical GC eligibility for inlined insert rows.
/// Eligible when: `end_snapshot IS NOT NULL AND end_snapshot <= oldest_retained_snapshot`.
#[inline]
pub fn is_insert_gc_eligible(end_snapshot: Option<u64>, oldest_retained_snapshot: u64) -> bool {
    match end_snapshot {
        None => false,
        Some(end) => end <= oldest_retained_snapshot,
    }
}

/// Determine the latest visible version from a set of versions of the same entity.
/// Returns the version with the largest `begin_snapshot` that is still visible.
pub fn latest_visible_version<T>(
    versions: impl IntoIterator<Item = (u64, Option<u64>, T)>,
    dl_snapshot_id: SnapshotId,
) -> Option<T> {
    let mut best: Option<(u64, T)> = None;
    for (begin, end, item) in versions {
        if is_visible(begin, end, dl_snapshot_id) {
            match &best {
                None => best = Some((begin, item)),
                Some((prev_begin, _)) if begin > *prev_begin => {
                    best = Some((begin, item));
                }
                _ => {}
            }
        }
    }
    best.map(|(_, item)| item)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_when_no_end() {
        assert!(is_visible(1, None, SnapshotId(1)));
        assert!(is_visible(1, None, SnapshotId(100)));
    }

    #[test]
    fn not_visible_before_begin() {
        assert!(!is_visible(5, None, SnapshotId(4)));
    }

    #[test]
    fn visible_at_begin() {
        assert!(is_visible(5, None, SnapshotId(5)));
    }

    #[test]
    fn not_visible_at_end() {
        assert!(!is_visible(1, Some(5), SnapshotId(5)));
    }

    #[test]
    fn visible_just_before_end() {
        assert!(is_visible(1, Some(5), SnapshotId(4)));
    }

    #[test]
    fn not_visible_after_end() {
        assert!(!is_visible(1, Some(5), SnapshotId(6)));
    }

    #[test]
    fn gc_eligible() {
        assert!(is_insert_gc_eligible(Some(3), 3));
        assert!(is_insert_gc_eligible(Some(3), 5));
        assert!(!is_insert_gc_eligible(Some(3), 2));
        assert!(!is_insert_gc_eligible(None, 100));
    }

    #[test]
    fn latest_visible() {
        let versions = vec![
            (1u64, Some(5u64), "v1"),
            (5, Some(10), "v2"),
            (10, None, "v3"),
        ];
        assert_eq!(
            latest_visible_version(versions.clone(), SnapshotId(3)),
            Some("v1")
        );
        assert_eq!(
            latest_visible_version(versions.clone(), SnapshotId(7)),
            Some("v2")
        );
        assert_eq!(latest_visible_version(versions, SnapshotId(15)), Some("v3"));
    }

    #[test]
    fn latest_visible_none_when_all_ended() {
        let versions: Vec<(u64, Option<u64>, &str)> = vec![(1, Some(3), "v1"), (3, Some(5), "v2")];
        assert_eq!(latest_visible_version(versions, SnapshotId(6)), None);
    }
}
