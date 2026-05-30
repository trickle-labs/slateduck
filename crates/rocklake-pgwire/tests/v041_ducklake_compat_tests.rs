//! v0.41.0 — DuckLake Forward Compatibility tests (PG-Wire layer).
//!
//! These tests validate the version-gating behaviour for DuckLake v1.1
//! pre-release catalogs.
//!
//! Test inventory (2 tests):
//! 1. `ducklake_v11_rejection_gate`      — migration of v1.1 source rejected without flag
//! 2. `ducklake_v11_accept_version_flag` — migration accepted with ACCEPT_VERSION_V1_1_DEV_1

use std::sync::Arc;

use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;

use rocklake_catalog::export::ExportedRow;
use rocklake_catalog::migrate_from_ducklake::{
    migrate_from_source, InMemoryDuckLakeSource, ACCEPT_VERSION_V1_1_DEV_1,
    DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION,
};
use rocklake_catalog::CatalogError;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn fresh_db() -> slatedb::Db {
    let store = Arc::new(InMemory::new()) as Arc<dyn object_store::ObjectStore>;
    slatedb::Db::builder(ObjectPath::from("catalog"), store)
        .build()
        .await
        .unwrap()
}

fn v11_snapshot_row() -> ExportedRow {
    ExportedRow {
        table: "ducklake_snapshot".to_string(),
        data: serde_json::json!({
            "snapshot_id": 1_u64,
            "schema_version": 8_u64,
            "snapshot_time": "2024-01-01T00:00:00Z",
            "author": null,
            "message": null,
        }),
    }
}

// ---------------------------------------------------------------------------
// Test 1: ducklake_v11_rejection_gate
// ---------------------------------------------------------------------------

/// Migration from a v1.1 pre-release source (catalog_version=8) must be
/// rejected by default with SQLSTATE 0A000 (UnsupportedDuckLakeVersion).
///
/// This mirrors the PG-Wire executor gate: a client connecting to RockLake
/// and attempting to migrate from a v1.1 source must receive an error
/// unless --accept-version V1_1_DEV_1 is explicitly passed.
#[tokio::test]
async fn ducklake_v11_rejection_gate() {
    let db = fresh_db().await;

    let mut source = InMemoryDuckLakeSource::new(DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION);
    source.add_rows("ducklake_snapshot", vec![v11_snapshot_row()]);

    let result = migrate_from_source(&mut source, &db, &[], false).await;

    match result {
        Err(CatalogError::UnsupportedDuckLakeVersion { version, message }) => {
            assert_eq!(version, DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION);
            assert!(
                message.contains("0A000") || message.to_lowercase().contains("unsupported"),
                "error message must reference SQLSTATE 0A000 or be descriptive; got: {message}"
            );
        }
        other => panic!(
            "expected UnsupportedDuckLakeVersion for catalog_version={}, got: {:?}",
            DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION, other
        ),
    }
}

// ---------------------------------------------------------------------------
// Test 2: ducklake_v11_accept_version_flag
// ---------------------------------------------------------------------------

/// When ACCEPT_VERSION_V1_1_DEV_1 is supplied, migration from a v1.1 source
/// must succeed and the report must reflect the correct catalog version.
#[tokio::test]
async fn ducklake_v11_accept_version_flag() {
    let db = fresh_db().await;

    let mut source = InMemoryDuckLakeSource::new(DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION);
    source.add_rows("ducklake_snapshot", vec![v11_snapshot_row()]);

    let report = migrate_from_source(&mut source, &db, &[ACCEPT_VERSION_V1_1_DEV_1], false)
        .await
        .unwrap();

    assert_eq!(
        report.source_catalog_version,
        DUCKLAKE_V1_1_DEV_1_CATALOG_VERSION
    );
    assert!(!report.dry_run);
}
