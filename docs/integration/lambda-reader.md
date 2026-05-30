# Lambda Reader Pattern

RockLake supports a **read-only serverless reader pattern** for AWS Lambda and
similar Function-as-a-Service environments. A Lambda function can query the
catalog without running a persistent sidecar or holding a write lock.

## Architecture

```
     ┌─────────────────────────────────────┐
     │  AWS Lambda (cold start: ~200ms)    │
     │                                     │
     │  1. open DbReader (no Db writer)    │
     │  2. read checkpoint-pinned snapshot │
     │  3. list_data_files(table_id)       │
     │  4. return JSON response            │
     └──────────────┬──────────────────────┘
                    │  S3 GetObject (SST files)
                    ▼
     ┌─────────────────────────────────────┐
     │  Amazon S3 Express One Zone         │
     │  (catalog prefix, read-only)        │
     └─────────────────────────────────────┘
```

## Key Properties

- **No writer handle opened.** The handler uses `CatalogClientBuilder` which
  opens the store in read-only mode. No write epochs are acquired.
- **Checkpoint-pinned reads.** By pinning a named checkpoint with
  `rocklake checkpoint pin --name release --snapshot-id N`, the Lambda always
  reads a stable, well-known snapshot even if concurrent writers advance the
  catalog.
- **`/tmp` caching.** On warm invocations the SlateDB SST files cached in
  `/tmp` avoid re-downloading from S3 Express, reducing cold-start latency to
  under 50ms on subsequent calls.

## IAM Policy

Minimum required permissions for the Lambda execution role:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:GetObject",
        "s3:ListBucket",
        "s3express:CreateSession"
      ],
      "Resource": [
        "arn:aws:s3:::my-rocklake-bucket/catalog/*",
        "arn:aws:s3:::my-rocklake-bucket"
      ]
    }
  ]
}
```

## Pinning a Checkpoint for Lambda Readers

Before deploying a Lambda release, pin a named checkpoint so readers always
get a stable view:

```sh
# Pin the current latest snapshot as "production"
rocklake checkpoint pin --name production --snapshot-id $(rocklake snapshot current)

# List all pins
rocklake checkpoint list

# Unpin when no longer needed
rocklake checkpoint unpin --name production
```

## Example Handler

See
[`crates/rocklake-client/examples/lambda_reader.rs`](https://github.com/trickle-labs/rocklake/blob/main/crates/rocklake-client/examples/lambda_reader.rs)
for a complete example handler that:

1. Opens the catalog from `ROCKLAKE_CATALOG_URI`
2. Lists data files for `ROCKLAKE_TABLE_ID`
3. Prints them as a JSON-compatible list

To build and test locally:

```sh
ROCKLAKE_CATALOG_URI=file:///tmp/my-catalog \
ROCKLAKE_TABLE_ID=1 \
cargo run --example lambda_reader -p rocklake-client
```

## Cold-Start Latency

| Scenario | Latency |
|----------|---------|
| Cold start, SSTs not in `/tmp` | ~200–400 ms |
| Warm start, SSTs in `/tmp` | ~20–50 ms |
| S3 Express vs standard S3 | ~3–5× faster on cold start |

S3 Express One Zone is recommended for the catalog prefix to minimise cold-start
latency. See [S3 Express Validation](../performance/s3-express-validation.md)
for acceptance benchmarks.
