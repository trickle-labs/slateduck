# Crash Safety

Every crash point has a guarantee: operation completed atomically, or didn't happen.

## Crash Points

| Point | Guarantee |
|-------|-----------|
| After Parquet write, before catalog commit | File exists but unreferenced; orphan sweep cleans up |
| During WAL PutObject | Atomic: fully written or not |
| After commit, before flush | Transaction IS committed (in WAL); visible after next flush |
| Two writers running | Fencing ensures exactly one can commit |
| During batch assembly | No partial snapshot visible (all in memory) |

## Testing

Crash-injection tests using `fail-parallel`: inject failure, verify consistency after restart.
