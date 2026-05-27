# Deploying on Google Cloud Storage

Google Cloud Storage (GCS) is a natural choice for Rocklake deployments running on Google Cloud Platform. It provides eleven-nines durability, deep integration with GCP's IAM system, competitive pricing, and consistent performance across regions. This guide covers everything from creating your bucket to configuring authentication in Cloud Run, GKE, or on bare Compute Engine instances — including the authentication subtleties that trip up most first-time GCS users.

## Prerequisites

- A Google Cloud project with billing enabled
- The `gcloud` CLI installed and authenticated (`gcloud auth login`)
- Rocklake binary or Docker image
- IAM permissions to create GCS buckets and service accounts (typically `roles/storage.admin` and `roles/iam.admin`)

## Creating the Bucket

Choose a region close to your compute for the lowest latency. For multi-regional durability, use a multi-region or dual-region bucket type:

```bash
# Single-region bucket (lowest cost, highest performance)
gcloud storage buckets create gs://my-lakehouse \
  --location=us-central1 \
  --uniform-bucket-level-access

# Multi-region bucket (higher durability, wider availability)
gcloud storage buckets create gs://my-lakehouse \
  --location=us \
  --uniform-bucket-level-access
```

The `--uniform-bucket-level-access` flag disables ACLs in favor of IAM-only access control — this is the modern, recommended approach and simplifies permission management considerably.

**Enable versioning** (recommended for additional protection against accidental deletion):

```bash
gcloud storage buckets update gs://my-lakehouse --versioning
```

**Default encryption** in GCS uses Google-managed keys at no additional cost. For customer-managed keys using Cloud KMS:

```bash
gcloud storage buckets update gs://my-lakehouse \
  --default-encryption-key=projects/my-project/locations/us-central1/keyRings/my-ring/cryptoKeys/my-key
```

## IAM Configuration

GCS uses a different IAM model from AWS: permissions are granted as predefined roles on the bucket (or via custom roles), rather than as individual action strings in a policy document.

### Creating a Service Account

Never use your personal credentials or the default Compute Engine service account for Rocklake in production. Create a dedicated service account:

```bash
# Create the service account
gcloud iam service-accounts create rocklake-catalog \
  --display-name="Rocklake Catalog Service Account" \
  --project=my-project

# The full email will be: rocklake-catalog@my-project.iam.gserviceaccount.com
```

### Granting Bucket Access

Rocklake needs to read and write SlateDB's SST files and WAL segments. The `roles/storage.objectAdmin` role on the catalog prefix is the appropriate permission:

```bash
# Grant access to the catalog prefix only
gcloud storage buckets add-iam-policy-binding gs://my-lakehouse \
  --member="serviceAccount:rocklake-catalog@my-project.iam.gserviceaccount.com" \
  --role="roles/storage.objectAdmin" \
  --condition='title=CatalogPrefixOnly,expression=resource.name.startsWith("projects/_/buckets/my-lakehouse/objects/catalog/")'
```

The condition restricts access to the `catalog/` prefix. Without the condition, the service account would have access to the entire bucket, which violates the least-privilege principle.

!!! note "Prefix conditions in IAM"
    IAM conditions for GCS bucket prefixes require `resource.name` to start with `projects/_/buckets/{bucket}/objects/{prefix}/`. The `_` is a wildcard for the project part of the resource name and is correct — do not replace it with your project ID.

### Separate Credentials for Data Plane

For security, create a separate service account for DuckDB's data plane access (reading and writing Parquet files):

```bash
# Read-write access for DuckDB writers (data ingestion)
gcloud iam service-accounts create duckdb-writer \
  --display-name="DuckDB Data Writer"

gcloud storage buckets add-iam-policy-binding gs://my-lakehouse \
  --member="serviceAccount:duckdb-writer@my-project.iam.gserviceaccount.com" \
  --role="roles/storage.objectCreator" \
  --condition='title=DataPrefixWrite,expression=resource.name.startsWith("projects/_/buckets/my-lakehouse/objects/data/")'

# Read-only access for DuckDB readers (query execution)
gcloud iam service-accounts create duckdb-reader \
  --display-name="DuckDB Data Reader"

gcloud storage buckets add-iam-policy-binding gs://my-lakehouse \
  --member="serviceAccount:duckdb-reader@my-project.iam.gserviceaccount.com" \
  --role="roles/storage.objectViewer" \
  --condition='title=DataPrefixRead,expression=resource.name.startsWith("projects/_/buckets/my-lakehouse/objects/data/")'
```

## Authentication Methods

GCS authentication works differently from AWS, and the differences matter for how you configure Rocklake. There are three main approaches:

### Application Default Credentials (ADC)

ADC is the recommended approach for GCP-managed environments (Compute Engine, GKE, Cloud Run, Cloud Functions). When your application runs on GCP infrastructure, it automatically receives credentials from the metadata server attached to the service account you configured for the instance or pod. No credential files, no environment variables.

To use ADC:

1. Attach the service account to your Compute Engine instance, GKE node pool, or Cloud Run service (see the section for your deployment target below).
2. Run Rocklake with a GCS storage URL and no credential configuration.

```bash
rocklake serve \
  --catalog gs://my-lakehouse/catalog/ \
  --bind 0.0.0.0:5432
```

Rocklake will automatically discover the ADC credentials from the metadata server.

### Service Account JSON Key

For deployments outside GCP (on-premises, other cloud providers, CI/CD systems), download a service account key file:

```bash
gcloud iam service-accounts keys create rocklake-key.json \
  --iam-account=rocklake-catalog@my-project.iam.gserviceaccount.com
```

!!! warning "Key security"
    JSON key files contain sensitive credentials. Never commit them to version control, never embed them in Docker images, and never expose them in environment variables in plain text. Use a secrets management solution (GCP Secret Manager, Vault, Kubernetes secrets) to inject them at runtime.

Set the `GOOGLE_APPLICATION_CREDENTIALS` environment variable to the path of the key file:

```bash
export GOOGLE_APPLICATION_CREDENTIALS=/path/to/rocklake-key.json
rocklake serve --catalog gs://my-lakehouse/catalog/ --bind 0.0.0.0:5432
```

### Workload Identity Federation (Recommended for Non-GCP)

For deployments on other cloud providers or on-premises, use Workload Identity Federation instead of a long-lived key file. This allows external identities (AWS IAM roles, Azure managed identities, Kubernetes service account tokens) to impersonate a GCP service account without any long-lived secrets. Configuration is more involved but provides much stronger security guarantees.

See the [Google Cloud documentation on Workload Identity Federation](https://cloud.google.com/iam/docs/workload-identity-federation) for setup instructions.

## Deployment-Specific Configuration

### Compute Engine

Create an instance with the Rocklake service account attached:

```bash
gcloud compute instances create rocklake-server \
  --zone=us-central1-a \
  --machine-type=e2-medium \
  --service-account=rocklake-catalog@my-project.iam.gserviceaccount.com \
  --scopes=https://www.googleapis.com/auth/devstorage.read_write \
  --image-family=debian-12 \
  --image-project=debian-cloud
```

SSH to the instance and start Rocklake:

```bash
gcloud compute ssh rocklake-server
# On the instance:
./rocklake serve --catalog gs://my-lakehouse/catalog/ --bind 0.0.0.0:5432
```

### Google Kubernetes Engine (GKE)

Use Workload Identity to bind a Kubernetes service account to the GCP service account:

```bash
# Enable Workload Identity on the cluster (if not already enabled)
gcloud container clusters update my-cluster \
  --zone=us-central1-a \
  --workload-pool=my-project.svc.id.goog

# Create Kubernetes service account
kubectl create serviceaccount rocklake -n rocklake-ns

# Bind KSA to GSA
gcloud iam service-accounts add-iam-policy-binding \
  rocklake-catalog@my-project.iam.gserviceaccount.com \
  --role="roles/iam.workloadIdentityUser" \
  --member="serviceAccount:my-project.svc.id.goog[rocklake-ns/rocklake]"

# Annotate the Kubernetes service account
kubectl annotate serviceaccount rocklake \
  -n rocklake-ns \
  iam.gke.io/gcp-service-account=rocklake-catalog@my-project.iam.gserviceaccount.com
```

Deploy Rocklake:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rocklake
  namespace: rocklake-ns
spec:
  replicas: 1
  selector:
    matchLabels:
      app: rocklake
  template:
    metadata:
      labels:
        app: rocklake
    spec:
      serviceAccountName: rocklake  # The KSA with Workload Identity
      containers:
        - name: rocklake
          image: ghcr.io/rocklake/rocklake:latest
          args:
            - serve
            - --catalog
            - gs://my-lakehouse/catalog/
            - --bind
            - 0.0.0.0:5432
          ports:
            - containerPort: 5432
```

### Cloud Run

Cloud Run automatically provides ADC credentials via the service account attached to the service. This makes it one of the simplest deployment targets:

```bash
gcloud run deploy rocklake \
  --image=ghcr.io/rocklake/rocklake:latest \
  --region=us-central1 \
  --service-account=rocklake-catalog@my-project.iam.gserviceaccount.com \
  --args="serve,--catalog,gs://my-lakehouse/catalog/,--bind,0.0.0.0:5432" \
  --port=5432 \
  --no-allow-unauthenticated \
  --min-instances=1
```

Note `--min-instances=1`: Rocklake must maintain its writer lock to function as a catalog. Cloud Run's default scale-to-zero behavior would drop the writer lock. Setting a minimum of 1 instance keeps Rocklake alive continuously.

## Connecting DuckDB

Connect DuckDB to a Rocklake instance running on GCS:

```sql
INSTALL ducklake;
LOAD ducklake;

-- Connect to local Rocklake
ATTACH 'ducklake:host=localhost;port=5432' AS lake;

-- Or connect to a remote Rocklake endpoint
ATTACH 'ducklake:host=rocklake.internal;port=5432' AS lake;

-- Verify the catalog is accessible
SELECT * FROM lake.information_schema.tables;
```

DuckDB communicates with Rocklake over the PostgreSQL wire protocol. DuckDB's own GCS credentials (for reading Parquet data files) are configured separately using DuckDB's `gcs_hmac_key_id` and `gcs_hmac_secret` settings or by using the `google_cloud_storage_oauth_token` secret type.

## Performance Characteristics

GCS provides more consistent latency than S3 Standard for object operations:

| Operation | GCS Standard | GCS Nearline | Notes |
|-----------|-------------|--------------|-------|
| PUT (WAL segment) | 20–60 ms | Not suitable | Nearline has retrieval fees |
| GET (SST block) | 10–40 ms | 40–80 ms | Standard preferred for catalogs |
| LIST | 20–50 ms | 20–50 ms | Same performance tier |
| Point read (cached) | < 1 ms | < 1 ms | Block cache hit |

For catalog-intensive workloads, GCS Standard in the same region as your compute is the right choice. Nearline and Coldline are designed for infrequent access and are inappropriate for active catalog storage.

## Monitoring

Enable GCS access logs to track catalog request patterns:

```bash
# Create a logging bucket
gcloud storage buckets create gs://my-lakehouse-logs \
  --location=us-central1

# Enable access logging on the catalog bucket
gcloud storage buckets update gs://my-lakehouse \
  --log-bucket=gs://my-lakehouse-logs \
  --log-object-prefix=catalog-access-
```

Useful Cloud Monitoring metrics:
- `storage.googleapis.com/api/request_count` — Total requests by method (GET, PUT, DELETE)
- `storage.googleapis.com/api/total_latencies` — End-to-end request latency
- `storage.googleapis.com/storage/object_count` — Total objects in the catalog prefix (monitor for unbounded growth)

## Troubleshooting

**`google.golang.org/grpc: code = PermissionDenied`** — The service account does not have the required permissions on the bucket. Re-check the IAM binding and condition syntax.

**`storage: bucket doesn't exist`** — The bucket name is wrong or in a different project. Verify with `gcloud storage ls gs://my-lakehouse`.

**`The user-specified credential type is not supported`** — You are using a key type (e.g., a user account key) that is not supported. Use a service account key instead.

**Metadata server unavailable** — On Compute Engine or GKE, this usually means the network path to `169.254.169.254` is blocked. Check your VPC firewall rules.

## Further Reading

- **[Credential Isolation](credential-isolation.md)** — Separate IAM identities for catalog and data plane
- **[Kubernetes Deployment](kubernetes.md)** — Full GKE deployment guide with Workload Identity
- **[Object Store Durability](../concepts/object-store-durability.md)** — Why object storage provides strong durability guarantees
