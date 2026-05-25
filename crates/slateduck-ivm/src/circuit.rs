//! DBSP compatibility shim: incremental GROUP BY engine.
//!
//! This module provides a lightweight adaptation of DBSP's Z-difference
//! (Zset) model for SlateDuck's append-only CDC stream.  As of v0.13 it also
//! provides `IvmJoinCircuit`, which layers a multi-input hash-join operator
//! in front of the GROUP BY aggregation.
//!
//! ## Model
//! A **Z-difference** (`ZDelta`) is a multiset where each element carries an
//! integer weight: +1 for an insert, -1 for a delete/retract.  Applying a
//! sequence of ZDeltas to a trace incrementally produces the same result as
//! recomputing from scratch over the full history.
//!
//! ## Why a shim instead of the full DBSP API?
//! The `dbsp` crate (Feldera 0.299.0) provides a rich streaming circuit API
//! that requires specifying the full dataflow graph at construction time.
//! For v0.11 SlateDuck uses append-only tables, so retraction is not needed.
//! This shim exposes the minimal subset needed: `push_batch` + `step`.
//!
//! The workspace-level `dbsp` dependency is preserved for future use when
//! full retraction support is added.
//!
//! ## v0.13 additions
//! `IvmJoinCircuit` adds:
//! - Per-input `HashJoinState` for broadcast / co-partitioned strategies.
//! - `ExchangeBuffer` for the reshuffle strategy.
//! - `push_right` / `push_right_delta` to load / update the "small" side.
//! - `push_left_batch` to stream new left-side rows and emit joined ZDeltas
//!   into the downstream `IvmCircuit`.

use std::collections::HashMap;

use serde_json::Value;

use crate::join::{hash_join_batch, HashJoinState, JoinStrategy};
use crate::plan::{AggregateKind, IvmPlan};

/// A row in the Z-difference stream.  Weight is always +1 in v0.11 (append-only).
#[derive(Debug, Clone)]
pub struct ZDelta {
    /// Column name → value mapping for this row.
    pub fields: HashMap<String, Value>,
    /// +1 = insert, -1 = delete/retract.
    pub weight: i64,
}

/// Incremental GROUP BY circuit over a single IVM plan.
///
/// Wraps an [`IvmPlan`] and maintains aggregate state as a
/// `HashMap<group_key, AggState>`.  Call `push_batch` with new ZDeltas, then
/// `read_output` to enumerate the current output.
pub struct IvmCircuit {
    plan: IvmPlan,
    /// Keyed by serialised group-by values (JSON array string).
    state: HashMap<String, AggState>,
}

/// Per-group aggregate state.
#[derive(Debug, Clone, Default)]
pub struct AggState {
    /// Aggregate index → i64 accumulator.
    ///
    /// For COUNT: sum of weights.
    /// For SUM: sum of (value * weight).
    /// For MIN/MAX: tracked separately (see MinMaxState).
    pub accumulators: Vec<i64>,
    /// For MIN/MAX aggregates we keep a sorted multiset to handle retractions.
    pub minmax: Vec<MinMaxState>,
    /// Total row count (for COUNT(*) shortcut).
    pub row_count: i64,
}

/// Sorted multiset for MIN/MAX with retraction support.
/// Keys are stored as i64 (cast from f64) for BTreeMap ordering.
/// For v0.11 this handles integer-valued aggregates correctly;
/// full IEEE-754 ordering is a v0.12 concern.
#[derive(Debug, Clone, Default)]
pub struct MinMaxState {
    counts: std::collections::BTreeMap<i64, i64>,
}

impl MinMaxState {
    fn f64_to_key(v: f64) -> i64 {
        // Use IEEE 754 bit-level total order: flip sign bit for positives,
        // flip all bits for negatives so BTreeMap gives correct numeric order.
        let bits = v.to_bits() as i64;
        if bits >= 0 {
            bits
        } else {
            !bits
        }
    }
    fn key_to_f64(k: i64) -> f64 {
        let bits = if k >= 0 { k as u64 } else { !k as u64 };
        f64::from_bits(bits)
    }
    fn add(&mut self, v: f64, weight: i64) {
        let key = Self::f64_to_key(v);
        let e = self.counts.entry(key).or_insert(0);
        *e += weight;
        if *e == 0 {
            self.counts.remove(&key);
        }
    }
    fn min(&self) -> Option<f64> {
        self.counts.keys().next().map(|&k| Self::key_to_f64(k))
    }
    fn max(&self) -> Option<f64> {
        self.counts.keys().next_back().map(|&k| Self::key_to_f64(k))
    }
}

impl IvmCircuit {
    /// Create a new circuit for the given plan.
    pub fn new(plan: IvmPlan) -> Self {
        Self {
            plan,
            state: HashMap::new(),
        }
    }

    /// Process a batch of ZDeltas and update internal aggregate state.
    pub fn push_batch(&mut self, batch: &[ZDelta]) {
        for delta in batch {
            let key = self.group_key(&delta.fields);
            let n_aggs = self.plan.aggregates.len();
            let entry = self.state.entry(key).or_insert_with(|| AggState {
                accumulators: vec![0i64; n_aggs],
                minmax: vec![MinMaxState::default(); n_aggs],
                row_count: 0,
            });

            entry.row_count += delta.weight;

            for (i, agg) in self.plan.aggregates.iter().enumerate() {
                match &agg.kind {
                    AggregateKind::Count => {
                        entry.accumulators[i] += delta.weight;
                    }
                    AggregateKind::Sum => {
                        if let Some(col) = &agg.input_col {
                            let v = value_to_f64(delta.fields.get(col));
                            entry.accumulators[i] += (v * delta.weight as f64) as i64;
                        }
                    }
                    AggregateKind::Min => {
                        if let Some(col) = &agg.input_col {
                            let v = value_to_f64(delta.fields.get(col));
                            entry.minmax[i].add(v, delta.weight);
                        }
                    }
                    AggregateKind::Max => {
                        if let Some(col) = &agg.input_col {
                            let v = value_to_f64(delta.fields.get(col));
                            entry.minmax[i].add(v, delta.weight);
                        }
                    }
                }
            }
        }
        // Remove zero-count groups.
        self.state.retain(|_, s| s.row_count > 0);
    }

    /// Read the current output as a list of (group_key_fields, agg_values) pairs.
    pub fn read_output(&self) -> Vec<HashMap<String, Value>> {
        let mut result = Vec::with_capacity(self.state.len());
        for (key_str, state) in &self.state {
            let group_vals: Vec<Value> = serde_json::from_str(key_str).unwrap_or_default();
            let mut row = HashMap::new();
            for (i, col) in self.plan.group_by_cols.iter().enumerate() {
                row.insert(
                    col.clone(),
                    group_vals.get(i).cloned().unwrap_or(Value::Null),
                );
            }
            for (i, agg) in self.plan.aggregates.iter().enumerate() {
                let v = match &agg.kind {
                    AggregateKind::Count | AggregateKind::Sum => {
                        Value::Number(serde_json::Number::from(state.accumulators[i]))
                    }
                    AggregateKind::Min => {
                        state.minmax[i].min().map(json_f64).unwrap_or(Value::Null)
                    }
                    AggregateKind::Max => {
                        state.minmax[i].max().map(json_f64).unwrap_or(Value::Null)
                    }
                };
                row.insert(agg.output_col.clone(), v);
            }
            result.push(row);
        }
        result
    }

    /// Return the current number of output groups.
    pub fn group_count(&self) -> usize {
        self.state.len()
    }

    // ─── Helpers ───────────────────────────────────────────────────────────

    fn group_key(&self, fields: &HashMap<String, Value>) -> String {
        let vals: Vec<Value> = self
            .plan
            .group_by_cols
            .iter()
            .map(|c| fields.get(c).cloned().unwrap_or(Value::Null))
            .collect();
        serde_json::to_string(&vals).unwrap_or_default()
    }
}

fn value_to_f64(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.parse().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn json_f64(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

// ─── v0.13: IVM join circuit ───────────────────────────────────────────────

/// Multi-input incremental join circuit.
///
/// Wraps an [`IvmCircuit`] (GROUP BY + aggregation) with a hash-join layer
/// that implements the three v0.13 join strategies:
///
/// | Strategy        | Right side           | Left side               |
/// |-----------------|----------------------|-------------------------|
/// | Broadcast       | fully replicated     | streaming delta         |
/// | CoPartitioned   | local key range      | local key range (same)  |
/// | Reshuffle       | exchange buffer      | streaming delta         |
///
/// # Usage
/// 1. Call `load_right_side` once with all rows from the "small" / dimension
///    side (or call `push_right_delta` for incremental updates).
/// 2. Call `push_left_batch` on each new batch of left-side rows.
/// 3. Read the aggregated output from `inner.read_output()`.
pub struct IvmJoinCircuit {
    /// Downstream GROUP BY + aggregation circuit.
    pub inner: IvmCircuit,
    /// Hash-join state for each join clause (indexed by join position).
    pub join_states: Vec<HashJoinState>,
    /// Join strategies for each clause.
    pub strategies: Vec<JoinStrategy>,
    /// Left join key column for each clause.
    pub left_cols: Vec<String>,
}

impl IvmJoinCircuit {
    /// Create a new join circuit for the given plan.
    ///
    /// `strategies` and `left_cols` must have the same length as `plan.joins`.
    pub fn new(plan: IvmPlan, strategies: Vec<JoinStrategy>, left_cols: Vec<String>) -> Self {
        let n = strategies.len();
        Self {
            inner: IvmCircuit::new(plan),
            join_states: vec![HashJoinState::new(); n],
            strategies,
            left_cols,
        }
    }

    /// Load (or replace) the complete right side for join `idx`.
    ///
    /// Used for the initial broadcast load and for co-partitioned views where
    /// the right side is small enough to fit in memory.
    pub fn load_right_side(
        &mut self,
        idx: usize,
        rows: &[HashMap<String, Value>],
        right_col: &str,
    ) {
        self.join_states[idx] = crate::join::build_right_side(rows, right_col);
    }

    /// Apply an incremental update to the right side for join `idx`.
    ///
    /// `weight = +1` for inserts, `-1` for deletes.
    pub fn push_right_delta(
        &mut self,
        idx: usize,
        row: HashMap<String, Value>,
        right_col: &str,
        weight: i64,
    ) {
        let key = match row.get(right_col) {
            Some(v) => serde_json::to_string(v).unwrap_or_default(),
            None => return,
        };
        if weight >= 1 {
            self.join_states[idx].insert_right(&key, row);
        } else {
            self.join_states[idx].retract_right(&key, &row);
        }
    }

    /// Stream a batch of left-side rows through the join pipeline and into
    /// the aggregation circuit.
    ///
    /// Returns the number of joined rows fed to `inner.push_batch`.
    pub fn push_left_batch(&mut self, rows: &[(HashMap<String, Value>, i64)]) -> usize {
        if self.join_states.is_empty() {
            // No join clauses — fall through directly.
            let deltas: Vec<ZDelta> = rows
                .iter()
                .map(|(f, w)| ZDelta {
                    fields: f.clone(),
                    weight: *w,
                })
                .collect();
            self.inner.push_batch(&deltas);
            return deltas.len();
        }

        // Apply each join in sequence.  The first join takes the raw input;
        // each subsequent join takes the output of the previous one.
        let mut current: Vec<(HashMap<String, Value>, i64)> = rows.to_vec();
        for (i, state) in self.join_states.iter().enumerate() {
            let left_col = self.left_cols.get(i).map(|s| s.as_str()).unwrap_or("");
            current = hash_join_batch(&current, state, left_col);
        }

        let n = current.len();
        let deltas: Vec<ZDelta> = current
            .into_iter()
            .map(|(f, w)| ZDelta {
                fields: f,
                weight: w,
            })
            .collect();
        self.inner.push_batch(&deltas);
        n
    }

    /// Return the current output from the aggregation circuit.
    pub fn read_output(&self) -> Vec<HashMap<String, Value>> {
        self.inner.read_output()
    }

    /// Return the current group count.
    pub fn group_count(&self) -> usize {
        self.inner.group_count()
    }
}

// ─── Unit tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::IvmPlan;

    fn make_delta(fields: &[(&str, Value)], weight: i64) -> ZDelta {
        ZDelta {
            fields: fields
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
            weight,
        }
    }

    #[test]
    fn count_star_incremental() {
        let plan =
            IvmPlan::parse("SELECT region, COUNT(*) AS cnt FROM sales GROUP BY region").unwrap();
        let mut c = IvmCircuit::new(plan);

        c.push_batch(&[
            make_delta(&[("region", Value::String("us".into()))], 1),
            make_delta(&[("region", Value::String("us".into()))], 1),
            make_delta(&[("region", Value::String("eu".into()))], 1),
        ]);

        let out = c.read_output();
        let us = out
            .iter()
            .find(|r| r["region"] == Value::String("us".into()))
            .unwrap();
        assert_eq!(us["cnt"], Value::Number(2.into()));
        let eu = out
            .iter()
            .find(|r| r["region"] == Value::String("eu".into()))
            .unwrap();
        assert_eq!(eu["cnt"], Value::Number(1.into()));
    }

    #[test]
    fn sum_aggregate() {
        let plan =
            IvmPlan::parse("SELECT dept, SUM(amount) AS total FROM orders GROUP BY dept").unwrap();
        let mut c = IvmCircuit::new(plan);
        c.push_batch(&[
            make_delta(
                &[
                    ("dept", Value::String("eng".into())),
                    ("amount", Value::Number(100.into())),
                ],
                1,
            ),
            make_delta(
                &[
                    ("dept", Value::String("eng".into())),
                    ("amount", Value::Number(200.into())),
                ],
                1,
            ),
        ]);
        let out = c.read_output();
        let eng = out
            .iter()
            .find(|r| r["dept"] == Value::String("eng".into()))
            .unwrap();
        assert_eq!(eng["total"], Value::Number(300.into()));
    }

    #[test]
    fn retraction_removes_group() {
        let plan =
            IvmPlan::parse("SELECT region, COUNT(*) AS cnt FROM sales GROUP BY region").unwrap();
        let mut c = IvmCircuit::new(plan);
        c.push_batch(&[make_delta(&[("region", Value::String("us".into()))], 1)]);
        // Retract the single row.
        c.push_batch(&[make_delta(&[("region", Value::String("us".into()))], -1)]);
        assert_eq!(c.group_count(), 0);
    }

    // ── IvmJoinCircuit tests ────────────────────────────────────────────────

    #[test]
    fn join_circuit_broadcast_count() {
        // events JOIN categories ON events.cat_id = categories.cat_id
        // GROUP BY cat_name — COUNT(*)
        let sql = "SELECT c.cat_name, COUNT(*) AS cnt \
                   FROM events e \
                   JOIN categories c ON e.cat_id = c.cat_id \
                   GROUP BY c.cat_name";
        let plan = IvmPlan::parse(sql).unwrap();
        let left_cols = vec!["cat_id".to_string()];
        let strategies = vec![JoinStrategy::Broadcast];
        let mut jc = IvmJoinCircuit::new(plan, strategies, left_cols);

        // Load the "small" dimension side.
        let cat_rows: Vec<HashMap<String, Value>> = vec![
            [
                ("cat_id".into(), Value::Number(1.into())),
                ("cat_name".into(), Value::String("Sports".into())),
            ]
            .into_iter()
            .collect(),
            [
                ("cat_id".into(), Value::Number(2.into())),
                ("cat_name".into(), Value::String("Music".into())),
            ]
            .into_iter()
            .collect(),
        ];
        jc.load_right_side(0, &cat_rows, "cat_id");

        // Stream events.
        let events: Vec<(HashMap<String, Value>, i64)> = vec![
            (
                [("cat_id".into(), Value::Number(1.into()))]
                    .into_iter()
                    .collect(),
                1,
            ),
            (
                [("cat_id".into(), Value::Number(1.into()))]
                    .into_iter()
                    .collect(),
                1,
            ),
            (
                [("cat_id".into(), Value::Number(2.into()))]
                    .into_iter()
                    .collect(),
                1,
            ),
        ];
        jc.push_left_batch(&events);

        let out = jc.read_output();
        let sports = out
            .iter()
            .find(|r| r["cat_name"] == Value::String("Sports".into()))
            .unwrap();
        assert_eq!(sports["cnt"], Value::Number(2.into()));
        let music = out
            .iter()
            .find(|r| r["cat_name"] == Value::String("Music".into()))
            .unwrap();
        assert_eq!(music["cnt"], Value::Number(1.into()));
    }

    #[test]
    fn join_circuit_delete_propagation() {
        let sql = "SELECT e.cat_id, COUNT(*) AS cnt \
                   FROM events e \
                   JOIN categories c ON e.cat_id = c.cat_id \
                   GROUP BY e.cat_id";
        let plan = IvmPlan::parse(sql).unwrap();
        let mut jc = IvmJoinCircuit::new(
            plan,
            vec![JoinStrategy::Broadcast],
            vec!["cat_id".to_string()],
        );
        let cat: HashMap<String, Value> = [("cat_id".into(), Value::Number(1.into()))]
            .into_iter()
            .collect();
        jc.load_right_side(0, &[cat], "cat_id");

        let insert: Vec<(HashMap<String, Value>, i64)> = vec![(
            [("cat_id".into(), Value::Number(1.into()))]
                .into_iter()
                .collect(),
            1,
        )];
        jc.push_left_batch(&insert);
        assert_eq!(jc.group_count(), 1);

        let delete: Vec<(HashMap<String, Value>, i64)> = vec![(
            [("cat_id".into(), Value::Number(1.into()))]
                .into_iter()
                .collect(),
            -1,
        )];
        jc.push_left_batch(&delete);
        assert_eq!(jc.group_count(), 0, "retracted row must remove the group");
    }
}
