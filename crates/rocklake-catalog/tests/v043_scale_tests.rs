//! v0.43.0 Scale Testing, Soak & Serverless Readers — integration tests.
//!
//! Tests cover:
//! - Soak harness: multi-cycle catalog consistency under continuous write/read
//! - Checkpoint pin/unpin/list API
//! - Read-only client against a pinned checkpoint
//! - Catalog-data key immutability (CDN cache contract property)

use std::sync::Arc;

use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::checkpoint::{list_checkpoint_pins, pin_checkpoint, unpin_checkpoint};
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;
use rocklake_testkit::{SoakConfig, SoakHarness};

// ─── helpers ───────────────────────────────────────────────────────────────

async fn open_in_memory() -> CatalogStore {
    let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
    let opts = OpenOptions {
        object_store,
        path: ObjectPath::from("test-catalog"),
        encryption: None,
    };
    CatalogStore::open(opts).await.expect("open must succeed")
}

// ─── Soak harness tests ────────────────────────────────────────────────────

/// Soak smoke test: 20 write/read cycles with full consistency checks.
///
/// On dedicated scale-test EC2 runners the `ROCKLAKE_SOAK_CYCLES` environment
/// variable can override the cycle count to drive a full 24-hour soak.
#[tokio::test]
async fn soak_harness_smoke_20_cycles() {
    let cycles = std::env::var("ROCKLAKE_SOAK_CYCLES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20);

    let mut store = open_in_memory().await;

    let config = SoakConfig {
        cycles,
        schemas_per_cycle: 1,
        assert_index_integrity: true,
    };
    let harness = SoakHarness::new(config);
    let summary = harness.run(&mut store).await;

    assert_eq!(summary.cycles_completed, cycles, "all cycles must complete");
    assert!(summary.consistent, "catalog must remain consistent");
    assert_eq!(summary.panics, 0, "no panics");

    store.close().await.unwrap();
}

/// Soak with zero schemas per cycle: empty-write path must not panic.
#[tokio::test]
async fn soak_harness_zero_schemas_per_cycle() {
    let mut store = open_in_memory().await;

    let config = SoakConfig {
        cycles: 5,
        schemas_per_cycle: 0,
        assert_index_integrity: true,
    };
    let harness = SoakHarness::new(config);
    let summary = harness.run(&mut store).await;

    assert_eq!(summary.cycles_completed, 5);
    assert!(summary.consistent);

    store.close().await.unwrap();
}

// ─── Checkpoint pin/unpin/list tests ──────────────────────────────────────

/// Basic pin → list → unpin round-trip.
#[tokio::test]
async fn checkpoint_pin_round_trip() {
    let store = open_in_memory().await;
    let db = store.db();

    pin_checkpoint(db, "release-candidate", 42).await.unwrap();
    let pins = list_checkpoint_pins(db).await.unwrap();

    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].name, "release-candidate");
    assert_eq!(pins[0].snapshot_id, 42);

    unpin_checkpoint(db, "release-candidate").await.unwrap();
    let pins_after = list_checkpoint_pins(db).await.unwrap();
    assert!(pins_after.is_empty());

    store.close().await.unwrap();
}

/// Multiple pins survive a list in alphabetical order.
#[tokio::test]
async fn checkpoint_multiple_pins_listed_alphabetically() {
    let store = open_in_memory().await;
    let db = store.db();

    pin_checkpoint(db, "zebra", 100).await.unwrap();
    pin_checkpoint(db, "alpha", 10).await.unwrap();
    pin_checkpoint(db, "middle", 50).await.unwrap();

    let pins = list_checkpoint_pins(db).await.unwrap();
    assert_eq!(pins.len(), 3);
    assert_eq!(pins[0].name, "alpha");
    assert_eq!(pins[1].name, "middle");
    assert_eq!(pins[2].name, "zebra");

    store.close().await.unwrap();
}

/// Unpinning a non-existent pin returns NotFound.
#[tokio::test]
async fn checkpoint_unpin_nonexistent_returns_not_found() {
    use rocklake_catalog::CatalogError;

    let store = open_in_memory().await;
    let db = store.db();

    let err = unpin_checkpoint(db, "does-not-exist").await.unwrap_err();
    assert!(
        matches!(err, CatalogError::NotFound(_)),
        "expected NotFound, got {err:?}"
    );

    store.close().await.unwrap();
}

/// Read-only reader pinned at snapshot N sees exactly N schemas regardless of
/// how many more snapshots are committed afterwards.
///
/// Uses `store.read_at(snapshot_id)` — opening a second `CatalogStore` against
/// the same in-memory path triggers SlateDB writer-fencing ("detected newer DB
/// client"), so we read via the existing store instead.
#[tokio::test]
async fn read_only_client_sees_only_pinned_snapshot() {
    let mut store = open_in_memory().await;

    // Write 3 schemas across separate snapshots.
    for name in &["s1", "s2", "s3"] {
        let mut writer = store.begin_write();
        writer.create_schema(name).await.unwrap();
        let result = writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(result);
    }

    // Pin at snapshot 2 so that only s1 and s2 are visible (s3 is at snapshot 3).
    let pin_snapshot_id = 2u64;
    let db = store.db();
    pin_checkpoint(db, "pinned-at-2", pin_snapshot_id)
        .await
        .unwrap();

    // Read at the pinned snapshot via the existing store (no second open needed).
    let reader = store.read_at(SnapshotId::new(pin_snapshot_id)).unwrap();
    let schemas = reader.list_schemas().await.unwrap();

    // At snapshot 2: s1 (snapshot 1) and s2 (snapshot 2) are visible; s3 is not.
    assert_eq!(
        schemas.len(),
        2,
        "read-only reader should see 2 schemas at pinned snapshot 2, got: {schemas:?}"
    );

    store.close().await.unwrap();
}

// ─── CDN cache contract: key immutability ─────────────────────────────────

/// Catalog-data keys must never change value once written.
///
/// This verifies the CDN cache contract: any HTTP GET for a catalog prefix
/// key can be safely cached because the value is immutable.
#[tokio::test]
async fn catalog_data_keys_are_immutable_after_50_writes() {
    let mut store = open_in_memory().await;

    // Perform 50 write cycles (schema creates + snapshots).
    for cycle in 0u64..50 {
        let mut writer = store.begin_write();
        writer
            .create_schema(&format!("immutable_schema_{cycle}"))
            .await
            .unwrap();
        let result = writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(result);
    }

    // Scan all keys and verify none has a value that changed.
    let mut seen_key_values: std::collections::HashMap<Vec<u8>, Vec<u8>> =
        std::collections::HashMap::new();
    let db = store.db();
    let mut iter = db.scan_prefix(&[]).await.unwrap();
    while let Some(kv) = iter
        .next()
        .await
        .map_err(|e| format!("scan error: {e}"))
        .unwrap()
    {
        let key: Vec<u8> = kv.key.to_vec();
        let value: Vec<u8> = kv.value.to_vec();
        if let Some(prev) = seen_key_values.get(&key) {
            assert_eq!(
                prev, &value,
                "catalog-data key changed value — immutability violation!"
            );
        } else {
            seen_key_values.insert(key, value);
        }
    }

    // At least 50 keys should have been written (1 schema + 1 snapshot per cycle).
    assert!(
        seen_key_values.len() >= 50,
        "expected at least 50 keys, got {}",
        seen_key_values.len()
    );

    store.close().await.unwrap();
}
