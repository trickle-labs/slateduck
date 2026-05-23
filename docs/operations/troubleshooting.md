# Troubleshooting

## Connection Refused on Port 5432

1. Check process: `ps aux | grep slateduck`
2. Check bind address: `ss -tlnp | grep 5432`
3. Check startup logs

## SQLSTATE 0A000 (Feature Not Supported)

The SQL statement doesn't match any bounded dispatcher pattern. Check [SQL Supported](../reference/sql-supported.md).

## SQLSTATE 57P04 (Writer Fenced)

Another writer took the epoch. Expected during restarts. Client should reconnect.

## SQLSTATE 08006 (Connection Failure)

Object-store access failing. Check IAM credentials, bucket access, VPC endpoints.

## High Latency (> 1s)

1. Check `slateduck_storage_get_duration_seconds`
2. If high: S3 throttling or network issue
3. If read amplification high: run GC
4. Consider `--tuning-profile read_heavy`
