# Environment Variables

## Server

| Variable | Default | Description |
|----------|---------|-------------|
| `SLATEDUCK_LISTEN_ADDR` | `0.0.0.0:5432` | PG wire address |
| `SLATEDUCK_METRICS_ADDR` | `0.0.0.0:9090` | Metrics endpoint |
| `SLATEDUCK_LOG_LEVEL` | `info` | Log level |
| `SLATEDUCK_LOG_FORMAT` | `text` | text or json |

## Catalog

| Variable | Default | Description |
|----------|---------|-------------|
| `SLATEDUCK_CATALOG_PATH` | (required) | Object-store path |
| `SLATEDUCK_TUNING_PROFILE` | `default` | SlateDB profile |
| `SLATEDUCK_BLOCK_CACHE_MB` | `16` | Cache size |
| `SLATEDUCK_RETENTION_DAYS` | unlimited | GC retention |

## AWS

| Variable | Description |
|----------|-------------|
| `AWS_REGION` | Region |
| `AWS_ACCESS_KEY_ID` | Access key |
| `AWS_SECRET_ACCESS_KEY` | Secret key |
| `AWS_ENDPOINT_URL` | Custom endpoint |

## GCS

| Variable | Description |
|----------|-------------|
| `GOOGLE_APPLICATION_CREDENTIALS` | Service account JSON path |

## Azure

| Variable | Description |
|----------|-------------|
| `AZURE_STORAGE_ACCOUNT` | Account name |
| `AZURE_STORAGE_ACCESS_KEY` | Account key |
