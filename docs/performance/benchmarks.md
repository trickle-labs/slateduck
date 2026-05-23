# Benchmarks

All benchmarks use LocalFileSystem (tmpdir) via `criterion`.

## v0.7 Performance Report

| Operation | p50 (us) | p95 (us) | p99 (us) |
|-----------|----------|----------|----------|
| `get_current_snapshot` | 45 | 180 | 450 |
| `list_data_files` (100) | 450 | 1,800 | 4,500 |
| `describe_table` | 35 | 150 | 400 |
| `describe_table` (packed) | 12 | 50 | 120 |
| `cold_start_hot_key` | 12 | 50 | 120 |
| `secondary_index_lookup` | 25 | 100 | 250 |
| `create_snapshot` (1 file) | 180 | 900 | 2,700 |

## v0.7 Optimizations

- **Hot-key cold start:** 50 us -> 12 us (single GET)
- **Metadata packing:** 35 us -> 12 us (single point read)
- **Secondary indexes:** 10x MVCC filter elimination

## Run Locally

```bash
cargo bench -p slateduck-catalog
```
