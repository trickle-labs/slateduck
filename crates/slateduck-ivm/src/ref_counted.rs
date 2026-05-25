//! Reference-counted DISTINCT and set operators for IVM correctness.
//!
//! The base DISTINCT implementation does not track duplicate counts, producing
//! incorrect output when the same row is inserted multiple times and then
//! partially deleted.
//!
//! Solution: `__sd_ref_count: i64` auxiliary column. INSERT increments; DELETE
//! decrements; row visible in output only when `ref_count > 0`.
//!
//! ## Set operator semantics
//! - `UNION DISTINCT`: `MAX(count_A, count_B)` — present if in *either* operand
//! - `INTERSECT`: `MIN(count_A, count_B)` — present only when both contribute
//! - `EXCEPT`: `count_A - count_B`, clamped to 0

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Reference count entry for a distinct row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefCount {
    /// Current reference count.
    pub count: i64,
}

impl RefCount {
    pub fn new(count: i64) -> Self {
        Self { count }
    }

    /// Is this row visible in the output?
    pub fn is_visible(&self) -> bool {
        self.count > 0
    }
}

/// Reference-counted DISTINCT state.
#[derive(Debug, Clone, Default)]
pub struct RefCountedDistinct {
    /// Row hash → reference count.
    pub counts: HashMap<Vec<u8>, RefCount>,
}

impl RefCountedDistinct {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a row (increment ref count).
    /// Returns (was_invisible, is_visible_now) — true if the row became visible.
    pub fn insert(&mut self, row_key: Vec<u8>) -> bool {
        let entry = self.counts.entry(row_key).or_insert(RefCount::new(0));
        let was_invisible = !entry.is_visible();
        entry.count += 1;
        was_invisible && entry.is_visible()
    }

    /// Delete a row (decrement ref count).
    /// Returns true if the row became invisible (should be removed from output).
    pub fn delete(&mut self, row_key: &[u8]) -> bool {
        let entry = self
            .counts
            .entry(row_key.to_vec())
            .or_insert(RefCount::new(0));
        let was_visible = entry.is_visible();
        entry.count -= 1;
        was_visible && !entry.is_visible()
    }

    /// Get the ref count for a row.
    pub fn get_count(&self, row_key: &[u8]) -> i64 {
        self.counts.get(row_key).map(|rc| rc.count).unwrap_or(0)
    }

    /// Check if a row is visible in the output.
    pub fn is_visible(&self, row_key: &[u8]) -> bool {
        self.counts
            .get(row_key)
            .map(|rc| rc.is_visible())
            .unwrap_or(false)
    }

    /// Get the number of visible rows.
    pub fn visible_count(&self) -> usize {
        self.counts.values().filter(|rc| rc.is_visible()).count()
    }

    /// Get all visible row keys.
    pub fn visible_rows(&self) -> Vec<&Vec<u8>> {
        self.counts
            .iter()
            .filter(|(_, rc)| rc.is_visible())
            .map(|(k, _)| k)
            .collect()
    }
}

/// Set operator kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SetOperator {
    /// `UNION DISTINCT`: MAX(count_A, count_B)
    UnionDistinct,
    /// `INTERSECT`: MIN(count_A, count_B)
    Intersect,
    /// `EXCEPT`: count_A - count_B, clamped to 0
    Except,
}

/// Reference-counted set operator state (two-input).
#[derive(Debug, Clone, Default)]
pub struct RefCountedSetOp {
    /// Left operand counts: row_key → count.
    pub left_counts: HashMap<Vec<u8>, i64>,
    /// Right operand counts: row_key → count.
    pub right_counts: HashMap<Vec<u8>, i64>,
}

impl RefCountedSetOp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert into left operand.
    pub fn insert_left(&mut self, row_key: Vec<u8>) {
        *self.left_counts.entry(row_key).or_insert(0) += 1;
    }

    /// Delete from left operand.
    pub fn delete_left(&mut self, row_key: &[u8]) {
        if let Some(count) = self.left_counts.get_mut(row_key) {
            *count -= 1;
            if *count <= 0 {
                self.left_counts.remove(row_key);
            }
        }
    }

    /// Insert into right operand.
    pub fn insert_right(&mut self, row_key: Vec<u8>) {
        *self.right_counts.entry(row_key).or_insert(0) += 1;
    }

    /// Delete from right operand.
    pub fn delete_right(&mut self, row_key: &[u8]) {
        if let Some(count) = self.right_counts.get_mut(row_key) {
            *count -= 1;
            if *count <= 0 {
                self.right_counts.remove(row_key);
            }
        }
    }

    /// Compute the output count for a row under the given set operator.
    pub fn output_count(&self, row_key: &[u8], operator: SetOperator) -> i64 {
        let left = self.left_counts.get(row_key).copied().unwrap_or(0);
        let right = self.right_counts.get(row_key).copied().unwrap_or(0);

        match operator {
            SetOperator::UnionDistinct => {
                // MAX(count_A, count_B): present if in *either*
                if left > 0 || right > 0 {
                    1
                } else {
                    0
                }
            }
            SetOperator::Intersect => {
                // MIN(count_A, count_B): present only when both contribute
                if left > 0 && right > 0 {
                    1
                } else {
                    0
                }
            }
            SetOperator::Except => {
                // count_A - count_B, clamped to 0
                let result = left - right;
                if result > 0 {
                    1
                } else {
                    0
                }
            }
        }
    }

    /// Check if a row is visible under the given operator.
    pub fn is_visible(&self, row_key: &[u8], operator: SetOperator) -> bool {
        self.output_count(row_key, operator) > 0
    }

    /// Get all visible rows under the given operator.
    pub fn visible_rows(&self, operator: SetOperator) -> Vec<Vec<u8>> {
        let all_keys: std::collections::HashSet<&Vec<u8>> = self
            .left_counts
            .keys()
            .chain(self.right_counts.keys())
            .collect();

        all_keys
            .into_iter()
            .filter(|k| self.is_visible(k, operator))
            .cloned()
            .collect()
    }
}

/// Adaptive cost mode configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveCostConfig {
    /// Threshold for switching DIFFERENTIAL → FULL.
    /// Switch when: Δ_rows / N_rows × multiplier > threshold
    pub threshold: f64,
    /// Query complexity multiplier table.
    pub multipliers: ComplexityMultipliers,
}

impl Default for AdaptiveCostConfig {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            multipliers: ComplexityMultipliers::default(),
        }
    }
}

/// Query complexity multiplier table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityMultipliers {
    pub scan: f64,
    pub filter: f64,
    pub aggregate: f64,
    pub join: f64,
    pub join_aggregate: f64,
    pub window: f64,
    pub recursive: f64,
}

impl Default for ComplexityMultipliers {
    fn default() -> Self {
        Self {
            scan: 1.0,
            filter: 1.1,
            aggregate: 1.5,
            join: 2.5,
            join_aggregate: 4.0,
            window: 3.0,
            recursive: 5.0,
        }
    }
}

/// Per-view rolling statistics for adaptive cost mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ViewRollingStats {
    pub rows_in: u64,
    pub rows_out: u64,
    pub ms_spent: u64,
    pub last_full_cost_ms: u64,
    /// Smoothed delta ratio.
    pub smoothed_delta_ratio: f64,
}

/// Determine if DIFFERENTIAL→FULL switch should occur.
pub fn should_switch_to_full(
    delta_rows: u64,
    total_rows: u64,
    multiplier: f64,
    threshold: f64,
) -> bool {
    if total_rows == 0 {
        return false;
    }
    let ratio = delta_rows as f64 / total_rows as f64;
    ratio * multiplier > threshold
}

/// Get the complexity multiplier for a query operator type.
pub fn get_multiplier(multipliers: &ComplexityMultipliers, operator: &str) -> f64 {
    match operator.to_lowercase().as_str() {
        "scan" => multipliers.scan,
        "filter" => multipliers.filter,
        "aggregate" | "group_by" => multipliers.aggregate,
        "join" => multipliers.join,
        "join_aggregate" => multipliers.join_aggregate,
        "window" => multipliers.window,
        "recursive" | "cte" => multipliers.recursive,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_counted_insert_delete_cycle() {
        let mut distinct = RefCountedDistinct::new();

        // Insert same row 3 times
        let key = b"row1".to_vec();
        assert!(distinct.insert(key.clone())); // becomes visible
        assert!(!distinct.insert(key.clone())); // already visible
        assert!(!distinct.insert(key.clone())); // already visible

        assert_eq!(distinct.get_count(&key), 3);
        assert!(distinct.is_visible(&key));

        // Delete 2 times — still visible
        assert!(!distinct.delete(&key));
        assert!(!distinct.delete(&key));
        assert_eq!(distinct.get_count(&key), 1);
        assert!(distinct.is_visible(&key));

        // Delete once more — becomes invisible
        assert!(distinct.delete(&key));
        assert_eq!(distinct.get_count(&key), 0);
        assert!(!distinct.is_visible(&key));
    }

    #[test]
    fn ref_counted_visible_count() {
        let mut distinct = RefCountedDistinct::new();
        distinct.insert(b"a".to_vec());
        distinct.insert(b"b".to_vec());
        distinct.insert(b"c".to_vec());
        distinct.insert(b"a".to_vec()); // duplicate

        assert_eq!(distinct.visible_count(), 3);
    }

    #[test]
    fn union_distinct_semantics() {
        let mut set_op = RefCountedSetOp::new();
        let key = b"shared_row".to_vec();

        // Insert into both sides
        set_op.insert_left(key.clone());
        set_op.insert_right(key.clone());

        // UNION DISTINCT: exactly one output row
        assert!(set_op.is_visible(&key, SetOperator::UnionDistinct));
        assert_eq!(set_op.output_count(&key, SetOperator::UnionDistinct), 1);

        // Remove from left — still visible (present in right)
        set_op.delete_left(&key);
        assert!(set_op.is_visible(&key, SetOperator::UnionDistinct));
    }

    #[test]
    fn intersect_semantics() {
        let mut set_op = RefCountedSetOp::new();
        let key = b"shared".to_vec();

        set_op.insert_left(key.clone());
        // Not in right yet → not visible under INTERSECT
        assert!(!set_op.is_visible(&key, SetOperator::Intersect));

        set_op.insert_right(key.clone());
        // Now in both → visible
        assert!(set_op.is_visible(&key, SetOperator::Intersect));

        set_op.delete_right(&key);
        // Removed from right → not visible
        assert!(!set_op.is_visible(&key, SetOperator::Intersect));
    }

    #[test]
    fn except_semantics() {
        let mut set_op = RefCountedSetOp::new();
        let key = b"row".to_vec();

        set_op.insert_left(key.clone());
        set_op.insert_left(key.clone());
        // 2 in left, 0 in right → visible (2 - 0 = 2 > 0)
        assert!(set_op.is_visible(&key, SetOperator::Except));

        set_op.insert_right(key.clone());
        set_op.insert_right(key.clone());
        // 2 in left, 2 in right → not visible (2 - 2 = 0)
        assert!(!set_op.is_visible(&key, SetOperator::Except));

        set_op.insert_left(key.clone());
        // 3 in left, 2 in right → visible (3 - 2 = 1 > 0)
        assert!(set_op.is_visible(&key, SetOperator::Except));
    }

    #[test]
    fn adaptive_cost_switch_decision() {
        let threshold = 0.5;

        // Low delta ratio: stay DIFFERENTIAL
        assert!(!should_switch_to_full(100, 10000, 1.5, threshold));

        // High delta ratio with join multiplier: switch to FULL
        assert!(should_switch_to_full(3000, 10000, 2.5, threshold));

        // Edge case: zero total rows
        assert!(!should_switch_to_full(100, 0, 1.0, threshold));
    }

    #[test]
    fn complexity_multiplier_lookup() {
        let m = ComplexityMultipliers::default();
        assert_eq!(get_multiplier(&m, "scan"), 1.0);
        assert_eq!(get_multiplier(&m, "join"), 2.5);
        assert_eq!(get_multiplier(&m, "window"), 3.0);
        assert_eq!(get_multiplier(&m, "recursive"), 5.0);
        assert_eq!(get_multiplier(&m, "unknown"), 1.0);
    }
}
