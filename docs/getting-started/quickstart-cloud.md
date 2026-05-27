# Quickstart (Cloud)

The local quickstart showed you how Rocklake works on your machine with a local filesystem path. That is useful for experimentation, but the real value of Rocklake emerges when you point it at cloud object storage. Once your catalog lives in S3, GCS, or Azure Blob Storage, you get eleven-nines durability without managing any disks, automatic replication across availability zones without any configuration, and the ability to scale readers across regions by simply starting additional Rocklake instances pointed at the same prefix.

This guide walks you through connecting Rocklake to each major cloud provider, verifying that your data is durable, understanding the latency characteristics of object-store-backed catalogs, and optionally enabling TLS and authentication for production use. By the end, you will have a fully cloud-native lakehouse catalog that survives any single machine failure and costs fractions of a cent per month to store.

## Prerequisites

Before you begin, make sure you have:

- **Rocklake binary** installed and available on your `PATH` (see the [local quickstart](quickstart.md) for installation)
- **DuckDB 1.2+** with the `ducklake` extension installed (`INSTALL ducklake; LOAD ducklake;`)
- **A cloud storage bucket** with write access — one of:
    - An AWS S3 bucket (standard or Express One Zone)
    - A Google Cloud Storage bucket
    - An Azure Blob Storage container
- **Appropriate credentials** configured in your environment (details below)

If you have not yet completed the local quickstart, we recommend doing so first. The cloud quickstart assumes familiarity with the basic Rocklake workflow (start server, attach from DuckDB, create schemas and tables).

## Step 1: Configure Cloud Credentials

Rocklake uses the standard credential discovery mechanisms for each cloud provider. You do not pass credentials as command-line arguments to Rocklake — instead, you configure them in your environment the same way you would for the AWS CLI, `gsutil`, or `az` commands. This means your existing cloud authentication setup works without modification.

### AWS S3

The most common configuration uses environment variables:

```bash
export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
export AWS_REGION=us-east-1
```

Alternatively, Rocklake reads from `~/.aws/credentials` and `~/.aws/config` — the same files the AWS CLI uses. If you can run `aws s3 ls s3://your-bucket/` successfully, Rocklake can access the same bucket.

For production deployments on AWS infrastructure, Rocklake automatically uses:

- **EC2 instance profiles** (instance metadata service)
- **ECS task roles** (container credential provider)
- **Lambda execution roles** (environment credential provider)
- **EKS IAM Roles for Service Accounts (IRSA)** (web identity token)

No explicit credential configuration is needed in these cases — the role attached to your compute resource is used automatically.

**Minimum IAM permissions required:**

```json
{
  "Effect": "Allow",
  "Action": [
    "s3:GetObject",
    "s3:PutObject",
    "s3:DeleteObject",
    "s3:ListBucket"
  ],
  "Resource": [
    "arn:aws:s3:::my-lakehouse-bucket",
    "arn:aws:s3:::my-lakehouse-bucket/catalog/*"
  ]
}
```

### Google Cloud Storage

For development, use application default credentials:

```bash
gcloud auth application-default login
```

For production or CI environments, use a service account key:

```bash
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account-key.json
```

On GCE instances and GKE pods, the attached service account is used automatically via the metadata server. Rocklake also supports Workload Identity on GKE.

**Minimum IAM role:** `roles/storage.objectAdmin` on the target bucket.

### Azure Blob Storage

For development with storage keys:

```bash
export AZURE_STORAGE_ACCOUNT=myaccount
export AZURE_STORAGE_KEY=base64encodedkey...
```

For production, Azure AD authentication with a service principal:

```bash
export AZURE_TENANT_ID=...
export AZURE_CLIENT_ID=...
export AZURE_CLIENT_SECRET=...
export AZURE_STORAGE_ACCOUNT=myaccount
```

On Azure VMs and AKS, managed identity is used automatically when no explicit credentials are set. Rocklake also supports Workload Identity on AKS.

### Verifying Credentials

Before starting Rocklake, verify that your credentials work with your cloud provider's CLI:

```bash
# AWS
aws s3 ls s3://my-lakehouse-bucket/ --region us-east-1

# GCS
gsutil ls gs://my-lakehouse-bucket/

# Azure
az storage blob list --account-name myaccount --container-name my-container
```

If the listing succeeds, Rocklake will be able to access the same location.

## Step 2: Start Rocklake with Cloud Storage

Point Rocklake at your cloud storage location using the appropriate URI scheme:

=== "AWS S3"

    ```bash
    rocklake serve --catalog s3://my-lakehouse-bucket/catalog/ --bind 0.0.0.0:5432
    ```

=== "Google Cloud Storage"

    ```bash
    rocklake serve --catalog gs://my-lakehouse-bucket/catalog/ --bind 0.0.0.0:5432
    ```

=== "Azure Blob Storage"

    ```bash
    rocklake serve --catalog az://my-container/catalog/ --bind 0.0.0.0:5432
    ```

On first start against an empty prefix, Rocklake initializes a new catalog. This creates the SlateDB manifest file, the initial WAL entry, and the system keys (counters, configuration). The initialization involves approximately 3–5 PUT requests and completes in under a second on all major providers.

You should see output like:

```
Rocklake v0.8.0
Catalog: s3://my-lakehouse-bucket/catalog/
Listening: 0.0.0.0:5432
Writer epoch: 1
```

On subsequent starts against the same prefix, Rocklake reads the existing manifest and resumes from the latest state. If another Rocklake instance is currently running against the same prefix, the new instance fences the old one (see [Writer Fencing](../concepts/writer-fencing.md)) and takes over as the single writer.

### Choosing a Prefix

The `--storage` path determines where all catalog data lives within the bucket. We recommend using a dedicated prefix (like `/catalog/`) rather than the bucket root. This keeps catalog files organized and makes IAM policies easier to scope:

```
s3://my-lakehouse-bucket/
├── catalog/           ← Rocklake catalog data (SlateDB SSTs, WAL, manifest)
├── data/              ← Parquet data files written by DuckDB
└── staging/           ← Temporary staging area
```

You can use any prefix structure that makes sense for your organization. Multiple independent catalogs can coexist in the same bucket under different prefixes.

## Step 3: Connect from DuckDB

The connection workflow is identical to the local quickstart — only the `host` parameter changes if you are connecting from a different machine:

```sql
-- Install and load the extension (first time only)
INSTALL ducklake;
LOAD ducklake;

-- Connect to Rocklake
ATTACH '' AS lakehouse (TYPE ducklake, PG 'host=127.0.0.1 port=5432');
USE lakehouse;
```

Now create a schema and table to verify everything works:

```sql
CREATE SCHEMA production;

CREATE TABLE production.events (
    event_id BIGINT,
    event_type VARCHAR,
    user_id BIGINT,
    payload JSON,
    created_at TIMESTAMP
);

-- Insert some sample data
INSERT INTO production.events VALUES
    (1, 'page_view', 100, '{"url": "/home"}', '2024-06-15 10:00:00'),
    (2, 'click', 100, '{"button": "signup"}', '2024-06-15 10:01:30'),
    (3, 'purchase', 101, '{"amount": 49.99}', '2024-06-15 10:05:00');

-- Verify the data
SELECT * FROM production.events;
```

If you see three rows returned, congratulations — your lakehouse catalog is live in the cloud.

## Step 4: Verify Cloud Persistence

One of the most satisfying aspects of cloud-native storage is being able to see your data in the bucket using standard cloud tools. Let's verify that the catalog state was actually persisted:

=== "AWS S3"

    ```bash
    aws s3 ls s3://my-lakehouse-bucket/catalog/ --recursive
    ```

=== "Google Cloud Storage"

    ```bash
    gsutil ls -r gs://my-lakehouse-bucket/catalog/
    ```

=== "Azure Blob Storage"

    ```bash
    az storage blob list --account-name myaccount --container-name my-container --prefix catalog/ --output table
    ```

You will see the SlateDB internal files — a manifest, WAL segments, and SST (sorted string table) files. These files contain your entire catalog encoded as key-value pairs. A typical fresh catalog with one schema, one table, and a handful of rows occupies less than 100 KB total.

### What the Files Mean

```
catalog/
├── manifest/
│   └── 00000000000000000001     ← SlateDB manifest (current state pointer)
├── wal/
│   └── 00000000000000000001     ← Write-ahead log entries
└── compacted/
    └── L0/
        └── 00000000000000000001.sst  ← Sorted string table (compacted data)
```

You never need to interact with these files directly. They are managed entirely by Rocklake and SlateDB. But knowing they exist in your bucket — alongside your Parquet data files — helps reinforce the mental model: everything is in one bucket, nothing is on a server's local disk.

## Step 5: Test Durability

To prove that the catalog is truly durable, stop Rocklake and restart it:

```bash
# Stop Rocklake (Ctrl+C or kill the process)
# Start it again
rocklake serve --catalog s3://my-lakehouse-bucket/catalog/ --bind 0.0.0.0:5432
```

Reconnect from DuckDB and verify your data is still there:

```sql
ATTACH '' AS lakehouse (TYPE ducklake, PG 'host=127.0.0.1 port=5432');
SELECT count(*) FROM lakehouse.production.events;
-- Returns: 3
```

Your catalog survived a full process restart because all state lives in object storage. You can start Rocklake from a completely different machine — even in a different region — and it will read the same catalog state.

## Understanding Cloud Latency

Cloud object storage has higher latency than local filesystem access. A typical S3 GET or PUT request takes 20–80ms depending on region, object size, and current load. This means individual catalog operations (creating a table, listing schemas) take 50–200ms when backed by S3.

Rocklake mitigates this latency through several techniques:

**Batched writes.** Multiple catalog changes within a single DuckDB transaction (which maps to a single Rocklake transaction) are written as a single SlateDB batch. This batch becomes one WAL PUT regardless of how many catalog entries change. A transaction that creates a table with 20 columns writes one ~2 KB object to S3, not 20+ separate objects.

**Hot key optimization.** Frequently-accessed metadata (current snapshot ID, table file counts, system configuration) is cached in memory after the first read. Cold start requires reading the manifest and a small number of SST files — typically 3–5 GETs — after which all common operations are served from the in-memory cache.

**Prefix-bounded scans.** When DuckDB queries `SELECT * FROM ducklake_table`, Rocklake does not scan all keys in the catalog. It uses a prefix range query that bounds the scan to only the keys for that specific table type, reducing the number of SST blocks that need to be read.

### Latency by Provider

| Provider | Typical GET | Typical PUT | Catalog Operation |
|----------|-------------|-------------|-------------------|
| AWS S3 Standard | 20–50ms | 30–80ms | 50–200ms |
| AWS S3 Express | 2–5ms | 3–8ms | 5–20ms |
| Google Cloud Storage | 20–50ms | 30–70ms | 50–180ms |
| Azure Blob Storage | 15–40ms | 25–60ms | 40–150ms |
| MinIO (local) | 1–5ms | 2–8ms | 5–25ms |

For interactive workloads (running queries in a notebook, exploring schemas), S3 Standard latency is perfectly acceptable. For high-throughput pipelines that create many snapshots per second, S3 Express One Zone or a local MinIO instance provide significantly lower latency.

## S3 Express One Zone (Low-Latency Option)

If you need the lowest possible latency on AWS, S3 Express One Zone (directory buckets) provides single-digit millisecond access. This is particularly valuable for interactive development workflows where you want catalog operations to feel instantaneous.

```bash
rocklake serve --catalog s3express://my-express-bucket--use1-az1--x-s3/catalog/ --bind 0.0.0.0:5432
```

Express One Zone costs more per GB stored and per request than S3 Standard, but for a catalog that typically occupies less than 100 MB, the cost difference is negligible (cents per month). The latency improvement — from ~100ms per operation to ~10ms — dramatically improves the interactive experience.

Note that Express One Zone stores data in a single availability zone. For the catalog, this is acceptable because SlateDB's manifest provides crash consistency, and the single-writer model means there is no split-brain risk. If the AZ experiences an outage, you can start a new Rocklake instance in a different AZ pointed at a replicated copy of the bucket, or simply wait for the AZ to recover (your data is not lost, only temporarily inaccessible).

## Enabling TLS and Authentication

For any deployment accessible over a network (not just localhost), you should enable TLS encryption and client authentication:

```bash
rocklake \
    --catalog s3://my-lakehouse-bucket/catalog/ \
    --bind 0.0.0.0:5432 \
    --tls-cert /etc/ssl/certs/rocklake.crt \
    --tls-key /etc/ssl/private/rocklake.key \
    --auth-user ducklake \
    --auth-password "${ROCKLAKE_PASSWORD}"
```

DuckDB connects with credentials in the connection string:

```sql
ATTACH '' AS lakehouse (
    TYPE ducklake,
    PG 'host=my-server.example.com port=5432 user=ducklake password=mysecretpassword sslmode=require'
);
```

The `sslmode=require` parameter ensures the connection uses TLS. For production deployments, always use TLS even within a VPC — defense in depth is important.

### Certificate Options

- **Self-signed certificates** work fine for internal deployments where you control both the server and client configuration.
- **Let's Encrypt / ACME** certificates work if your Rocklake instance is reachable from the internet (e.g., deployed on Fly.io with a public domain).
- **Private CA certificates** are recommended for enterprise deployments where you manage your own PKI.

## Multi-Region Readers

One of the most powerful features of a cloud-native catalog is the ability to deploy readers in multiple regions. Because Rocklake's immutability model means readers never write to the catalog, you can start read-only Rocklake instances in any region where your cloud provider has infrastructure:

```bash
# Writer in us-east-1
rocklake serve --catalog s3://my-lakehouse-bucket/catalog/ --bind 0.0.0.0:5432

# Reader in eu-west-1 (using cross-region S3 access or bucket replication)
rocklake serve --catalog s3://my-lakehouse-bucket-eu/catalog/ --bind 0.0.0.0:5432 --read-only
```

Readers in different regions see a slightly stale view of the catalog (the staleness is bounded by object storage replication lag, typically seconds). For analytics workloads, this is perfectly acceptable and provides dramatically lower query latency for users in distant regions.

## Cost Estimation

One of the pleasant surprises of running Rocklake in the cloud is how inexpensive it is. A catalog is just key-value data in object storage — there are no provisioned instances, no reserved capacity, no minimum charges:

| Workload | Catalog Size | Monthly Storage Cost | Monthly Request Cost |
|----------|--------------|---------------------|---------------------|
| Development (occasional use) | < 1 MB | < $0.01 | < $0.01 |
| Light production (100 snapshots/day) | 5–50 MB | $0.01–$0.05 | $0.05–$0.50 |
| Heavy production (10,000 snapshots/day) | 50–500 MB | $0.05–$0.50 | $0.50–$5.00 |

These costs are for the catalog only — they do not include the Parquet data files, which are typically the dominant storage cost. But they illustrate the point: the catalog infrastructure itself is essentially free at cloud scale.

## Troubleshooting

### "Access Denied" on startup

Your credentials do not have sufficient permissions on the bucket/prefix. Verify with the provider's CLI:

```bash
# Test write access
echo "test" | aws s3 cp - s3://my-lakehouse-bucket/catalog/.test
aws s3 rm s3://my-lakehouse-bucket/catalog/.test
```

### "Bucket not found" or "Container does not exist"

The bucket/container must be created before starting Rocklake. Rocklake creates objects within the bucket but does not create the bucket itself.

### High latency on first query after restart

This is expected. On cold start, Rocklake reads the manifest and loads the current state from SST files. Subsequent queries use the in-memory cache and are much faster. The cold start typically takes 200–500ms for a small catalog, scaling linearly with catalog size.

### "Writer fenced" error

Another Rocklake instance has taken over as the writer for this catalog. This happens when you start a new instance without stopping the old one. The old instance will refuse further write operations. See [Writer Fencing](../concepts/writer-fencing.md) for details.

## Next Steps

Now that your catalog is running in the cloud, you are ready to build a real lakehouse workflow:

- **[Your First Lakehouse](first-lakehouse.md)** — Schema evolution, data loading, time travel, and garbage collection in a complete end-to-end tutorial
- **[Deployment Guide](../deployment/index.md)** — Production deployment patterns for Docker, Kubernetes, and serverless
- **[Configuration Reference](../deployment/configuration.md)** — Full reference for all server options
- **[TLS Configuration](../deployment/tls.md)** — Detailed TLS setup for production environments
