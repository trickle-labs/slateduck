# MVCC Filter

Two integer comparisons per row:

```rust
fn is_visible(row: &VersionedRow, target: u64) -> bool {
    row.begin_snapshot <= target
        && row.end_snapshot.map_or(true, |end| target < end)
}
```

## Read Amplification

Prefix scans return all versions. For frequently-mutated entities, this is significant. Secondary indexes (v0.7) address the hottest paths.
