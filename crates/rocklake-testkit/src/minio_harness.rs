//! MinioHarness: manages a MinIO container for object-store-backed tests.
//!
//! For Tier 4+ integration tests that need a real S3-compatible object store
//! rather than the in-memory `object_store::memory::InMemory` backend.
//!
//! ## Prerequisites
//! - Docker must be available in the test environment.
//! - Tests using this harness should be gated behind `#[cfg(feature = "minio-tests")]`
//!   or similar to avoid CI failure when Docker is unavailable.
//!
//! ## Usage
//! ```ignore
//! let minio = MinioHarness::start().await.unwrap();
//! let store = minio.object_store();
//! // Use `store` with CatalogStore::open(...)
//! minio.stop().await;
//! ```

use std::sync::Arc;
use std::time::Duration;

use object_store::aws::AmazonS3Builder;

/// MinIO container harness for S3-compatible object store tests.
pub struct MinioHarness {
    container_id: String,
    endpoint: String,
    bucket: String,
}

/// Default MinIO credentials used in the test container.
const MINIO_ACCESS_KEY: &str = "minioadmin";
const MINIO_SECRET_KEY: &str = "minioadmin";
const MINIO_BUCKET: &str = "rocklake-test";
const MINIO_IMAGE: &str = "minio/minio:latest";

impl MinioHarness {
    /// Start a MinIO container and create the test bucket.
    ///
    /// Returns `Err` if Docker is not available or the container fails to start.
    pub async fn start() -> Result<Self, MinioHarnessError> {
        let port = find_available_port().await?;
        let endpoint = format!("http://127.0.0.1:{port}");

        // Start the MinIO container.
        let output = tokio::process::Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "-p",
                &format!("{port}:9000"),
                "-e",
                &format!("MINIO_ROOT_USER={MINIO_ACCESS_KEY}"),
                "-e",
                &format!("MINIO_ROOT_PASSWORD={MINIO_SECRET_KEY}"),
                MINIO_IMAGE,
                "server",
                "/data",
            ])
            .output()
            .await
            .map_err(|e| MinioHarnessError::Docker(format!("failed to run docker: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(MinioHarnessError::Docker(format!(
                "docker run failed: {stderr}"
            )));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Wait for MinIO to become ready.
        let harness = Self {
            container_id,
            endpoint: endpoint.clone(),
            bucket: MINIO_BUCKET.to_string(),
        };
        harness.wait_for_ready(Duration::from_secs(30)).await?;

        // Create the test bucket using mc or the S3 API.
        harness.create_bucket().await?;

        Ok(harness)
    }

    /// Get an `object_store::ObjectStore` instance configured for this MinIO.
    pub fn object_store(&self) -> Arc<dyn object_store::ObjectStore> {
        let s3 = AmazonS3Builder::new()
            .with_endpoint(&self.endpoint)
            .with_bucket_name(&self.bucket)
            .with_access_key_id(MINIO_ACCESS_KEY)
            .with_secret_access_key(MINIO_SECRET_KEY)
            .with_region("us-east-1")
            .with_allow_http(true)
            .build()
            .expect("failed to build S3 client for MinIO");
        Arc::new(s3)
    }

    /// The S3 endpoint URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// Stop and remove the MinIO container.
    pub async fn stop(&self) {
        let _ = tokio::process::Command::new("docker")
            .args(["kill", &self.container_id])
            .output()
            .await;
    }

    /// Wait until MinIO's health endpoint responds.
    async fn wait_for_ready(&self, timeout: Duration) -> Result<(), MinioHarnessError> {
        let start = std::time::Instant::now();
        let client = reqwest::Client::new();
        loop {
            if start.elapsed() > timeout {
                return Err(MinioHarnessError::Timeout(
                    "MinIO did not become ready in time".into(),
                ));
            }
            match client
                .get(format!("{}/minio/health/live", self.endpoint))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }

    /// Create the test bucket via the S3 API.
    async fn create_bucket(&self) -> Result<(), MinioHarnessError> {
        let client = reqwest::Client::new();
        // MinIO supports bucket creation via PUT on the bucket path.
        let resp = client
            .put(format!("{}/{}", self.endpoint, self.bucket))
            .send()
            .await
            .map_err(|e| MinioHarnessError::BucketCreate(e.to_string()))?;

        // 200 or 409 (already exists) are both fine.
        if resp.status().is_success() || resp.status().as_u16() == 409 {
            Ok(())
        } else {
            Err(MinioHarnessError::BucketCreate(format!(
                "unexpected status: {}",
                resp.status()
            )))
        }
    }
}

impl Drop for MinioHarness {
    fn drop(&mut self) {
        // Best-effort cleanup: spawn a blocking task to kill the container.
        let id = self.container_id.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("docker")
                .args(["kill", &id])
                .output();
        });
    }
}

/// Errors from the MinIO harness.
#[derive(Debug, thiserror::Error)]
pub enum MinioHarnessError {
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
async fn find_available_port() -> Result<u16, MinioHarnessError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| MinioHarnessError::PortAllocation(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| MinioHarnessError::PortAllocation(e.to_string()))?
        .port();
    drop(listener);
    Ok(port)
}
