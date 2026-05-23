# Configuration

## CLI Flags

```
slateduck serve [OPTIONS]
  --catalog-path <PATH>       Object-store path (required)
  --listen-addr <ADDR>        PG wire listen address [default: 0.0.0.0:5432]
  --metrics-addr <ADDR>       Metrics address [default: 0.0.0.0:9090]
  --tuning-profile <PROFILE>  SlateDB profile [default: default]
  --retention-days <DAYS>     Snapshot retention [default: unlimited]
  --log-level <LEVEL>         Log level [default: info]
  --log-format <FORMAT>       text or json [default: text]
  --block-cache-mb <MB>       Cache size [default: 16]
```

## TOML Config File

```toml
[catalog]
path = "s3://my-bucket/catalogs/warehouse"
tuning_profile = "default"
block_cache_mb = 32
retention_days = 90

[server]
listen_addr = "0.0.0.0:5432"
metrics_addr = "0.0.0.0:9090"

[logging]
level = "info"
format = "json"
```
