//! Integration tests for the metrics HTTP server path routing (v0.21).
//!
//! Verifies that `start_metrics_server` serves metrics on the configured path
//! and returns 404 for any other path.

use rocklake_catalog::metrics::{start_metrics_server, CatalogMetrics};
use std::sync::Arc;
use std::time::Duration;

/// Find a free TCP port by binding to port 0 and reading the assigned address.
fn find_free_port() -> u16 {
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Perform a minimal raw HTTP GET request and return the status code and body.
async fn http_get(port: u16, path: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("could not connect to metrics server");

    let request = format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write failed");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read failed");

    let raw = String::from_utf8_lossy(&response);
    let status: u16 = raw
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);

    let body = raw
        .find("\r\n\r\n")
        .map(|pos| &raw[pos + 4..])
        .unwrap_or("")
        .to_string();

    (status, body)
}

#[tokio::test]
async fn metrics_path_routing_custom_path_200() {
    let port = find_free_port();
    let metrics = Arc::new(CatalogMetrics::new(100));
    let metrics_path = "/custom-metrics".to_string();

    // Start server in background.
    let srv_metrics = metrics.clone();
    let srv_path = metrics_path.clone();
    tokio::spawn(async move {
        start_metrics_server(srv_metrics, port, &srv_path)
            .await
            .ok();
    });

    // Give server a moment to bind.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request on the configured path → 200.
    let (status, body) = http_get(port, &metrics_path).await;
    assert_eq!(status, 200, "expected 200 on configured path, got {status}");
    // The body should contain Prometheus exposition format.
    assert!(
        body.contains('#') || body.is_empty(),
        "unexpected body: {body:?}"
    );
}

#[tokio::test]
async fn metrics_path_routing_wrong_path_404() {
    let port = find_free_port();
    let metrics = Arc::new(CatalogMetrics::new(100));

    // Start server with default /metrics path.
    let srv_metrics = metrics.clone();
    tokio::spawn(async move {
        start_metrics_server(srv_metrics, port, "/metrics")
            .await
            .ok();
    });

    // Give server a moment to bind.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request to a different path → 404.
    let (status, _body) = http_get(port, "/wrong-path").await;
    assert_eq!(status, 404, "expected 404 on wrong path, got {status}");
}

#[tokio::test]
async fn metrics_path_routing_root_404_when_custom() {
    let port = find_free_port();
    let metrics = Arc::new(CatalogMetrics::new(100));

    // Start server with custom path, not the root.
    let srv_metrics = metrics.clone();
    tokio::spawn(async move {
        start_metrics_server(srv_metrics, port, "/custom-metrics")
            .await
            .ok();
    });

    // Give server a moment to bind.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Request to /metrics → 404 (only /custom-metrics serves 200).
    let (status, _body) = http_get(port, "/metrics").await;
    assert_eq!(
        status, 404,
        "expected 404 on /metrics when custom path is set, got {status}"
    );
}
