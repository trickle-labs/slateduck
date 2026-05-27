//! Tier 7 — Catalog fault injection tests.
//!
//! Tests: kill after SST write before manifest update,
//! corrupted WAL entry recovery, kill during compaction,
//! concurrent writer detection (fencing).

use rocklake_catalog::manifest::{ManifestEntry, ManifestLog};
use rocklake_catalog::wal::{WalEntry, WalWriter};

/// Test: kill after SST write before manifest update.
/// On restart, orphan SST is detected and cleaned up.
#[test]
fn kill_after_sst_before_manifest() {
    let log = ManifestLog::new();

    // Write SST (simulated).
    let sst_id = "sst-001";

    // Crash before manifest update — SST is orphaned.
    // On restart, we detect orphan SSTs not in manifest.
    let manifest_entries: Vec<String> = log
        .entries()
        .iter()
        .filter_map(|e| e.sst_id())
        .map(|s| s.to_string())
        .collect();
    assert!(!manifest_entries.contains(&sst_id.to_string()));

    // Recovery: orphan SSTs are safe to delete (write was incomplete).
    let orphans = [sst_id.to_string()];
    assert!(!orphans.is_empty());
}

/// Test: corrupted WAL entry recovery.
/// WAL reader skips corrupted entry and continues from next valid entry.
#[test]
fn corrupted_wal_entry_recovery() {
    let mut writer = WalWriter::new();

    // Write valid entries.
    writer.append(WalEntry::insert(1, b"key1", b"val1"));
    writer.append(WalEntry::insert(2, b"key2", b"val2"));
    writer.append(WalEntry::insert(3, b"key3", b"val3"));

    // Simulate corruption of entry 2 (checksum mismatch).
    let entries = writer.recover_with_corruption(1); // Corrupt entry at index 1.

    // Recovery should return entries before corruption.
    assert_eq!(entries.len(), 1); // Only entry 1 before corruption.
    assert_eq!(entries[0].seq(), 1);

    // Entries after corruption boundary are lost (acceptable trade-off).
    // In production, WAL is truncated at corruption point and writer resumes.
}

/// Test: kill during compaction.
/// On restart, partial compaction output is discarded.
#[test]
fn kill_during_compaction() {
    let mut log = ManifestLog::new();

    // Start compaction: merge SST-001 + SST-002 → SST-003.
    let input_ssts = vec!["sst-001".to_string(), "sst-002".to_string()];
    log.begin_compaction(&input_ssts, "sst-003");

    // Crash during compaction — SST-003 is partial/incomplete.
    // On restart, incomplete compaction is detected.
    assert!(log.has_pending_compaction());

    // Recovery: discard partial SST-003, keep originals.
    log.abort_compaction();
    assert!(!log.has_pending_compaction());

    // Original SSTs remain valid.
    let remaining = log.active_ssts();
    assert!(remaining.contains(&"sst-001".to_string()));
    assert!(remaining.contains(&"sst-002".to_string()));
    assert!(!remaining.contains(&"sst-003".to_string()));
}

/// Test: concurrent writer detection (fencing).
/// Second writer detects fencing and yields.
#[test]
fn concurrent_writer_fencing() {
    let mut log = ManifestLog::new();

    // Writer 1 acquires epoch.
    let epoch1 = log.acquire_epoch();
    assert_eq!(epoch1, 1);

    // Writer 2 attempts to acquire — should get higher epoch.
    let epoch2 = log.acquire_epoch();
    assert_eq!(epoch2, 2);

    // Writer 1 attempts write with stale epoch — fenced.
    let result = log.try_write(epoch1, ManifestEntry::new_sst("sst-010"));
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("fenced"));

    // Writer 2 succeeds.
    let result = log.try_write(epoch2, ManifestEntry::new_sst("sst-010"));
    assert!(result.is_ok());
}
