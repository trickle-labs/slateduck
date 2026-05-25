//! IVM trace: aggregate state store with checkpoint/restore.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::circuit::IvmCircuit;
use crate::plan::IvmPlan;

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
}

impl IvmTrace {
    /// Create a fresh trace for the given plan.
    pub fn new(plan: IvmPlan) -> Self {
        Self {
            circuit: IvmCircuit::new(plan),
            last_input_snapshot: 0,
            last_output_snapshot: 0,
            seq: 0,
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
}
