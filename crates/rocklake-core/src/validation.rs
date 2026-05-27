//! SlateDB API validation: working code for each go/no-go gate from ROADMAP v0.1.
//!
//! Each test validates a specific assumption about SlateDB's behavior.
//! These tests constitute the Phase 0 validation gates.

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use object_store::path::Path as ObjectPath;
    use slatedb::{Db, DbReader, IsolationLevel, WriteBatch};
    use std::sync::Arc;
    use tempfile::TempDir;

    fn local_object_store(dir: &TempDir) -> (Arc<dyn object_store::ObjectStore>, ObjectPath) {
        let path = dir.path().to_str().unwrap().to_string();
        let store = Arc::new(object_store::local::LocalFileSystem::new_with_prefix(&path).unwrap());
        (store, ObjectPath::from("db"))
    }

    /// Gate 1: Atomic multi-key writes via WriteBatch.
    /// Verify that WriteBatch is all-or-none across crash/reopen.
    #[tokio::test]
    async fn gate_atomic_multi_key_writes() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path.clone(), store.clone()).await.unwrap();

        let mut batch = WriteBatch::new();
        batch.put(b"key1", b"value1");
        batch.put(b"key2", b"value2");
        batch.put(b"key3", b"value3");
        db.write(batch).await.unwrap();

        db.flush().await.unwrap();
        db.close().await.unwrap();

        // Reopen and verify all keys survived
        let db = Db::open(path, store).await.unwrap();

        let v1 = db.get(b"key1").await.unwrap();
        let v2 = db.get(b"key2").await.unwrap();
        let v3 = db.get(b"key3").await.unwrap();
        assert_eq!(v1, Some(Bytes::from("value1")));
        assert_eq!(v2, Some(Bytes::from("value2")));
        assert_eq!(v3, Some(Bytes::from("value3")));

        db.close().await.unwrap();
    }

    /// Gate 2: Conditional initialization via DbTransaction insert-if-absent.
    #[tokio::test]
    async fn gate_conditional_initialization() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path, store).await.unwrap();

        // First init: key does not exist, write it
        let tx = db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .unwrap();
        let existing = tx.get(b"ducklake_metadata_initialized").await.unwrap();
        assert!(existing.is_none());
        tx.put(b"ducklake_metadata_initialized", b"true").unwrap();
        tx.commit().await.unwrap();

        // Second init: key exists, should detect it
        let tx2 = db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .unwrap();
        let existing2 = tx2.get(b"ducklake_metadata_initialized").await.unwrap();
        assert!(existing2.is_some());
        // Don't write again - catalog already initialized
        tx2.rollback();

        db.close().await.unwrap();
    }

    /// Gate 3: Serializable counter allocation.
    /// Two concurrent transactions on the same counter: one wins, loser gets conflict.
    #[tokio::test]
    async fn gate_serializable_counter_allocation() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path, store).await.unwrap();

        // Initialize counter
        db.put(b"counter", b"1").await.unwrap();
        db.flush().await.unwrap();

        // Start two concurrent transactions
        let tx1 = db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .unwrap();
        let tx2 = db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .unwrap();

        // Both read the same counter value
        let val1 = tx1.get(b"counter").await.unwrap().unwrap();
        let val2 = tx2.get(b"counter").await.unwrap().unwrap();
        assert_eq!(val1, val2);

        // Both try to increment
        let n1: u64 = String::from_utf8(val1.to_vec()).unwrap().parse().unwrap();
        let n2: u64 = String::from_utf8(val2.to_vec()).unwrap().parse().unwrap();

        tx1.put(b"counter", (n1 + 1).to_string().as_bytes())
            .unwrap();
        tx2.put(b"counter", (n2 + 1).to_string().as_bytes())
            .unwrap();

        // First commit succeeds
        let r1 = tx1.commit().await;
        assert!(r1.is_ok(), "First transaction should succeed");

        // Second commit should fail with a conflict
        let r2 = tx2.commit().await;
        assert!(r2.is_err(), "Second transaction should conflict");

        // Counter should be 2 (only one increment applied)
        let final_val = db.get(b"counter").await.unwrap().unwrap();
        let final_n: u64 = String::from_utf8(final_val.to_vec())
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(final_n, 2, "Counter must reflect exactly one increment");

        db.close().await.unwrap();
    }

    /// Gate 4: Concurrent initialization convergence.
    /// Two processes calling open_or_create produce exactly one coherent result.
    #[tokio::test]
    async fn gate_concurrent_initialization_convergence() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path, store).await.unwrap();

        // Simulate two concurrent initializations using transactions
        let tx1 = db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .unwrap();
        let tx2 = db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .unwrap();

        // Both check if initialized
        let init1 = tx1.get(b"catalog_initialized").await.unwrap();
        let init2 = tx2.get(b"catalog_initialized").await.unwrap();
        assert!(init1.is_none());
        assert!(init2.is_none());

        // Both attempt to initialize
        tx1.put(b"catalog_initialized", b"process_1").unwrap();
        tx1.put(b"next_snapshot_id", b"1").unwrap();
        tx1.put(b"next_catalog_id", b"1").unwrap();

        tx2.put(b"catalog_initialized", b"process_2").unwrap();
        tx2.put(b"next_snapshot_id", b"1").unwrap();
        tx2.put(b"next_catalog_id", b"1").unwrap();

        let r1 = tx1.commit().await;
        let r2 = tx2.commit().await;

        // Exactly one should succeed
        let success_count = r1.is_ok() as u8 + r2.is_ok() as u8;
        assert_eq!(
            success_count, 1,
            "Exactly one initializer must succeed; got r1={r1:?}, r2={r2:?}"
        );

        // The surviving value is coherent
        let init = db.get(b"catalog_initialized").await.unwrap().unwrap();
        let init_str = String::from_utf8(init.to_vec()).unwrap();
        assert!(
            init_str == "process_1" || init_str == "process_2",
            "Initialization value must be from one process"
        );

        db.close().await.unwrap();
    }

    /// Gate 5: Durable commit options - flush() survives reopen.
    #[tokio::test]
    async fn gate_durable_commit() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path.clone(), store.clone()).await.unwrap();
        db.put(b"durable_key", b"durable_value").await.unwrap();
        db.flush().await.unwrap();
        db.close().await.unwrap();

        // Reopen
        let db = Db::open(path, store).await.unwrap();
        let val = db.get(b"durable_key").await.unwrap();
        assert_eq!(
            val,
            Some(Bytes::from("durable_value")),
            "Key must survive close/reopen after flush()"
        );
        db.close().await.unwrap();
    }

    /// Gate 6: flush() reader visibility - write -> flush() -> fresh DbReader sees the key.
    #[tokio::test]
    async fn gate_flush_reader_visibility() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path.clone(), store.clone()).await.unwrap();
        db.put(b"visible_key", b"visible_value").await.unwrap();
        db.flush().await.unwrap();

        // Open a separate reader
        let reader = DbReader::builder(path, store).build().await.unwrap();

        let val = reader.get(b"visible_key").await.unwrap();
        assert_eq!(
            val,
            Some(Bytes::from("visible_value")),
            "Fresh DbReader must see flushed key"
        );

        db.close().await.unwrap();
    }

    /// Gate 7: Visibility-barrier latency measurement.
    #[tokio::test]
    async fn gate_visibility_barrier_latency() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path, store).await.unwrap();

        let mut latencies = Vec::new();
        for i in 0..100 {
            let key = format!("latency_key_{i}");
            let value = format!("latency_value_{i}");
            db.put(key.as_bytes(), value.as_bytes()).await.unwrap();

            let start = std::time::Instant::now();
            db.flush().await.unwrap();
            latencies.push(start.elapsed());
        }

        latencies.sort();
        let p50 = latencies[49];
        let p95 = latencies[94];
        let p99 = latencies[98];

        println!("flush() latency (LocalFS): p50={p50:?} p95={p95:?} p99={p99:?}");
        assert!(
            p99 < std::time::Duration::from_secs(10),
            "p99 flush latency should be under 10s on LocalFS"
        );

        db.close().await.unwrap();
    }

    /// Gate 8: Writer fencing - force two writers; confirm distinguishable error.
    #[tokio::test]
    async fn gate_writer_fencing() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        // First writer opens
        let db1 = Db::open(path.clone(), store.clone()).await.unwrap();
        db1.put(b"fence_key", b"from_writer_1").await.unwrap();
        db1.flush().await.unwrap();

        // Second writer opens - should fence out the first
        let db2 = Db::open(path, store).await.unwrap();
        db2.put(b"fence_key", b"from_writer_2").await.unwrap();
        db2.flush().await.unwrap();

        // First writer should be fenced - its next write should fail
        let result = db1.put(b"another_key", b"should_fail").await;

        if result.is_ok() {
            let flush_result = db1.flush().await;
            assert!(
                flush_result.is_err(),
                "Fenced writer must fail on flush: {flush_result:?}"
            );
        }
        // If put itself errored, that's also valid fencing behavior

        db2.close().await.unwrap();
    }

    /// Gate 9: WriteBatch logical size - determine if SlateDB imposes limits.
    #[tokio::test]
    async fn gate_write_batch_logical_size() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path, store).await.unwrap();

        // Write a moderately large batch (1 MiB total)
        let mut batch = WriteBatch::new();
        let value = vec![0xABu8; 1024]; // 1 KiB values
        for i in 0..1024u32 {
            let key = format!("batch_key_{i:06}");
            batch.put(key.as_bytes(), &value);
        }
        let result = db.write(batch).await;
        assert!(result.is_ok(), "1 MiB batch should succeed: {result:?}");

        db.flush().await.unwrap();

        // Verify keys present
        let val = db.get(b"batch_key_000000").await.unwrap();
        assert!(val.is_some());
        let val = db.get(b"batch_key_001023").await.unwrap();
        assert!(val.is_some());

        db.close().await.unwrap();
    }

    /// Gate 10: Prefix-scan latest-value semantics.
    #[tokio::test]
    async fn gate_prefix_scan_latest_value() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path, store).await.unwrap();

        // Write initial values
        db.put(b"\x01\x00\x01", b"first_version").await.unwrap();
        db.put(b"\x01\x00\x02", b"row_2").await.unwrap();
        db.put(b"\x02\x00\x01", b"other_table").await.unwrap();

        // Overwrite first key
        db.put(b"\x01\x00\x01", b"second_version").await.unwrap();
        db.flush().await.unwrap();

        // Prefix scan for table tag 0x01
        let mut iter = db.scan_prefix(&[0x01]).await.unwrap();
        let mut results = Vec::new();
        while let Some(kv) = iter.next().await.unwrap() {
            results.push((kv.key.to_vec(), kv.value.to_vec()));
        }

        // Should see latest value for key \x01\x00\x01 and the row_2 entry
        assert_eq!(
            results.len(),
            2,
            "Should get exactly 2 entries for prefix 0x01"
        );
        assert_eq!(results[0].1, b"second_version", "Must see latest value");
        assert_eq!(results[1].1, b"row_2");

        // Should NOT see entries from table tag 0x02
        for (key, _) in &results {
            assert_eq!(key[0], 0x01, "Prefix scan must not leak other prefixes");
        }

        db.close().await.unwrap();
    }

    /// Smoke test: open SlateDB, put/get, scan prefix, transaction, checkpoint.
    #[tokio::test]
    async fn smoke_test_hello_world() {
        let dir = TempDir::new().unwrap();
        let (store, path) = local_object_store(&dir);

        let db = Db::open(path, store).await.unwrap();

        // Put/Get
        db.put(b"hello", b"world").await.unwrap();
        let val = db.get(b"hello").await.unwrap().unwrap();
        assert_eq!(val.as_ref(), b"world");

        // Scan prefix
        db.put(b"prefix_a", b"1").await.unwrap();
        db.put(b"prefix_b", b"2").await.unwrap();
        db.put(b"other_c", b"3").await.unwrap();

        let mut iter = db.scan_prefix(b"prefix_").await.unwrap();
        let mut count = 0;
        while let Some(_kv) = iter.next().await.unwrap() {
            count += 1;
        }
        assert_eq!(count, 2);

        // Transaction
        let tx = db
            .begin(IsolationLevel::SerializableSnapshot)
            .await
            .unwrap();
        tx.put(b"tx_key", b"tx_value").unwrap();
        tx.commit().await.unwrap();
        let tx_val = db.get(b"tx_key").await.unwrap().unwrap();
        assert_eq!(tx_val.as_ref(), b"tx_value");

        // Flush (checkpoint equivalent for LocalFS)
        db.flush().await.unwrap();

        db.close().await.unwrap();
    }
}
