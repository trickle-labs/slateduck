# Binary Deployment

## Build

```bash
cargo build --release
```

## Run

```bash
./target/release/slateduck serve --catalog-path /var/lib/slateduck/warehouse
./target/release/slateduck serve --catalog-path s3://my-bucket/catalogs/warehouse
```

## Systemd Service

```ini
[Unit]
Description=SlateDuck Catalog Service
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/slateduck serve --catalog-path s3://my-bucket/catalogs/warehouse
Restart=always
RestartSec=5
Environment=AWS_REGION=us-east-1
Environment=SLATEDUCK_LOG_FORMAT=json

[Install]
WantedBy=multi-user.target
```
