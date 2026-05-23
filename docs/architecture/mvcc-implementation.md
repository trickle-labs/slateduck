# MVCC Implementation

SlateDuck implements MVCC at the application layer.

## Version Creation (ALTER TABLE)

1. Existing version row gets `end_snapshot = new_snapshot_id`
2. New version row created with `begin_snapshot = new_snapshot_id, end_snapshot = NULL`
3. Both in the same `DbTransaction` — atomic

## Read Path

```rust
pub fn list_tables(&self, schema_id: u64, snapshot_id: u64) -> Vec<Table> {
    let prefix = encode_table_prefix(schema_id);
    self.store.prefix_scan(&prefix)
        .filter(|row| {
            row.begin_snapshot <= snapshot_id
                && row.end_snapshot.map_or(true, |end| snapshot_id < end)
        })
        .collect()
}
```

## Garbage Collection

GC removes rows where `end_snapshot <= oldest_retained_snapshot`. These rows are invisible to any retained snapshot.
