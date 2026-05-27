//! v0.27.12 catalog backend compatibility tests.
//!
//! Tests the unified `catalog_backend_compat_test!` macro against:
//!  - In-memory backend (always runs in CI)
//!  - GCS emulator backend (requires `--features gcs-emulator` + Docker)
//!  - Azure Blob Storage emulator backend (requires `--features azure-emulator` + Docker)
//!  - MinIO backend (requires `--features minio-tests` + Docker)
//!
//! ## Running emulator tests
//!
//! ```sh
//! # GCS only
//! cargo test -p rocklake-catalog --test v02712_backend_compat_tests \
//!   --features gcs-emulator
//!
//! # Azure only
//! cargo test -p rocklake-catalog --test v02712_backend_compat_tests \
//!   --features azure-emulator
//!
//! # All emulators
//! cargo test -p rocklake-catalog --test v02712_backend_compat_tests \
//!   --features gcs-emulator,azure-emulator
//! ```

use rocklake_testkit::catalog_backend_compat_test;

// ── In-memory (always runs) ───────────────────────────────────────────────────

catalog_backend_compat_test!(
    inmem,
    std::sync::Arc::new(object_store::memory::InMemory::new())
);

// ── GCS emulator (requires --features gcs-emulator + Docker) ─────────────────

#[cfg(feature = "gcs-emulator")]
mod gcs_compat {
    use rocklake_testkit::catalog_backend_compat_test;
    use rocklake_testkit::GcsEmulatorHarness;

    /// Run the GCS emulator and return an `Arc<dyn ObjectStore>`.
    ///
    /// If Docker is unavailable, the test is skipped gracefully via panic with
    /// a descriptive message.
    async fn gcs_store() -> std::sync::Arc<dyn object_store::ObjectStore> {
        let harness = match GcsEmulatorHarness::start().await {
            Ok(h) => h,
            Err(e) => {
                panic!(
                    "GCS emulator unavailable (skipping emulator tests): {e}. \
                     Ensure Docker is installed and fake-gcs-server image is accessible."
                );
            }
        };
        harness
            .create_bucket("rocklake-test")
            .await
            .expect("failed to create GCS emulator bucket");
        harness.object_store("rocklake-test")
    }

    catalog_backend_compat_test!(gcs, super::gcs_store().await);
}

// ── Azure emulator (requires --features azure-emulator + Docker) ──────────────

#[cfg(feature = "azure-emulator")]
mod azure_compat {
    use rocklake_testkit::catalog_backend_compat_test;
    use rocklake_testkit::AzureEmulatorHarness;

    /// Run the Azurite emulator and return an `Arc<dyn ObjectStore>`.
    ///
    /// If Docker is unavailable, the test panics with a descriptive message.
    async fn azure_store() -> std::sync::Arc<dyn object_store::ObjectStore> {
        let harness = match AzureEmulatorHarness::start().await {
            Ok(h) => h,
            Err(e) => {
                panic!(
                    "Azure emulator unavailable (skipping emulator tests): {e}. \
                     Ensure Docker is installed and Azurite image is accessible."
                );
            }
        };
        harness
            .create_container("rocklake-test")
            .await
            .expect("failed to create Azure emulator container");
        harness.object_store("rocklake-test")
    }

    catalog_backend_compat_test!(azure, super::azure_store().await);
}
