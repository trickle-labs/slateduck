//! Backpressure protocol and per-shard publication modes.
//!
//! Workers stall ingest when output plane is N snapshots behind.
//! Per-shard `output_mode = 'per_shard'` publishes individual shard frontiers.

use std::collections::HashMap;

/// Backpressure configuration.
#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    /// Maximum number of snapshots the output plane can lag behind.
    pub max_lag_snapshots: u64,
    /// Multiplier for skewed-shard detection (5× median by default).
    pub skew_threshold_multiplier: f64,
}

impl Default for BackpressureConfig {
    fn default() -> Self {
        Self {
            max_lag_snapshots: 100,
            skew_threshold_multiplier: 5.0,
        }
    }
}

/// Per-shard output publication mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputMode {
    /// Merged: all shards contribute to a single output snapshot.
    #[default]
    Merged,
    /// PerShard: each shard publishes its own frontier; query layer merges.
    PerShard,
}

/// Backpressure state for a worker.
#[derive(Debug)]
pub struct BackpressureState {
    pub config: BackpressureConfig,
    /// Per-shard: (matview_id, shard_id) → current output lag in snapshots.
    pub shard_lag: HashMap<(u64, u32), u64>,
    /// Per-shard: (matview_id, shard_id) → current input frontier.
    pub input_frontiers: HashMap<(u64, u32), u64>,
    /// Per-shard: (matview_id, shard_id) → current output frontier.
    pub output_frontiers: HashMap<(u64, u32), u64>,
}

/// Shard health status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShardHealth {
    /// Shard is operating normally.
    Healthy,
    /// Shard is stalled due to backpressure.
    Stalled { lag: u64 },
    /// Shard is skewed (lag exceeds threshold × median).
    Skewed { lag: u64, median_lag: u64 },
}

impl BackpressureState {
    /// Create a new backpressure state tracker.
    pub fn new(config: BackpressureConfig) -> Self {
        Self {
            config,
            shard_lag: HashMap::new(),
            input_frontiers: HashMap::new(),
            output_frontiers: HashMap::new(),
        }
    }

    /// Update the lag for a shard.
    pub fn update_lag(&mut self, matview_id: u64, shard_id: u32, input: u64, output: u64) {
        let lag = input.saturating_sub(output);
        self.shard_lag.insert((matview_id, shard_id), lag);
        self.input_frontiers.insert((matview_id, shard_id), input);
        self.output_frontiers.insert((matview_id, shard_id), output);
    }

    /// Check if a shard should stall ingest due to backpressure.
    pub fn should_stall(&self, matview_id: u64, shard_id: u32) -> bool {
        self.shard_lag
            .get(&(matview_id, shard_id))
            .map(|&lag| lag >= self.config.max_lag_snapshots)
            .unwrap_or(false)
    }

    /// Get the health status of a shard.
    pub fn shard_health(&self, matview_id: u64, shard_id: u32) -> ShardHealth {
        let lag = self
            .shard_lag
            .get(&(matview_id, shard_id))
            .copied()
            .unwrap_or(0);

        if lag >= self.config.max_lag_snapshots {
            return ShardHealth::Stalled { lag };
        }

        let median = self.median_lag();
        if median > 0 && lag as f64 > median as f64 * self.config.skew_threshold_multiplier {
            return ShardHealth::Skewed {
                lag,
                median_lag: median,
            };
        }

        ShardHealth::Healthy
    }

    /// Compute median lag across all tracked shards.
    pub fn median_lag(&self) -> u64 {
        let mut lags: Vec<u64> = self.shard_lag.values().copied().collect();
        if lags.is_empty() {
            return 0;
        }
        lags.sort();
        lags[lags.len() / 2]
    }

    /// Detect skewed shards (lag > threshold × median).
    pub fn detect_skewed_shards(&self) -> Vec<((u64, u32), u64)> {
        let median = self.median_lag();
        if median == 0 {
            return Vec::new();
        }
        let threshold = (median as f64 * self.config.skew_threshold_multiplier) as u64;
        self.shard_lag
            .iter()
            .filter(|(_, &lag)| lag > threshold)
            .map(|(&key, &lag)| (key, lag))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backpressure_stalls_at_threshold() {
        let mut state = BackpressureState::new(BackpressureConfig::default());
        state.update_lag(1, 0, 50, 50);
        assert!(!state.should_stall(1, 0));

        state.update_lag(1, 0, 200, 50);
        assert!(state.should_stall(1, 0));
    }

    #[test]
    fn skewed_shard_detection() {
        let mut state = BackpressureState::new(BackpressureConfig::default());
        // Normal shards.
        state.update_lag(1, 0, 20, 10);
        state.update_lag(1, 1, 20, 10);
        state.update_lag(1, 2, 20, 10);
        // Skewed shard.
        state.update_lag(1, 3, 100, 10);

        let skewed = state.detect_skewed_shards();
        assert_eq!(skewed.len(), 1);
        assert_eq!(skewed[0].0, (1, 3));
    }

    #[test]
    fn shard_health_states() {
        let mut state = BackpressureState::new(BackpressureConfig::default());
        state.update_lag(1, 0, 10, 10);
        assert_eq!(state.shard_health(1, 0), ShardHealth::Healthy);

        state.update_lag(1, 1, 200, 0);
        assert!(matches!(
            state.shard_health(1, 1),
            ShardHealth::Stalled { .. }
        ));
    }
}
