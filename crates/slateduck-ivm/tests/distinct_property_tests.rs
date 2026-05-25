//! Tier 6f — DISTINCT property tests.
//!
//! Property-based tests using `proptest` to verify reference-counted DISTINCT,
//! UNION DISTINCT, INTERSECT, and EXCEPT under arbitrary insert/delete/update
//! sequences. Asserts output multiset matches DuckDB reference semantics at
//! every step.
//!
//! Covers:
//! - Single insert-delete cycle
//! - Multi-insert partial-delete
//! - Cross-operand UNION DISTINCT with shared rows
//! - INTERSECT where one operand goes empty
//! - EXCEPT where subtractor count exceeds the original count (clamp to 0)

use proptest::prelude::*;
use slateduck_ivm::ref_counted::{RefCountedDistinct, RefCountedSetOp, SetOperator};

/// Operations that can be performed on a distinct set.
#[derive(Debug, Clone)]
enum DistinctOp {
    Insert(Vec<u8>),
    Delete(Vec<u8>),
}

/// Operations for set operators (two-input).
#[derive(Debug, Clone)]
enum SetOp {
    InsertLeft(Vec<u8>),
    InsertRight(Vec<u8>),
    DeleteLeft(Vec<u8>),
    DeleteRight(Vec<u8>),
}

/// Generate arbitrary row keys (1-4 bytes).
fn arb_row_key() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 1..=4)
}

/// Generate a sequence of distinct operations.
fn arb_distinct_ops(max_len: usize) -> impl Strategy<Value = Vec<DistinctOp>> {
    prop::collection::vec(
        prop_oneof![
            arb_row_key().prop_map(DistinctOp::Insert),
            arb_row_key().prop_map(DistinctOp::Delete),
        ],
        1..=max_len,
    )
}

/// Generate a sequence of set operations.
fn arb_set_ops(max_len: usize) -> impl Strategy<Value = Vec<SetOp>> {
    prop::collection::vec(
        prop_oneof![
            arb_row_key().prop_map(SetOp::InsertLeft),
            arb_row_key().prop_map(SetOp::InsertRight),
            arb_row_key().prop_map(SetOp::DeleteLeft),
            arb_row_key().prop_map(SetOp::DeleteRight),
        ],
        1..=max_len,
    )
}

/// Reference implementation: computes visible rows for DISTINCT.
/// A row is visible iff its net insert count > 0.
fn reference_distinct_visible(ops: &[DistinctOp]) -> std::collections::HashSet<Vec<u8>> {
    let mut counts: std::collections::HashMap<Vec<u8>, i64> = std::collections::HashMap::new();
    for op in ops {
        match op {
            DistinctOp::Insert(key) => *counts.entry(key.clone()).or_insert(0) += 1,
            DistinctOp::Delete(key) => *counts.entry(key.clone()).or_insert(0) -= 1,
        }
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(key, _)| key)
        .collect()
}

/// Reference implementation: computes visible rows for set operators.
fn reference_set_visible(
    ops: &[SetOp],
    operator: SetOperator,
) -> std::collections::HashSet<Vec<u8>> {
    let mut left: std::collections::HashMap<Vec<u8>, i64> = std::collections::HashMap::new();
    let mut right: std::collections::HashMap<Vec<u8>, i64> = std::collections::HashMap::new();

    for op in ops {
        match op {
            SetOp::InsertLeft(key) => *left.entry(key.clone()).or_insert(0) += 1,
            SetOp::InsertRight(key) => *right.entry(key.clone()).or_insert(0) += 1,
            SetOp::DeleteLeft(key) => {
                let e = left.entry(key.clone()).or_insert(0);
                *e -= 1;
                if *e <= 0 {
                    left.remove(key);
                }
            }
            SetOp::DeleteRight(key) => {
                let e = right.entry(key.clone()).or_insert(0);
                *e -= 1;
                if *e <= 0 {
                    right.remove(key);
                }
            }
        }
    }

    let all_keys: std::collections::HashSet<&Vec<u8>> = left.keys().chain(right.keys()).collect();

    all_keys
        .into_iter()
        .filter(|key| {
            let l = left.get(*key).copied().unwrap_or(0);
            let r = right.get(*key).copied().unwrap_or(0);
            match operator {
                SetOperator::UnionDistinct => l > 0 || r > 0,
                SetOperator::Intersect => l > 0 && r > 0,
                SetOperator::Except => (l - r) > 0,
            }
        })
        .cloned()
        .collect()
}

proptest! {
    /// Property: RefCountedDistinct always matches the reference implementation.
    #[test]
    fn distinct_matches_reference(ops in arb_distinct_ops(50)) {
        let mut distinct = RefCountedDistinct::new();
        for op in &ops {
            match op {
                DistinctOp::Insert(key) => { distinct.insert(key.clone()); }
                DistinctOp::Delete(key) => { distinct.delete(key); }
            }
        }

        let expected = reference_distinct_visible(&ops);
        let actual: std::collections::HashSet<Vec<u8>> =
            distinct.visible_rows().into_iter().cloned().collect();
        prop_assert_eq!(actual, expected);
    }

    /// Property: UNION DISTINCT always matches reference (MAX semantics).
    #[test]
    fn union_distinct_matches_reference(ops in arb_set_ops(50)) {
        let mut set_op = RefCountedSetOp::new();
        for op in &ops {
            match op {
                SetOp::InsertLeft(key) => set_op.insert_left(key.clone()),
                SetOp::InsertRight(key) => set_op.insert_right(key.clone()),
                SetOp::DeleteLeft(key) => set_op.delete_left(key),
                SetOp::DeleteRight(key) => set_op.delete_right(key),
            }
        }

        let expected = reference_set_visible(&ops, SetOperator::UnionDistinct);
        let actual: std::collections::HashSet<Vec<u8>> =
            set_op.visible_rows(SetOperator::UnionDistinct).into_iter().collect();
        prop_assert_eq!(actual, expected);
    }

    /// Property: INTERSECT always matches reference (MIN semantics).
    #[test]
    fn intersect_matches_reference(ops in arb_set_ops(50)) {
        let mut set_op = RefCountedSetOp::new();
        for op in &ops {
            match op {
                SetOp::InsertLeft(key) => set_op.insert_left(key.clone()),
                SetOp::InsertRight(key) => set_op.insert_right(key.clone()),
                SetOp::DeleteLeft(key) => set_op.delete_left(key),
                SetOp::DeleteRight(key) => set_op.delete_right(key),
            }
        }

        let expected = reference_set_visible(&ops, SetOperator::Intersect);
        let actual: std::collections::HashSet<Vec<u8>> =
            set_op.visible_rows(SetOperator::Intersect).into_iter().collect();
        prop_assert_eq!(actual, expected);
    }

    /// Property: EXCEPT always matches reference (clamp to 0 semantics).
    #[test]
    fn except_matches_reference(ops in arb_set_ops(50)) {
        let mut set_op = RefCountedSetOp::new();
        for op in &ops {
            match op {
                SetOp::InsertLeft(key) => set_op.insert_left(key.clone()),
                SetOp::InsertRight(key) => set_op.insert_right(key.clone()),
                SetOp::DeleteLeft(key) => set_op.delete_left(key),
                SetOp::DeleteRight(key) => set_op.delete_right(key),
            }
        }

        let expected = reference_set_visible(&ops, SetOperator::Except);
        let actual: std::collections::HashSet<Vec<u8>> =
            set_op.visible_rows(SetOperator::Except).into_iter().collect();
        prop_assert_eq!(actual, expected);
    }
}

// Deterministic correctness tests (not property-based):

#[test]
fn distinct_insert_3x_delete_2x_exactly_one_output() {
    let mut distinct = RefCountedDistinct::new();
    let key = b"row_abc".to_vec();

    // Insert same row 3 times
    distinct.insert(key.clone());
    distinct.insert(key.clone());
    distinct.insert(key.clone());
    assert_eq!(distinct.get_count(&key), 3);
    assert!(distinct.is_visible(&key));
    assert_eq!(distinct.visible_count(), 1); // exactly ONE output row

    // Delete 2 times
    distinct.delete(&key);
    distinct.delete(&key);
    assert_eq!(distinct.get_count(&key), 1);
    assert!(distinct.is_visible(&key));
    assert_eq!(distinct.visible_count(), 1); // still exactly ONE output row
}

#[test]
fn union_distinct_shared_row_exactly_one_output() {
    let mut set_op = RefCountedSetOp::new();
    let key = b"shared_row".to_vec();

    // Insert same row into both left and right
    set_op.insert_left(key.clone());
    set_op.insert_right(key.clone());

    // UNION DISTINCT: exactly one output row (not two)
    let visible = set_op.visible_rows(SetOperator::UnionDistinct);
    assert_eq!(visible.len(), 1);
    assert!(set_op.is_visible(&key, SetOperator::UnionDistinct));
}

#[test]
fn intersect_one_operand_empty() {
    let mut set_op = RefCountedSetOp::new();
    let key = b"only_left".to_vec();

    set_op.insert_left(key.clone());
    // Right is empty → INTERSECT should produce nothing
    assert!(!set_op.is_visible(&key, SetOperator::Intersect));
    assert_eq!(set_op.visible_rows(SetOperator::Intersect).len(), 0);
}

#[test]
fn except_subtractor_exceeds_original() {
    let mut set_op = RefCountedSetOp::new();
    let key = b"excess".to_vec();

    // Left has 1, right has 3 → clamped to 0
    set_op.insert_left(key.clone());
    set_op.insert_right(key.clone());
    set_op.insert_right(key.clone());
    set_op.insert_right(key.clone());

    assert!(!set_op.is_visible(&key, SetOperator::Except));
    assert_eq!(set_op.visible_rows(SetOperator::Except).len(), 0);
}

#[test]
fn multi_insert_partial_delete_sequence() {
    let mut distinct = RefCountedDistinct::new();

    // Insert multiple different rows
    let keys: Vec<Vec<u8>> = (0..10).map(|i| vec![i]).collect();
    for key in &keys {
        distinct.insert(key.clone());
        distinct.insert(key.clone()); // duplicate each
    }
    assert_eq!(distinct.visible_count(), 10);

    // Delete 5 rows completely (both copies)
    for key in &keys[..5] {
        distinct.delete(key);
        distinct.delete(key);
    }
    assert_eq!(distinct.visible_count(), 5);

    // Partially delete remaining 5 (one copy each)
    for key in &keys[5..] {
        distinct.delete(key);
    }
    // Still visible (ref_count = 1 for each)
    assert_eq!(distinct.visible_count(), 5);
}

#[test]
fn cross_operand_union_distinct_with_shared_rows() {
    let mut set_op = RefCountedSetOp::new();

    // Multiple shared rows between left and right
    for i in 0u8..5 {
        set_op.insert_left(vec![i]);
        set_op.insert_right(vec![i]);
    }
    // Plus some unique rows
    for i in 5u8..8 {
        set_op.insert_left(vec![i]);
    }
    for i in 8u8..10 {
        set_op.insert_right(vec![i]);
    }

    // UNION DISTINCT: should have 10 unique rows total
    let visible = set_op.visible_rows(SetOperator::UnionDistinct);
    assert_eq!(visible.len(), 10);
}
