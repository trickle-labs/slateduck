# SQLSTATE Mapping

| SQLSTATE | Condition | When |
|----------|-----------|------|
| `02000` | Row not found | Missing entity |
| `0A000` | Unsupported feature | Unrecognized SQL |
| `22023` | Invalid parameter | Snapshot out of retention |
| `23505` | Unique violation | Counter collision |
| `3D000` | Invalid catalog | Not initialized |
| `40001` | Serialization failure | Counter conflict (retry) |
| `42501` | Permission denied | IAM denied |
| `54001` | Program limit | Batch > 64 MiB |
| `57P04` | Writer fenced | Another writer took epoch |
| `08006` | Connection failure | S3 timeout |
| `XX000` | Internal error | Unexpected |
| `XX001` | Data corrupted | Magic mismatch |
