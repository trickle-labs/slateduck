# Binary Deployment

The simplest way to run Rocklake is as a standalone binary on a VM or bare-metal server. There is no container runtime to install, no orchestrator to configure, no sidecar to coordinate — just a single executable that reads its configuration from command-line flags and environment variables. This deployment model is appropriate for development, testing, small-scale production, and situations where container infrastructure is unavailable or adds unnecessary complexity.

The Rocklake binary is statically linked (on Linux) and has no runtime dependencies beyond libc. It does not require a Java runtime, Python interpreter, or any other language runtime. It does not write to local disk during normal operation (all state goes to object storage). You can literally `scp` the binary to a server and start it.

## Obtaining the Binary

### Pre-built Releases

Download the pre-built binary for your platform from the GitHub releases page:

=== "Linux (x86_64)"

    ```bash
    curl -L https://github.com/rocklake/rocklake/releases/latest/download/rocklake-linux-x86_64 -o rocklake
    chmod +x rocklake
    sudo mv rocklake /usr/local/bin/
    ```

=== "Linux (ARM64 / aarch64)"

    ```bash
    curl -L https://github.com/rocklake/rocklake/releases/latest/download/rocklake-linux-aarch64 -o rocklake
    chmod +x rocklake
    sudo mv rocklake /usr/local/bin/
    ```

=== "macOS (Apple Silicon)"

    ```bash
    curl -L https://github.com/rocklake/rocklake/releases/latest/download/rocklake-darwin-aarch64 -o rocklake
    chmod +x rocklake
    sudo mv rocklake /usr/local/bin/
    ```

=== "macOS (Intel)"

    ```bash
    curl -L https://github.com/rocklake/rocklake/releases/latest/download/rocklake-darwin-x86_64 -o rocklake
    chmod +x rocklake
    sudo mv rocklake /usr/local/bin/
    ```

Verify the installation:

```bash
rocklake --version
# Rocklake v0.8.0
```

### Building from Source

If you need a custom build (different feature flags, specific Rust version, or development patches):

```bash
git clone https://github.com/rocklake/rocklake.git
cd rocklake
cargo build --release
# Binary is at target/release/rocklake
sudo cp target/release/rocklake /usr/local/bin/
```

Building from source requires Rust 1.75+ and takes approximately 60–90 seconds on a modern machine.

## Running Rocklake

### Development Mode (Local Filesystem)

For local development and testing, point Rocklake at a filesystem path:

```bash
rocklake serve --catalog ./my-catalog --bind 127.0.0.1:5432
```

This creates the catalog in the `./my-catalog` directory. Data is stored as files on the local filesystem using SlateDB's filesystem object store backend. This is fast (no network latency) but not durable beyond the local machine.

### Production Mode (Cloud Storage)

For production, point Rocklake at a cloud storage location:

```bash
# AWS S3
AWS_REGION=us-east-1 rocklake serve --catalog s3://my-lakehouse-bucket/catalog/ --bind 0.0.0.0:5432

# Google Cloud Storage
rocklake serve --catalog gs://my-lakehouse-bucket/catalog/ --bind 0.0.0.0:5432

# Azure Blob Storage
rocklake serve --catalog az://my-container/catalog/ --bind 0.0.0.0:5432
```

The process runs in the foreground by default, logging to stderr. For background operation, use your operating system's process management (systemd, launchd, supervisord).

### Common Flags

```bash
rocklake \
    --catalog s3://bucket/catalog/ \   # Required: where to store catalog data
    --bind 0.0.0.0:5432 \             # Listen address and port (default: 127.0.0.1:5432)
    --tls-cert /path/to/cert.pem \    # Optional: TLS certificate
    --tls-key /path/to/key.pem \      # Optional: TLS private key
    --auth-user ducklake \             # Optional: require username
    --auth-password "$PASSWORD" \      # Optional: require password
    --max-sessions 100 \              # Optional: max concurrent connections (default: 50)
    --log-level info                  # Optional: log verbosity (default: info)
```

## systemd Service (Linux Production)

For production Linux deployments, run Rocklake as a systemd service. This ensures automatic restart on crash, proper logging integration with journald, and controlled startup/shutdown behavior.

Create the service user (for privilege separation):

```bash
sudo useradd --system --no-create-home --shell /usr/sbin/nologin rocklake
```

Create the service file at `/etc/systemd/system/rocklake.service`:

```ini
[Unit]
Description=Rocklake Catalog Server
Documentation=https://rocklake.dev/deployment/binary/
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=rocklake
Group=rocklake
ExecStart=/usr/local/bin/rocklake \
    --catalog s3://my-lakehouse-bucket/catalog/ \
    --bind 0.0.0.0:5432 \
    --tls-cert /etc/rocklake/tls/cert.pem \
    --tls-key /etc/rocklake/tls/key.pem \
    --auth-user ducklake \
    --auth-password ${ROCKLAKE_PASSWORD}

# Restart behavior
Restart=always
RestartSec=5
StartLimitBurst=5
StartLimitIntervalSec=60

# Environment
Environment=AWS_REGION=us-east-1
Environment=RUST_LOG=rocklake=info
EnvironmentFile=-/etc/rocklake/env

# Resource limits
LimitNOFILE=65536
MemoryMax=512M
CPUQuota=200%

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true
ReadOnlyPaths=/etc/rocklake

[Install]
WantedBy=multi-user.target
```

Enable and start the service:

```bash
sudo systemctl daemon-reload
sudo systemctl enable rocklake
sudo systemctl start rocklake

# Check status
sudo systemctl status rocklake

# View logs
sudo journalctl -u rocklake -f
```

### Environment File

Store sensitive configuration in `/etc/rocklake/env` with restricted permissions:

```bash
# /etc/rocklake/env (chmod 600, owned by root)
ROCKLAKE_PASSWORD=your-secure-password-here
AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
```

## launchd Service (macOS)

For macOS production or development servers, create a launchd plist at `~/Library/LaunchAgents/dev.rocklake.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.rocklake</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/rocklake</string>
        <string>--storage</string>
        <string>s3://my-bucket/catalog/</string>
        <string>--bind</string>
        <string>127.0.0.1:5432</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/rocklake.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/rocklake.err</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>AWS_REGION</key>
        <string>us-east-1</string>
    </dict>
</dict>
</plist>
```

Load and start:

```bash
launchctl load ~/Library/LaunchAgents/dev.rocklake.plist
```

## Resource Requirements

Rocklake is lightweight compared to traditional database servers:

| Resource | Requirement | Notes |
|----------|------------|-------|
| **Memory** | 50–200 MB typical | Scales with catalog size and concurrent sessions. Hot key cache uses ~10 MB. Each session uses ~1 MB. |
| **CPU** | 1 core sufficient | Scales to multiple cores for concurrent reads. Write path is single-threaded (single-writer). |
| **Disk** | None required | All state in object storage. No local WAL, no temp files, no swap. |
| **Network** | Reliable, <100ms to storage | Latency to object storage directly affects catalog operation latency. |

For cost-optimized deployments, Rocklake runs comfortably on the smallest VM instances:

- AWS: `t3.micro` (1 vCPU, 1 GB RAM) — sufficient for light workloads
- AWS: `t3.small` (2 vCPU, 2 GB RAM) — recommended for production
- GCP: `e2-micro` / `e2-small` — equivalent
- Azure: `B1s` / `B1ms` — equivalent

## Cloud Credentials

Rocklake uses the standard cloud SDK credential discovery chain. It does not implement its own credential management — it relies on the same mechanisms used by the AWS CLI, gsutil, and az commands.

### AWS Credential Chain (in order of precedence)

1. Environment variables: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`
2. Shared credentials file: `~/.aws/credentials`
3. AWS config file: `~/.aws/config` (with `credential_process` or SSO)
4. EC2 instance metadata (instance profile / IAM role)
5. ECS container credentials (task role)
6. EKS IRSA (web identity token from projected service account)

### GCS Credential Chain

1. Environment variable: `GOOGLE_APPLICATION_CREDENTIALS` (path to service account JSON)
2. Application default credentials (`gcloud auth application-default login`)
3. GCE metadata service (attached service account)
4. GKE Workload Identity

### Azure Credential Chain

1. Environment variables: `AZURE_STORAGE_ACCOUNT` + `AZURE_STORAGE_KEY`
2. Environment variables: `AZURE_TENANT_ID` + `AZURE_CLIENT_ID` + `AZURE_CLIENT_SECRET`
3. Managed Identity (Azure VM, AKS)
4. Azure CLI credentials (`az login`)

## Health Checking

Rocklake exposes a health endpoint that can be used by load balancers and monitoring systems:

```bash
# TCP health check (connection accepted = healthy)
nc -z localhost 5432

# PG protocol health check (SELECT 1 succeeds = healthy)
psql -h localhost -p 5432 -c "SELECT 1"
```

For systemd, add a health check with a watchdog:

```ini
[Service]
WatchdogSec=30
NotifyAccess=main
```

## Graceful Shutdown

Rocklake handles SIGTERM gracefully:

1. Stops accepting new connections
2. Waits for in-flight transactions to complete (up to 30 seconds)
3. Flushes any buffered WAL entries to object storage
4. Exits with code 0

This ensures no data loss during planned restarts or upgrades. systemd's `TimeoutStopSec` (default 90 seconds) provides ample time for graceful shutdown.

## Upgrading

To upgrade Rocklake:

1. Download the new binary
2. Replace the old binary (`/usr/local/bin/rocklake`)
3. Restart the service (`sudo systemctl restart rocklake`)

Because all state is in object storage, there is no local state to migrate. The new version reads the catalog from object storage and resumes operation. Format version compatibility is checked on startup — if the catalog was written by an incompatible future version, Rocklake will refuse to start with a clear error message.

## Troubleshooting

### "Address already in use" on startup

Another process is listening on port 5432 (possibly PostgreSQL, another Rocklake instance, or a stale process). Use `--bind` with a different port or stop the conflicting process.

### "Permission denied" accessing credentials

The rocklake user does not have access to the AWS/GCS/Azure credential files. Ensure the environment file or instance role is properly configured.

### High memory usage

If memory usage exceeds expectations, check the number of concurrent sessions (`--max-sessions`) and reduce if necessary. Each idle session consumes approximately 1 MB.

## Security Hardening

### Network Isolation

Bind Rocklake to a private interface unless external access is required:

```bash
# Only accessible from localhost (development)
rocklake serve --catalog s3://bucket/catalog/ --bind 127.0.0.1:5432

# Only accessible from private network (production)
rocklake serve --catalog s3://bucket/catalog/ --bind 10.0.1.5:5432
```

If external access is needed, place Rocklake behind a reverse proxy or cloud load balancer with TLS termination and IP allowlisting.

### Firewall Rules

Restrict access at the OS level:

```bash
# Allow only specific CIDR (iptables)
iptables -A INPUT -p tcp --dport 5432 -s 10.0.0.0/8 -j ACCEPT
iptables -A INPUT -p tcp --dport 5432 -j DROP

# macOS: use pf or application firewall
```

### Credential Rotation

For long-running deployments with static credentials, implement rotation:

```bash
# Update credentials in the environment file
echo 'AWS_ACCESS_KEY_ID=new-key' > /etc/rocklake/env
echo 'AWS_SECRET_ACCESS_KEY=new-secret' >> /etc/rocklake/env
chmod 600 /etc/rocklake/env

# Restart to pick up new credentials
sudo systemctl restart rocklake
```

For production, prefer instance roles (EC2 IAM roles, GCE service accounts) which rotate credentials automatically without restarts.

### Least-Privilege IAM Policy

Rocklake needs only specific S3 permissions for its catalog prefix:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:GetObject",
        "s3:PutObject",
        "s3:DeleteObject",
        "s3:ListBucket"
      ],
      "Resource": [
        "arn:aws:s3:::my-bucket/catalog/*",
        "arn:aws:s3:::my-bucket"
      ],
      "Condition": {
        "StringLike": {
          "s3:prefix": ["catalog/*"]
        }
      }
    }
  ]
}
```

Do not grant `s3:*` or full bucket access. Rocklake does not need access to data files (Parquet files in the data lake) — only to its own catalog prefix.

## Further Reading

- **[Configuration](configuration.md)** — Full reference for all configuration options
- **[Docker](docker.md)** — Container-based deployment as an alternative
- **[High Availability](high-availability.md)** — Running with failover for uptime SLAs
- **[Operations: Health Checks](../operations/health-checks.md)** — Detailed monitoring integration
