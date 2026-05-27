//! GcsEmulatorHarness: manages a fake-gcs-server container for GCS-backed tests.
//!
//! Enabled by the `gcs-emulator` feature flag.  Uses `fsouza/fake-gcs-server`
//! to provide a GCS-compatible HTTP API on a random local port.
//!
//! ## Prerequisites
//! - Docker must be available in the test environment.
//! - Tests using this harness should be gated behind `#[cfg(feature = "gcs-emulator")]`.
//!
//! ## Usage
//! ```ignore
//! let gcs = GcsEmulatorHarness::start().await.unwrap();
//! let store = gcs.object_store("my-bucket");
//! // Use `store` with CatalogStore::open(...)
//! gcs.stop().await;
//! ```

use std::sync::Arc;
use std::time::Duration;

use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::path::Path as ObjectPath;
use object_store::ObjectStore;

/// GCS emulator container harness for GCS-backed integration tests.
pub struct GcsEmulatorHarness {
    container_id: String,
    port: u16,
    endpoint: String,
}

const GCS_IMAGE: &str = "fsouza/fake-gcs-server:latest";
const GCS_DEFAULT_BUCKET: &str = "rocklake-test";

impl GcsEmulatorHarness {
    /// Start a fake-gcs-server container.
    ///
    /// Returns `Err` if Docker is not available or the container fails to start.
    pub async fn start() -> Result<Self, GcsHarnessError> {
        let port = find_available_port().await?;
        let endpoint = format!("http://127.0.0.1:{port}");

        let output = tokio::process::Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "-p",
                &format!("{port}:4443"),
                GCS_IMAGE,
                "-scheme",
                "http",
                "-port",
                "4443",
                "-public-host",
                &format!("127.0.0.1:{port}"),
            ])
            .output()
            .await
            .map_err(|e| GcsHarnessError::Docker(format!("failed to run docker: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(GcsHarnessError::Docker(format!(
                "docker run failed: {stderr}"
            )));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let harness = Self {
            container_id,
            port,
            endpoint: endpoint.clone(),
        };
        harness.wait_for_ready(Duration::from_secs(30)).await?;
        Ok(harness)
    }

    /// The emulator HTTP endpoint.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The port the emulator is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Get an `ObjectStore` instance configured to use the GCS emulator.
    ///
    /// The `STORAGE_EMULATOR_HOST` environment variable is set to the emulator
    /// endpoint before building the client.
    pub fn object_store(&self, bucket: &str) -> Arc<dyn ObjectStore> {
        // fake-gcs-server respects STORAGE_EMULATOR_HOST for endpoint override.
        // We set it per-harness.  Tests using this harness should run serially
        // to avoid environment variable conflicts.
        std::env::set_var("STORAGE_EMULATOR_HOST", &self.endpoint);

        let store = GoogleCloudStorageBuilder::from_env()
            .with_bucket_name(bucket)
            .build()
            .expect("failed to build GCS client for emulator");
        Arc::new(store)
    }

    /// Get an object store for the default test bucket.
    pub fn default_object_store(&self) -> Arc<dyn ObjectStore> {
        self.object_store(GCS_DEFAULT_BUCKET)
    }

    /// The default test bucket name.
    pub fn bucket(&self) -> &str {
        GCS_DEFAULT_BUCKET
    }

    /// Stop and remove the container.
    pub async fn stop(&self) {
        let _ = tokio::process::Command::new("docker")
            .args(["kill", &self.container_id])
            .output()
            .await;
        std::env::remove_var("STORAGE_EMULATOR_HOST");
    }

    /// Wait until the emulator HTTP endpoint is reachable.
    async fn wait_for_ready(&self, timeout: Duration) -> Result<(), GcsHarnessError> {
        let start = std::time::Instant::now();
        let client = reqwest::Client::new();
        loop {
            if start.elapsed() > timeout {
                return Err(GcsHarnessError::Timeout(
                    "GCS emulator did not become ready in time".into(),
                ));
            }
            // fake-gcs-server responds to GET / with JSON metadata.
            match client.get(format!("{}/", self.endpoint)).send().await {
                Ok(_) => return Ok(()),
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }

    /// Create a bucket in the emulator via the GCS JSON API.
    pub async fn create_bucket(&self, bucket: &str) -> Result<ObjectPath, GcsHarnessError> {
        let client = reqwest::Client::new();
        // fake-gcs-server accepts bucket creation at POST /storage/v1/b.
        let url = format!("{}/storage/v1/b", self.endpoint);
        let body = serde_json::json!({ "name": bucket });
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| GcsHarnessError::BucketCreate(e.to_string()))?;
        if resp.status().is_success() || resp.status().as_u16() == 409 {
            Ok(ObjectPath::from(""))
        } else {
            Err(GcsHarnessError::BucketCreate(format!(
                "unexpected status: {}",
                resp.status()
            )))
        }
    }
}

impl Drop for GcsEmulatorHarness {
    fn drop(&mut self) {
        let id = self.container_id.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("docker")
                .args(["kill", &id])
                .output();
        });
    }
}

/// Errors from the GCS emulator harness.
#[derive(Debug, thiserror::Error)]
pub enum GcsHarnessError {
    #[error("docker error: {0}")]
    Docker(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("bucket creation failed: {0}")]
    BucketCreate(String),
    #[error("port allocation failed: {0}")]
    PortAllocation(String),
}

/// Find an available TCP port by binding to port 0.
async fn find_available_port() -> Result<u16, GcsHarnessError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| GcsHarnessError::PortAllocation(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| GcsHarnessError::PortAllocation(e.to_string()))?
        .port();
    drop(listener);
    Ok(port)
}
