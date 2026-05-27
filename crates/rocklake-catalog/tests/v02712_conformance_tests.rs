//! v0.27.12 data-file and delete-file conformance tests.
//!
//! Verifies that `register_data_file_with_metadata` and
//! `register_delete_file_with_metadata` correctly persist and expose all
//! DuckLake v1.0 spec fields: `footer_size`, `encryption_key`, `partition_id`,
//! `mapping_id`, and `partial_max`.

use std::sync::Arc;

use object_store::memory::InMemory;
use object_store::path::Path as ObjectPath;
use rocklake_catalog::{CatalogStore, OpenOptions};

fn make_opts() -> (Arc<dyn object_store::ObjectStore>, OpenOptions) {
    let store: Arc<dyn object_store::ObjectStore> = Arc::new(InMemory::new());
    let opts = OpenOptions {
        object_store: Arc::clone(&store),
        path: ObjectPath::from("v02712-conformance"),
        encryption: None,
    };
    (store, opts)
}

// ─── Test 1: data file with all extended fields round-trips correctly ──────

#[tokio::test]
async fn data_file_extended_fields_round_trip() {
    let (_, opts) = make_opts();
    let mut cat = CatalogStore::open(opts).await.unwrap();

    let mut writer = cat.begin_write();
    let schema_id = writer.create_schema("public").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "fact_table", None)
        .await
        .unwrap();

    let data_file_id = writer
        .register_data_file_with_metadata(
            table_id,
            "data/fact_table/part-0001.parquet",
            "parquet",
            1_000,
            65_536,
            Some(4096),             // footer_size
            Some("enc-key-abc123"), // encryption_key
            None,                   // partition_id
            None,                   // mapping_id
            Some("2024-12-31"),     // partial_max
        )
        .await
        .unwrap();

    let snap = writer
        .create_snapshot(Some("v02712-test"), Some("data_file_extended"))
        .await
        .unwrap();
    cat.commit_writer(snap);

    let reader = cat.read_latest();
    let files = reader.list_data_files(table_id).await.unwrap();
    assert_eq!(files.len(), 1, "expected exactly one data file");

    let f = &files[0];
    assert_eq!(f.data_file_id, data_file_id);
    assert_eq!(f.footer_size, Some(4096), "footer_size must round-trip");
    assert_eq!(
        f.encryption_key.as_deref(),
        Some("enc-key-abc123"),
        "encryption_key must round-trip"
    );
    assert_eq!(f.partition_id, None, "partition_id should be None");
    assert_eq!(f.mapping_id, None, "mapping_id should be None");
    assert_eq!(
        f.partial_max.as_deref(),
        Some("2024-12-31"),
        "partial_max must round-trip"
    );
}

// ─── Test 2: delete file with extended fields round-trips correctly ─────────

#[tokio::test]
async fn delete_file_extended_fields_round_trip() {
    let (_, opts) = make_opts();
    let mut cat = CatalogStore::open(opts).await.unwrap();

    let mut writer = cat.begin_write();
    let schema_id = writer.create_schema("public").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "del_table", None)
        .await
        .unwrap();
    let data_file_id = writer
        .register_data_file_with_metadata(
            table_id,
            "data/del_table/part-0001.parquet",
            "parquet",
            500,
            32_768,
            Some(512),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

    let delete_file_id = writer
        .register_delete_file_with_metadata(
            data_file_id,
            "data/del_table/part-0001.del.parquet",
            100,
            8_192,
            Some(256),           // footer_size
            Some("upper-bound"), // partial_max
        )
        .await
        .unwrap();

    let snap = writer
        .create_snapshot(Some("v02712-test"), Some("delete_file_extended"))
        .await
        .unwrap();
    cat.commit_writer(snap);

    let reader = cat.read_latest();
    let del_files = reader.list_delete_files(table_id).await.unwrap();
    assert_eq!(del_files.len(), 1, "expected exactly one delete file");

    let df = &del_files[0];
    assert_eq!(df.delete_file_id, delete_file_id);
    assert_eq!(df.footer_size, Some(256), "footer_size must round-trip");
    assert_eq!(
        df.partial_max.as_deref(),
        Some("upper-bound"),
        "partial_max must round-trip"
    );
}

// ─── Test 3: data file partition_id and mapping_id round-trip ──────────────

#[tokio::test]
async fn data_file_partition_and_mapping_round_trip() {
    let (_, opts) = make_opts();
    let mut cat = CatalogStore::open(opts).await.unwrap();

    let mut writer = cat.begin_write();
    let schema_id = writer.create_schema("public").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "partitioned_table", None)
        .await
        .unwrap();

    let data_file_id = writer
        .register_data_file_with_metadata(
            table_id,
            "data/partitioned_table/year=2024/part-0001.parquet",
            "parquet",
            10_000,
            1_048_576,
            None,
            None,
            Some(42), // partition_id
            Some(7),  // mapping_id
            None,
        )
        .await
        .unwrap();

    let snap = writer
        .create_snapshot(Some("v02712-test"), Some("partition_mapping"))
        .await
        .unwrap();
    cat.commit_writer(snap);

    let reader = cat.read_latest();
    let files = reader.list_data_files(table_id).await.unwrap();
    assert_eq!(files.len(), 1);

    let f = &files[0];
    assert_eq!(f.data_file_id, data_file_id);
    assert_eq!(f.partition_id, Some(42), "partition_id must round-trip");
    assert_eq!(f.mapping_id, Some(7), "mapping_id must round-trip");
    assert_eq!(f.footer_size, None, "footer_size should be None");
    assert_eq!(f.encryption_key, None, "encryption_key should be None");
}

// ─── Test 4: backward compat — original register_data_file still works ──────

#[tokio::test]
async fn original_register_data_file_still_works() {
    let (_, opts) = make_opts();
    let mut cat = CatalogStore::open(opts).await.unwrap();

    let mut writer = cat.begin_write();
    let schema_id = writer.create_schema("public").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "compat_table", None)
        .await
        .unwrap();

    let data_file_id = writer
        .register_data_file(
            table_id,
            "data/compat_table/part.parquet",
            "parquet",
            100,
            4096,
        )
        .await
        .unwrap();

    let snap = writer
        .create_snapshot(Some("v02712-test"), Some("compat"))
        .await
        .unwrap();
    cat.commit_writer(snap);

    let reader = cat.read_latest();
    let files = reader.list_data_files(table_id).await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].data_file_id, data_file_id);
    // Extended fields are None for files registered via the old API.
    assert_eq!(files[0].footer_size, None);
    assert_eq!(files[0].encryption_key, None);
    assert_eq!(files[0].partition_id, None);
    assert_eq!(files[0].mapping_id, None);
    assert_eq!(files[0].partial_max, None);
}
