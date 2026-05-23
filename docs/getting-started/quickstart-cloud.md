# Quickstart (Cloud)

This guide runs SlateDuck against a real S3 bucket.

## Prerequisites

- Everything from the [local quickstart](quickstart.md)
- An S3 bucket with write access
- AWS credentials configured (`aws configure` or environment variables)

## Start with S3 Backend

```bash
export AWS_REGION=us-east-1
./target/release/slateduck serve --catalog-path s3://your-bucket/catalogs/production
```

## Connect from DuckDB

```sql
LOAD ducklake;
LOAD postgres;
ATTACH 'ducklake:postgres:host=localhost port=5432 dbname=warehouse' AS lake;
USE lake;
CREATE TABLE analytics.events (id BIGINT, event_type VARCHAR, ts TIMESTAMP);
```

## Other Cloud Providers

### Google Cloud Storage

```bash
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json
./target/release/slateduck serve --catalog-path gs://your-bucket/catalogs/production
```

### Azure Blob Storage

```bash
export AZURE_STORAGE_ACCOUNT=youraccount
export AZURE_STORAGE_ACCESS_KEY=your-key
./target/release/slateduck serve --catalog-path az://your-container/catalogs/production
```
