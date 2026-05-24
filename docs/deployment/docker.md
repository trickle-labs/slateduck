# Docker Deployment

Running SlateDuck in Docker provides process isolation, reproducible environments, and seamless integration with container orchestration platforms. Because SlateDuck is a single stateless binary with no local storage requirements, it is an ideal containerization candidate — the container needs no volumes, no init systems, and no sidecar processes. The official Docker image is minimal (based on `distroless/static`) and contains only the SlateDuck binary plus root CA certificates for TLS to object storage.

This page covers everything from a one-line quick start to production-ready Docker Compose stacks, custom image builds, security hardening, and operational patterns.

## Official Image

The official SlateDuck container image is published to GitHub Container Registry:

```
ghcr.io/slateduck/slateduck:latest
ghcr.io/slateduck/slateduck:0.8.0
ghcr.io/slateduck/slateduck:0.8
```

Image characteristics:

- **Base:** `gcr.io/distroless/static` (no shell, no package manager, minimal attack surface)
- **Size:** ~12 MB compressed
- **User:** Non-root (UID 65534, `nobody`)
- **Entrypoint:** `/usr/local/bin/slateduck`
- **Exposed port:** 5432

## Quick Start

The simplest possible Docker deployment — connect SlateDuck to an S3 bucket:

```bash
docker run -d \
  --name slateduck \
  -p 5432:5432 \
  -e AWS_REGION=us-east-1 \
  -e AWS_ACCESS_KEY_ID=your-key \
  -e AWS_SECRET_ACCESS_KEY=your-secret \
  ghcr.io/slateduck/slateduck:latest \
  --catalog s3://my-bucket/catalog/ \
  --bind 0.0.0.0:5432
```

Verify it is running:

```bash
docker logs slateduck
# INFO  SlateDuck v0.8.0 starting
# INFO  Storage: s3://my-bucket/catalog/
# INFO  Listening on 0.0.0.0:5432

# Connect with DuckDB
duckdb -c "ATTACH 'ducklake:host=localhost;port=5432' AS lake;"
```

## Docker Compose: Development Stack

For local development, use Docker Compose to run SlateDuck with MinIO (S3-compatible local storage). This gives you a fully functional lakehouse environment without any cloud credentials:

```yaml
services:
  minio:
    image: minio/minio:latest
    command: server /data --console-address ":9001"
    ports:
      - "9000:9000"   # S3 API
      - "9001:9001"   # Web console
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    volumes:
      - minio-data:/data
    healthcheck:
      test: ["CMD", "mc", "ready", "local"]
      interval: 5s
      timeout: 3s
      retries: 5

  minio-init:
    image: minio/mc:latest
    depends_on:
      minio:
        condition: service_healthy
    entrypoint: >
      /bin/sh -c "
      mc alias set local http://minio:9000 minioadmin minioadmin;
      mc mb local/slateduck-catalog --ignore-existing;
      exit 0;
      "

  slateduck:
    image: ghcr.io/slateduck/slateduck:latest
    ports:
      - "5432:5432"
    environment:
      AWS_ACCESS_KEY_ID: minioadmin
      AWS_SECRET_ACCESS_KEY: minioadmin
      AWS_ENDPOINT_URL: http://minio:9000
      AWS_REGION: us-east-1
    command: ["--storage", "s3://slateduck-catalog/", "--bind", "0.0.0.0:5432"]
    depends_on:
      minio-init:
        condition: service_completed_successfully
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -h localhost -p 5432 || exit 1"]
      interval: 10s
      timeout: 5s
      retries: 3
      start_period: 10s

volumes:
  minio-data:
```

Start the stack:

```bash
docker compose up -d

# Wait for health checks to pass
docker compose ps

# Connect with DuckDB
duckdb -c "ATTACH 'ducklake:host=localhost;port=5432' AS dev_lake;"
```

The MinIO web console is available at `http://localhost:9001` (login: minioadmin/minioadmin) where you can browse the raw object storage contents.

## Docker Compose: Production Stack

A production-ready Compose file with TLS, authentication, JSON logging, and resource limits:

```yaml
services:
  slateduck:
    image: ghcr.io/slateduck/slateduck:0.8.0
    ports:
      - "5432:5432"
    environment:
      SLATEDUCK_STORAGE: s3://production-lakehouse/catalog/
      SLATEDUCK_BIND: 0.0.0.0:5432
      SLATEDUCK_AUTH_USER: ducklake
      SLATEDUCK_PASSWORD: ${SLATEDUCK_PASSWORD}
      SLATEDUCK_LOG_FORMAT: json
      SLATEDUCK_LOG_LEVEL: info
      SLATEDUCK_MAX_SESSIONS: 100
      SLATEDUCK_TLS_CERT: /etc/slateduck/tls/cert.pem
      SLATEDUCK_TLS_KEY: /etc/slateduck/tls/key.pem
      AWS_REGION: us-east-1
    volumes:
      - ./tls:/etc/slateduck/tls:ro
    deploy:
      resources:
        limits:
          cpus: "2.0"
          memory: 512M
        reservations:
          cpus: "0.5"
          memory: 128M
    restart: unless-stopped
    logging:
      driver: json-file
      options:
        max-size: "10m"
        max-file: "3"
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -h localhost -p 5432 || exit 1"]
      interval: 10s
      timeout: 5s
      retries: 3
      start_period: 15s
```

Start with an environment file:

```bash
# Create .env with secrets
echo "SLATEDUCK_PASSWORD=$(openssl rand -base64 32)" > .env

docker compose -f docker-compose.prod.yml up -d
```

## Docker Compose: Read Replica Fleet

Run one writer and multiple read replicas from the same storage:

```yaml
services:
  writer:
    image: ghcr.io/slateduck/slateduck:0.8.0
    ports:
      - "5432:5432"
    environment:
      SLATEDUCK_STORAGE: s3://my-bucket/catalog/
      SLATEDUCK_BIND: 0.0.0.0:5432
      AWS_REGION: us-east-1
    restart: unless-stopped

  reader-1:
    image: ghcr.io/slateduck/slateduck:0.8.0
    ports:
      - "5433:5432"
    environment:
      SLATEDUCK_STORAGE: s3://my-bucket/catalog/
      SLATEDUCK_BIND: 0.0.0.0:5432
      SLATEDUCK_READ_ONLY: "true"
      AWS_REGION: us-east-1
    restart: unless-stopped

  reader-2:
    image: ghcr.io/slateduck/slateduck:0.8.0
    ports:
      - "5434:5432"
    environment:
      SLATEDUCK_STORAGE: s3://my-bucket/catalog/
      SLATEDUCK_BIND: 0.0.0.0:5432
      SLATEDUCK_READ_ONLY: "true"
      AWS_REGION: us-east-1
    restart: unless-stopped
```

Read replicas see committed changes within seconds (bounded by object storage consistency — typically <1 second on S3).

## Building a Custom Image

If you need custom CA certificates, additional tooling, or want to build from source:

### Minimal Production Image

```dockerfile
FROM rust:1.80-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --bin slateduck

FROM gcr.io/distroless/static:nonroot
COPY --from=builder /src/target/release/slateduck /usr/local/bin/slateduck
EXPOSE 5432
USER nonroot:nonroot
ENTRYPOINT ["slateduck"]
```

### Image with Custom CA Certificates

For environments with internal certificate authorities (corporate proxies, private S3-compatible stores):

```dockerfile
FROM rust:1.80-bookworm AS builder
WORKDIR /src
COPY . .
RUN cargo build --release --bin slateduck

FROM debian:bookworm-slim
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Add custom CA
COPY internal-ca.pem /usr/local/share/ca-certificates/internal-ca.crt
RUN update-ca-certificates

# Create non-root user
RUN useradd --system --no-create-home slateduck
USER slateduck

COPY --from=builder /src/target/release/slateduck /usr/local/bin/slateduck
EXPOSE 5432
ENTRYPOINT ["slateduck"]
```

Build and push:

```bash
docker build -t my-registry/slateduck:0.8.0 .
docker push my-registry/slateduck:0.8.0
```

## Health Checks

SlateDuck accepts PostgreSQL protocol connections, so standard PostgreSQL health check tools work:

```yaml
healthcheck:
  test: ["CMD-SHELL", "pg_isready -h localhost -p 5432 || exit 1"]
  interval: 10s
  timeout: 5s
  retries: 3
  start_period: 15s
```

If `pg_isready` is not available in your image (e.g., distroless), use a TCP check:

```yaml
healthcheck:
  test: ["CMD-SHELL", "cat < /dev/tcp/localhost/5432 || exit 1"]
  interval: 10s
  timeout: 5s
  retries: 3
```

Or install a small binary health checker during the build.

## Graceful Shutdown

SlateDuck handles `SIGTERM` (sent by `docker stop`) gracefully:

1. Stops accepting new connections
2. Waits for in-flight transactions to complete (up to 30 seconds)
3. Flushes buffered state to object storage
4. Exits with code 0

Docker's default stop timeout is 10 seconds. For production, increase it:

```yaml
services:
  slateduck:
    stop_grace_period: 60s
```

This ensures clean shutdown even with long-running transactions.

## Security Hardening

### Run as Non-Root

The official image already runs as non-root (`nobody`). For custom images, always add:

```dockerfile
USER nonroot:nonroot
```

### Read-Only Filesystem

SlateDuck does not write to local disk, so you can mount the filesystem read-only:

```yaml
services:
  slateduck:
    read_only: true
    tmpfs:
      - /tmp:size=10M
```

### No Capabilities

Drop all Linux capabilities since SlateDuck needs none:

```yaml
services:
  slateduck:
    cap_drop:
      - ALL
    security_opt:
      - no-new-privileges:true
```

### Secret Management

Never embed credentials in the image or Compose file. Use Docker secrets or external secret managers:

```yaml
services:
  slateduck:
    secrets:
      - slateduck_password
      - aws_credentials

secrets:
  slateduck_password:
    external: true
  aws_credentials:
    external: true
```

For AWS credentials in production, prefer IAM roles (ECS task roles, EKS IRSA) over static credentials.

## Networking Patterns

### Host Networking (Lowest Latency)

```bash
docker run --network host \
  ghcr.io/slateduck/slateduck:latest \
  --catalog s3://bucket/catalog/ --bind 0.0.0.0:5432
```

Useful for bare-metal deployments where Docker provides isolation but not networking.

### Bridge Networking with DNS

Within a Docker Compose network, other containers reach SlateDuck by service name:

```sql
-- From another container in the same Compose stack
ATTACH 'ducklake:host=slateduck;port=5432' AS lake;
```

### Reverse Proxy (Nginx / Traefik)

SlateDuck uses the PostgreSQL wire protocol, which is TCP-based. Configure TCP proxying (not HTTP):

```yaml
# Traefik TCP router example
services:
  traefik:
    labels:
      - "traefik.tcp.routers.slateduck.rule=HostSNI(`*`)"
      - "traefik.tcp.routers.slateduck.entrypoints=postgres"
      - "traefik.tcp.services.slateduck.loadbalancer.server.port=5432"
```

## Logging

### JSON Logging for Production

Set `SLATEDUCK_LOG_FORMAT=json` for structured logs compatible with CloudWatch, Datadog, Loki, and other log aggregators:

```json
{"timestamp":"2024-01-15T10:30:00Z","level":"INFO","target":"slateduck_pgwire","message":"Session connected","session_id":"abc123","remote_addr":"172.18.0.5:41234"}
```

### Log Aggregation

For Docker's built-in logging drivers:

```yaml
services:
  slateduck:
    logging:
      driver: fluentd
      options:
        fluentd-address: fluentd:24224
        tag: slateduck
```

## Upgrading

To upgrade SlateDuck in Docker:

```bash
# Pull new version
docker pull ghcr.io/slateduck/slateduck:0.9.0

# Stop current container (graceful shutdown)
docker stop slateduck

# Start new version (same configuration)
docker run -d --name slateduck-new ... ghcr.io/slateduck/slateduck:0.9.0 ...
```

With Docker Compose:

```bash
# Update image tag in docker-compose.yml, then:
docker compose pull
docker compose up -d
```

Because all state is in object storage, the new container resumes exactly where the old one left off. There is no migration step.

## Troubleshooting

### Container exits immediately

Check logs: `docker logs slateduck`. Common causes:

- Missing `--storage` flag
- Invalid credentials (container can't reach object storage)
- Port already in use on host

### Cannot connect from host

Ensure you're binding to `0.0.0.0` inside the container (not `127.0.0.1`) and port mapping is correct (`-p 5432:5432`).

### High memory usage

Check `--max-sessions` — each session uses ~1 MB. Set resource limits and let Docker OOM-kill if necessary rather than allowing unbounded growth.

## Logging Best Practices

### Structured Logging

In containerized environments, structured (JSON) logging integrates best with log aggregation systems:

```bash
docker run -d \
    --name slateduck \
    -e SLATEDUCK_LOG_FORMAT=json \
    -e RUST_LOG=slateduck=info \
    ghcr.io/slateduck/slateduck:0.8.0 \
    --catalog s3://bucket/catalog/ --bind 0.0.0.0:5432
```

JSON logs work natively with Docker's logging drivers (fluentd, gelf, awslogs) and can be parsed by any log aggregation system without custom grok patterns.

### Log Rotation

Docker's default `json-file` log driver can accumulate unbounded log files. Configure rotation:

```json
{
  "log-driver": "json-file",
  "log-opts": {
    "max-size": "10m",
    "max-file": "3"
  }
}
```

Or in `docker run`:

```bash
docker run -d \
    --log-opt max-size=10m \
    --log-opt max-file=3 \
    --name slateduck \
    ghcr.io/slateduck/slateduck:0.8.0 \
    --catalog s3://bucket/catalog/ --bind 0.0.0.0:5432
```

### Correlating Logs with Queries

Enable trace-level logging for the wire protocol to see individual SQL statements:

```bash
docker run -d \
    --name slateduck \
    -e RUST_LOG=slateduck_pgwire=debug,slateduck=info \
    ghcr.io/slateduck/slateduck:0.8.0 \
    --catalog s3://bucket/catalog/ --bind 0.0.0.0:5432
```

This produces log lines with session IDs that can be correlated with DuckDB client connections for debugging.

## Multi-Container Development Stack

For development environments that simulate a production-like setup:

```yaml
version: "3.8"
services:
  slateduck:
    image: ghcr.io/slateduck/slateduck:0.8.0
    ports:
      - "5432:5432"
    environment:
      - AWS_ACCESS_KEY_ID=minioadmin
      - AWS_SECRET_ACCESS_KEY=minioadmin
      - AWS_REGION=us-east-1
      - RUST_LOG=slateduck=debug
    command: >
      --catalog s3://lakehouse/catalog/
      --bind 0.0.0.0:5432
      --s3-endpoint http://minio:9000
      --s3-force-path-style
    depends_on:
      minio:
        condition: service_healthy

  minio:
    image: minio/minio:latest
    ports:
      - "9000:9000"
      - "9001:9001"
    environment:
      - MINIO_ROOT_USER=minioadmin
      - MINIO_ROOT_PASSWORD=minioadmin
    command: server /data --console-address ":9001"
    healthcheck:
      test: ["CMD", "mc", "ready", "local"]
      interval: 5s
      timeout: 5s
      retries: 5

  minio-init:
    image: minio/mc:latest
    depends_on:
      minio:
        condition: service_healthy
    entrypoint: >
      /bin/sh -c "
      mc alias set local http://minio:9000 minioadmin minioadmin;
      mc mb local/lakehouse --ignore-existing;
      exit 0;
      "

  duckdb:
    image: datacoves/duckdb:latest
    depends_on:
      - slateduck
    stdin_open: true
    tty: true
```

Start the full stack:

```bash
docker compose up -d
docker compose exec duckdb duckdb -c "
ATTACH 'ducklake:host=slateduck;port=5432;user=ducklake' AS lake;
USE lake;
CREATE TABLE hello (msg VARCHAR);
INSERT INTO hello VALUES ('Docker Compose works!');
SELECT * FROM hello;
"
```

## Further Reading

- **[Binary Deployment](binary.md)** — For non-containerized environments
- **[Kubernetes](kubernetes.md)** — Orchestrated container deployment
- **[Configuration](configuration.md)** — Complete configuration reference
- **[Networking](networking.md)** — Network topology and security
