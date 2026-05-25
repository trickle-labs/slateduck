//! v0.13: IVM join strategies — broadcast, co-partitioned, re-shuffle.
//!
//! Three join strategies are supported for incremental materialized views:
//!
//! - **Broadcast**: the "small" dimension table is fully replicated to every
//!   shard.  Incremental updates from the streaming side are joined locally
//!   against the in-memory replica.  Selected when the right-side estimated
//!   row count is below `broadcast_threshold` (default 1 000 000 rows).
//!
//! - **CoPartitioned**: both input tables are sharded on the same column.
//!   Each shard holds matching key ranges for both sides; the join is
//!   entirely local (no cross-shard communication).  Selected when the
//!   join predicate columns match both sides' `shard_key`.
//!
//! - **Reshuffle**: one side is repartitioned through a temporary SlateDB
//!   exchange region keyed by the join column.  Readers on the other side
//!   pull the matching key range.  This is the most general strategy and
//!   the most expensive (one extra round-trip per join input per tick).
//!
//! ## Delete propagation
//!
//! A row with `weight = -1` (a retraction / delete) propagates through
//! all three join strategies.  `hash_join_batch` accepts a signed weight
//! and emits `(merged_row, weight)` pairs; negative-weight tuples are
//! retracted from the downstream circuit.

use std::collections::HashMap;

use serde_json::Value;

/// The default broadcast threshold in estimated row count.
pub const DEFAULT_BROADCAST_THRESHOLD: u64 = 1_000_000;

// ─── Join strategy ─────────────────────────────────────────────────────────

/// Join strategy selected at view creation time.
///
/// Can be overridden per view via `WITH (join_strategy = '...')`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinStrategy {
    /// Small dimension side is broadcast to every shard.
    Broadcast,
    /// Both inputs share the same shard key; join is local.
    CoPartitioned,
    /// One side is repartitioned through a temporary exchange region.
    Reshuffle,
}

impl std::fmt::Display for JoinStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JoinStrategy::Broadcast => write!(f, "broadcast"),
            JoinStrategy::CoPartitioned => write!(f, "co_partition"),
            JoinStrategy::Reshuffle => write!(f, "reshuffle"),
        }
    }
}

impl std::str::FromStr for JoinStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "broadcast" => Ok(JoinStrategy::Broadcast),
            "co_partition" | "copartition" | "co-partition" => Ok(JoinStrategy::CoPartitioned),
            "reshuffle" => Ok(JoinStrategy::Reshuffle),
            _ => Err(format!("unknown join strategy: {s}")),
        }
    }
}

// ─── Join clause ───────────────────────────────────────────────────────────

/// A single JOIN clause extracted from the view SQL.
#[derive(Debug, Clone)]
pub struct JoinClause {
    /// Left (streaming / fact) table name.
    pub left_table: String,
    /// Right table name.
    pub right_table: String,
    /// Left join predicate column.
    pub left_col: String,
    /// Right join predicate column.
    pub right_col: String,
    /// Selected join strategy.
    pub strategy: JoinStrategy,
    /// Broadcast threshold used during strategy selection.
    pub broadcast_threshold: u64,
}

// ─── Hash-join state ───────────────────────────────────────────────────────

/// Per-shard hash-join state.  Used by all three strategies; the broadcast
/// and co-partitioned cases pre-load one side before streaming the other.
#[derive(Debug, Clone, Default)]
pub struct HashJoinState {
    /// join_key_serialised → list of right-side rows.
    pub right_index: HashMap<String, Vec<HashMap<String, Value>>>,
}

impl HashJoinState {
    /// Create an empty hash-join state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a right-side row keyed by `key`.
    pub fn insert_right(&mut self, key: &str, row: HashMap<String, Value>) {
        self.right_index
            .entry(key.to_string())
            .or_default()
            .push(row);
    }

    /// Retract a right-side row (delete propagation, weight = -1).
    pub fn retract_right(&mut self, key: &str, row: &HashMap<String, Value>) {
        if let Some(bucket) = self.right_index.get_mut(key) {
            if let Some(pos) = bucket.iter().position(|r| r == row) {
                bucket.swap_remove(pos);
            }
            if bucket.is_empty() {
                self.right_index.remove(key);
            }
        }
    }

    /// Probe: return all right-side rows matching `key`.
    pub fn probe(&self, key: &str) -> &[HashMap<String, Value>] {
        self.right_index
            .get(key)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Number of distinct keys in the right index.
    pub fn key_count(&self) -> usize {
        self.right_index.len()
    }

    /// Total number of rows across all buckets.
    pub fn row_count(&self) -> usize {
        self.right_index.values().map(|v| v.len()).sum()
    }
}

// ─── Strategy selection ────────────────────────────────────────────────────

/// Select a join strategy given runtime hints.
///
/// Decision tree:
/// 1. `right_estimated_rows < broadcast_threshold` → `Broadcast`
/// 2. `left_shard_key == join_col_left && right_shard_key == join_col_right` → `CoPartitioned`
/// 3. Otherwise → `Reshuffle`
pub fn select_strategy(
    right_estimated_rows: u64,
    broadcast_threshold: u64,
    left_shard_key: Option<&str>,
    right_shard_key: Option<&str>,
    join_col_left: &str,
    join_col_right: &str,
) -> JoinStrategy {
    if right_estimated_rows < broadcast_threshold {
        return JoinStrategy::Broadcast;
    }
    let left_ok = left_shard_key.map(|k| k == join_col_left).unwrap_or(false);
    let right_ok = right_shard_key
        .map(|k| k == join_col_right)
        .unwrap_or(false);
    if left_ok && right_ok {
        return JoinStrategy::CoPartitioned;
    }
    JoinStrategy::Reshuffle
}

// ─── Core join operation ───────────────────────────────────────────────────

/// Perform an incremental hash join.
///
/// Emits `(merged_row, weight)` pairs where `weight` is the product of the
/// left-row weight and 1 (right side is assumed stable during this batch for
/// broadcast / co-partitioned strategies).
///
/// For delete propagation, pass `weight = -1`; the emitted tuples will also
/// carry `weight = -1` and will be retracted by the downstream circuit.
pub fn hash_join_batch(
    left_batch: &[(HashMap<String, Value>, i64)],
    right_state: &HashJoinState,
    left_col: &str,
) -> Vec<(HashMap<String, Value>, i64)> {
    let mut out = Vec::new();
    for (left_row, weight) in left_batch {
        let key = match left_row.get(left_col) {
            Some(v) => serde_json::to_string(v).unwrap_or_default(),
            None => continue,
        };
        for right_row in right_state.probe(&key) {
            // Merge: left columns take precedence on name conflict.
            let mut merged: HashMap<String, Value> = right_row.clone();
            merged.extend(left_row.iter().map(|(k, v)| (k.clone(), v.clone())));
            out.push((merged, *weight));
        }
    }
    out
}

/// Load all right-side rows into a `HashJoinState` with `right_col` as the key.
pub fn build_right_side(rows: &[HashMap<String, Value>], right_col: &str) -> HashJoinState {
    let mut state = HashJoinState::new();
    for row in rows {
        let key = match row.get(right_col) {
            Some(v) => serde_json::to_string(v).unwrap_or_default(),
            None => continue,
        };
        state.insert_right(&key, row.clone());
    }
    state
}

// ─── Exchange region (reshuffle stub) ─────────────────────────────────────

/// Row weight pair used throughout the join engine.
type RowWeight = (HashMap<String, Value>, i64);

/// Intermediate exchange buffer for the reshuffle strategy.
///
/// In a full distributed deployment this would write key-range partitions
/// to a temporary SlateDB region.  In the current in-process implementation
/// the exchange is an in-memory buffer that partitions rows by a hash of the
/// join key, enabling the reader side to pull only matching rows.
#[derive(Debug, Clone, Default)]
pub struct ExchangeBuffer {
    /// shard_index → list of (row, weight) pairs destined for that shard.
    pub buckets: HashMap<u32, Vec<RowWeight>>,
}

impl ExchangeBuffer {
    /// Create an empty exchange buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Partition `rows` into shards using the given shard count and join column.
    pub fn partition(
        &mut self,
        rows: &[(HashMap<String, Value>, i64)],
        join_col: &str,
        shard_count: u32,
    ) {
        for (row, weight) in rows {
            let key = match row.get(join_col) {
                Some(v) => serde_json::to_string(v).unwrap_or_default(),
                None => continue,
            };
            let shard = crate::shard_key::hash_key_value(&key) as u32 % shard_count;
            self.buckets
                .entry(shard)
                .or_default()
                .push((row.clone(), *weight));
        }
    }

    /// Drain the rows destined for `shard_id`.
    pub fn drain_shard(&mut self, shard_id: u32) -> Vec<(HashMap<String, Value>, i64)> {
        self.buckets.remove(&shard_id).unwrap_or_default()
    }
}

// ─── Unit tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_and_parse_roundtrip() {
        for s in [
            JoinStrategy::Broadcast,
            JoinStrategy::CoPartitioned,
            JoinStrategy::Reshuffle,
        ] {
            let rendered = s.to_string();
            let parsed: JoinStrategy = rendered.parse().unwrap();
            assert_eq!(s, parsed);
        }
    }

    #[test]
    fn strategy_selection_broadcast() {
        let s = select_strategy(100, DEFAULT_BROADCAST_THRESHOLD, None, None, "a", "a");
        assert_eq!(s, JoinStrategy::Broadcast);
    }

    #[test]
    fn strategy_selection_copartitioned() {
        let s = select_strategy(
            2_000_000,
            DEFAULT_BROADCAST_THRESHOLD,
            Some("k"),
            Some("k"),
            "k",
            "k",
        );
        assert_eq!(s, JoinStrategy::CoPartitioned);
    }

    #[test]
    fn strategy_selection_reshuffle() {
        // left shard key "id", but join col is "region" — mismatch → Reshuffle
        let s = select_strategy(
            2_000_000,
            DEFAULT_BROADCAST_THRESHOLD,
            Some("id"),
            Some("id"),
            "region",
            "region",
        );
        assert_eq!(s, JoinStrategy::Reshuffle);
    }

    #[test]
    fn hash_join_basic() {
        let mut right = HashJoinState::new();
        let cat_row: HashMap<String, Value> = [
            ("cat_id".into(), Value::Number(1.into())),
            ("cat_name".into(), Value::String("Sports".into())),
        ]
        .into_iter()
        .collect();
        right.insert_right("1", cat_row);

        let event_row: HashMap<String, Value> = [
            ("cat_id".into(), Value::Number(1.into())),
            ("event_name".into(), Value::String("Marathon".into())),
        ]
        .into_iter()
        .collect();

        let result = hash_join_batch(&[(event_row, 1)], &right, "cat_id");
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].0.get("cat_name"),
            Some(&Value::String("Sports".into()))
        );
        assert_eq!(result[0].1, 1);
    }

    #[test]
    fn hash_join_delete_propagation() {
        let mut right = HashJoinState::new();
        let row: HashMap<String, Value> = [("id".into(), Value::Number(1.into()))]
            .into_iter()
            .collect();
        right.insert_right("1", row.clone());

        let left: HashMap<String, Value> = [
            ("id".into(), Value::Number(1.into())),
            ("val".into(), Value::Number(42.into())),
        ]
        .into_iter()
        .collect();

        let result = hash_join_batch(&[(left, -1)], &right, "id");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, -1, "delete weight must propagate");
    }

    #[test]
    fn hash_join_retract_right() {
        let mut right = HashJoinState::new();
        let row: HashMap<String, Value> = [("id".into(), Value::Number(1.into()))]
            .into_iter()
            .collect();
        right.insert_right("1", row.clone());
        right.retract_right("1", &row);
        assert!(right.right_index.is_empty());
    }

    #[test]
    fn exchange_buffer_partition_and_drain() {
        let mut buf = ExchangeBuffer::new();
        let rows: Vec<(HashMap<String, Value>, i64)> = (0u32..8)
            .map(|i| {
                let r: HashMap<String, Value> =
                    [("k".into(), Value::Number(serde_json::Number::from(i)))]
                        .into_iter()
                        .collect();
                (r, 1)
            })
            .collect();
        buf.partition(&rows, "k", 4);
        // All 8 rows should appear exactly once across all shards.
        let total: usize = (0..4).map(|s| buf.drain_shard(s).len()).sum();
        assert_eq!(total, 8);
    }
}
