//! Property-based tests for key encoding, value round-trips, and prefix isolation.

use proptest::prelude::*;
use slateduck_core::keys::*;
use slateduck_core::mvcc::{is_visible, SnapshotId};
use slateduck_core::rows::*;
use slateduck_core::tags::*;
use slateduck_core::values::{decode_value, encode_value};

// ─── Round-Trip Tests ──────────────────────────────────────────────────────

proptest! {
    #[test]
    fn round_trip_snapshot_row(
        snapshot_id in 1u64..u64::MAX,
        schema_version in 0u64..1000,
        author in proptest::option::of("[a-z]{1,20}"),
        message in proptest::option::of("[a-z ]{1,50}"),
    ) {
        let row = SnapshotRow {
            snapshot_id,
            schema_version,
            snapshot_time: "2024-01-01T00:00:00Z".to_string(),
            author,
            message,
        };
        let encoded = encode_value(&row);
        let decoded: SnapshotRow = decode_value(&encoded).unwrap();
        prop_assert_eq!(row, decoded);
    }

    #[test]
    fn round_trip_schema_row(
        schema_id in 1u64..u64::MAX,
        schema_name in "[a-z_]{1,30}",
        begin_snapshot in 1u64..u64::MAX / 2,
        has_end in proptest::bool::ANY,
    ) {
        let end_snapshot = if has_end { Some(begin_snapshot + 1) } else { None };
        let row = SchemaRow {
            schema_id,
            schema_name,
            begin_snapshot,
            end_snapshot,
        };
        let encoded = encode_value(&row);
        let decoded: SchemaRow = decode_value(&encoded).unwrap();
        prop_assert_eq!(row, decoded);
    }

    #[test]
    fn round_trip_table_row(
        table_id in 1u64..u64::MAX,
        schema_id in 1u64..u64::MAX,
        table_name in "[a-z_]{1,30}",
        begin_snapshot in 1u64..u64::MAX / 2,
        has_end in proptest::bool::ANY,
        has_path in proptest::bool::ANY,
    ) {
        let end_snapshot = if has_end { Some(begin_snapshot + 1) } else { None };
        let data_path = if has_path { Some("s3://bucket/data/".to_string()) } else { None };
        let row = TableRow {
            table_id,
            schema_id,
            table_name,
            begin_snapshot,
            end_snapshot,
            data_path,
        };
        let encoded = encode_value(&row);
        let decoded: TableRow = decode_value(&encoded).unwrap();
        prop_assert_eq!(row, decoded);
    }

    #[test]
    fn round_trip_column_row(
        column_id in 1u64..u64::MAX,
        table_id in 1u64..u64::MAX,
        column_name in "[a-z_]{1,20}",
        data_type in "(INTEGER|VARCHAR|BOOLEAN|TIMESTAMP)",
        column_index in 0u64..100,
        begin_snapshot in 1u64..u64::MAX / 2,
        is_nullable in proptest::bool::ANY,
    ) {
        let row = ColumnRow {
            column_id,
            table_id,
            column_name,
            data_type,
            column_index,
            begin_snapshot,
            end_snapshot: None,
            default_value: None,
            is_nullable,
        };
        let encoded = encode_value(&row);
        let decoded: ColumnRow = decode_value(&encoded).unwrap();
        prop_assert_eq!(row, decoded);
    }

    #[test]
    fn round_trip_data_file_row(
        data_file_id in 1u64..u64::MAX,
        table_id in 1u64..u64::MAX,
        row_count in 0u64..1_000_000,
        file_size_bytes in 0u64..10_000_000_000,
        snapshot_id in 1u64..u64::MAX,
    ) {
        let row = DataFileRow {
            data_file_id,
            table_id,
            path: "data/table1/file.parquet".to_string(),
            file_format: "parquet".to_string(),
            row_count,
            file_size_bytes,
            snapshot_id,
            footer_size: None,
        };
        let encoded = encode_value(&row);
        let decoded: DataFileRow = decode_value(&encoded).unwrap();
        prop_assert_eq!(row, decoded);
    }

    #[test]
    fn round_trip_inlined_insert_row(
        table_id in 1u64..u64::MAX,
        schema_version in 0u64..100,
        row_id in 0u64..u64::MAX,
        payload in proptest::collection::vec(any::<u8>(), 0..100),
        begin_snapshot in 1u64..u64::MAX / 2,
    ) {
        let row = InlinedInsertRow {
            table_id,
            schema_version,
            row_id,
            payload,
            begin_snapshot,
            end_snapshot: None,
        };
        let encoded = encode_value(&row);
        let decoded: InlinedInsertRow = decode_value(&encoded).unwrap();
        prop_assert_eq!(row, decoded);
    }

    #[test]
    fn round_trip_inlined_delete_row(
        table_id in 1u64..u64::MAX,
        data_file_id in 1u64..u64::MAX,
        row_id in 0u64..u64::MAX,
        begin_snapshot in 1u64..u64::MAX,
    ) {
        let row = InlinedDeleteRow {
            table_id,
            data_file_id,
            row_id,
            begin_snapshot,
        };
        let encoded = encode_value(&row);
        let decoded: InlinedDeleteRow = decode_value(&encoded).unwrap();
        prop_assert_eq!(row, decoded);
    }

    #[test]
    fn round_trip_file_column_stats(
        table_id in 1u64..u64::MAX,
        column_id in 1u64..u64::MAX,
        data_file_id in 1u64..u64::MAX,
        has_null in proptest::bool::ANY,
        contains_nan in proptest::bool::ANY,
    ) {
        let row = FileColumnStatsRow {
            table_id,
            column_id,
            data_file_id,
            has_null,
            min_value: Some("10".to_string()),
            max_value: Some("100".to_string()),
            contains_nan,
        };
        let encoded = encode_value(&row);
        let decoded: FileColumnStatsRow = decode_value(&encoded).unwrap();
        prop_assert_eq!(row, decoded);
    }
}

// ─── Key Ordering Tests ───────────────────────────────────────────────────

proptest! {
    #[test]
    fn key_ordering_snapshot(a in 1u64..u64::MAX - 1) {
        let k1 = key_snapshot(a);
        let k2 = key_snapshot(a + 1);
        prop_assert!(k1 < k2);
    }

    #[test]
    fn key_ordering_schema(a in 1u64..u64::MAX - 1) {
        let k1 = key_schema(a);
        let k2 = key_schema(a + 1);
        prop_assert!(k1 < k2);
    }

    #[test]
    fn key_ordering_table(
        schema_id in 1u64..1000,
        table_id in 1u64..1000,
        begin in 1u64..u64::MAX - 1,
    ) {
        let k1 = key_table(schema_id, table_id, begin);
        let k2 = key_table(schema_id, table_id, begin + 1);
        prop_assert!(k1 < k2);
    }

    #[test]
    fn key_ordering_data_file(
        table_id in 1u64..1000,
        file_id in 1u64..u64::MAX - 1,
    ) {
        let k1 = key_data_file(table_id, file_id);
        let k2 = key_data_file(table_id, file_id + 1);
        prop_assert!(k1 < k2);
    }

    #[test]
    fn key_ordering_column(
        table_id in 1u64..1000,
        col_id in 1u64..1000,
        begin in 1u64..u64::MAX - 1,
    ) {
        let k1 = key_column(table_id, col_id, begin);
        let k2 = key_column(table_id, col_id, begin + 1);
        prop_assert!(k1 < k2);
    }
}

// ─── Prefix Isolation Tests ───────────────────────────────────────────────

proptest! {
    #[test]
    fn prefix_isolation_different_tags(
        id1 in 1u64..u64::MAX,
        id2 in 1u64..u64::MAX,
    ) {
        // Keys with different tags never share a prefix
        let schema_key = key_schema(id1);
        let table_key = key_table(id2, 1, 1);
        let data_key = key_data_file(id1, id2);

        // First bytes (tags) differ
        prop_assert_ne!(schema_key[0], table_key[0]);
        prop_assert_ne!(schema_key[0], data_key[0]);
        prop_assert_ne!(table_key[0], data_key[0]);
    }

    #[test]
    fn prefix_scan_only_same_table(
        table_id in 1u64..1000,
        other_table in 1001u64..2000,
        file_id in 1u64..u64::MAX,
    ) {
        let prefix = prefix_data_files_for_table(table_id);
        let own_key = key_data_file(table_id, file_id);
        let other_key = key_data_file(other_table, file_id);

        // Own key starts with prefix
        prop_assert!(own_key.starts_with(&prefix));
        // Other table's key does not start with our prefix
        prop_assert!(!other_key.starts_with(&prefix));
    }
}

// ─── MVCC Visibility Tests ────────────────────────────────────────────────

proptest! {
    #[test]
    fn mvcc_visible_at_begin(begin in 1u64..u64::MAX) {
        prop_assert!(is_visible(begin, None, SnapshotId(begin)));
    }

    #[test]
    fn mvcc_not_visible_before_begin(begin in 2u64..u64::MAX) {
        prop_assert!(!is_visible(begin, None, SnapshotId(begin - 1)));
    }

    #[test]
    fn mvcc_not_visible_at_end(begin in 1u64..u64::MAX / 2) {
        let end = begin + 1;
        prop_assert!(!is_visible(begin, Some(end), SnapshotId(end)));
    }

    #[test]
    fn mvcc_visible_between_begin_and_end(
        begin in 1u64..u64::MAX / 4,
        gap in 2u64..1000,
    ) {
        let end = begin + gap;
        let mid = begin + gap / 2;
        prop_assert!(is_visible(begin, Some(end), SnapshotId(mid)));
    }
}

// ─── No Key Collisions Between Tags ──────────────────────────────────────

#[test]
fn no_collisions_across_all_tag_prefixes() {
    // All single-byte prefixes for different tags must be distinct
    let prefixes: Vec<Vec<u8>> = ALL_TAGS.iter().map(|t| vec![t.tag]).collect();
    let unique: std::collections::HashSet<_> = prefixes.iter().collect();
    assert_eq!(unique.len(), prefixes.len());
}

// ─── ID Monotonicity ─────────────────────────────────────────────────────

#[test]
fn id_monotonicity_across_operations() {
    use slateduck_core::counters::CounterCache;

    let mut cache = CounterCache::new(1, 1, 1);
    let mut prev_snap = 0u64;
    let mut prev_cat = 0u64;
    let mut prev_file = 0u64;

    for _ in 0..1000 {
        let s = cache.alloc_snapshot_id();
        assert!(
            s > prev_snap,
            "snapshot ID not monotonic: {} <= {}",
            s,
            prev_snap
        );
        prev_snap = s;

        let c = cache.alloc_catalog_id();
        assert!(
            c > prev_cat,
            "catalog ID not monotonic: {} <= {}",
            c,
            prev_cat
        );
        prev_cat = c;

        let f = cache.alloc_file_id();
        assert!(
            f > prev_file,
            "file ID not monotonic: {} <= {}",
            f,
            prev_file
        );
        prev_file = f;
    }
}
