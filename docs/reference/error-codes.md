# Error Codes

| SQLSTATE | Condition | Recovery |
|----------|-----------|----------|
| `02000` | Row not found | Expected for missing entities |
| `08006` | Connection failure | Retry; check object-store |
| `0A000` | Unsupported feature | Use supported shape |
| `22023` | Invalid parameter | Query newer snapshot |
| `23505` | Unique violation | Retry with different ID |
| `3D000` | Catalog not initialized | Run `slateduck serve` |
| `40001` | Serialization failure | Automatic retry (3x) |
| `42501` | Permission denied | Check IAM |
| `54001` | Batch too large | Split transaction |
| `57P04` | Writer fenced | Reconnect |
| `XX000` | Internal error | Check logs |
| `XX001` | Data corrupted | Restore from checkpoint |
