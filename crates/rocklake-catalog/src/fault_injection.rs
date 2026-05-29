//! Fault injection infrastructure for catalog correctness testing.
//!
//! Provides:
//! - `WriteFaultPoint`: named fail points at catalog write boundaries
//!   (before SlateDB commit, after Parquet write but before register_data_file,
//!   between primary and secondary key writes in register_data_file)
//! - `ErrorInjectedObjectStore`: wraps object_store with configurable errors
//!   for S3 error injection testing (503 responses, connection drops, partial
//!   reads)
//! - SLO measurement helpers for kill-9 recovery time assertion
//! - Crash simulation helpers via abrupt Db drop and catalog re-open

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ─── WriteFaultPoint ──────────────────────────────────────────────────────────

/// Named write boundaries where fail points can be injected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WriteFaultPoint {
    /// Before the SlateDB commit in `create_snapshot()`.
    BeforeSlateDbCommit,
    /// After a Parquet data file is written to object storage but before
    /// `register_data_file` is called (simulates a crash between data and
    /// catalog write, leaving an orphan file).
    AfterParquetWriteBeforeRegisterDataFile,
    /// Between writing the primary data-file key and the secondary
    /// `TAG_DATA_FILE_BY_SNAPSHOT` index key in `register_data_file`.
    BetweenPrimaryAndSecondaryKeyWrite,
}

impl std::fmt::Display for WriteFaultPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            WriteFaultPoint::BeforeSlateDbCommit => "before-slatedb-commit",
            WriteFaultPoint::AfterParquetWriteBeforeRegisterDataFile => {
                "after-parquet-write-before-register"
            }
            WriteFaultPoint::BetweenPrimaryAndSecondaryKeyWrite => {
                "between-primary-secondary-key-write"
            }
        };
        write!(f, "{name}")
    }
}

/// Global registry of active fault points.
static FAULT_REGISTRY: std::sync::OnceLock<Arc<Mutex<HashMap<String, FaultAction>>>> =
    std::sync::OnceLock::new();

fn fault_registry() -> Arc<Mutex<HashMap<String, FaultAction>>> {
    FAULT_REGISTRY
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

/// The action to take when a fault point is triggered.
#[derive(Debug, Clone)]
pub enum FaultAction {
    /// Return an error immediately.
    ReturnError(String),
    /// Pause for the given duration before continuing.
    Pause(Duration),
}

/// Fault injector: set and clear fail points at write boundaries.
///
/// All fault points are stored in a global registry so they can be set
/// from test code and checked from within catalog code paths.
#[derive(Debug, Clone, Default)]
pub struct FaultInjector;

impl FaultInjector {
    /// Create a new fault injector.
    pub fn new() -> Self {
        Self
    }

    /// Configure a fault point to return an error.
    pub fn set_error(&self, point: WriteFaultPoint, message: &str) {
        let registry = fault_registry();
        let mut map = registry.lock().unwrap();
        map.insert(
            point.to_string(),
            FaultAction::ReturnError(message.to_string()),
        );
    }

    /// Configure a fault point to pause for a duration.
    pub fn set_pause(&self, point: WriteFaultPoint, duration: Duration) {
        let registry = fault_registry();
        let mut map = registry.lock().unwrap();
        map.insert(point.to_string(), FaultAction::Pause(duration));
    }

    /// Remove a fault point (clear it).
    pub fn clear(&self, point: WriteFaultPoint) {
        let registry = fault_registry();
        let mut map = registry.lock().unwrap();
        map.remove(&point.to_string());
    }

    /// Clear all active fault points.
    pub fn clear_all(&self) {
        let registry = fault_registry();
        let mut map = registry.lock().unwrap();
        map.clear();
    }

    /// Check if a fault point is active and return its action, if any.
    pub fn check(&self, point: &WriteFaultPoint) -> Option<FaultAction> {
        let registry = fault_registry();
        let map = registry.lock().unwrap();
        map.get(&point.to_string()).cloned()
    }

    /// Trigger a fault point — panics with the error message if the point is
    /// active with `ReturnError` (for use in test-only code paths).
    pub fn trigger_or_panic(&self, point: &WriteFaultPoint) {
        if let Some(FaultAction::ReturnError(msg)) = self.check(point) {
            panic!("Injected fault at {point}: {msg}");
        }
    }
}

// ─── SLO Measurement ─────────────────────────────────────────────────────────

/// Measure the time from a simulated kill-9 (catalog drop) to when the next
/// writer becomes available (catalog re-open + epoch acquisition).
///
/// Returns the duration of the recovery.  The caller should assert this is
/// below the target SLO (p99 < 10 seconds).
pub async fn measure_kill9_recovery_slo<F, Fut>(recovery_fn: F) -> Duration
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let start = Instant::now();
    recovery_fn().await;
    start.elapsed()
}

/// Assert that a measured recovery time satisfies the kill-9 SLO.
///
/// The target is p99 < 10 seconds.  In a test environment with a local
/// filesystem backend, recovery is typically < 500 ms.
pub fn assert_kill9_slo(duration: Duration) {
    let slo = Duration::from_secs(10);
    assert!(
        duration < slo,
        "Kill-9 recovery SLO violated: took {duration:?}, target p99 < {slo:?}"
    );
}

// ─── ErrorInjectedObjectStore ─────────────────────────────────────────────────

use futures::stream::BoxStream;
use object_store::{
    path::Path as ObjectPath, Error as OsError, GetOptions, GetResult, ListResult, MultipartUpload,
    ObjectMeta, ObjectStore, PutMultipartOptions, PutOptions, PutPayload, PutResult,
};

/// An `ObjectStore` wrapper that injects configurable errors on specific
/// operations.  Used to test S3 503 responses, connection drops, and partial
/// reads without needing a real S3 or toxiproxy setup.
#[derive(Debug)]
pub struct ErrorInjectedStore {
    inner: Arc<dyn ObjectStore>,
    /// Fail the next `put` call with a 503-like error.
    fail_next_put: Arc<Mutex<Option<String>>>,
    /// Fail the next `get` call with a connection-drop error.
    fail_next_get: Arc<Mutex<Option<String>>>,
    /// Fail the next `list` call.
    fail_next_list: Arc<Mutex<Option<String>>>,
    /// Count of successful operations.
    put_count: Arc<Mutex<u64>>,
    get_count: Arc<Mutex<u64>>,
}

impl ErrorInjectedStore {
    /// Wrap an existing `ObjectStore` with error injection.
    pub fn new(inner: Arc<dyn ObjectStore>) -> Self {
        Self {
            inner,
            fail_next_put: Arc::new(Mutex::new(None)),
            fail_next_get: Arc::new(Mutex::new(None)),
            fail_next_list: Arc::new(Mutex::new(None)),
            put_count: Arc::new(Mutex::new(0)),
            get_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Configure the next `put` to return a 503-like error.
    pub fn inject_put_error(&self, message: &str) {
        *self.fail_next_put.lock().unwrap() = Some(message.to_string());
    }

    /// Configure the next `get` to return a connection-drop error.
    pub fn inject_get_error(&self, message: &str) {
        *self.fail_next_get.lock().unwrap() = Some(message.to_string());
    }

    /// Configure the next `list` to return an error.
    pub fn inject_list_error(&self, message: &str) {
        *self.fail_next_list.lock().unwrap() = Some(message.to_string());
    }

    /// Get the number of successful put operations.
    pub fn put_count(&self) -> u64 {
        *self.put_count.lock().unwrap()
    }

    /// Get the number of successful get operations.
    pub fn get_count(&self) -> u64 {
        *self.get_count.lock().unwrap()
    }

    fn take_put_error(&self) -> Option<String> {
        self.fail_next_put.lock().unwrap().take()
    }

    fn take_get_error(&self) -> Option<String> {
        self.fail_next_get.lock().unwrap().take()
    }

    fn take_list_error(&self) -> Option<String> {
        self.fail_next_list.lock().unwrap().take()
    }
}

impl std::fmt::Display for ErrorInjectedStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ErrorInjectedStore({})", self.inner)
    }
}

#[async_trait::async_trait]
impl ObjectStore for ErrorInjectedStore {
    async fn put_opts(
        &self,
        location: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> Result<PutResult, OsError> {
        if let Some(msg) = self.take_put_error() {
            return Err(OsError::Generic {
                store: "ErrorInjectedStore",
                source: msg.into(),
            });
        }
        let result = self.inner.put_opts(location, payload, options).await?;
        *self.put_count.lock().unwrap() += 1;
        Ok(result)
    }

    async fn put_multipart_opts(
        &self,
        location: &ObjectPath,
        options: PutMultipartOptions,
    ) -> Result<Box<dyn MultipartUpload>, OsError> {
        self.inner.put_multipart_opts(location, options).await
    }

    async fn get_opts(
        &self,
        location: &ObjectPath,
        options: GetOptions,
    ) -> Result<GetResult, OsError> {
        if let Some(msg) = self.take_get_error() {
            return Err(OsError::Generic {
                store: "ErrorInjectedStore",
                source: msg.into(),
            });
        }
        let result = self.inner.get_opts(location, options).await?;
        *self.get_count.lock().unwrap() += 1;
        Ok(result)
    }

    async fn head(&self, location: &ObjectPath) -> Result<ObjectMeta, OsError> {
        self.inner.head(location).await
    }

    async fn delete(&self, location: &ObjectPath) -> Result<(), OsError> {
        self.inner.delete(location).await
    }

    fn list(&self, prefix: Option<&ObjectPath>) -> BoxStream<'static, Result<ObjectMeta, OsError>> {
        if let Some(msg) = self.take_list_error() {
            let err = OsError::Generic {
                store: "ErrorInjectedStore",
                source: msg.into(),
            };
            return Box::pin(futures::stream::once(async move { Err(err) }));
        }
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> Result<ListResult, OsError> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy(&self, from: &ObjectPath, to: &ObjectPath) -> Result<(), OsError> {
        self.inner.copy(from, to).await
    }

    async fn copy_if_not_exists(&self, from: &ObjectPath, to: &ObjectPath) -> Result<(), OsError> {
        self.inner.copy_if_not_exists(from, to).await
    }
}
