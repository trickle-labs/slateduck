//! v0.36.0 — Catalog backend compatibility tests.
//!
//! Wires the shared `catalog_backend_compat_test!` macro against:
//!  - In-memory backend (always runs in CI)
//!  - LocalFS backend (always runs in CI)
//!  - GCS emulator backend (requires `--features gcs-emulator` + Docker)
//!  - Azure Blob Storage emulator backend (requires `--features azure-emulator` + Docker)
//!  - MinIO backend (requires `--features minio-tests` + Docker)
//!
//! This file is the canonical `crates/rocklake-catalog/tests/backend_compat.rs`
//! entry point referenced in the v0.36.0 roadmap.
//!
//! ## Running emulator tests
//!
//! ```sh
//! # In-memory + LocalFS (default CI)
//! cargo test -p rocklake-catalog --test backend_compat
//!
//! # GCS emulator (requires Docker)
//! cargo test -p rocklake-catalog --test backend_compat --features gcs-emulator
//!
//! # Azure emulator (requires Docker)
//! cargo test -p rocklake-catalog --test backend_compat --features azure-emulator
//!
//! # All emulators
//! cargo test -p rocklake-catalog --test backend_compat \
//!   --features gcs-emulator,azure-emulator
//! ```

use rocklake_testkit::catalog_backend_compat_test;

// ── In-memory (always runs) ────────────────────────────────────────────────

catalog_backend_compat_test!(
    inmem,
    std::sync::Arc::new(object_store::memory::InMemory::new())
);

// ── LocalFS (always runs) ─────────────────────────────────────────────────
//
// Uses a temporary directory on the local filesystem.  This covers the
// real I/O path that development and single-host deployments use.

catalog_backend_compat_test!(localfs, {
    let tmp = tempfile::TempDir::new().expect("localfs: tempdir failed");
    // Leak the TempDir so it is not cleaned up while the tests run.
    let path = tmp.keep();
    std::sync::Arc::new(
        object_store::local::LocalFileSystem::new_with_prefix(&path)
            .expect("localfs: LocalFileSystem::new_with_prefix failed"),
    )
});

// ── GCS emulator (requires --features gcs-emulator + Docker) ──────────────

#[cfg(feature = "gcs-emulator")]
mod gcs_compat {
    use rocklake_testkit::catalog_backend_compat_test;
    use rocklake_testkit::GcsEmulatorHarness;

    async fn gcs_store() -> std::sync::Arc<dyn object_store::ObjectStore> {
        let harness = match GcsEmulatorHarness::start().await {
            Ok(h) => h,
            Err(e) => {
                panic!(
                    "GCS emulator unavailable (requires Docker + fake-gcs-server): {e}. \
                     Run: docker pull fsouza/fake-gcs-server:latest"
                );
            }
        };
        harness.create_bucket("rocklake-test").await.ok();
        harness.object_store("rocklake-test")
    }

    catalog_backend_compat_test!(gcs, super::gcs_store().await);
}

// ── Azure Blob Storage emulator (requires --features azure-emulator + Docker) ─

#[cfg(feature = "azure-emulator")]
mod azure_compat {
    use rocklake_testkit::catalog_backend_compat_test;
    use rocklake_testkit::AzureEmulatorHarness;

    async fn azure_store() -> std::sync::Arc<dyn object_store::ObjectStore> {
        let harness = match AzureEmulatorHarness::start().await {
            Ok(h) => h,
            Err(e) => {
                panic!(
                    "Azure emulator unavailable (requires Docker + Azurite): {e}. \
                     Run: docker pull mcr.microsoft.com/azure-storage/azurite:latest"
                );
            }
        };
        harness.create_container("rocklake-test").await.ok();
        harness.object_store("rocklake-test")
    }

    catalog_backend_compat_test!(azure, super::azure_store().await);
}
