//! Lambda reader example — serverless catalog reader pattern.
//!
//! This example demonstrates how to open a RockLake catalog in **read-only**
//! mode inside an AWS Lambda function (or any serverless environment with
//! ephemeral storage) and return the list of data files for a given table as
//! a JSON response.
//!
//! # Key properties
//!
//! - No `Db` writer handle is opened; the catalog is opened with a
//!   `CatalogStore` in read-only mode.
//! - The handler opens the catalog against a **named checkpoint pin** so that
//!   the snapshot it reads is stable regardless of concurrent writers.
//! - On cold start the SlateDB SST files are streamed from S3 Express; after
//!   the first invocation they remain in `/tmp` for subsequent warm invocations.
//!
//! # IAM policy (minimum required permissions)
//!
//! ```json
//! {
//!   "Version": "2012-10-17",
//!   "Statement": [{
//!     "Effect": "Allow",
//!     "Action": ["s3:GetObject", "s3:ListBucket"],
//!     "Resource": [
//!       "arn:aws:s3:::my-rocklake-bucket/catalog/*",
//!       "arn:aws:s3:::my-rocklake-bucket"
//!     ]
//!   }]
//! }
//! ```
//!
//! # Building
//!
//! ```sh
//! cargo build --release --example lambda_reader -p rocklake-client
//! ```

use rocklake_client::{CatalogClient, CatalogClientBuilder};

/// Simulated Lambda event: the table ID and (optional) pinned snapshot ID to
/// read from.
#[derive(Debug)]
struct LambdaEvent {
    table_id: u64,
    snapshot_id: Option<u64>,
}

/// Simulated Lambda response: data file paths as JSON-serialisable strings.
#[derive(Debug)]
struct LambdaResponse {
    snapshot_id: u64,
    data_file_paths: Vec<String>,
}

/// Lambda handler logic.
///
/// In a real Lambda this would be `async fn handler(event: LambdaEvent,
/// _ctx: lambda_runtime::Context) -> Result<LambdaResponse, lambda_runtime::Error>`.
async fn handler(
    client: &CatalogClient,
    event: LambdaEvent,
) -> Result<LambdaResponse, rocklake_client::ClientError> {
    // Use the provided snapshot ID or fall back to the latest.
    let snapshot_id = match event.snapshot_id {
        Some(id) => id,
        None => client.snapshot_id().await?,
    };

    let data_files = client.list_data_files(event.table_id, snapshot_id).await?;

    let paths: Vec<String> = data_files.iter().map(|f| f.path.clone()).collect();

    Ok(LambdaResponse {
        snapshot_id,
        data_file_paths: paths,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // In a real Lambda the catalog URI would come from an environment variable.
    let catalog_uri =
        std::env::var("ROCKLAKE_CATALOG_URI").unwrap_or_else(|_| "file:///tmp/catalog".to_string());

    let table_id: u64 = std::env::var("ROCKLAKE_TABLE_ID")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);

    // Open a read-only catalog client.  No write handle is created.
    let client = CatalogClientBuilder::new(&catalog_uri)
        .build()
        .await
        .map_err(|e| format!("failed to open catalog: {e}"))?;

    let event = LambdaEvent {
        table_id,
        snapshot_id: None, // use latest
    };

    match handler(&client, event).await {
        Ok(response) => {
            println!(
                "snapshot_id={} data_files={}",
                response.snapshot_id,
                response.data_file_paths.len()
            );
            for path in &response.data_file_paths {
                println!("  {path}");
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }

    client.close().await;
    Ok(())
}
