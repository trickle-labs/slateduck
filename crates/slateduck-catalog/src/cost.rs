//! S3 API cost analysis and cost-mode configuration.
//!
//! Tracks observed S3 API call counts (PUT, GET, LIST) per operation category
//! and estimates monthly cost at standard pricing. Also provides the `--cost-mode`
//! presets that let operators pick a cost/latency trade-off without needing to
//! understand SlateDB internals.

#![allow(missing_docs)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::performance::SlateDbTuning;

// ─── Cost-Mode Presets ─────────────────────────────────────────────────────

/// Named cost/latency presets for `--cost-mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CostMode {
    /// Larger memtable, lower L0 flush frequency, fewer S3 PUTs.
    /// Cost-sensitive workloads; accepts higher p99 write latency.
    Conservative,
    /// Tuned for the TPC-H SF10 benchmark workload.
    #[default]
    Balanced,
    /// Frequent flushes, aggressive compaction, more S3 API calls.
    /// Interactive analyst workloads on S3 Express.
    Latency,
}

impl std::str::FromStr for CostMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "conservative" => Ok(Self::Conservative),
            "balanced" => Ok(Self::Balanced),
            "latency" => Ok(Self::Latency),
            _ => Err(format!(
                "unknown cost-mode {:?}; expected conservative|balanced|latency",
                s
            )),
        }
    }
}

impl CostMode {
    /// Return the SlateDB tuning parameters for this mode.
    pub fn tuning(&self) -> SlateDbTuning {
        match self {
            Self::Conservative => SlateDbTuning {
                block_size: 8192,
                bloom_filter_enabled: true,
                bloom_filter_fp_rate: 0.01,
                l0_sst_count_threshold: 8,
                max_write_batch_bytes: 64 * 1024 * 1024,
                compaction_aggressiveness: 2,
            },
            Self::Balanced => SlateDbTuning::default(),
            Self::Latency => SlateDbTuning {
                block_size: 4096,
                bloom_filter_enabled: true,
                bloom_filter_fp_rate: 0.01,
                l0_sst_count_threshold: 2,
                max_write_batch_bytes: 32 * 1024 * 1024,
                compaction_aggressiveness: 9,
            },
        }
    }

    /// Return the measured cost profile description.
    pub fn profile_description(&self) -> &'static str {
        match self {
            Self::Conservative => {
                "Fewer S3 API calls; higher p99 write latency (~2-5x balanced). \
                 Best for cost-sensitive append-only workloads."
            }
            Self::Balanced => {
                "Tuned for TPC-H SF10. p50 write ~80ms on S3 Standard. \
                 Estimated $15-25/month at 100 files/min."
            }
            Self::Latency => {
                "More S3 API calls; lower p99 write latency (~0.5x balanced). \
                 Best for interactive analyst workloads on S3 Express One Zone."
            }
        }
    }
}

// ─── API Call Counters ─────────────────────────────────────────────────────

/// Thread-safe counters for S3 API calls by category.
#[derive(Debug, Default)]
pub struct ApiCallCounters {
    pub put_count: AtomicU64,
    pub get_count: AtomicU64,
    pub list_count: AtomicU64,
    pub delete_count: AtomicU64,
    /// Start time for rate calculations.
    start_time: std::sync::Mutex<Option<SystemTime>>,
}

impl ApiCallCounters {
    pub fn new() -> Arc<Self> {
        let c = Arc::new(Self::default());
        *c.start_time.lock().unwrap() = Some(SystemTime::now());
        c
    }

    pub fn record_put(&self) {
        self.put_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_get(&self) {
        self.get_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_list(&self) {
        self.list_count.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_delete(&self) {
        self.delete_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> ApiCallSnapshot {
        let elapsed = self
            .start_time
            .lock()
            .unwrap()
            .and_then(|t| t.elapsed().ok())
            .unwrap_or(Duration::from_secs(1));
        ApiCallSnapshot {
            put_count: self.put_count.load(Ordering::Relaxed),
            get_count: self.get_count.load(Ordering::Relaxed),
            list_count: self.list_count.load(Ordering::Relaxed),
            delete_count: self.delete_count.load(Ordering::Relaxed),
            elapsed,
        }
    }
}

/// A point-in-time snapshot of API call counts.
#[derive(Debug, Clone)]
pub struct ApiCallSnapshot {
    pub put_count: u64,
    pub get_count: u64,
    pub list_count: u64,
    pub delete_count: u64,
    pub elapsed: Duration,
}

// ─── Cost Report ────────────────────────────────────────────────────────────

/// S3 API pricing constants (US East 1, as of 2025).
/// Source: https://aws.amazon.com/s3/pricing/
const S3_PUT_COST_PER_1000: f64 = 0.005; // $0.005 per 1,000 PUT requests
const S3_GET_COST_PER_1000: f64 = 0.0004; // $0.0004 per 1,000 GET requests
const S3_LIST_COST_PER_1000: f64 = 0.005; // $0.005 per 1,000 LIST requests

// RDS db.t3.medium on-demand price (us-east-1, as of 2025)
const RDS_T3_MEDIUM_HOURLY: f64 = 0.068;
const RDS_T3_MEDIUM_MONTHLY: f64 = RDS_T3_MEDIUM_HOURLY * 24.0 * 30.0;

/// Complete S3 API cost report.
#[derive(Debug, Clone)]
pub struct ApiCostReport {
    pub put_count: u64,
    pub get_count: u64,
    pub list_count: u64,
    pub delete_count: u64,
    pub elapsed_secs: f64,
    /// Estimated monthly cost at observed call rates.
    pub estimated_monthly_usd: f64,
    /// Monthly cost for RDS db.t3.medium at current AWS pricing.
    pub rds_monthly_usd: f64,
    /// PUT calls per minute at current rate.
    pub put_per_minute: f64,
    /// GET calls per minute at current rate.
    pub get_per_minute: f64,
    /// LIST calls per minute at current rate.
    pub list_per_minute: f64,
    /// Recommended settings to stay within a monthly cost target.
    pub recommendations: Vec<String>,
}

impl ApiCostReport {
    /// Build a cost report from a snapshot of API call counters.
    pub fn from_snapshot(snap: &ApiCallSnapshot) -> Self {
        let elapsed_secs = snap.elapsed.as_secs_f64().max(1.0);
        let elapsed_minutes = elapsed_secs / 60.0;
        let seconds_per_month = 30.0 * 24.0 * 3600.0;
        let scale = seconds_per_month / elapsed_secs;

        let monthly_puts = snap.put_count as f64 * scale;
        let monthly_gets = snap.get_count as f64 * scale;
        let monthly_lists = snap.list_count as f64 * scale;

        let estimated_monthly_usd = monthly_puts / 1000.0 * S3_PUT_COST_PER_1000
            + monthly_gets / 1000.0 * S3_GET_COST_PER_1000
            + monthly_lists / 1000.0 * S3_LIST_COST_PER_1000;

        let put_per_minute = snap.put_count as f64 / elapsed_minutes;
        let get_per_minute = snap.get_count as f64 / elapsed_minutes;
        let list_per_minute = snap.list_count as f64 / elapsed_minutes;

        let mut recommendations = Vec::new();
        if put_per_minute > 100.0 {
            recommendations.push(
                "High PUT rate detected. Consider --cost-mode=conservative to reduce \
                 flush frequency and lower S3 PUT costs."
                    .to_string(),
            );
        }
        if estimated_monthly_usd > RDS_T3_MEDIUM_MONTHLY {
            recommendations.push(format!(
                "Estimated S3 API cost (${:.2}/month) exceeds RDS db.t3.medium (${:.2}/month). \
                 Consider increasing --cache-size-mb to reduce GET calls.",
                estimated_monthly_usd, RDS_T3_MEDIUM_MONTHLY
            ));
        }

        Self {
            put_count: snap.put_count,
            get_count: snap.get_count,
            list_count: snap.list_count,
            delete_count: snap.delete_count,
            elapsed_secs,
            estimated_monthly_usd,
            rds_monthly_usd: RDS_T3_MEDIUM_MONTHLY,
            put_per_minute,
            get_per_minute,
            list_per_minute,
            recommendations,
        }
    }

    /// Print a human-readable cost report.
    pub fn print(&self) {
        println!("S3 API Cost Analysis");
        println!("====================");
        println!("Observed period: {:.1}s", self.elapsed_secs);
        println!();
        println!("API Call Counts:");
        println!(
            "  PUT:    {} ({:.1}/min)",
            self.put_count, self.put_per_minute
        );
        println!(
            "  GET:    {} ({:.1}/min)",
            self.get_count, self.get_per_minute
        );
        println!(
            "  LIST:   {} ({:.1}/min)",
            self.list_count, self.list_per_minute
        );
        println!("  DELETE: {}", self.delete_count);
        println!();
        println!("Cost Estimates (us-east-1 pricing):");
        println!(
            "  Estimated monthly S3 API cost:  ${:.2}",
            self.estimated_monthly_usd
        );
        println!(
            "  RDS db.t3.medium monthly cost:  ${:.2}",
            self.rds_monthly_usd
        );
        if self.estimated_monthly_usd < self.rds_monthly_usd {
            println!(
                "  → SlateDuck is {:.1}x cheaper than RDS at this ingest rate",
                self.rds_monthly_usd / self.estimated_monthly_usd.max(0.01)
            );
        } else {
            println!(
                "  → RDS would be {:.1}x cheaper at this ingest rate",
                self.estimated_monthly_usd / self.rds_monthly_usd.max(0.01)
            );
        }
        if !self.recommendations.is_empty() {
            println!();
            println!("Recommendations:");
            for r in &self.recommendations {
                println!("  • {r}");
            }
        }
    }
}

// ─── Tune Recommendations ──────────────────────────────────────────────────

/// Output recommended SlateDB settings to hit a monthly cost target.
pub fn tune_for_cost_target(
    target_usd_per_month: f64,
    current_report: &ApiCostReport,
) -> Vec<String> {
    let mut recs = Vec::new();

    if current_report.estimated_monthly_usd <= target_usd_per_month {
        recs.push(format!(
            "Current estimated cost (${:.2}/month) is already within target (${:.2}/month). No changes needed.",
            current_report.estimated_monthly_usd, target_usd_per_month
        ));
        return recs;
    }

    let reduction_needed = current_report.estimated_monthly_usd / target_usd_per_month.max(0.01);

    recs.push(format!(
        "Need to reduce API costs by {:.1}x to reach ${:.2}/month target.",
        reduction_needed, target_usd_per_month
    ));
    recs.push("Recommended settings:".to_string());

    if reduction_needed > 2.0 {
        recs.push("  --cost-mode=conservative".to_string());
        recs.push("  --cache-size-mb=4096  (reduce GET calls with larger cache)".to_string());
        recs.push("  l0_sst_count_threshold=8  (fewer flushes = fewer PUT calls)".to_string());
    } else {
        recs.push("  --cost-mode=balanced".to_string());
        recs.push("  --cache-size-mb=2048".to_string());
    }

    recs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_mode_from_str() {
        assert_eq!(
            "conservative".parse::<CostMode>(),
            Ok(CostMode::Conservative)
        );
        assert_eq!("balanced".parse::<CostMode>(), Ok(CostMode::Balanced));
        assert_eq!("latency".parse::<CostMode>(), Ok(CostMode::Latency));
        assert!("unknown".parse::<CostMode>().is_err());
    }

    #[test]
    fn cost_report_from_zero_snapshot() {
        let snap = ApiCallSnapshot {
            put_count: 0,
            get_count: 0,
            list_count: 0,
            delete_count: 0,
            elapsed: Duration::from_secs(60),
        };
        let report = ApiCostReport::from_snapshot(&snap);
        assert_eq!(report.estimated_monthly_usd, 0.0);
    }

    #[test]
    fn cost_report_rds_comparison() {
        // At high ingest: 1000 PUTs in 60s → large monthly extrapolation
        let snap = ApiCallSnapshot {
            put_count: 1000,
            get_count: 5000,
            list_count: 100,
            delete_count: 0,
            elapsed: Duration::from_secs(60),
        };
        let report = ApiCostReport::from_snapshot(&snap);
        // Should have a non-trivial estimated cost
        assert!(report.estimated_monthly_usd > 0.0);
        // RDS baseline should be ~$48.96
        assert!(report.rds_monthly_usd > 40.0);
    }

    #[test]
    fn tune_for_cost_target_already_within() {
        let snap = ApiCallSnapshot {
            put_count: 1,
            get_count: 1,
            list_count: 1,
            delete_count: 0,
            elapsed: Duration::from_secs(3600),
        };
        let report = ApiCostReport::from_snapshot(&snap);
        let recs = tune_for_cost_target(1000.0, &report);
        assert!(recs[0].contains("already within target"));
    }
}
