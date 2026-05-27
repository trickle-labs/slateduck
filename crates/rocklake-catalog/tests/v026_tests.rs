//! v0.26 conformance tests: Stats, Types, Partitioning & Sorting.
//!
//! Covers all phases from the v0.26 roadmap:
//!   Phase 1 -- Full file column stats (contains_null, extra fields)
//!   Phase 2 -- Full table column stats (contains_nan, extra_stats)
//!   Phase 3 -- Variant stats (variant_key, shredded_type, extra fields)
//!   Phase 4 -- Geometry stats (GeometryExtraStats, JSON validation)
//!   Phase 5 -- DuckLake type parser (all spec types, nested types)
//!   Phase 6 -- Nested column tree reads (parent_column ordering)
//!   Phase 7 -- Sort expression spec fields (sort_direction, null_order)
//!   Phase 8 -- Partition column table_id
//!   Phase 9 -- File partition value partition_value rename
//!   Phase 10 -- Files scheduled for deletion (path_is_relative, optional file_type)
//!   Phase 11 -- Partial-file partial_max pruning shortcut

use object_store::path::Path as ObjectPath;
use rocklake_catalog::writer::stats::{FileColumnStatsInput, FileVariantStatsInput};
use rocklake_catalog::{CatalogStore, OpenOptions};
use rocklake_core::mvcc::SnapshotId;
use rocklake_core::rows::*;
use rocklake_core::types::{validate_extra_stats, DuckLakeType, GeometryExtraStats, PruneResult};
use std::sync::Arc;
use tempfile::TempDir;

fn test_opts(dir: &TempDir) -> OpenOptions {
    let path = dir.path().to_str().unwrap().to_string();
    let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
    OpenOptions {
        object_store: store,
        path: ObjectPath::from("catalog"),
        encryption: None,
    }
}

// ─── Phase 1: File Column Stats ──────────────────────────────────────────────

#[test]
fn file_column_stats_row_has_v026_fields() {
    let row = FileColumnStatsRow {
        table_id: 1,
        column_id: 2,
        data_file_id: 3,
        contains_null: true,
        min_value: Some("0".to_string()),
        max_value: Some("100".to_string()),
        contains_nan: false,
        column_size_bytes: Some(4096),
        value_count: Some(1000),
        null_count: Some(5),
        extra_stats: Some(r#"{"type":"int32"}"#.to_string()),
    };
    assert!(row.contains_null);
    assert_eq!(row.column_size_bytes, Some(4096));
    assert_eq!(row.value_count, Some(1000));
    assert_eq!(row.null_count, Some(5));
    assert!(row.extra_stats.is_some());
}

#[tokio::test]
async fn upsert_file_column_stats_with_new_fields() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let col_id = writer
        .add_column(table_id, "amount", "int64", 0, false, None)
        .await
        .unwrap();
    let file_id = writer
        .register_data_file(table_id, "f1.parquet", "parquet", 1000, 65536)
        .await
        .unwrap();

    writer
        .upsert_file_column_stats(FileColumnStatsInput {
            table_id,
            column_id: col_id,
            data_file_id: file_id,
            contains_null: true,
            min_value: Some("1"),
            max_value: Some("999"),
            contains_nan: false,
            column_size_bytes: Some(8192),
            value_count: Some(1000),
            null_count: Some(10),
            extra_stats: Some(r#"{"info":"test"}"#),
        })
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    // Verify pruning still works with renamed field.
    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let kept = reader
        .prune_files(
            table_id,
            col_id,
            "500",
            &DuckLakeType::Integer {
                signed: true,
                width_bits: 64,
            },
        )
        .await
        .unwrap();
    assert_eq!(kept, vec![file_id]);

    let pruned = reader
        .prune_files(
            table_id,
            col_id,
            "2000",
            &DuckLakeType::Integer {
                signed: true,
                width_bits: 64,
            },
        )
        .await
        .unwrap();
    assert!(pruned.is_empty());

    store.close().await.unwrap();
}

// ─── Phase 2: Table Column Stats ─────────────────────────────────────────────

#[test]
fn table_column_stats_row_has_v026_fields() {
    let row = TableColumnStatsRow {
        table_id: 1,
        column_id: 2,
        contains_null: false,
        min_value: Some("a".to_string()),
        max_value: Some("z".to_string()),
        contains_nan: Some(true),
        extra_stats: Some(r#"{"key":"value"}"#.to_string()),
    };
    assert!(!row.contains_null);
    assert_eq!(row.contains_nan, Some(true));
    assert!(row.extra_stats.is_some());
}

#[tokio::test]
async fn upsert_table_column_stats_v026() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let col_id = writer
        .add_column(table_id, "score", "float64", 0, true, None)
        .await
        .unwrap();

    writer
        .upsert_table_column_stats(
            table_id,
            col_id,
            false,
            Some("0.0"),
            Some("99.9"),
            Some(true),
            None,
        )
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    store.close().await.unwrap();
}

// ─── Phase 3: Variant Stats ───────────────────────────────────────────────────

#[test]
fn file_variant_stats_row_has_v026_fields() {
    #[allow(deprecated)]
    let row = FileVariantStatsRow {
        table_id: 1,
        column_id: 2,
        deprecated_variant_path_hash: None,
        data_file_id: 3,
        variant_key: "$.name".to_string(),
        min_value: Some("alice".to_string()),
        max_value: Some("zoe".to_string()),
        shredded_type: Some("varchar".to_string()),
        column_size_bytes: Some(2048),
        value_count: Some(500),
        null_count: Some(0),
        contains_nan: Some(false),
        extra_stats: None,
    };
    assert_eq!(row.variant_key, "$.name");
    assert_eq!(row.shredded_type.as_deref(), Some("varchar"));
    assert_eq!(row.column_size_bytes, Some(2048));
    assert_eq!(row.value_count, Some(500));
    assert_eq!(row.null_count, Some(0));
    assert_eq!(row.contains_nan, Some(false));
}

#[tokio::test]
async fn upsert_file_variant_stats_v026() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let col_id = writer
        .add_column(table_id, "doc", "variant", 0, true, None)
        .await
        .unwrap();
    let file_id = writer
        .register_data_file(table_id, "v1.parquet", "parquet", 100, 1024)
        .await
        .unwrap();

    writer
        .upsert_file_variant_stats(FileVariantStatsInput {
            table_id,
            column_id: col_id,
            variant_key: "$.name",
            data_file_id: file_id,
            min_value: Some("alice"),
            max_value: Some("zoe"),
            shredded_type: Some("varchar"),
            column_size_bytes: Some(1024),
            value_count: Some(100),
            null_count: Some(0),
            contains_nan: None,
            extra_stats: None,
        })
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let stats = reader
        .list_file_variant_stats(table_id, col_id)
        .await
        .unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].variant_key, "$.name");
    assert_eq!(stats[0].shredded_type.as_deref(), Some("varchar"));
    assert_eq!(stats[0].column_size_bytes, Some(1024));
    assert_eq!(stats[0].value_count, Some(100));
    assert_eq!(stats[0].null_count, Some(0));

    store.close().await.unwrap();
}

// ─── Phase 4: Geometry Stats ──────────────────────────────────────────────────

#[test]
fn geometry_extra_stats_serialize_deserialize() {
    let geo = GeometryExtraStats {
        min_x: Some(-180.0),
        max_x: Some(180.0),
        min_y: Some(-90.0),
        max_y: Some(90.0),
        min_z: None,
        max_z: None,
        min_m: None,
        max_m: None,
        geometry_type: Some("POLYGON".to_string()),
        srid: Some(4326),
    };
    let json = geo.to_json().unwrap();
    let decoded = GeometryExtraStats::from_json(&json).unwrap();
    assert_eq!(decoded.min_x, Some(-180.0));
    assert_eq!(decoded.max_x, Some(180.0));
    assert_eq!(decoded.geometry_type.as_deref(), Some("POLYGON"));
    assert_eq!(decoded.srid, Some(4326));
    // min_z/max_z/min_m/max_m should be absent from JSON (skip_serializing_if)
    assert!(!json.contains("min_z"));
    assert!(!json.contains("min_m"));
}

#[test]
fn geometry_extra_stats_validate_ok() {
    let geo = GeometryExtraStats {
        min_x: Some(-10.0),
        max_x: Some(10.0),
        min_y: Some(-5.0),
        max_y: Some(5.0),
        min_z: None,
        max_z: None,
        min_m: None,
        max_m: None,
        geometry_type: None,
        srid: None,
    };
    assert!(geo.validate().is_ok());
}

#[test]
fn geometry_extra_stats_validate_invalid_bbox() {
    let geo = GeometryExtraStats {
        min_x: Some(10.0),
        max_x: Some(-10.0), // invalid: min_x > max_x
        min_y: None,
        max_y: None,
        min_z: None,
        max_z: None,
        min_m: None,
        max_m: None,
        geometry_type: None,
        srid: None,
    };
    assert!(geo.validate().is_err());
}

#[test]
fn geometry_extra_stats_prune_by_point() {
    let geo = GeometryExtraStats {
        min_x: Some(0.0),
        max_x: Some(10.0),
        min_y: Some(0.0),
        max_y: Some(10.0),
        min_z: None,
        max_z: None,
        min_m: None,
        max_m: None,
        geometry_type: None,
        srid: None,
    };
    assert_eq!(geo.prune_by_point(5.0, 5.0), PruneResult::Keep);
    assert_eq!(geo.prune_by_point(15.0, 5.0), PruneResult::Prune);
    assert_eq!(geo.prune_by_point(5.0, -1.0), PruneResult::Prune);
}

#[test]
fn validate_extra_stats_valid_json() {
    assert!(validate_extra_stats(None).is_ok());
    assert!(validate_extra_stats(Some("")).is_ok());
    assert!(validate_extra_stats(Some(r#"{"key":"value"}"#)).is_ok());
}

#[test]
fn validate_extra_stats_invalid_json() {
    assert!(validate_extra_stats(Some("not-json")).is_err());
    assert!(validate_extra_stats(Some("{unclosed")).is_err());
}

// ─── Phase 5: DuckLake Type Parser ────────────────────────────────────────────

#[test]
fn parse_signed_integers() {
    assert_eq!(
        DuckLakeType::parse("int8"),
        DuckLakeType::Integer {
            signed: true,
            width_bits: 8
        }
    );
    assert_eq!(
        DuckLakeType::parse("int32"),
        DuckLakeType::Integer {
            signed: true,
            width_bits: 32
        }
    );
    assert_eq!(
        DuckLakeType::parse("int64"),
        DuckLakeType::Integer {
            signed: true,
            width_bits: 64
        }
    );
    assert_eq!(
        DuckLakeType::parse("bigint"),
        DuckLakeType::Integer {
            signed: true,
            width_bits: 64
        }
    );
    assert_eq!(
        DuckLakeType::parse("hugeint"),
        DuckLakeType::Integer {
            signed: true,
            width_bits: 128
        }
    );
}

#[test]
fn parse_unsigned_integers() {
    assert_eq!(
        DuckLakeType::parse("uint8"),
        DuckLakeType::Integer {
            signed: false,
            width_bits: 8
        }
    );
    assert_eq!(
        DuckLakeType::parse("uint64"),
        DuckLakeType::Integer {
            signed: false,
            width_bits: 64
        }
    );
}

#[test]
fn parse_floats() {
    assert_eq!(
        DuckLakeType::parse("float"),
        DuckLakeType::Float { width_bits: 32 }
    );
    assert_eq!(
        DuckLakeType::parse("double"),
        DuckLakeType::Float { width_bits: 64 }
    );
    assert_eq!(
        DuckLakeType::parse("float64"),
        DuckLakeType::Float { width_bits: 64 }
    );
}

#[test]
fn parse_decimal_with_precision_scale() {
    assert_eq!(
        DuckLakeType::parse("decimal(18,3)"),
        DuckLakeType::Decimal {
            precision: 18,
            scale: 3
        }
    );
    assert_eq!(
        DuckLakeType::parse("numeric(10,2)"),
        DuckLakeType::Decimal {
            precision: 10,
            scale: 2
        }
    );
}

#[test]
fn parse_timestamp_variants() {
    assert_eq!(
        DuckLakeType::parse("timestamp"),
        DuckLakeType::Timestamp {
            with_timezone: false,
            precision: 6
        }
    );
    assert_eq!(
        DuckLakeType::parse("timestamp_s"),
        DuckLakeType::Timestamp {
            with_timezone: false,
            precision: 0
        }
    );
    assert_eq!(
        DuckLakeType::parse("timestamp_ms"),
        DuckLakeType::Timestamp {
            with_timezone: false,
            precision: 3
        }
    );
    assert_eq!(
        DuckLakeType::parse("timestamp_us"),
        DuckLakeType::Timestamp {
            with_timezone: false,
            precision: 6
        }
    );
    assert_eq!(
        DuckLakeType::parse("timestamp_ns"),
        DuckLakeType::Timestamp {
            with_timezone: false,
            precision: 9
        }
    );
    assert_eq!(
        DuckLakeType::parse("timestamptz"),
        DuckLakeType::Timestamp {
            with_timezone: true,
            precision: 6
        }
    );
}

#[test]
fn parse_date_time_types() {
    assert_eq!(DuckLakeType::parse("date"), DuckLakeType::Date);
    assert_eq!(
        DuckLakeType::parse("time"),
        DuckLakeType::Time {
            with_timezone: false
        }
    );
    assert_eq!(DuckLakeType::parse("interval"), DuckLakeType::Interval);
}

#[test]
fn parse_string_and_binary_types() {
    assert_eq!(DuckLakeType::parse("varchar"), DuckLakeType::Varchar);
    assert_eq!(DuckLakeType::parse("text"), DuckLakeType::Varchar);
    assert_eq!(DuckLakeType::parse("boolean"), DuckLakeType::Boolean);
    assert_eq!(DuckLakeType::parse("uuid"), DuckLakeType::Uuid);
    assert_eq!(DuckLakeType::parse("blob"), DuckLakeType::Blob);
    assert_eq!(DuckLakeType::parse("json"), DuckLakeType::Json);
    assert_eq!(DuckLakeType::parse("variant"), DuckLakeType::Variant);
    assert_eq!(DuckLakeType::parse("geometry"), DuckLakeType::Geometry);
}

#[test]
fn parse_list_type() {
    assert_eq!(
        DuckLakeType::parse("list<int32>"),
        DuckLakeType::List(Box::new(DuckLakeType::Integer {
            signed: true,
            width_bits: 32
        }))
    );
    // Nested list
    assert_eq!(
        DuckLakeType::parse("list<list<varchar>>"),
        DuckLakeType::List(Box::new(DuckLakeType::List(Box::new(
            DuckLakeType::Varchar
        ))))
    );
}

#[test]
fn parse_struct_type() {
    let t = DuckLakeType::parse("struct<x:float,y:float>");
    assert_eq!(
        t,
        DuckLakeType::Struct(vec![
            ("x".to_string(), DuckLakeType::Float { width_bits: 32 }),
            ("y".to_string(), DuckLakeType::Float { width_bits: 32 }),
        ])
    );
}

#[test]
fn parse_map_type() {
    let t = DuckLakeType::parse("map<varchar,int64>");
    assert_eq!(
        t,
        DuckLakeType::Map {
            key: Box::new(DuckLakeType::Varchar),
            value: Box::new(DuckLakeType::Integer {
                signed: true,
                width_bits: 64
            }),
        }
    );
}

#[test]
fn parse_unknown_type_falls_back() {
    let t = DuckLakeType::parse("custom_type_xyz");
    assert!(matches!(t, DuckLakeType::Unknown(_)));
}

// ─── Phase 6: Nested Column Tree Reads ────────────────────────────────────────

#[tokio::test]
async fn nested_columns_tree_ordering() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();

    // Top-level struct column
    let struct_col = writer
        .add_column(
            table_id,
            "address",
            "struct<street:varchar,city:varchar>",
            0,
            true,
            None,
        )
        .await
        .unwrap();
    // Child columns
    let _street = writer
        .add_column_with_opts(
            table_id,
            "street",
            "varchar",
            1,
            true,
            None,
            None,
            None,
            None,
            Some(struct_col),
        )
        .await
        .unwrap();
    let _city = writer
        .add_column_with_opts(
            table_id,
            "city",
            "varchar",
            2,
            true,
            None,
            None,
            None,
            None,
            Some(struct_col),
        )
        .await
        .unwrap();
    // Another top-level column
    let _age = writer
        .add_column(table_id, "age", "int32", 3, false, None)
        .await
        .unwrap();

    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let (_, columns) = reader.describe_table(table_id).await.unwrap().unwrap();

    // address (parent=None), street (parent=address), city (parent=address), age (parent=None)
    assert_eq!(columns.len(), 4);
    assert_eq!(columns[0].column_name, "address");
    assert!(columns[0].parent_column.is_none());
    // street and city follow address
    let struct_followers: Vec<&str> = columns[1..3]
        .iter()
        .map(|c| c.column_name.as_str())
        .collect();
    assert!(struct_followers.contains(&"street"));
    assert!(struct_followers.contains(&"city"));
    // Both children have parent set to struct_col
    assert!(columns[1..3]
        .iter()
        .all(|c| c.parent_column == Some(struct_col)));
    assert_eq!(columns[3].column_name, "age");

    store.close().await.unwrap();
}

// ─── Phase 7: Sort Expression Spec Fields ────────────────────────────────────

#[test]
fn sort_expression_row_has_v026_fields() {
    let row = SortExpressionRow {
        sort_id: 1,
        sort_key_index: 0,
        column_id: 5,
        sort_direction: Some("ASC".to_string()),
        null_order: Some("NULLS LAST".to_string()),
        table_id: Some(10),
        expression: None,
        dialect: None,
    };
    assert_eq!(row.sort_direction.as_deref(), Some("ASC"));
    assert_eq!(row.null_order.as_deref(), Some("NULLS LAST"));
    assert_eq!(row.table_id, Some(10));
}

#[test]
fn sort_expression_row_with_expression() {
    let row = SortExpressionRow {
        sort_id: 2,
        sort_key_index: 1,
        column_id: 0,
        sort_direction: Some("DESC".to_string()),
        null_order: Some("NULLS FIRST".to_string()),
        table_id: Some(20),
        expression: Some("lower(name)".to_string()),
        dialect: Some("duckdb".to_string()),
    };
    assert_eq!(row.expression.as_deref(), Some("lower(name)"));
    assert_eq!(row.dialect.as_deref(), Some("duckdb"));
}

// ─── Phase 8: Partition Column table_id ──────────────────────────────────────

#[test]
fn partition_column_row_has_table_id() {
    let row = PartitionColumnRow {
        partition_id: 1,
        partition_key_index: 0,
        column_id: 5,
        transform: Some("YEAR".to_string()),
        table_id: Some(42),
    };
    assert_eq!(row.table_id, Some(42));
    assert_eq!(row.transform.as_deref(), Some("YEAR"));
}

// ─── Phase 9: File Partition Value rename ────────────────────────────────────

#[test]
fn file_partition_value_row_uses_partition_value() {
    let row = FilePartitionValueRow {
        table_id: 1,
        partition_key_index: 0,
        data_file_id: 3,
        partition_value: Some("2024-01-01".to_string()),
    };
    assert_eq!(row.partition_value.as_deref(), Some("2024-01-01"));
}

// ─── Phase 10: Files Scheduled for Deletion ───────────────────────────────────

#[test]
fn files_scheduled_for_deletion_optional_file_type() {
    let row = FilesScheduledForDeletionRow {
        data_file_id: 1,
        schedule_start: 1_700_000_000,
        path: "s3://bucket/old.parquet".to_string(),
        file_type: Some("data".to_string()),
        path_is_relative: Some(false),
    };
    assert_eq!(row.file_type.as_deref(), Some("data"));
    assert_eq!(row.path_is_relative, Some(false));
}

#[test]
fn files_scheduled_for_deletion_no_file_type() {
    // file_type is now optional
    let row = FilesScheduledForDeletionRow {
        data_file_id: 2,
        schedule_start: 1_700_000_001,
        path: "s3://bucket/other.parquet".to_string(),
        file_type: None,
        path_is_relative: None,
    };
    assert!(row.file_type.is_none());
    assert!(row.path_is_relative.is_none());
}

#[tokio::test]
async fn schedule_file_deletion_writer_v026() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let file_id = writer
        .register_data_file(table_id, "old.parquet", "parquet", 10, 500)
        .await
        .unwrap();

    writer
        .schedule_file_deletion(file_id, "old.parquet", "data")
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();
    let scheduled = reader.list_files_scheduled_for_deletion().await.unwrap();
    assert_eq!(scheduled.len(), 1);
    assert_eq!(scheduled[0].data_file_id, file_id);
    assert_eq!(scheduled[0].path, "old.parquet");
    // file_type is now optional
    assert_eq!(scheduled[0].file_type.as_deref(), Some("data"));

    store.close().await.unwrap();
}

// ─── Phase 11: Partial-max Pruning ───────────────────────────────────────────

#[tokio::test]
async fn partial_max_pruning_shortcut() {
    let dir = TempDir::new().unwrap();
    let mut store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let mut writer = store.begin_write();

    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer.create_table(schema_id, "t", None).await.unwrap();
    let col_id = writer
        .add_column(table_id, "row_num", "int64", 0, false, None)
        .await
        .unwrap();

    // Register two files: one full, one partial (partial_max=50)
    let full_file = writer
        .register_data_file(table_id, "full.parquet", "parquet", 100, 8192)
        .await
        .unwrap();
    let partial_file = writer
        .register_data_file_partial(table_id, "partial.parquet", "parquet", 50, 4096, Some("50"))
        .await
        .unwrap();

    let int_type = DuckLakeType::Integer {
        signed: true,
        width_bits: 64,
    };

    // Both files: min=1, max=100
    writer
        .upsert_file_column_stats(FileColumnStatsInput {
            table_id,
            column_id: col_id,
            data_file_id: full_file,
            contains_null: false,
            min_value: Some("1"),
            max_value: Some("100"),
            contains_nan: false,
            column_size_bytes: None,
            value_count: None,
            null_count: None,
            extra_stats: None,
        })
        .await
        .unwrap();
    writer
        .upsert_file_column_stats(FileColumnStatsInput {
            table_id,
            column_id: col_id,
            data_file_id: partial_file,
            contains_null: false,
            min_value: Some("1"),
            max_value: Some("100"),
            contains_nan: false,
            column_size_bytes: None,
            value_count: None,
            null_count: None,
            extra_stats: None,
        })
        .await
        .unwrap();
    let _snap = writer.create_snapshot(None, None).await.unwrap();

    let reader = store.read_at(SnapshotId::new(1)).unwrap();

    // Predicate=30: within partial_max=50 → both kept
    let kept = reader
        .prune_files(table_id, col_id, "30", &int_type)
        .await
        .unwrap();
    assert!(kept.contains(&full_file));
    assert!(kept.contains(&partial_file));

    // Predicate=75: exceeds partial_max=50 → partial file pruned, full file kept
    let kept = reader
        .prune_files(table_id, col_id, "75", &int_type)
        .await
        .unwrap();
    assert!(kept.contains(&full_file));
    assert!(
        !kept.contains(&partial_file),
        "partial file should be pruned for predicate > partial_max"
    );

    store.close().await.unwrap();
}
