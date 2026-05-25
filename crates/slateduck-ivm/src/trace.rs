//! IVM trace: aggregate state store with checkpoint/restore.
//!
//! ## v0.17: `__sd_ref_count` auxiliary column
//! For views containing DISTINCT or UNION DISTINCT / INTERSECT / EXCEPT,
//! an `__sd_ref_count: i64` auxiliary column is maintained. INSERT increments;
//! DELETE decrements; row visible in output only when `__sd_ref_count > 0`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::circuit::IvmCircuit;
use crate::plan::IvmPlan;
use crate::ref_counted::RefCountedDistinct;

/// Serialisable snapshot of a circuit's output state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSnapshot {
    pub last_input_snapshot: u64,
    pub last_output_snapshot: u64,
    pub seq: u64,
    /// Serialised group key → aggregate values.
    pub groups: Vec<TraceGroup>,
}

/// One output group in the snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceGroup {
    pub key: Vec<Value>,
    pub values: HashMap<String, Value>,
}

/// Wraps an [`IvmCircuit`] with checkpoint metadata.
pub struct IvmTrace {
    pub circuit: IvmCircuit,
    pub last_input_snapshot: u64,
    pub last_output_snapshot: u64,
    pub seq: u64,
    /// Whether this trace tracks reference counts for DISTINCT.
    pub has_ref_count: bool,
    /// Reference count state (when has_ref_count is true).
    pub ref_counts: RefCountedDistinct,
}

impl IvmTrace {
    /// Create a fresh trace for the given plan.
    pub fn new(plan: IvmPlan) -> Self {
        Self {
            circuit: IvmCircuit::new(plan),
            last_input_snapshot: 0,
            last_output_snapshot: 0,
            seq: 0,
            has_ref_count: false,
            ref_counts: RefCountedDistinct::new(),
        }
    }

    /// Create a trace with `__sd_ref_count` enabled (for DISTINCT views).
    pub fn new_with_ref_count(plan: IvmPlan) -> Self {
        Self {
            circuit: IvmCircuit::new(plan),
            last_input_snapshot: 0,
            last_output_snapshot: 0,
            seq: 0,
            has_ref_count: true,
            ref_counts: RefCountedDistinct::new(),
        }
    }

    /// Advance the seq counter and record checkpoint metadata.
    pub fn advance_checkpoint(&mut self, input_snapshot: u64, output_snapshot: u64) {
        self.seq += 1;
        self.last_input_snapshot = input_snapshot;
        self.last_output_snapshot = output_snapshot;
    }

    /// Read all current output rows.
    pub fn read_output(&self) -> Vec<HashMap<String, Value>> {
        self.circuit.read_output()
    }

    /// Insert a row with reference counting.
    /// Returns true if the row became newly visible in the output.
    pub fn ref_count_insert(&mut self, row_key: Vec<u8>) -> bool {
        if self.has_ref_count {
            self.ref_counts.insert(row_key)
        } else {
            true // always visible without ref counting
        }
    }

    /// Delete a row with reference counting.
    /// Returns true if the row became invisible in the output.
    pub fn ref_count_delete(&mut self, row_key: &[u8]) -> bool {
        if self.has_ref_count {
            self.ref_counts.delete(row_key)
        } else {
            true // always remove without ref counting
        }
    }

    /// Get the __sd_ref_count for a row.
    pub fn get_ref_count(&self, row_key: &[u8]) -> i64 {
        self.ref_counts.get_count(row_key)
    }

    /// Get the number of visible rows (ref_count > 0).
    pub fn visible_row_count(&self) -> usize {
        if self.has_ref_count {
            self.ref_counts.visible_count()
        } else {
            self.circuit.read_output().len()
        }
    }
}
