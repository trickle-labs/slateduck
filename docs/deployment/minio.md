# Deploying with MinIO

MinIO is a high-performance, S3-compatible object storage server that you can run on your own infrastructure. It is the ideal backend for SlateDuck when you cannot or do not want to use cloud storage: on-premises data centers, air-gapped environments, development workstations without cloud access, CI/CD pipelines, and edge deployments where data sovereignty prevents sending catalog metadata to a public cloud. Because MinIO implements the S3 API, SlateDuck treats it identically to Amazon S3 — the only difference is the endpoint URL.

This guide covers running MinIO alongside SlateDuck for development, deploying MinIO in single-node and distributed configurations for production-like environments, configuring SlateDuck to connect to MinIO, and the common pitfalls that appear when using S3-compatible APIs.

## Why MinIO?

MinIO occupies a specific niche: it is the best choice when you need S3 API compatibility without S3's cloud costs or data egress restrictions. The most common use cases are:

**Local development.** Running a full lakehouse stack on a developer laptop with no cloud credentials, no network dependency, and no cost. MinIO starts in seconds and provides the full S3 API for testing SlateDuck, DuckDB, and data pipelines against a realistic object-store backend.

**CI/CD pipelines.** Integration tests that exercise the full storage stack need a real object store, not a mock. MinIO as a Docker service in a CI pipeline gives tests realistic latency and error modes without requiring cloud credentials in CI secrets.

**On-premises production.** Organizations that require data to remain within their own data centers can deploy MinIO on bare metal or on-premises Kubernetes. MinIO's distributed mode provides erasure coding and cross-node redundancy comparable to cloud storage.

**Edge and air-gapped environments.** Deployments where the SlateDuck sidecar runs alongside processing systems in environments without internet connectivity.

## Quick Start: MinIO with Docker

The fastest path to a working local environment combines MinIO and SlateDuck using Docker Compose:

```yaml
# docker-compose.yml
services:
  minio:
    image: minio/minio:latest
    command: server /data --console-address ":9001"
    ports:
      - "9000:9000"   # S3 API endpoint
      - "9001:9001"   # MinIO web console
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
        mc mb local/my-lakehouse;
        mc mb local/my-lakehouse-data;
        echo 'Buckets created';
      "

  slateduck:
    image: ghcr.io/slateduck/slateduck:latest
    depends_on:
      minio:
        condition: service_healthy
      minio-init:
        condition: service_completed_successfully
    command: >
      serve
      --catalog s3://my-lakehouse/catalog/
      --bind 0.0.0.0:5432
    ports:
      - "5432:5432"
    environment:
      AWS_ENDPOINT_URL: http://minio:9000
      AWS_ACCESS_KEY_ID: minioadmin
      AWS_SECRET_ACCESS_KEY: minioadmin
      AWS_REGION: us-east-1
      SLATEDUCK_S3_PATH_STYLE: "true"

volumes:
  minio-data:
```

Start the stack:

```bash
docker compose up -d

# Verify everything is running
docker compose ps

# Watch SlateDuck's logs
docker compose logs -f slateduck
```

Expected SlateDuck startup output:

```
INFO  SlateDuck v0.8.0 starting
INFO  Storage backend: aws-s3 (endpoint: http://minio:9000)
INFO  Path-style addressing: enabled
INFO  Catalog path: s3://my-lakehouse/catalog/
INFO  Opening SlateDB...
INFO  Catalog initialized (new catalog)
INFO  Listening on 0.0.0.0:5432
INFO  Ready to accept connections
```

Connect DuckDB:

```sql
INSTALL ducklake;
LOAD ducklake;

ATTACH 'ducklake:host=localhost;port=5432' AS lake;

CREATE SCHEMA lake.analytics;
CREATE TABLE lake.analytics.events (
  id BIGINT,
  name VARCHAR,
  ts TIMESTAMP
);
INSERT INTO lake.analytics.events VALUES (1, 'hello', NOW());
SELECT * FROM lake.analytics.events;
```

## Without Docker: Running MinIO Directly

If you prefer to run MinIO directly as a binary (useful for development when Docker is not available):

```bash
# Download the MinIO binary for macOS
curl -O https://dl.min.io/server/minio/release/darwin-amd64/minio
chmod +x minio

# For Linux:
# curl -O https://dl.min.io/server/minio/release/linux-amd64/minio

# Start MinIO
export MINIO_ROOT_USER=minioadmin
export MINIO_ROOT_PASSWORD=minioadmin
mkdir -p ~/minio-data
./minio server ~/minio-data --console-address ":9001"
```

Create a bucket using the MinIO CLI (`mc`):

```bash
# Install mc
curl -O https://dl.min.io/client/mc/release/darwin-amd64/mc
chmod +x mc

# Configure the alias
./mc alias set local http://localhost:9000 minioadmin minioadmin

# Create the catalog bucket
./mc mb local/my-lakehouse
```

Start SlateDuck:

```bash
export AWS_ENDPOINT_URL=http://localhost:9000
export AWS_ACCESS_KEY_ID=minioadmin
export AWS_SECRET_ACCESS_KEY=minioadmin
export AWS_REGION=us-east-1

slateduck serve \
  --catalog s3://my-lakehouse/catalog/ \
  --bind 127.0.0.1:5432 \
  --s3-path-style
```

## Critical Configuration: Path-Style Addressing

!!! warning "Path-style is required for MinIO"
    MinIO requires **path-style addressing** (`http://localhost:9000/my-bucket/key`) rather than virtual-hosted-style (`http://my-bucket.localhost:9000/key`). Virtual-hosted-style requires DNS configuration that makes bucket names part of the hostname, which is not practical for local development.

    Always set `--s3-path-style` (or `SLATEDUCK_S3_PATH_STYLE=true`) when using MinIO.

Without path-style addressing, you will see errors like:

```
Error: NoSuchBucket: The specified bucket does not exist
```

or connection refused errors as SlateDuck tries to connect to a hostname like `my-lakehouse.localhost` that does not exist.

## Endpoint Configuration

SlateDuck needs to know MinIO's endpoint URL. There are two ways to configure this:

**Environment variable (recommended):**

```bash
export AWS_ENDPOINT_URL=http://localhost:9000
```

**Command-line flag:**

```bash
slateduck serve \
  --catalog s3://my-lakehouse/catalog/ \
  --s3-endpoint http://localhost:9000 \
  --s3-path-style \
  --bind 127.0.0.1:5432
```

For Docker deployments where SlateDuck and MinIO are on the same Docker network, use the service name as the hostname:

```bash
AWS_ENDPOINT_URL=http://minio:9000  # Inside Docker network
```

## MinIO Authentication

MinIO uses the same access key / secret key model as AWS S3. The credentials you set with `MINIO_ROOT_USER` and `MINIO_ROOT_PASSWORD` when starting MinIO are the root credentials. For production MinIO deployments, create dedicated users and policies (equivalent to IAM policies) through MinIO's identity management:

```bash
# Using mc
mc admin user add local slateduck-user slateduck-password

# Create a policy
cat > slateduck-policy.json << 'EOF'
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject", "s3:ListBucket"],
      "Resource": ["arn:aws:s3:::my-lakehouse/*", "arn:aws:s3:::my-lakehouse"]
    }
  ]
}
EOF

mc admin policy create local slateduck-policy slateduck-policy.json
mc admin policy attach local slateduck-policy --user slateduck-user
```

Then use the dedicated credentials for SlateDuck:

```bash
export AWS_ACCESS_KEY_ID=slateduck-user
export AWS_SECRET_ACCESS_KEY=slateduck-password
```

## Distributed MinIO for Production-Like Environments

For environments that need production-comparable durability (CI integration environments, staging clusters, on-premises deployments), run MinIO in distributed mode with erasure coding:

```yaml
# docker-compose.yml for distributed MinIO (4-node setup)
services:
  minio1:
    image: minio/minio:latest
    command: server --console-address ":9001"
      http://minio{1...4}/data
    ports:
      - "9001:9001"
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    volumes:
      - minio1-data:/data
    networks:
      - minio-net

  minio2:
    image: minio/minio:latest
    command: server --console-address ":9001"
      http://minio{1...4}/data
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    volumes:
      - minio2-data:/data
    networks:
      - minio-net

  minio3:
    image: minio/minio:latest
    command: server --console-address ":9001"
      http://minio{1...4}/data
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    volumes:
      - minio3-data:/data
    networks:
      - minio-net

  minio4:
    image: minio/minio:latest
    command: server --console-address ":9001"
      http://minio{1...4}/data
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    volumes:
      - minio4-data:/data
    networks:
      - minio-net

  haproxy:
    image: haproxy:latest
    ports:
      - "9000:9000"
    volumes:
      - ./haproxy.cfg:/usr/local/etc/haproxy/haproxy.cfg
    depends_on:
      - minio1
      - minio2
      - minio3
      - minio4
    networks:
      - minio-net

  slateduck:
    image: ghcr.io/slateduck/slateduck:latest
    depends_on:
      - haproxy
    command: >
      serve
      --catalog s3://my-lakehouse/catalog/
      --bind 0.0.0.0:5432
    ports:
      - "5432:5432"
    environment:
      AWS_ENDPOINT_URL: http://haproxy:9000
      AWS_ACCESS_KEY_ID: minioadmin
      AWS_SECRET_ACCESS_KEY: minioadmin
      AWS_REGION: us-east-1
      SLATEDUCK_S3_PATH_STYLE: "true"
    networks:
      - minio-net

volumes:
  minio1-data:
  minio2-data:
  minio3-data:
  minio4-data:

networks:
  minio-net:
    driver: bridge
```

A 4-node MinIO deployment with erasure coding provides N/2 redundancy: the cluster can lose any 2 nodes and continue operating. For CI, a 2-node setup is typically sufficient.

## TLS with MinIO

For any environment beyond local development, enable TLS on MinIO. This requires generating or providing certificates:

```bash
# For development: generate self-signed certificates
mkdir -p ~/minio-certs
openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
  -keyout ~/minio-certs/private.key \
  -out ~/minio-certs/public.crt \
  -subj "/CN=minio.local"

# Start MinIO with TLS
./minio server ~/minio-data \
  --certs-dir ~/minio-certs
```

Tell SlateDuck to use HTTPS and where to find the CA certificate:

```bash
export AWS_ENDPOINT_URL=https://localhost:9000
# For self-signed certificates, add the CA to the system trust store
# or use the --tls-ca-bundle flag:
slateduck serve \
  --catalog s3://my-lakehouse/catalog/ \
  --s3-endpoint https://localhost:9000 \
  --s3-path-style \
  --tls-ca-bundle ~/minio-certs/public.crt \
  --bind 127.0.0.1:5432
```

## Performance Characteristics

MinIO on local hardware typically provides lower latency than cloud object storage:

| Operation | MinIO (local NVMe) | MinIO (spinning disk) | Notes |
|-----------|-------------------|----------------------|-------|
| PUT (WAL segment) | 1–5 ms | 5–20 ms | Network latency + disk write |
| GET (SST block) | 0.5–3 ms | 3–10 ms | Often served from OS cache |
| LIST | 2–8 ms | 5–15 ms | Similar across disk types |

Local MinIO is 5–20× faster than S3 Standard for catalog operations. This makes it attractive for development workflows and integration testing where fast iteration matters. It also means performance tests run against MinIO may underestimate the latency you will see in production against S3.

## Troubleshooting

**`NoSuchBucket` despite bucket existing:**
Forgot `--s3-path-style`. Enable it with the flag or environment variable.

**`SignatureDoesNotMatch`:**
The access key or secret key is wrong. Verify against the MinIO credentials you configured.

**`Connection refused` or timeout:**
MinIO is not running or the endpoint URL is wrong. Check `docker ps` or `./minio server` output to confirm MinIO is listening on the expected port.

**Bucket names with uppercase letters or special characters:**
MinIO follows S3's bucket naming rules: lowercase letters, numbers, and hyphens only. Bucket names with uppercase letters or dots cause signature calculation mismatches.

**SlateDuck connects but reads/writes fail:**
The bucket exists but the user does not have the required permissions. Check MinIO's access policies with `mc admin policy list local`.

## Further Reading

- **[Docker Deployment](docker.md)** — Running SlateDuck and MinIO together with Docker Compose
- **[AWS S3 Deployment](aws-s3.md)** — When you are ready to move from MinIO to AWS S3 in production
- **[Credential Isolation](credential-isolation.md)** — Applying the least-privilege principle to MinIO deployments
- **[Object Store Durability](../concepts/object-store-durability.md)** — The durability model and its implications for MinIO vs. cloud storage
