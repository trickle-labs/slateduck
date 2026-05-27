//! AzureEmulatorHarness: manages an Azurite container for Azure Blob Storage tests.
//!
//! Enabled by the `azure-emulator` feature flag.  Uses
//! `mcr.microsoft.com/azure-storage/azurite` (the official Microsoft storage
//! emulator) to provide an Azure Blob Storage-compatible API on a random port.
//!
//! ## Prerequisites
//! - Docker must be available in the test environment.
//! - Tests using this harness should be gated behind `#[cfg(feature = "azure-emulator")]`.
//!
//! ## Usage
//! ```ignore
//! let az = AzureEmulatorHarness::start().await.unwrap();
//! let store = az.object_store("my-container");
//! // Use `store` with CatalogStore::open(...)
//! az.stop().await;
//! ```

use std::sync::Arc;
use std::time::Duration;

use object_store::azure::MicrosoftAzureBuilder;
use object_store::ObjectStore;

/// Azurite container harness for Azure Blob Storage-backed integration tests.
pub struct AzureEmulatorHarness {
    container_id: String,
    blob_port: u16,
}

const AZURITE_IMAGE: &str = "mcr.microsoft.com/azure-storage/azurite:latest";
/// Default Azurite development account name (fixed by Azurite).
const AZURITE_ACCOUNT: &str = "devstoreaccount1";
/// Default Azurite development account key (fixed by Azurite).
const AZURITE_KEY: &str =
    "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";
const AZURE_DEFAULT_CONTAINER: &str = "rocklake-test";

impl AzureEmulatorHarness {
    /// Start an Azurite container.
    ///
    /// Returns `Err` if Docker is not available or the container fails to start.
    pub async fn start() -> Result<Self, AzureHarnessError> {
        let blob_port = find_available_port().await?;

        let output = tokio::process::Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "-p",
                &format!("{blob_port}:10000"),
                AZURITE_IMAGE,
                "azurite-blob",
                "--blobHost",
                "0.0.0.0",
                "--blobPort",
                "10000",
            ])
            .output()
            .await
            .map_err(|e| AzureHarnessError::Docker(format!("failed to run docker: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AzureHarnessError::Docker(format!(
                "docker run failed: {stderr}"
            )));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();

        let harness = Self {
            container_id,
            blob_port,
        };
        harness.wait_for_ready(Duration::from_secs(30)).await?;
        Ok(harness)
    }

    /// The blob service endpoint URL.
    pub fn endpoint(&self) -> String {
        format!("http://127.0.0.1:{}/{AZURITE_ACCOUNT}", self.blob_port)
    }

    /// The blob port.
    pub fn blob_port(&self) -> u16 {
        self.blob_port
    }

    /// Get an `ObjectStore` instance configured to use the Azurite emulator.
    pub fn object_store(&self, container: &str) -> Arc<dyn ObjectStore> {
        let store = MicrosoftAzureBuilder::new()
            .with_account(AZURITE_ACCOUNT)
            .with_access_key(AZURITE_KEY)
            .with_container_name(container)
            .with_use_emulator(true)
            // Override the emulator port if it's not the default 10000.
            .with_endpoint(format!("http://127.0.0.1:{}", self.blob_port))
            .build()
            .expect("failed to build Azure client for Azurite emulator");
        Arc::new(store)
    }

    /// Get an object store for the default test container.
    pub fn default_object_store(&self) -> Arc<dyn ObjectStore> {
        self.object_store(AZURE_DEFAULT_CONTAINER)
    }

    /// The default test container name.
    pub fn container(&self) -> &str {
        AZURE_DEFAULT_CONTAINER
    }

    /// Stop and remove the container.
    pub async fn stop(&self) {
        let _ = tokio::process::Command::new("docker")
            .args(["kill", &self.container_id])
            .output()
            .await;
    }

    /// Wait until the Azurite blob service responds.
    async fn wait_for_ready(&self, timeout: Duration) -> Result<(), AzureHarnessError> {
        let start = std::time::Instant::now();
        let client = reqwest::Client::new();
        loop {
            if start.elapsed() > timeout {
                return Err(AzureHarnessError::Timeout(
                    "Azurite did not become ready in time".into(),
                ));
            }
            // Azurite responds to GET /<account>?comp=list with an XML listing.
            let url = format!(
                "http://127.0.0.1:{}/{AZURITE_ACCOUNT}?comp=list",
                self.blob_port
            );
            match client.get(&url).send().await {
                Ok(_) => return Ok(()),
                _ => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }

    /// Create a container (bucket) in Azurite.
    pub async fn create_container(&self, container: &str) -> Result<(), AzureHarnessError> {
        let client = reqwest::Client::new();
        let url = format!(
            "http://127.0.0.1:{}/{AZURITE_ACCOUNT}/{container}?restype=container",
            self.blob_port
        );
        let resp = client
            .put(&url)
            .header("x-ms-date", chrono::Utc::now().to_rfc2822())
            .header("x-ms-version", "2020-04-08")
            .send()
            .await
            .map_err(|e| AzureHarnessError::ContainerCreate(e.to_string()))?;
        if resp.status().is_success() || resp.status().as_u16() == 409 {
            Ok(())
        } else {
            Err(AzureHarnessError::ContainerCreate(format!(
                "unexpected status: {}",
                resp.status()
            )))
        }
    }
}

impl Drop for AzureEmulatorHarness {
    fn drop(&mut self) {
        let id = self.container_id.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("docker")
                .args(["kill", &id])
                .output();
        });
    }
}

/// Errors from the Azure emulator harness.
#[derive(Debug, thiserror::Error)]
pub enum AzureHarnessError {
    #[error("docker error: {0}")]
    Docker(String),
    #[error("timeout: {0}")]
    Timeout(String),
    #[error("container creation failed: {0}")]
    ContainerCreate(String),
    #[error("port allocation failed: {0}")]
    PortAllocation(String),
}

/// Find an available TCP port.
async fn find_available_port() -> Result<u16, AzureHarnessError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AzureHarnessError::PortAllocation(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| AzureHarnessError::PortAllocation(e.to_string()))?
        .port();
    drop(listener);
    Ok(port)
}
