//! DuckDbHarness: reference SQL execution engine for correctness comparisons.
//!
//! `DuckDbHarness` provides a lightweight in-process reference implementation
//! that computes the expected result of GROUP BY aggregations over in-memory
//! rows, used to validate SQL correctness against a ground-truth batch result.
//!
//! ## Design
//! Rather than spawning an external DuckDB process (which would require a
//! binary or Docker image in CI), this harness implements the same GROUP BY +
//! aggregate semantics in pure Rust.  The result is deterministic and
//! comparable across platforms.
//!
//! ## Supported operations
//! - `run_group_by_count(rows, group_cols)` — COUNT(*) per group
//! - `run_group_by_sum(rows, group_cols, sum_col)` — SUM per group
//! - `assert_result_sets_equal` — order-independent comparison helper
//! - `join_rows` — cross product / equality join

use std::collections::HashMap;

use serde_json::Value;

/// Reference SQL execution harness for correctness assertions.
pub struct DuckDbHarness;

impl DuckDbHarness {
    /// Compute `SELECT <group_cols>, COUNT(*) AS cnt FROM rows GROUP BY <group_cols>`.
    ///
    /// Returns one row per distinct group with a `cnt` column.
    pub fn run_group_by_count(
        rows: &[HashMap<String, Value>],
        group_cols: &[&str],
    ) -> Vec<HashMap<String, Value>> {
        let mut counts: HashMap<String, i64> = HashMap::new();
        let mut keys_map: HashMap<String, HashMap<String, Value>> = HashMap::new();

        for row in rows {
            let key = group_key(row, group_cols);
            *counts.entry(key.clone()).or_insert(0) += 1;
            keys_map.entry(key).or_insert_with(|| {
                group_cols
                    .iter()
                    .map(|&c| (c.to_string(), row.get(c).cloned().unwrap_or(Value::Null)))
                    .collect()
            });
        }

        let mut result: Vec<HashMap<String, Value>> = counts
            .into_iter()
            .map(|(key, cnt)| {
                let mut row = keys_map.remove(&key).unwrap_or_default();
                row.insert("cnt".to_string(), Value::Number(cnt.into()));
                row
            })
            .collect();
        result.sort_by_key(|r| serde_json::to_string(r).unwrap_or_default());
        result
    }

    /// Compute `SELECT <group_cols>, SUM(<sum_col>) AS total FROM rows GROUP BY <group_cols>`.
    pub fn run_group_by_sum(
        rows: &[HashMap<String, Value>],
        group_cols: &[&str],
        sum_col: &str,
    ) -> Vec<HashMap<String, Value>> {
        let mut sums: HashMap<String, i64> = HashMap::new();
        let mut keys_map: HashMap<String, HashMap<String, Value>> = HashMap::new();

        for row in rows {
            let key = group_key(row, group_cols);
            let v = row
                .get(sum_col)
                .and_then(|v| match v {
                    Value::Number(n) => n.as_i64(),
                    _ => None,
                })
                .unwrap_or(0);
            *sums.entry(key.clone()).or_insert(0) += v;
            keys_map.entry(key).or_insert_with(|| {
                group_cols
                    .iter()
                    .map(|&c| (c.to_string(), row.get(c).cloned().unwrap_or(Value::Null)))
                    .collect()
            });
        }

        let mut result: Vec<HashMap<String, Value>> = sums
            .into_iter()
            .map(|(key, total)| {
                let mut row = keys_map.remove(&key).unwrap_or_default();
                row.insert("total".to_string(), Value::Number(total.into()));
                row
            })
            .collect();
        result.sort_by_key(|r| serde_json::to_string(r).unwrap_or_default());
        result
    }

    /// Perform an equality join: return all merged rows where `left[left_col] == right[right_col]`.
    pub fn join_rows(
        left: &[HashMap<String, Value>],
        right: &[HashMap<String, Value>],
        left_col: &str,
        right_col: &str,
    ) -> Vec<HashMap<String, Value>> {
        let mut result = Vec::new();
        for l in left {
            for r in right {
                if l.get(left_col) == r.get(right_col) {
                    let mut merged = r.clone();
                    merged.extend(l.iter().map(|(k, v)| (k.clone(), v.clone())));
                    result.push(merged);
                }
            }
        }
        result
    }

    /// Assert that two result sets are equal, ignoring row order.
    ///
    /// Panics with a descriptive message if they differ.
    pub fn assert_result_sets_equal(
        actual: &[HashMap<String, Value>],
        expected: &[HashMap<String, Value>],
        key_cols: &[&str],
        compare_col: &str,
        msg: &str,
    ) {
        fn index(
            rows: &[HashMap<String, Value>],
            key_cols: &[&str],
        ) -> HashMap<String, HashMap<String, Value>> {
            rows.iter()
                .map(|r| {
                    let k = group_key(r, key_cols);
                    (k, r.clone())
                })
                .collect()
        }

        let act_idx = index(actual, key_cols);
        let exp_idx = index(expected, key_cols);

        for (k, exp_row) in &exp_idx {
            let act_row = act_idx.get(k).unwrap_or_else(|| {
                panic!(
                    "{msg}: key {k:?} present in expected but missing from actual.\n\
                     expected={expected:?}\n\
                     actual={actual:?}"
                )
            });
            let act_val = act_row.get(compare_col);
            let exp_val = exp_row.get(compare_col);
            assert_eq!(
                act_val, exp_val,
                "{msg}: mismatch for key {k:?} on column {compare_col:?}: \
                 actual={act_val:?} expected={exp_val:?}"
            );
        }
        assert_eq!(
            act_idx.len(),
            exp_idx.len(),
            "{msg}: actual has {} keys, expected has {}.\nactual={actual:?}\nexpected={expected:?}",
            act_idx.len(),
            exp_idx.len()
        );
    }
}

/// Serialise a group key from the given columns of a row.
fn group_key(row: &HashMap<String, Value>, cols: &[&str]) -> String {
    let vals: Vec<Value> = cols
        .iter()
        .map(|&c| row.get(c).cloned().unwrap_or(Value::Null))
        .collect();
    serde_json::to_string(&vals).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_by_count_basic() {
        let rows: Vec<HashMap<String, Value>> = vec![
            [("region".into(), Value::String("us".into()))]
                .into_iter()
                .collect(),
            [("region".into(), Value::String("us".into()))]
                .into_iter()
                .collect(),
            [("region".into(), Value::String("eu".into()))]
                .into_iter()
                .collect(),
        ];
        let result = DuckDbHarness::run_group_by_count(&rows, &["region"]);
        let us = result
            .iter()
            .find(|r| r["region"] == Value::String("us".into()))
            .unwrap();
        assert_eq!(us["cnt"], Value::Number(2.into()));
    }

    #[test]
    fn group_by_sum_basic() {
        let rows: Vec<HashMap<String, Value>> = vec![
            [
                ("dept".into(), Value::String("eng".into())),
                ("amount".into(), Value::Number(100.into())),
            ]
            .into_iter()
            .collect(),
            [
                ("dept".into(), Value::String("eng".into())),
                ("amount".into(), Value::Number(200.into())),
            ]
            .into_iter()
            .collect(),
        ];
        let result = DuckDbHarness::run_group_by_sum(&rows, &["dept"], "amount");
        assert_eq!(result[0]["total"], Value::Number(300.into()));
    }

    #[test]
    fn join_rows_basic() {
        let events: Vec<HashMap<String, Value>> = vec![[
            ("cat_id".into(), Value::Number(1.into())),
            ("name".into(), Value::String("E1".into())),
        ]
        .into_iter()
        .collect()];
        let cats: Vec<HashMap<String, Value>> = vec![[
            ("cat_id".into(), Value::Number(1.into())),
            ("label".into(), Value::String("Sports".into())),
        ]
        .into_iter()
        .collect()];
        let joined = DuckDbHarness::join_rows(&events, &cats, "cat_id", "cat_id");
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0]["label"], Value::String("Sports".into()));
    }

    #[test]
    fn assert_result_sets_equal_passes() {
        let a: Vec<HashMap<String, Value>> = vec![[
            ("region".into(), Value::String("us".into())),
            ("cnt".into(), Value::Number(2.into())),
        ]
        .into_iter()
        .collect()];
        let b = a.clone();
        DuckDbHarness::assert_result_sets_equal(&a, &b, &["region"], "cnt", "should match");
    }
}
