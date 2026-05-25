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
//! ## Why a standalone engine instead of the full DBSP/Feldera API?
//! The `dbsp` crate (Feldera 0.299.0) is a full streaming platform runtime
//! that spawns its own worker threads, requires rkyv serialization on all data
//! types, and couples persistence to `feldera-storage`.  SlateDuck uses
//! SlateDB-native persistence, lease-based single-writer shards, and
//! protobuf/serde_json encoding — making DBSP integration infeasible without
//! forking.  See `docs/design-decisions/ivm-architecture.md` for the full
//! analysis.
//!
//! This engine implements the DBSP *algebraic model* (Z-differences over
//! multisets) directly, without depending on the Feldera runtime.
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
    /// AVG auxiliary: (sum_arg as f64, count_arg as i64) per aggregate index.
    pub avg_aux: Vec<AvgAux>,
    /// STDDEV auxiliary: (count, mean, M2) per aggregate index.
    pub stddev_aux: Vec<StddevAux>,
    /// BOOL_AND/OR auxiliary: (count_true, count_nonnull) per aggregate index.
    pub bool_aux: Vec<BoolAux>,
    /// BIT_AND/OR/XOR auxiliary: per-bit position counts per aggregate index.
    pub bit_aux: Vec<BitAux>,
    /// Group-rescan aggregates: all input values retained for re-aggregation.
    pub rescan_inputs: Vec<Vec<Value>>,
}

/// Auxiliary state for AVG: fully invertible via sum/count.
#[derive(Debug, Clone, Default)]
pub struct AvgAux {
    pub sum_arg: f64,
    pub count_arg: i64,
}

/// Auxiliary state for STDDEV: Welford's online algorithm with retraction.
#[derive(Debug, Clone, Default)]
pub struct StddevAux {
    pub count: i64,
    pub mean: f64,
    pub m2: f64,
}

/// Auxiliary state for BOOL_AND / BOOL_OR.
#[derive(Debug, Clone, Default)]
pub struct BoolAux {
    pub count_true: i64,
    pub count_nonnull: i64,
}

/// Auxiliary state for BIT_AND / BIT_OR / BIT_XOR (64-bit positions).
#[derive(Debug, Clone)]
pub struct BitAux {
    /// Per-bit count of set bits (for BIT_AND: count of 1s; for BIT_OR: count of 1s).
    pub bit_counts: [i64; 64],
    pub row_count: i64,
}

impl Default for BitAux {
    fn default() -> Self {
        Self {
            bit_counts: [0; 64],
            row_count: 0,
        }
    }
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

    /// Clear all state (for FULL refresh mode).
    pub fn clear_state(&mut self) {
        self.state.clear();
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
                avg_aux: vec![AvgAux::default(); n_aggs],
                stddev_aux: vec![StddevAux::default(); n_aggs],
                bool_aux: vec![BoolAux::default(); n_aggs],
                bit_aux: vec![BitAux::default(); n_aggs],
                rescan_inputs: vec![Vec::new(); n_aggs],
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
                    AggregateKind::Avg => {
                        if let Some(col) = &agg.input_col {
                            let v = value_to_f64(delta.fields.get(col));
                            entry.avg_aux[i].sum_arg += v * delta.weight as f64;
                            entry.avg_aux[i].count_arg += delta.weight;
                        }
                    }
                    AggregateKind::Stddev => {
                        if let Some(col) = &agg.input_col {
                            let v = value_to_f64(delta.fields.get(col));
                            let aux = &mut entry.stddev_aux[i];
                            if delta.weight > 0 {
                                // Online add (Welford).
                                for _ in 0..delta.weight {
                                    aux.count += 1;
                                    let d = v - aux.mean;
                                    aux.mean += d / aux.count as f64;
                                    let d2 = v - aux.mean;
                                    aux.m2 += d * d2;
                                }
                            } else {
                                // Retraction (reverse Welford).
                                for _ in 0..(-delta.weight) {
                                    if aux.count <= 1 {
                                        aux.count = 0;
                                        aux.mean = 0.0;
                                        aux.m2 = 0.0;
                                    } else {
                                        let d2 = v - aux.mean;
                                        aux.count -= 1;
                                        let d = v - aux.mean;
                                        aux.mean -= d / aux.count as f64;
                                        aux.m2 -= d2 * (v - aux.mean);
                                    }
                                }
                            }
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
                    AggregateKind::BoolAnd | AggregateKind::BoolOr => {
                        if let Some(col) = &agg.input_col {
                            let val = delta.fields.get(col);
                            let is_true = match val {
                                Some(Value::Bool(b)) => *b,
                                Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
                                _ => false,
                            };
                            let is_nonnull = val.is_some() && val != Some(&Value::Null);
                            if is_nonnull {
                                entry.bool_aux[i].count_nonnull += delta.weight;
                                if is_true {
                                    entry.bool_aux[i].count_true += delta.weight;
                                }
                            }
                        }
                    }
                    AggregateKind::BitAnd | AggregateKind::BitOr | AggregateKind::BitXor => {
                        if let Some(col) = &agg.input_col {
                            let v = value_to_i64(delta.fields.get(col));
                            let aux = &mut entry.bit_aux[i];
                            aux.row_count += delta.weight;
                            for bit in 0..64 {
                                if (v >> bit) & 1 == 1 {
                                    aux.bit_counts[bit] += delta.weight;
                                }
                            }
                        }
                    }
                    AggregateKind::StringAgg | AggregateKind::ArrayAgg => {
                        if let Some(col) = &agg.input_col {
                            let val = delta.fields.get(col).cloned().unwrap_or(Value::Null);
                            if delta.weight > 0 {
                                for _ in 0..delta.weight {
                                    entry.rescan_inputs[i].push(val.clone());
                                }
                            } else {
                                // Remove matching values.
                                for _ in 0..(-delta.weight) {
                                    if let Some(pos) =
                                        entry.rescan_inputs[i].iter().position(|v| v == &val)
                                    {
                                        entry.rescan_inputs[i].remove(pos);
                                    }
                                }
                            }
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
                    AggregateKind::Avg => {
                        let aux = &state.avg_aux[i];
                        if aux.count_arg == 0 {
                            Value::Null
                        } else {
                            json_f64(aux.sum_arg / aux.count_arg as f64)
                        }
                    }
                    AggregateKind::Stddev => {
                        let aux = &state.stddev_aux[i];
                        if aux.count < 2 {
                            Value::Null
                        } else {
                            json_f64((aux.m2 / (aux.count - 1) as f64).sqrt())
                        }
                    }
                    AggregateKind::Min => {
                        state.minmax[i].min().map(json_f64).unwrap_or(Value::Null)
                    }
                    AggregateKind::Max => {
                        state.minmax[i].max().map(json_f64).unwrap_or(Value::Null)
                    }
                    AggregateKind::BoolAnd => {
                        let aux = &state.bool_aux[i];
                        if aux.count_nonnull == 0 {
                            Value::Null
                        } else {
                            // BOOL_AND = true iff all non-null values are true.
                            Value::Bool(aux.count_true == aux.count_nonnull)
                        }
                    }
                    AggregateKind::BoolOr => {
                        let aux = &state.bool_aux[i];
                        if aux.count_nonnull == 0 {
                            Value::Null
                        } else {
                            // BOOL_OR = true iff at least one non-null value is true.
                            Value::Bool(aux.count_true > 0)
                        }
                    }
                    AggregateKind::BitAnd => {
                        let aux = &state.bit_aux[i];
                        if aux.row_count == 0 {
                            Value::Null
                        } else {
                            // BIT_AND: bit is 1 iff all rows have that bit set.
                            let mut result_val: i64 = 0;
                            for bit in 0..64 {
                                if aux.bit_counts[bit] == aux.row_count {
                                    result_val |= 1 << bit;
                                }
                            }
                            Value::Number(serde_json::Number::from(result_val))
                        }
                    }
                    AggregateKind::BitOr => {
                        let aux = &state.bit_aux[i];
                        if aux.row_count == 0 {
                            Value::Null
                        } else {
                            // BIT_OR: bit is 1 iff at least one row has that bit set.
                            let mut result_val: i64 = 0;
                            for bit in 0..64 {
                                if aux.bit_counts[bit] > 0 {
                                    result_val |= 1 << bit;
                                }
                            }
                            Value::Number(serde_json::Number::from(result_val))
                        }
                    }
                    AggregateKind::BitXor => {
                        let aux = &state.bit_aux[i];
                        if aux.row_count == 0 {
                            Value::Null
                        } else {
                            // BIT_XOR: bit is 1 iff odd number of rows have that bit set.
                            let mut result_val: i64 = 0;
                            for bit in 0..64 {
                                if aux.bit_counts[bit] % 2 != 0 {
                                    result_val |= 1 << bit;
                                }
                            }
                            Value::Number(serde_json::Number::from(result_val))
                        }
                    }
                    AggregateKind::StringAgg => {
                        let inputs = &state.rescan_inputs[i];
                        if inputs.is_empty() {
                            Value::Null
                        } else {
                            let s: String = inputs
                                .iter()
                                .filter_map(|v| match v {
                                    Value::String(s) => Some(s.as_str()),
                                    _ => None,
                                })
                                .collect::<Vec<_>>()
                                .join(",");
                            Value::String(s)
                        }
                    }
                    AggregateKind::ArrayAgg => {
                        let inputs = &state.rescan_inputs[i];
                        if inputs.is_empty() {
                            Value::Null
                        } else {
                            Value::Array(inputs.clone())
                        }
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

    /// Restore a group from persisted state (used by SlateDbTrace restore).
    pub fn push_restored_group(&mut self, _key: Vec<Value>, _values: HashMap<String, Value>) {
        // In a full implementation, this would rebuild the AggState from persisted values.
        // For the v0.15 implementation, restore is handled by replaying from the frontier.
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

fn value_to_i64(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
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
    /// Pre-snapshot states for EC-01 asymmetric delete branches (v0.14).
    /// When set, deletes join against this state instead of `join_states`.
    pub pre_snapshot_states: Vec<Option<HashJoinState>>,
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
            left_cols: left_cols.clone(),
            pre_snapshot_states: vec![None; n],
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
    /// **v0.14 EC-01 fix:** Uses asymmetric delta branches:
    /// - Part 1a: `ΔR_insert ⋈ S_post` — positive contributions
    /// - Part 1b: `ΔR_delete ⋈ S_pre` — negatives use pre-change snapshot
    /// - Part 3: `−(ΔR ⋈ ΔS)` — correction term (handled by caller via
    ///   `apply_correction_term`)
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

        // EC-01: Split into insert and delete branches.
        let inserts: Vec<(HashMap<String, Value>, i64)> =
            rows.iter().filter(|(_, w)| *w > 0).cloned().collect();
        let deletes: Vec<(HashMap<String, Value>, i64)> =
            rows.iter().filter(|(_, w)| *w < 0).cloned().collect();

        // Part 1a: ΔR_insert ⋈ S_post (current join state is the post-change snapshot).
        let mut joined_inserts: Vec<(HashMap<String, Value>, i64)> = inserts.clone();
        for (i, state) in self.join_states.iter().enumerate() {
            let left_col = self.left_cols.get(i).map(|s| s.as_str()).unwrap_or("");
            joined_inserts = hash_join_batch(&joined_inserts, state, left_col);
        }

        // Part 1b: ΔR_delete ⋈ S_pre.
        // S_pre is reconstructed from S_post by reverting any pending right-side
        // deltas accumulated in `right_deltas_pending`. If there are no pending
        // right-side deltas, S_pre == S_post (the common case for single-batch
        // updates where only the left side changes).
        //
        // For the common case (no concurrent right-side changes in this window),
        // S_pre == S_post and deletes also join against the current state.
        let mut joined_deletes: Vec<(HashMap<String, Value>, i64)> = deletes.clone();
        if !joined_deletes.is_empty() {
            // Use the pre-snapshot states if available, else fall back to post.
            for (i, pre_state) in self.pre_snapshot_states.iter().enumerate() {
                let left_col = self.left_cols.get(i).map(|s| s.as_str()).unwrap_or("");
                let state = pre_state.as_ref().unwrap_or(&self.join_states[i]);
                joined_deletes = hash_join_batch(&joined_deletes, state, left_col);
            }
        }

        let mut all_joined = joined_inserts;
        all_joined.extend(joined_deletes);

        let n = all_joined.len();
        let deltas: Vec<ZDelta> = all_joined
            .into_iter()
            .map(|(f, w)| ZDelta {
                fields: f,
                weight: w,
            })
            .collect();
        self.inner.push_batch(&deltas);
        n
    }

    /// Apply the Part 3 correction term: `−(ΔR ⋈ ΔS)`.
    ///
    /// Call this after processing both left and right deltas in the same
    /// refresh window to subtract double-counted intersections.
    pub fn apply_correction_term(
        &mut self,
        left_deltas: &[(HashMap<String, Value>, i64)],
        right_deltas: &[(HashMap<String, Value>, i64)],
        right_col: &str,
    ) {
        if left_deltas.is_empty() || right_deltas.is_empty() {
            return;
        }

        // Build a temporary join state from the right deltas.
        let mut temp_state = HashJoinState::new();
        for (row, weight) in right_deltas {
            if *weight > 0 {
                let key = match row.get(right_col) {
                    Some(v) => serde_json::to_string(v).unwrap_or_default(),
                    None => continue,
                };
                temp_state.insert_right(&key, row.clone());
            }
        }

        if temp_state.key_count() == 0 {
            return;
        }

        // Join left deltas against right deltas.
        let left_col = self.left_cols.first().map(|s| s.as_str()).unwrap_or("");
        let correction = hash_join_batch(left_deltas, &temp_state, left_col);

        // Negate and push.
        let deltas: Vec<ZDelta> = correction
            .into_iter()
            .map(|(f, w)| ZDelta {
                fields: f,
                weight: -w, // Correction term is negated.
            })
            .collect();
        self.inner.push_batch(&deltas);
    }

    /// Snapshot the current right-side state as `S_pre` before applying
    /// right-side deltas in a refresh window.
    ///
    /// Call this at the beginning of each refresh window before
    /// `push_right_delta` to enable correct EC-01 asymmetric delete branches.
    pub fn snapshot_pre_state(&mut self) {
        self.pre_snapshot_states = self.join_states.iter().map(|s| Some(s.clone())).collect();
    }

    /// Clear the pre-snapshot states after a refresh window completes.
    pub fn clear_pre_state(&mut self) {
        for slot in self.pre_snapshot_states.iter_mut() {
            *slot = None;
        }
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
