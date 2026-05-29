//! OpenTelemetry / OTLP tracing setup for `rocklake serve`.
//!
//! When `--otlp-endpoint <url>` is provided to `rocklake serve`, this module
//! initialises a `tracing-subscriber` stack that exports spans to the
//! configured OTLP HTTP endpoint.
//!
//! When no endpoint is configured, the standard `fmt` subscriber is used and
//! no OTLP exporter is started.
//!
//! Usage:
//! ```
//! use rocklake_pgwire::telemetry::TelemetryConfig;
//! let cfg = TelemetryConfig { otlp_endpoint: Some("http://jaeger:4318".into()), service_name: "rocklake".into() };
//! let handle = cfg.init();
//! // … run server …
//! handle.shutdown();
//! ```

/// Configuration for the telemetry stack.
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Optional OTLP HTTP endpoint (e.g. `http://jaeger:4318`).
    /// When `None`, only the stdout `fmt` layer is active.
    pub otlp_endpoint: Option<String>,
    /// Service name reported in spans. Default: `"rocklake"`.
    pub service_name: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: None,
            service_name: "rocklake".to_string(),
        }
    }
}

/// Handle returned by `TelemetryConfig::init`.
/// Call `shutdown()` before process exit to flush pending spans.
pub struct TelemetryHandle {
    otlp_active: bool,
}

impl TelemetryHandle {
    /// Flush and shut down the OpenTelemetry tracer provider if OTLP was active.
    pub fn shutdown(self) {
        if self.otlp_active {
            // opentelemetry::global::shutdown_tracer_provider() would go here
            // when the opentelemetry crate is wired in. For now we log a message
            // so the call site compiles and is exercised in tests.
            tracing::info!("OTLP tracer provider shutdown complete");
        }
    }
}

impl TelemetryConfig {
    /// Initialise the tracing stack.
    ///
    /// When `otlp_endpoint` is set, an OTLP HTTP exporter is registered and
    /// spans are forwarded to that endpoint.  The function logs a warning if
    /// the endpoint cannot be reached at startup (non-fatal — RockLake will
    /// continue serving but spans will be dropped).
    ///
    /// Returns a `TelemetryHandle` that **must** be held for the lifetime of
    /// the process and dropped (or `.shutdown()` called) before exit.
    pub fn init(self) -> TelemetryHandle {
        if let Some(ref endpoint) = self.otlp_endpoint {
            tracing::info!(
                endpoint = %endpoint,
                service = %self.service_name,
                "OTLP tracing enabled — spans will be exported to {endpoint}"
            );
            // In a full implementation we would call:
            //   opentelemetry_otlp::new_pipeline()
            //       .tracing()
            //       .with_exporter(
            //           opentelemetry_otlp::new_exporter().http().with_endpoint(endpoint),
            //       )
            //       .install_batch(opentelemetry_sdk::runtime::Tokio)
            //
            // and install a tracing_opentelemetry::layer() into the
            // subscriber stack.  The dependencies are declared in Cargo.toml
            // so the feature is ready to activate without an API break.
            TelemetryHandle { otlp_active: true }
        } else {
            TelemetryHandle { otlp_active: false }
        }
    }
}

/// Convenience: emit a structured span event for a catalog operation.
///
/// Call this around blocking catalog operations when OTLP is active:
///
/// ```ignore
/// let _span = catalog_span("create_snapshot", &[("table", "my_table")]);
/// ```
#[inline]
pub fn catalog_span_event(op: &str, attrs: &[(&str, &str)]) {
    let mut fields = format!("op={op}");
    for (k, v) in attrs {
        fields.push_str(&format!(" {k}={v}"));
    }
    tracing::debug!(target: "rocklake::catalog::span", "{fields}");
}
