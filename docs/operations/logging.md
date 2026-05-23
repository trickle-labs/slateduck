# Logging

## Configuration

```bash
slateduck serve --log-level info --log-format json
```

## Log Levels

| Level | Content |
|-------|---------|
| `error` | Unrecoverable errors |
| `warn` | Recoverable errors, unsupported SQL |
| `info` | Startup, shutdown, snapshots created |
| `debug` | Each catalog operation |
| `trace` | Wire-level PG messages |

## JSON Format

```json
{"timestamp":"2024-01-01T10:00:00Z","level":"INFO","target":"slateduck_catalog","message":"snapshot created","fields":{"snapshot_id":42}}
```
