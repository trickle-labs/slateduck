//! Observability: catalog-level metrics and Prometheus-compatible `/metrics` endpoint.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Catalog-level metrics.
#[derive(Debug)]
pub struct CatalogMetrics {
    /// Snapshots created per second (total counter).
    pub snapshots_created: AtomicU64,
    /// Files registered per snapshot (last observation).
    pub files_per_snapshot: AtomicU64,
    /// Mean rows scanned per `list_data_files` (last observation).
    pub mean_rows_scanned: AtomicU64,
    /// Object-store request count.
    pub object_store_requests: AtomicU64,
    /// Object-store bytes read.
    pub object_store_bytes_read: AtomicU64,
    /// Object-store bytes written.
    pub object_store_bytes_written: AtomicU64,
    /// Object-store throttle count.
    pub object_store_throttles: AtomicU64,
    /// Object-store retry count.
    pub object_store_retries: AtomicU64,
    /// Active session count.
    pub active_sessions: AtomicU64,
    /// Max sessions configured.
    pub max_sessions: AtomicU64,
    /// Writer epoch age in milliseconds.
    pub writer_epoch_age_ms: AtomicU64,
    /// Per-query scanned key count (last).
    pub last_query_keys_scanned: AtomicU64,
    /// CDC record-count mismatches: fires when a Parquet file's scanned row
    /// count differs from the `record_count` stored in catalog metadata (N-04).
    pub cdc_record_count_mismatches: AtomicU64,
}

impl CatalogMetrics {
    pub fn new(max_sessions: u64) -> Self {
        Self {
            snapshots_created: AtomicU64::new(0),
            files_per_snapshot: AtomicU64::new(0),
            mean_rows_scanned: AtomicU64::new(0),
            object_store_requests: AtomicU64::new(0),
            object_store_bytes_read: AtomicU64::new(0),
            object_store_bytes_written: AtomicU64::new(0),
            object_store_throttles: AtomicU64::new(0),
            object_store_retries: AtomicU64::new(0),
            active_sessions: AtomicU64::new(0),
            max_sessions: AtomicU64::new(max_sessions),
            writer_epoch_age_ms: AtomicU64::new(0),
            last_query_keys_scanned: AtomicU64::new(0),
            cdc_record_count_mismatches: AtomicU64::new(0),
        }
    }

    pub fn increment_snapshots(&self) {
        self.snapshots_created.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_files_per_snapshot(&self, count: u64) {
        self.files_per_snapshot.store(count, Ordering::Relaxed);
    }

    pub fn increment_object_store_requests(&self) {
        self.object_store_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add_object_store_bytes_read(&self, bytes: u64) {
        self.object_store_bytes_read
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn add_object_store_bytes_written(&self, bytes: u64) {
        self.object_store_bytes_written
            .fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn increment_throttles(&self) {
        self.object_store_throttles.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_retries(&self) {
        self.object_store_retries.fetch_add(1, Ordering::Relaxed);
    }

    pub fn set_active_sessions(&self, count: u64) {
        self.active_sessions.store(count, Ordering::Relaxed);
    }

    pub fn set_writer_epoch_age_ms(&self, age: u64) {
        self.writer_epoch_age_ms.store(age, Ordering::Relaxed);
    }

    pub fn set_last_query_keys_scanned(&self, count: u64) {
        self.last_query_keys_scanned.store(count, Ordering::Relaxed);
    }

    /// Sync the CDC record-count mismatch counter from the global counter in
    /// `slateduck-sql`.  Call this from a background task in the PG-Wire binary.
    pub fn set_cdc_record_count_mismatches(&self, n: u64) {
        self.cdc_record_count_mismatches.store(n, Ordering::Relaxed);
    }

    /// Render Prometheus-compatible metrics output.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::new();

        out.push_str("# HELP slateduck_snapshots_created_total Total snapshots created.\n");
        out.push_str("# TYPE slateduck_snapshots_created_total counter\n");
        out.push_str(&format!(
            "slateduck_snapshots_created_total {}\n",
            self.snapshots_created.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP slateduck_files_per_snapshot Files registered in last snapshot.\n");
        out.push_str("# TYPE slateduck_files_per_snapshot gauge\n");
        out.push_str(&format!(
            "slateduck_files_per_snapshot {}\n",
            self.files_per_snapshot.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP slateduck_object_store_requests_total Object store request count.\n");
        out.push_str("# TYPE slateduck_object_store_requests_total counter\n");
        out.push_str(&format!(
            "slateduck_object_store_requests_total {}\n",
            self.object_store_requests.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP slateduck_object_store_bytes_read_total Bytes read from object store.\n",
        );
        out.push_str("# TYPE slateduck_object_store_bytes_read_total counter\n");
        out.push_str(&format!(
            "slateduck_object_store_bytes_read_total {}\n",
            self.object_store_bytes_read.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP slateduck_object_store_bytes_written_total Bytes written to object store.\n",
        );
        out.push_str("# TYPE slateduck_object_store_bytes_written_total counter\n");
        out.push_str(&format!(
            "slateduck_object_store_bytes_written_total {}\n",
            self.object_store_bytes_written.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP slateduck_object_store_throttles_total Object store throttle events.\n",
        );
        out.push_str("# TYPE slateduck_object_store_throttles_total counter\n");
        out.push_str(&format!(
            "slateduck_object_store_throttles_total {}\n",
            self.object_store_throttles.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP slateduck_object_store_retries_total Object store retry events.\n");
        out.push_str("# TYPE slateduck_object_store_retries_total counter\n");
        out.push_str(&format!(
            "slateduck_object_store_retries_total {}\n",
            self.object_store_retries.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP slateduck_active_sessions Current active PG sessions.\n");
        out.push_str("# TYPE slateduck_active_sessions gauge\n");
        out.push_str(&format!(
            "slateduck_active_sessions {}\n",
            self.active_sessions.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP slateduck_max_sessions Maximum allowed sessions.\n");
        out.push_str("# TYPE slateduck_max_sessions gauge\n");
        out.push_str(&format!(
            "slateduck_max_sessions {}\n",
            self.max_sessions.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP slateduck_writer_epoch_age_ms Writer epoch age in milliseconds.\n");
        out.push_str("# TYPE slateduck_writer_epoch_age_ms gauge\n");
        out.push_str(&format!(
            "slateduck_writer_epoch_age_ms {}\n",
            self.writer_epoch_age_ms.load(Ordering::Relaxed)
        ));

        out.push_str("# HELP slateduck_last_query_keys_scanned Keys scanned in last query.\n");
        out.push_str("# TYPE slateduck_last_query_keys_scanned gauge\n");
        out.push_str(&format!(
            "slateduck_last_query_keys_scanned {}\n",
            self.last_query_keys_scanned.load(Ordering::Relaxed)
        ));

        out.push_str(
            "# HELP slateduck_cdc_record_count_mismatch_total \
             Times a Parquet file's scanned row count differed from catalog metadata (N-04).\n",
        );
        out.push_str("# TYPE slateduck_cdc_record_count_mismatch_total counter\n");
        out.push_str(&format!(
            "slateduck_cdc_record_count_mismatch_total {}\n",
            self.cdc_record_count_mismatches.load(Ordering::Relaxed)
        ));

        out
    }
}

/// Start the metrics HTTP server on the given port, serving only on `metrics_path`.
///
/// Requests to any path other than `metrics_path` receive a 404 response.
pub async fn start_metrics_server(
    metrics: Arc<CatalogMetrics>,
    port: u16,
    metrics_path: &str,
) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    let metrics_path = metrics_path.to_string();
    tracing::info!("Metrics server listening on port {port}, path {metrics_path}");

    loop {
        let (mut socket, _) = listener.accept().await?;
        let metrics = metrics.clone();
        let path = metrics_path.clone();

        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            let n = tokio::io::AsyncReadExt::read(&mut socket, &mut buf)
                .await
                .unwrap_or(0);

            // Parse the HTTP request line to extract the requested path.
            let request_path = String::from_utf8_lossy(&buf[..n]);
            let req_path = request_path
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");

            if req_path != path {
                let response =
                    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = socket.write_all(response.as_bytes()).await;
                return;
            }

            let body = metrics.render_prometheus();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
    }
}
