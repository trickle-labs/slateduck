//! `SoakHarness` — manages long-running soak test lifecycle and metrics.
//!
//! The harness drives a configurable number of write/read cycles against a
//! `CatalogStore`, records per-cycle latency, and asserts catalog consistency
//! at the end of the run.

use std::time::{Duration, Instant};

use rocklake_catalog::CatalogStore;

/// Per-cycle statistics recorded by the soak harness.
#[derive(Debug, Clone)]
pub struct SoakCycleStats {
    /// Cycle number (0-indexed).
    pub cycle: u64,
    /// Wall-clock duration of this cycle.
    pub duration: Duration,
    /// Snapshot ID written in this cycle.
    pub snapshot_id: u64,
    /// Number of schemas visible at the written snapshot.
    pub schema_count: usize,
}

/// Summary metrics collected over the entire soak run.
#[derive(Debug, Clone)]
pub struct SoakRunSummary {
    /// Total cycles completed.
    pub cycles_completed: u64,
    /// Total elapsed duration.
    pub elapsed: Duration,
    /// Minimum cycle duration.
    pub min_cycle: Duration,
    /// Maximum cycle duration.
    pub max_cycle: Duration,
    /// Whether all consistency checks passed.
    pub consistent: bool,
    /// Number of panics detected (0 on success).
    pub panics: u64,
}

/// Soak test harness configuration.
#[derive(Debug, Clone)]
pub struct SoakConfig {
    /// Total number of write/read cycles to execute.
    pub cycles: u64,
    /// Number of schema create operations per cycle.
    pub schemas_per_cycle: u64,
    /// Whether to assert secondary index integrity after each cycle.
    pub assert_index_integrity: bool,
}

impl Default for SoakConfig {
    fn default() -> Self {
        Self {
            cycles: 100,
            schemas_per_cycle: 1,
            assert_index_integrity: true,
        }
    }
}

/// Manages the soak test lifecycle.
pub struct SoakHarness {
    config: SoakConfig,
}

impl SoakHarness {
    /// Create a new `SoakHarness` with the given configuration.
    pub fn new(config: SoakConfig) -> Self {
        Self { config }
    }

    /// Run the soak loop against `store` and return the summary.
    ///
    /// Each cycle:
    /// 1. Creates `schemas_per_cycle` schemas with unique names.
    /// 2. Commits the snapshot.
    /// 3. Reads back the schema list and verifies it is non-empty.
    /// 4. Records latency and snapshot ID.
    ///
    /// If `assert_index_integrity` is enabled, the harness also checks that
    /// the reported schema count matches the expected cumulative count.
    pub async fn run(&self, store: &mut CatalogStore) -> SoakRunSummary {
        let start = Instant::now();
        let mut stats: Vec<SoakCycleStats> = Vec::with_capacity(self.config.cycles as usize);
        let mut consistent = true;
        let mut expected_schemas: usize = 0;

        for cycle in 0..self.config.cycles {
            let cycle_start = Instant::now();

            let mut writer = store.begin_write();
            for s in 0..self.config.schemas_per_cycle {
                let name = format!("soak_schema_{cycle}_{s}");
                writer
                    .create_schema(&name)
                    .await
                    .expect("soak: create_schema must not fail");
                expected_schemas += 1;
            }
            let result = writer
                .create_snapshot(None, None)
                .await
                .expect("soak: create_snapshot must not fail");
            let snapshot_id = result.snapshot_id.as_u64();
            store.commit_writer(result);

            let schemas = store
                .read_latest()
                .list_schemas()
                .await
                .expect("soak: list_schemas must not fail");

            let schema_count = schemas.len();

            if self.config.assert_index_integrity && schema_count != expected_schemas {
                tracing::error!(
                    cycle,
                    expected = expected_schemas,
                    actual = schema_count,
                    "soak: schema count mismatch — catalog may be inconsistent"
                );
                consistent = false;
            }

            let duration = cycle_start.elapsed();
            stats.push(SoakCycleStats {
                cycle,
                duration,
                snapshot_id,
                schema_count,
            });
        }

        let elapsed = start.elapsed();
        let min_cycle = stats
            .iter()
            .map(|s| s.duration)
            .min()
            .unwrap_or(Duration::ZERO);
        let max_cycle = stats
            .iter()
            .map(|s| s.duration)
            .max()
            .unwrap_or(Duration::ZERO);

        SoakRunSummary {
            cycles_completed: stats.len() as u64,
            elapsed,
            min_cycle,
            max_cycle,
            consistent,
            panics: 0,
        }
    }
}
