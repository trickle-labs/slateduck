//! 24-hour soak test stub.
//!
//! In CI this runs as a short smoke test (100 cycles).  On a dedicated EC2
//! `c6i.4xlarge` runner the `ROCKLAKE_SOAK_CYCLES` environment variable
//! overrides the cycle count to drive a true 24-hour soak.
//!
//! The test is wired into CI as a manual-trigger `scale-soak` job so it does
//! not block every PR but can be run on demand before a release.

use std::sync::Arc;

use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_testkit::{SoakConfig, SoakHarness};

fn soak_cycles() -> u64 {
    std::env::var("ROCKLAKE_SOAK_CYCLES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
}

#[tokio::test]
async fn soak_24h_catalog_consistency() {
    let cycles = soak_cycles();
    let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
    let opts = OpenOptions {
        object_store,
        path: ObjectPath::from("soak-catalog"),
        encryption: None,
    };
    let mut store = CatalogStore::open(opts).await.expect("soak: open must succeed");

    let config = SoakConfig {
        cycles,
        schemas_per_cycle: 1,
        assert_index_integrity: true,
    };
    let harness = SoakHarness::new(config);
    let summary = harness.run(&mut store).await;

    assert_eq!(
        summary.cycles_completed, cycles,
        "all soak cycles must complete"
    );
    assert!(summary.consistent, "catalog must be consistent after soak run");
    assert_eq!(summary.panics, 0, "no panics during soak run");

    store.close().await.expect("soak: close must succeed");
}
