# Latency Model

## Object-Store Baselines

| Operation | S3 Standard | S3 Express | Local FS |
|-----------|-------------|------------|----------|
| GetObject | 20-40 ms | 3-8 ms | 0.01-0.1 ms |
| PutObject | 30-60 ms | 5-12 ms | 0.01-0.1 ms |

## Catalog Operation Latencies

| Operation | Local FS (p50) | S3 Standard |
|-----------|---------------|-------------|
| `get_current_snapshot` | 12 us | 20-40 ms |
| `list_data_files` (100) | 450 us | 60-120 ms |
| `describe_table` (packed) | 12 us | 20-40 ms |
| `create_snapshot` (1 file) | 180 us | 100-200 ms |

## The PostgreSQL Gap

PostgreSQL on same LAN: 1-5 ms. SlateDuck on S3: 20-50 ms. This 10-50x gap is usually acceptable because query execution dominates.
