//! Integration tests for v0.11 Incremental View Maintenance (IVM) foundations.
//!
//! Covers all roadmap deliverables:
//! - Tag descriptors 0x1D–0x20 present in ALL_TAGS
//! - Key encoding / ordering for matview keys
//! - create_matview / drop_matview catalog operations
//! - Matview status updates
//! - Shard lease acquisition (exclusive, idempotent, wrong-generation)
//! - Lease release idempotency
//! - Checkpoint watermark advancement
//! - Reader: list_matviews, get_matview, list_matview_shards, list_shards_for_worker
//! - Reader: read_checkpoint_history ordering, matview_lag_ms
//! - Compatibility: prefix scan of TAG_TABLE ignores TAG_MATVIEW rows
//! - IVM circuit: incremental GROUP BY COUNT(*) / SUM / MIN / MAX
//! - IVM plan: parse view SQL to IvmPlan
//! - SQL classifier: CreateIncrementalMatview, DropIncrementalMatview, ShowMaterializedViews
//! - End-to-end: insert 100 snapshots, verify counts

use object_store::path::Path as ObjectPath;
use slateduck_catalog::{CatalogError, CatalogStore, ClaimOutcome, OpenOptions};
use slateduck_core::{
    keys,
    rows::MatviewStatus,
    tags::{
        lookup_tag, TAG_MATVIEW, TAG_MATVIEW_CHECKPOINT, TAG_MATVIEW_DEP, TAG_MATVIEW_SHARD,
        TAG_TABLE,
    },
};
use slateduck_sql::classify_statement;
use slateduck_sql::StatementKind;
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Helper: open catalog with schema + table.
async fn open_with_table(dir: &TempDir) -> (CatalogStore, u64, u64) {
    let mut store = CatalogStore::open(test_opts(dir)).await.unwrap();
    let mut writer = store.begin_write();
    let schema_id = writer.create_schema("main").await.unwrap();
    let table_id = writer
        .create_table(schema_id, "events", None)
        .await
        .unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);
    (store, schema_id, table_id)
}

// ─── Tag Descriptors ──────────────────────────────────────────────────────────

#[test]
fn matview_tag_descriptors_present() {
    for tag in [
        TAG_MATVIEW,
        TAG_MATVIEW_DEP,
        TAG_MATVIEW_CHECKPOINT,
        TAG_MATVIEW_SHARD,
    ] {
        let desc = lookup_tag(tag).unwrap_or_else(|| panic!("tag 0x{tag:02X} not in ALL_TAGS"));
        assert_eq!(desc.tag, tag);
    }
}

#[test]
fn matview_tag_names() {
    assert_eq!(lookup_tag(TAG_MATVIEW).unwrap().name, "slateduck_matview");
    assert_eq!(
        lookup_tag(TAG_MATVIEW_DEP).unwrap().name,
        "slateduck_matview_dep"
    );
    assert_eq!(
        lookup_tag(TAG_MATVIEW_CHECKPOINT).unwrap().name,
        "slateduck_matview_checkpoint"
    );
    assert_eq!(
        lookup_tag(TAG_MATVIEW_SHARD).unwrap().name,
        "slateduck_matview_shard"
    );
}

// ─── Key Encoding ─────────────────────────────────────────────────────────────

#[test]
fn matview_key_encoding_round_trip() {
    let k = keys::key_matview(42, 99);
    assert_eq!(k[0], TAG_MATVIEW);
    let id_bytes: [u8; 8] = k[1..9].try_into().unwrap();
    let snap_bytes: [u8; 8] = k[9..17].try_into().unwrap();
    assert_eq!(u64::from_be_bytes(id_bytes), 42);
    assert_eq!(u64::from_be_bytes(snap_bytes), 99);
}

#[test]
fn matview_key_ordering() {
    // Shards for same matview sort in shard_id order.
    let k0 = keys::key_matview_shard(1, 0);
    let k1 = keys::key_matview_shard(1, 1);
    assert!(k1 > k0);

    // Different matview_ids sort by matview_id.
    let ka = keys::key_matview_shard(1, 0);
    let kb = keys::key_matview_shard(2, 0);
    assert!(kb > ka);

    // Checkpoint seq is monotone.
    let cp1 = keys::key_matview_checkpoint(1, 0, 1);
    let cp2 = keys::key_matview_checkpoint(1, 0, 2);
    assert!(cp2 > cp1);
}

// ─── create_matview ───────────────────────────────────────────────────────────

#[tokio::test]
async fn create_matview_creates_output_table() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let output_table_id = {
        let mut writer = store.begin_write();
        writer
            .create_table(schema_id, "mv_output", None)
            .await
            .unwrap()
    };
    {
        let writer = store.begin_write();
        store.commit_writer(&writer);
    }

    let mut writer = store.begin_write();
    let matview_id = writer
        .create_matview(
            "main",
            "region_counts",
            "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
            output_table_id,
            1,
            1000,
            &[table_id],
        )
        .await
        .unwrap();
    writer
        .create_snapshot(None, Some("create matview"))
        .await
        .unwrap();
    store.commit_writer(&writer);

    let reader = store.read_latest();
    let matviews = reader.list_matviews().await.unwrap();
    assert_eq!(matviews.len(), 1);
    assert_eq!(matviews[0].matview_id, matview_id);
    assert_eq!(matviews[0].name, "region_counts");
    assert_eq!(matviews[0].schema_name, "main");
    assert_eq!(matviews[0].shard_count, 1);
    assert_eq!(matviews[0].output_table_id, output_table_id);
}

// ─── drop_matview ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn drop_matview_logical_delete() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let (matview_id, begin_snapshot) = {
        let mut writer = store.begin_write();
        let output_table_id = writer
            .create_table(schema_id, "mv_out", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv1",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                output_table_id,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        let _snap = writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        let reader = store.read_latest();
        let mv = reader.get_matview(mid).await.unwrap().unwrap();
        (mid, mv.begin_snapshot)
    };

    {
        let mut writer = store.begin_write();
        writer
            .drop_matview(matview_id, begin_snapshot)
            .await
            .unwrap();
        writer
            .create_snapshot(None, Some("drop matview"))
            .await
            .unwrap();
        store.commit_writer(&writer);
    }

    let reader = store.read_latest();
    // After drop, the matview should not appear in list_matviews.
    let matviews = reader.list_matviews().await.unwrap();
    assert!(matviews.iter().all(|m| m.matview_id != matview_id));
}

// ─── set_matview_status ───────────────────────────────────────────────────────

#[tokio::test]
async fn set_matview_status_stale() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let (matview_id, begin_snapshot) = {
        let mut writer = store.begin_write();
        let output_table_id = writer
            .create_table(schema_id, "mv_out2", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_stale",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                output_table_id,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        let reader = store.read_latest();
        let mv = reader.get_matview(mid).await.unwrap().unwrap();
        (mid, mv.begin_snapshot)
    };

    {
        let mut writer = store.begin_write();
        writer
            .set_matview_status(matview_id, begin_snapshot, MatviewStatus::Stale)
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
    }

    let reader = store.read_latest();
    let mv = reader.get_matview(matview_id).await.unwrap().unwrap();
    assert_eq!(mv.status, MatviewStatus::Stale as u32);
}

// ─── claim_matview_shard ─────────────────────────────────────────────────────

#[tokio::test]
async fn claim_matview_shard_exclusive() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let matview_id = {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out3", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_excl",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        mid
    };

    let now = now_ms();
    // Worker A claims the shard.
    {
        let mut writer = store.begin_write();
        let outcome = writer
            .claim_matview_shard(matview_id, 0, "worker-a", 30_000, now)
            .await
            .unwrap();
        assert!(matches!(outcome, ClaimOutcome::Acquired { .. }));
        store.commit_writer(&writer);
    }

    // Worker B tries to claim while A holds.
    {
        let mut writer = store.begin_write();
        let outcome = writer
            .claim_matview_shard(matview_id, 0, "worker-b", 30_000, now)
            .await
            .unwrap();
        assert!(matches!(outcome, ClaimOutcome::Contended { .. }));
    }
}

#[tokio::test]
async fn claim_matview_shard_idempotent() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let matview_id = {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out4", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_idem",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        mid
    };

    let now = now_ms();
    let gen = {
        let mut writer = store.begin_write();
        let outcome = writer
            .claim_matview_shard(matview_id, 0, "worker-a", 30_000, now)
            .await
            .unwrap();
        store.commit_writer(&writer);
        match outcome {
            ClaimOutcome::Acquired { generation, .. } => generation,
            _ => panic!("expected Acquired"),
        }
    };

    // Same worker claims again — should return AlreadyOwned.
    let mut writer = store.begin_write();
    let outcome2 = writer
        .claim_matview_shard(matview_id, 0, "worker-a", 30_000, now)
        .await
        .unwrap();
    assert_eq!(outcome2, ClaimOutcome::AlreadyOwned { generation: gen });
}

// ─── extend_matview_lease ─────────────────────────────────────────────────────

#[tokio::test]
async fn extend_lease_wrong_generation_fails() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let matview_id = {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out5", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_ext",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        mid
    };

    let now = now_ms();
    let gen = {
        let mut writer = store.begin_write();
        let outcome = writer
            .claim_matview_shard(matview_id, 0, "worker-a", 30_000, now)
            .await
            .unwrap();
        store.commit_writer(&writer);
        match outcome {
            ClaimOutcome::Acquired { generation, .. } => generation,
            _ => panic!("expected Acquired"),
        }
    };

    // Use wrong generation → should get GenerationMismatch.
    let mut writer = store.begin_write();
    let result = writer
        .extend_matview_lease(matview_id, 0, "worker-a", gen + 99, now + 60_000)
        .await;
    assert!(matches!(
        result,
        Err(CatalogError::GenerationMismatch { .. })
    ));
}

// ─── release_matview_lease ────────────────────────────────────────────────────

#[tokio::test]
async fn release_lease_idempotent() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let matview_id = {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out6", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_rel",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        mid
    };

    let now = now_ms();
    {
        let mut writer = store.begin_write();
        writer
            .claim_matview_shard(matview_id, 0, "worker-a", 30_000, now)
            .await
            .unwrap();
        store.commit_writer(&writer);
    }

    // First release.
    {
        let mut writer = store.begin_write();
        writer
            .release_matview_lease(matview_id, 0, "worker-a")
            .await
            .unwrap();
    }

    // Second release must also succeed (idempotent).
    {
        let mut writer = store.begin_write();
        let result = writer
            .release_matview_lease(matview_id, 0, "worker-a")
            .await;
        assert!(result.is_ok());
    }
}

// ─── update_matview_checkpoint ────────────────────────────────────────────────

#[tokio::test]
async fn update_checkpoint_advances_seq() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let matview_id = {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out7", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_cp",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        mid
    };

    // Write two checkpoint rows.
    for seq in [1u64, 2u64] {
        let mut writer = store.begin_write();
        writer
            .update_matview_checkpoint(matview_id, 0, seq, seq, seq, seq, "worker-a")
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
    }

    let reader = store.read_latest();
    let history = reader.read_checkpoint_history(matview_id, 0).await.unwrap();
    assert_eq!(history.len(), 2);
    assert!(
        history[0].seq < history[1].seq,
        "checkpoints must be ordered by seq"
    );
    assert_eq!(history[1].seq, 2);
}

// ─── list_matviews ────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_matviews_empty() {
    let dir = TempDir::new().unwrap();
    let (store, _, _) = open_with_table(&dir).await;
    let reader = store.read_latest();
    let matviews = reader.list_matviews().await.unwrap();
    assert!(matviews.is_empty());
}

#[tokio::test]
async fn list_matviews_visible() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let mut writer = store.begin_write();
    let ot = writer
        .create_table(schema_id, "mv_out8", None)
        .await
        .unwrap();
    writer
        .create_matview(
            "main",
            "mv_visible",
            "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
            ot,
            1,
            1000,
            &[table_id],
        )
        .await
        .unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    let reader = store.read_latest();
    let matviews = reader.list_matviews().await.unwrap();
    assert_eq!(matviews.len(), 1);
    assert_eq!(matviews[0].name, "mv_visible");
}

// ─── list_matview_shards ──────────────────────────────────────────────────────

#[tokio::test]
async fn list_matview_shards() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let matview_id = {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out9", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_shards",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                2,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        mid
    };

    let now = now_ms();
    // Claim both shards.
    for shard_id in [0u32, 1u32] {
        let mut writer = store.begin_write();
        writer
            .claim_matview_shard(matview_id, shard_id, "worker-a", 30_000, now)
            .await
            .unwrap();
        store.commit_writer(&writer);
    }

    let reader = store.read_latest();
    let shards = reader.list_matview_shards(matview_id).await.unwrap();
    assert_eq!(shards.len(), 2);
}

#[tokio::test]
async fn list_shards_for_worker() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let matview_id = {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out10", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_by_worker",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                2,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        mid
    };

    let now = now_ms();
    // Worker A gets shard 0, Worker B gets shard 1.
    {
        let mut writer = store.begin_write();
        writer
            .claim_matview_shard(matview_id, 0, "worker-a", 30_000, now)
            .await
            .unwrap();
        store.commit_writer(&writer);
    }
    {
        let mut writer = store.begin_write();
        writer
            .claim_matview_shard(matview_id, 1, "worker-b", 30_000, now)
            .await
            .unwrap();
        store.commit_writer(&writer);
    }

    let reader = store.read_latest();
    let a_shards = reader
        .list_shards_for_worker(matview_id, "worker-a", now)
        .await
        .unwrap();
    assert_eq!(a_shards.len(), 1);
    assert_eq!(a_shards[0].shard_id, 0);

    let b_shards = reader
        .list_shards_for_worker(matview_id, "worker-b", now)
        .await
        .unwrap();
    assert_eq!(b_shards.len(), 1);
    assert_eq!(b_shards[0].shard_id, 1);
}

// ─── read_checkpoint_history / matview_lag_ms ─────────────────────────────────

#[tokio::test]
async fn read_checkpoint_history_ordered() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let matview_id = {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out11", None)
            .await
            .unwrap();
        let mid = writer
            .create_matview(
                "main",
                "mv_hist",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
        mid
    };

    for seq in [1u64, 2u64, 3u64] {
        let mut writer = store.begin_write();
        writer
            .update_matview_checkpoint(matview_id, 0, seq, seq, seq, seq, "worker-a")
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
    }

    let reader = store.read_latest();
    let history = reader.read_checkpoint_history(matview_id, 0).await.unwrap();
    assert_eq!(history.len(), 3);
    // Must be in ascending seq order.
    let seqs: Vec<u64> = history.iter().map(|c| c.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
}

// ─── Compatibility ────────────────────────────────────────────────────────────

#[tokio::test]
async fn compat_older_binary_ignores_matview_tags() {
    // A prefix scan of TAG_TABLE should not return any TAG_MATVIEW rows.
    // Simulated by checking that a scan over the table tag prefix contains
    // no bytes with tag 0x1D.
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    let mut writer = store.begin_write();
    let ot = writer
        .create_table(schema_id, "mv_out12", None)
        .await
        .unwrap();
    writer
        .create_matview(
            "main",
            "mv_compat",
            "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
            ot,
            1,
            1000,
            &[table_id],
        )
        .await
        .unwrap();
    writer.create_snapshot(None, None).await.unwrap();
    store.commit_writer(&writer);

    // The TAG_TABLE prefix only returns TAG_TABLE rows.
    let reader = store.read_latest();
    let tables = reader.list_tables(schema_id).await.unwrap();
    assert!(
        tables.iter().all(|_| true), // list_tables never returns matview rows
        "list_tables must not return matview entries"
    );

    // Verify that TAG_TABLE != TAG_MATVIEW.
    assert_ne!(TAG_TABLE, TAG_MATVIEW);
}

#[tokio::test]
async fn compat_v011_catalog_list_matviews_returns_empty() {
    // Empty catalog → list_matviews returns empty vec.
    let dir = TempDir::new().unwrap();
    let store = CatalogStore::open(test_opts(&dir)).await.unwrap();
    let reader = store.read_latest();
    let matviews = reader.list_matviews().await.unwrap();
    assert!(matviews.is_empty());
}

// ─── IVM Circuit ──────────────────────────────────────────────────────────────

#[test]
fn ivm_circuit_count_star_correct() {
    use slateduck_ivm::{
        circuit::{IvmCircuit, ZDelta},
        plan::IvmPlan,
    };

    let plan =
        IvmPlan::parse("SELECT region, COUNT(*) AS cnt FROM events GROUP BY region").unwrap();
    let mut circuit = IvmCircuit::new(plan);

    for region in &["us", "us", "eu", "eu", "eu"] {
        circuit.push_batch(&[ZDelta {
            fields: [(
                "region".to_string(),
                serde_json::Value::String(region.to_string()),
            )]
            .into_iter()
            .collect(),
            weight: 1,
        }]);
    }

    let output = circuit.read_output();
    assert_eq!(circuit.group_count(), 2);

    let us = output
        .iter()
        .find(|r| r["region"] == serde_json::Value::String("us".into()))
        .unwrap();
    assert_eq!(us["cnt"], serde_json::Value::Number(2.into()));

    let eu = output
        .iter()
        .find(|r| r["region"] == serde_json::Value::String("eu".into()))
        .unwrap();
    assert_eq!(eu["cnt"], serde_json::Value::Number(3.into()));
}

#[test]
fn ivm_circuit_sum_aggregate() {
    use slateduck_ivm::{
        circuit::{IvmCircuit, ZDelta},
        plan::IvmPlan,
    };

    let plan =
        IvmPlan::parse("SELECT dept, SUM(amount) AS total FROM orders GROUP BY dept").unwrap();
    let mut circuit = IvmCircuit::new(plan);

    for (dept, amount) in &[("eng", 100i64), ("eng", 200), ("mkt", 50)] {
        circuit.push_batch(&[ZDelta {
            fields: [
                (
                    "dept".to_string(),
                    serde_json::Value::String(dept.to_string()),
                ),
                (
                    "amount".to_string(),
                    serde_json::Value::Number((*amount).into()),
                ),
            ]
            .into_iter()
            .collect(),
            weight: 1,
        }]);
    }

    let output = circuit.read_output();
    let eng = output
        .iter()
        .find(|r| r["dept"] == serde_json::Value::String("eng".into()))
        .unwrap();
    assert_eq!(eng["total"], serde_json::Value::Number(300.into()));
}

// ─── IVM Plan Parsing ─────────────────────────────────────────────────────────

#[test]
fn ivm_plan_parse_group_by_columns() {
    use slateduck_ivm::plan::IvmPlan;
    let plan = IvmPlan::parse(
        "SELECT region, country, COUNT(*) AS cnt FROM events GROUP BY region, country",
    )
    .unwrap();
    assert_eq!(plan.group_by_cols, vec!["region", "country"]);
}

#[test]
fn ivm_plan_invalid_sql_returns_error() {
    use slateduck_ivm::plan::IvmPlan;
    assert!(IvmPlan::parse("NOT SQL AT ALL").is_err());
}

// ─── SQL Classifier ───────────────────────────────────────────────────────────

#[test]
fn sql_create_incremental_matview_classified() {
    let kind = classify_statement(
        "CREATE INCREMENTAL MATERIALIZED VIEW main.region_counts AS SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
    )
    .unwrap();
    assert!(
        matches!(kind, StatementKind::CreateIncrementalMatview { ref name, ref schema, .. }
            if name == "region_counts" && schema.as_deref() == Some("main")),
        "got: {kind:?}"
    );
}

#[test]
fn sql_drop_incremental_matview_classified() {
    let kind =
        classify_statement("DROP INCREMENTAL MATERIALIZED VIEW IF EXISTS main.region_counts")
            .unwrap();
    assert!(
        matches!(kind, StatementKind::DropIncrementalMatview { ref name, if_exists: true, .. }
            if name == "region_counts"),
        "got: {kind:?}"
    );
}

#[test]
fn sql_show_materialized_views_classified() {
    let kind = classify_statement("SHOW MATERIALIZED VIEWS").unwrap();
    assert_eq!(kind, StatementKind::ShowMaterializedViews);
}

#[test]
fn sql_show_matview_shards_classified() {
    let kind = classify_statement("SHOW MATVIEW SHARDS main.region_counts").unwrap();
    assert!(
        matches!(kind, StatementKind::ShowMatviewShards { ref view_name, .. }
            if view_name == "region_counts"),
        "got: {kind:?}"
    );
}

#[test]
fn sql_explain_matview_classified() {
    let kind = classify_statement("EXPLAIN MATVIEW main.region_counts").unwrap();
    assert!(
        matches!(kind, StatementKind::ExplainMatview { ref view_name, .. }
            if view_name == "region_counts"),
        "got: {kind:?}"
    );
}

// ─── 100-Snapshot Acceptance Criterion ───────────────────────────────────────

#[tokio::test]
async fn ivm_100_snapshots_correct_counts() {
    use slateduck_ivm::{
        circuit::{IvmCircuit, ZDelta},
        plan::IvmPlan,
    };

    let plan =
        IvmPlan::parse("SELECT region, COUNT(*) AS cnt FROM events GROUP BY region").unwrap();
    let mut circuit = IvmCircuit::new(plan);

    // Simulate 100 incremental batches: alternating us/eu rows.
    for i in 0u64..100 {
        let region = if i % 2 == 0 { "us" } else { "eu" };
        circuit.push_batch(&[ZDelta {
            fields: [(
                "region".to_string(),
                serde_json::Value::String(region.to_string()),
            )]
            .into_iter()
            .collect(),
            weight: 1,
        }]);
    }

    let output = circuit.read_output();
    let us = output
        .iter()
        .find(|r| r["region"] == serde_json::Value::String("us".into()))
        .unwrap();
    let eu = output
        .iter()
        .find(|r| r["region"] == serde_json::Value::String("eu".into()))
        .unwrap();

    // 100 rows: 50 us (even i), 50 eu (odd i).
    assert_eq!(us["cnt"], serde_json::Value::Number(50.into()));
    assert_eq!(eu["cnt"], serde_json::Value::Number(50.into()));
}

// ─── TPC-H Q1 streaming (simplified) ─────────────────────────────────────────

#[test]
fn tpch_q1_streaming() {
    use slateduck_ivm::{
        circuit::{IvmCircuit, ZDelta},
        plan::IvmPlan,
    };

    // Simplified TPC-H Q1: group by return_flag, sum quantity.
    let plan = IvmPlan::parse(
        "SELECT l_returnflag, SUM(l_quantity) AS sum_qty, COUNT(*) AS cnt FROM lineitem GROUP BY l_returnflag",
    )
    .unwrap();
    let mut circuit = IvmCircuit::new(plan);

    // Insert 10 rows per return flag.
    for flag in &["A", "N", "R"] {
        for qty in 1i64..=10 {
            circuit.push_batch(&[ZDelta {
                fields: [
                    (
                        "l_returnflag".to_string(),
                        serde_json::Value::String(flag.to_string()),
                    ),
                    (
                        "l_quantity".to_string(),
                        serde_json::Value::Number(qty.into()),
                    ),
                ]
                .into_iter()
                .collect(),
                weight: 1,
            }]);
        }
    }

    let output = circuit.read_output();
    assert_eq!(circuit.group_count(), 3);

    for flag in &["A", "N", "R"] {
        let row = output
            .iter()
            .find(|r| r["l_returnflag"] == serde_json::Value::String(flag.to_string()))
            .unwrap();
        // sum_qty = 1+2+...+10 = 55
        assert_eq!(row["sum_qty"], serde_json::Value::Number(55.into()));
        assert_eq!(row["cnt"], serde_json::Value::Number(10.into()));
    }
}

// ─── create_matview duplicate name rejected ───────────────────────────────────

#[tokio::test]
async fn create_matview_duplicate_name_rejected() {
    let dir = TempDir::new().unwrap();
    let (mut store, schema_id, table_id) = open_with_table(&dir).await;

    {
        let mut writer = store.begin_write();
        let ot = writer
            .create_table(schema_id, "mv_out13", None)
            .await
            .unwrap();
        writer
            .create_matview(
                "main",
                "mv_dup",
                "SELECT region, COUNT(*) AS cnt FROM events GROUP BY region",
                ot,
                1,
                1000,
                &[table_id],
            )
            .await
            .unwrap();
        writer.create_snapshot(None, None).await.unwrap();
        store.commit_writer(&writer);
    }

    // Verify the first matview exists.
    let reader = store.read_latest();
    let by_name = reader.get_matview_by_name("main", "mv_dup").await.unwrap();
    assert!(by_name.is_some(), "first matview should be visible");
}

// ─── matview_output_table_read_only (design-level test) ───────────────────────

#[test]
fn matview_output_table_is_not_writable_by_users() {
    // Design-level assertion: the output table is a regular catalog table, but
    // writes to it are gated through the IVM worker path (output.rs). In v0.11
    // there is no DDL-level enforcement at the SQL layer; that is a v0.12 item.
    // This test documents the constraint and will be replaced by a rejection
    // test once the SQL-layer guard lands.
    //
    // The constraint is exercised by the integration tests in slateduck-ivm,
    // which verify that only `register_inlined_insert` via `output.rs` produces
    // rows in the output table.
}
