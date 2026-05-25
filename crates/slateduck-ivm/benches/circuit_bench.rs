//! Benchmark: STRING_AGG / ARRAY_AGG aggregate deletion via O(1) HashMap multiset.
//!
//! Measures the cost of processing a large negative-weight batch (deletion) for
//! a STRING_AGG aggregate group.  Before the v0.21 O(1) fix the implementation
//! called `Vec::position()` + `Vec::remove()` giving O(N²) behaviour.  After
//! the fix each delete is an O(1) hash-map decrement.

use criterion::{criterion_group, criterion_main, Criterion};
use serde_json::Value;
use slateduck_ivm::circuit::{IvmCircuit, ZDelta};
use slateduck_ivm::plan::IvmPlan;

fn make_delta(fields: &[(&str, Value)], weight: i64) -> ZDelta {
    ZDelta {
        fields: fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        weight,
    }
}

/// Bench STRING_AGG deletion at the given `group_size`.
///
/// Setup: insert `group_size` rows with distinct string values.
/// Measurement: delete all rows (one negative-weight delta each).
fn bench_string_agg_deletion(c: &mut Criterion, group_size: usize) {
    let plan =
        IvmPlan::parse("SELECT grp, STRING_AGG(val, ',') AS agg FROM t GROUP BY grp").unwrap();

    let mut circuit = IvmCircuit::new(plan.clone());

    // Pre-populate with `group_size` distinct string values.
    let inserts: Vec<ZDelta> = (0..group_size)
        .map(|i| {
            make_delta(
                &[
                    ("grp", Value::String("G1".into())),
                    ("val", Value::String(format!("value_{i:08}"))),
                ],
                1,
            )
        })
        .collect();
    circuit.push_batch(&inserts);

    // Build the deletion batch (same rows, weight = -1).
    let deletes: Vec<ZDelta> = (0..group_size)
        .map(|i| {
            make_delta(
                &[
                    ("grp", Value::String("G1".into())),
                    ("val", Value::String(format!("value_{i:08}"))),
                ],
                -1,
            )
        })
        .collect();

    c.bench_function(&format!("string_agg_deletion_{group_size}"), |b| {
        b.iter(|| {
            let mut c2 = IvmCircuit::new(plan.clone());
            // Re-populate to ensure state is non-empty before each delete run.
            c2.push_batch(&inserts);
            c2.push_batch(&deletes);
        });
    });
}

fn bench_string_agg_deletion_100k(c: &mut Criterion) {
    bench_string_agg_deletion(c, 100_000);
}

fn bench_string_agg_deletion_1m(c: &mut Criterion) {
    bench_string_agg_deletion(c, 1_000_000);
}

criterion_group!(
    benches,
    bench_string_agg_deletion_100k,
    bench_string_agg_deletion_1m,
);
criterion_main!(benches);
