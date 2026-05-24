# Deploying on Fly.io

Fly.io is a global application platform that runs containers close to users with automatic TLS, global anycast networking, and Machines that boot in milliseconds. SlateDuck is an excellent fit for Fly.io: it is a small, stateless binary with fast startup time, low resource requirements, and no need for persistent local storage. You can have a globally-distributed lakehouse catalog running in production for under $5/month.

This page covers the complete setup: creating the Fly app, configuring the deployment, managing secrets, connecting DuckDB clients, scaling to multiple regions, and operational patterns specific to Fly.io's platform.

## Why Fly.io for SlateDuck

Fly.io offers several properties that align perfectly with SlateDuck's architecture:

- **Fast boot:** Fly Machines start in <300ms. Combined with SlateDuck's ~200ms startup, you get cold-start times under 500ms.
- **Auto-stop/start:** Machines can scale to zero when idle and wake on incoming connections — perfect for infrequently-accessed catalogs.
- **Global anycast:** A single hostname routes clients to the nearest region automatically.
- **Built-in TLS:** Fly terminates TLS at the edge, so DuckDB clients get encryption without managing certificates.
- **Simple deployment:** Push a Docker image with `fly deploy` — no Kubernetes, no Terraform, no infrastructure to manage.
- **Low cost:** A shared-cpu-1x machine with 256 MB RAM runs SlateDuck comfortably for ~$2–5/month.

## Prerequisites

Install the Fly.io CLI:

```bash
# macOS
brew install flyctl

# Linux
curl -L https://fly.io/install.sh | sh

# Authenticate
fly auth login
```

## Creating the App

```bash
# Create a new Fly app
fly apps create my-slateduck --org personal

# Create the fly.toml configuration
```

## Configuration (fly.toml)

Create `fly.toml` in your project directory:

```toml
app = "my-slateduck"
primary_region = "iad"

[build]
  image = "ghcr.io/slateduck/slateduck:0.8.0"

[env]
  AWS_REGION = "us-east-1"
  RUST_LOG = "slateduck=info"
  SLATEDUCK_LOG_FORMAT = "json"

# TCP service for PostgreSQL wire protocol
[[services]]
  protocol = "tcp"
  internal_port = 5432

  [[services.ports]]
    port = 5432
    handlers = []  # No TLS handler — raw TCP passthrough

  [[services.tcp_checks]]
    grace_period = "15s"
    interval = "10s"
    timeout = "5s"

# Machine configuration
[[vm]]
  cpu_kind = "shared"
  cpus = 1
  memory_mb = 256

# Process command
[processes]
  app = "--catalog s3://my-lakehouse-bucket/catalog/ --bind 0.0.0.0:5432 --auth-user ducklake"
```

### With Auto-Stop (Scale to Zero)

For catalogs accessed infrequently, enable auto-stop to reduce costs:

```toml
[http_service]
  internal_port = 5432
  auto_stop_machines = true
  auto_start_machines = true
  min_machines_running = 0  # Scale to zero when idle
```

With this configuration:

- The machine stops after 5 minutes of no connections (configurable)
- When a new connection arrives, Fly starts the machine automatically
- First connection after idle pays a ~500ms cold-start penalty
- Subsequent connections are instant (machine is running)

### Without Auto-Stop (Always On)

For catalogs with continuous access:

```toml
[http_service]
  internal_port = 5432
  auto_stop_machines = false
  min_machines_running = 1
```

## Secrets Management

Never put credentials in `fly.toml`. Use Fly secrets:

```bash
# AWS credentials for S3 access
fly secrets set AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
fly secrets set AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY

# SlateDuck password authentication
fly secrets set SLATEDUCK_PASSWORD=your-secure-random-password

# Verify secrets are set (values are not shown)
fly secrets list
```

Secrets are encrypted at rest and injected as environment variables at runtime.

## Deploying

```bash
# Deploy (builds/pulls image and starts machines)
fly deploy

# Check status
fly status

# View logs
fly logs

# Check machine health
fly checks list
```

## Connecting DuckDB

### Direct Connection (TCP)

Connect using the Fly app hostname:

```sql
ATTACH 'ducklake:host=my-slateduck.fly.dev;port=5432;user=ducklake;password=your-password' AS lake;
```

### With TLS (Recommended)

Fly.io can terminate TLS at the edge. Configure TLS handlers:

```toml
[[services.ports]]
  port = 5432
  handlers = ["tls"]  # Fly terminates TLS
```

Then connect with SSL:

```sql
ATTACH 'ducklake:host=my-slateduck.fly.dev;port=5432;user=ducklake;password=your-password;sslmode=require' AS lake;
```

### From Within Fly Network (Private)

If your DuckDB application also runs on Fly, use the internal DNS:

```sql
ATTACH 'ducklake:host=my-slateduck.internal;port=5432;user=ducklake;password=your-password' AS lake;
```

Internal connections are free (no bandwidth charges) and lower latency.

## Multi-Region Deployment

Fly.io's multi-region support enables globally-distributed catalog access.

### Writer + Readers Pattern

Deploy one writer in the primary region and readers in secondary regions:

```bash
# Primary writer in Washington DC
fly machine run ghcr.io/slateduck/slateduck:0.8.0 \
    --region iad \
    --env SLATEDUCK_STORAGE=s3://my-bucket/catalog/ \
    --env AWS_REGION=us-east-1 \
    -- --catalog s3://my-bucket/catalog/ --bind 0.0.0.0:5432 --auth-user ducklake

# Read replica in Paris
fly machine run ghcr.io/slateduck/slateduck:0.8.0 \
    --region cdg \
    --env SLATEDUCK_STORAGE=s3://my-bucket-eu/catalog/ \
    --env AWS_REGION=eu-west-1 \
    -- --catalog s3://my-bucket-eu/catalog/ --bind 0.0.0.0:5432 --read-only --auth-user ducklake

# Read replica in Singapore
fly machine run ghcr.io/slateduck/slateduck:0.8.0 \
    --region sin \
    --env SLATEDUCK_STORAGE=s3://my-bucket-ap/catalog/ \
    --env AWS_REGION=ap-southeast-1 \
    -- --catalog s3://my-bucket-ap/catalog/ --bind 0.0.0.0:5432 --read-only --auth-user ducklake
```

### Fly Region Replay (Experimental)

For a single-region writer with global access, use Fly's region replay to forward write requests to the primary:

```toml
primary_region = "iad"

# Fly replays non-GET requests to primary region
[env]
  FLY_REPLAY_BACKEND = "iad"
```

Note: This works for HTTP protocols. For TCP/PostgreSQL wire protocol, you need explicit writer/reader separation as shown above.

## Volumes (Optional)

SlateDuck does not need local storage (all state is in object storage). However, if you want to cache frequently-accessed catalog data locally for performance:

```bash
fly volumes create slateduck_cache --region iad --size 1
```

```toml
[mounts]
  source = "slateduck_cache"
  destination = "/cache"
```

This is rarely necessary — SlateDuck's hot key cache in memory is sufficient for most workloads.

## Monitoring

### Fly Metrics Dashboard

Fly provides built-in metrics:

- CPU and memory usage
- Network in/out
- Machine state transitions (started/stopped)

Access via `fly dashboard` or the Fly web console.

### Custom Metrics with Prometheus

Export SlateDuck metrics to a Prometheus-compatible endpoint:

```toml
[metrics]
  port = 9090
  path = "/metrics"
```

Use Fly's built-in Prometheus integration or a service like Grafana Cloud.

### Alerting

```bash
# Set up health check alerting
fly checks create tcp \
    --port 5432 \
    --interval 10s \
    --timeout 5s
```

## Cost Analysis

Fly.io pricing for SlateDuck deployments:

| Configuration | Monthly Cost | Use Case |
|---------------|-------------|----------|
| shared-cpu-1x, 256 MB, auto-stop | ~$2/month | Infrequent access, development |
| shared-cpu-1x, 256 MB, always-on | ~$4/month | Light production |
| shared-cpu-2x, 512 MB, always-on | ~$8/month | Medium production |
| 3 regions (1 writer + 2 readers) | ~$12/month | Global access |

Additional costs:

- Outbound bandwidth: $0.02/GB (first 100 GB/month free)
- Volumes (if used): $0.15/GB/month

For comparison, the equivalent on AWS (EC2 t3.micro + NLB) costs ~$25/month. Fly.io is significantly more cost-effective for small SlateDuck deployments.

## Operational Patterns

### Blue-Green Deployments

```bash
# Deploy new version alongside existing
fly deploy --strategy bluegreen
```

### Scaling Up for Burst Load

```bash
# Temporarily increase capacity
fly scale count 3 --region iad

# Scale back down
fly scale count 1 --region iad
```

### Debugging

```bash
# SSH into the running machine
fly ssh console

# Check process status
fly status --all

# View recent logs
fly logs --no-tail | tail -100
```

### Backup and Recovery

Because all state is in object storage, there is nothing to backup on the Fly machine. If the machine is destroyed, a new deploy immediately resumes from the same catalog state.

## Limitations on Fly.io

- **Shared CPU:** Under heavy load, shared CPU instances may be throttled. For sustained high throughput, use dedicated CPU instances.
- **Network latency to S3:** Fly machines are not in AWS/GCP/Azure VPCs. Access to object storage traverses the public internet. Use S3-compatible stores with Fly's internal network (like Tigris) for lowest latency.
- **No VPC peering:** Cannot peer with cloud provider VPCs directly. Use WireGuard or public endpoints.

### Using Tigris (Fly's Object Storage)

Fly.io offers Tigris — an S3-compatible object store integrated into their platform:

```bash
# Create Tigris bucket
fly storage create my-lakehouse

# Set storage URL
fly secrets set SLATEDUCK_STORAGE=s3://my-lakehouse/catalog/
fly secrets set AWS_ENDPOINT_URL=https://fly.storage.tigris.dev
```

Tigris provides the lowest latency from Fly machines (same network) and is globally replicated automatically.

## Troubleshooting Fly.io Deployments

### Machine Fails to Start

**Symptom:** `fly status` shows the machine in a crash loop.

Check logs for the startup error:

```bash
fly logs --no-tail | grep -i "error\|panic\|fatal"
```

Common causes:

- **Missing secrets:** SlateDuck cannot authenticate to S3 without credentials. Verify with `fly secrets list`.
- **Wrong storage URL:** A typo in the bucket name or region causes immediate failure on the first read.
- **Port conflict:** Ensure `internal_port` in `fly.toml` matches the `--bind` port in the process command.

### Connections Time Out

**Symptom:** DuckDB `ATTACH` hangs and eventually times out.

- **Auto-stop enabled:** The first connection after idle may take ~500ms. If your DuckDB client timeout is very short, increase it.
- **Wrong port/hostname:** Verify with `fly ips list` and ensure DNS resolves correctly.
- **TCP handler misconfiguration:** For raw TCP (PostgreSQL wire protocol), ensure you are NOT using HTTP handlers:

```toml
[[services.ports]]
  port = 5432
  handlers = []  # Raw TCP, not HTTP
```

### High Latency to Object Storage

**Symptom:** Queries are slow despite the catalog being small.

Fly machines communicate with AWS S3 over the public internet. Each SlateDB read requires at least one round-trip. Mitigations:

- **Use Tigris** (same-network object storage) for lowest latency
- **Colocate region:** Deploy to a Fly region near your S3 bucket (e.g., `iad` for us-east-1)
- **Increase memory:** More RAM means a larger block cache, reducing S3 round-trips for hot keys

### Machine Restarts Frequently

**Symptom:** Writer epoch keeps incrementing, or machines show multiple recent starts.

- **OOM kills:** Check if the machine runs out of memory. Increase `memory_mb` in `fly.toml`.
- **Health check failures:** If the TCP health check fails, Fly restarts the machine. Increase `grace_period` if SlateDuck needs more startup time.
- **Auto-stop/start cycling:** If traffic arrives in bursts with gaps just long enough to trigger auto-stop, the machine oscillates. Either disable auto-stop or increase the idle timeout.

## Complete Example: Production Setup

Here is a complete, production-ready `fly.toml` with security hardening, monitoring, and appropriate scaling:

```toml
app = "lakehouse-catalog"
primary_region = "iad"
kill_signal = "SIGTERM"
kill_timeout = "30s"

[build]
  image = "ghcr.io/slateduck/slateduck:0.8.0"

[env]
  AWS_REGION = "us-east-1"
  RUST_LOG = "slateduck=info,slateduck_pgwire=warn"
  SLATEDUCK_LOG_FORMAT = "json"
  SLATEDUCK_METRICS_PORT = "9090"

[processes]
  app = "--catalog s3://my-production-bucket/catalog/ --bind 0.0.0.0:5432 --auth-user ducklake"

[[services]]
  protocol = "tcp"
  internal_port = 5432
  auto_stop_machines = false
  min_machines_running = 1

  [[services.ports]]
    port = 5432
    handlers = ["tls"]

  [[services.tcp_checks]]
    grace_period = "15s"
    interval = "10s"
    timeout = "5s"

[[vm]]
  cpu_kind = "shared"
  cpus = 1
  memory_mb = 512
```

## Further Reading

- **[Docker](docker.md)** — Container configuration details
- **[Configuration](configuration.md)** — All environment variables
- **[Multi-Region](multi-region.md)** — Cross-region replication setup
- **[TLS](tls.md)** — Certificate management (if not using Fly's built-in TLS)
