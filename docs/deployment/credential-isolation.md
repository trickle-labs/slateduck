# Credential Isolation

One of the most important security practices for a SlateDuck deployment is keeping the credentials for the **catalog plane** (SlateDuck's access to SlateDB's SST files and WAL segments) completely separate from the credentials for the **data plane** (DuckDB's access to Parquet data files). When credentials are isolated, a compromise of any single component can only access what that component legitimately needs — it cannot pivot to the other plane. This page explains why isolation matters, how to implement it on each major cloud provider, and how to verify that your isolation is effective.

## Why Credential Isolation Matters

Without isolation, a single set of credentials provides access to both your catalog metadata and your analytical data. This creates unnecessary risk:

**Compromised catalog → data exfiltration.** If an attacker gains the credentials used by SlateDuck (perhaps through a configuration file in a compromised container or a logged environment variable), they can use those credentials to read all your Parquet data files — even though SlateDuck itself never reads those files. The credentials did not need to grant data access, but they did.

**Compromised data writer → catalog corruption.** If an attacker gains the write credentials used by a data ingestion pipeline, without isolation they could also write to the catalog prefix — corrupting catalog metadata or injecting malicious file registrations that point DuckDB at attacker-controlled files.

**Broad blast radius.** Without isolation, the failure mode for any credential compromise is "full lakehouse access." With isolation, the failure mode is either "catalog metadata access" (limited impact on confidentiality) or "data access" (limited impact on integrity), never both simultaneously.

The principle of least privilege — giving each component only the permissions it needs to function — is the right design for any production system, and the catalog/data plane separation makes it unusually clean to implement for SlateDuck.

## What Each Component Needs

Before configuring credentials, map out what each component actually accesses:

**SlateDuck (catalog plane):**
- Needs: Read/write/delete on the catalog prefix (e.g., `s3://bucket/catalog/`)
- Needs: List on the catalog prefix (for SST file enumeration during manifest reconciliation)
- Does NOT need: Any access to the data prefix (e.g., `s3://bucket/data/`)
- Does NOT need: Access to other buckets

**DuckDB data writer (ingestion pipelines):**
- Needs: Write and delete on the data prefix (e.g., `s3://bucket/data/`)
- Needs: List on the data prefix (for pipeline coordination)
- Needs: Write to SlateDuck via PG wire (to register files in the catalog)
- Does NOT need: Direct access to the catalog prefix
- Does NOT need: Read access to catalog SST files

**DuckDB data reader (query execution):**
- Needs: Read on the data prefix (e.g., `s3://bucket/data/`)
- Needs: Read from SlateDuck via PG wire (to look up catalog metadata)
- Does NOT need: Any direct bucket access beyond the data prefix
- Does NOT need: Write access anywhere

## AWS S3: IAM Policy Design

Create three separate IAM identities: one for SlateDuck, one for data writers, one for data readers.

### SlateDuck Catalog Policy

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "CatalogObjectOperations",
      "Effect": "Allow",
      "Action": [
        "s3:GetObject",
        "s3:PutObject",
        "s3:DeleteObject",
        "s3:HeadObject"
      ],
      "Resource": "arn:aws:s3:::my-lakehouse/catalog/*"
    },
    {
      "Sid": "CatalogListOperations",
      "Effect": "Allow",
      "Action": "s3:ListBucket",
      "Resource": "arn:aws:s3:::my-lakehouse",
      "Condition": {
        "StringLike": {
          "s3:prefix": ["catalog/*", "catalog/"]
        }
      }
    }
  ]
}
```

### Data Writer Policy

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "DataWriteOperations",
      "Effect": "Allow",
      "Action": [
        "s3:GetObject",
        "s3:PutObject",
        "s3:DeleteObject",
        "s3:HeadObject"
      ],
      "Resource": "arn:aws:s3:::my-lakehouse/data/*"
    },
    {
      "Sid": "DataListOperations",
      "Effect": "Allow",
      "Action": "s3:ListBucket",
      "Resource": "arn:aws:s3:::my-lakehouse",
      "Condition": {
        "StringLike": {
          "s3:prefix": ["data/*", "data/"]
        }
      }
    }
  ]
}
```

### Data Reader Policy

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "DataReadOperations",
      "Effect": "Allow",
      "Action": [
        "s3:GetObject",
        "s3:HeadObject"
      ],
      "Resource": "arn:aws:s3:::my-lakehouse/data/*"
    },
    {
      "Sid": "DataListForPlanning",
      "Effect": "Allow",
      "Action": "s3:ListBucket",
      "Resource": "arn:aws:s3:::my-lakehouse",
      "Condition": {
        "StringLike": {
          "s3:prefix": ["data/*", "data/"]
        }
      }
    }
  ]
}
```

### Creating IAM Roles

For EC2/ECS/EKS workloads, create IAM roles instead of static users:

```bash
# Create roles
aws iam create-role --role-name SlateDuckCatalogRole \
  --assume-role-policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}'

aws iam create-role --role-name DuckDBWriterRole \
  --assume-role-policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}'

aws iam create-role --role-name DuckDBReaderRole \
  --assume-role-policy-document '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"ec2.amazonaws.com"},"Action":"sts:AssumeRole"}]}'

# Create and attach policies
aws iam create-policy --policy-name SlateDuckCatalogPolicy \
  --policy-document file://slateduck-catalog-policy.json
aws iam attach-role-policy --role-name SlateDuckCatalogRole \
  --policy-arn arn:aws:iam::123456789012:policy/SlateDuckCatalogPolicy

# Repeat for writer and reader roles...
```

### Verifying the Isolation

Test that the SlateDuck role cannot access data files:

```bash
# Assume the SlateDuck role
aws sts assume-role \
  --role-arn arn:aws:iam::123456789012:role/SlateDuckCatalogRole \
  --role-session-name test-isolation

# Export the temporary credentials
export AWS_ACCESS_KEY_ID=...
export AWS_SECRET_ACCESS_KEY=...
export AWS_SESSION_TOKEN=...

# This should SUCCEED (catalog access)
aws s3 ls s3://my-lakehouse/catalog/
# Expected: list of SST files

# This should FAIL (data access denied)
aws s3 ls s3://my-lakehouse/data/
# Expected: AccessDenied error
```

If the data access check succeeds when it should fail, your policy conditions are not working correctly. Review the `StringLike` condition syntax in the `ListBucket` statement.

## Google Cloud Storage: Service Account Isolation

### Three Service Accounts

```bash
# Create the service accounts
gcloud iam service-accounts create slateduck-catalog \
  --display-name="SlateDuck Catalog" --project=my-project

gcloud iam service-accounts create duckdb-writer \
  --display-name="DuckDB Data Writer" --project=my-project

gcloud iam service-accounts create duckdb-reader \
  --display-name="DuckDB Data Reader" --project=my-project
```

### Scoped IAM Bindings

Grant each service account access only to its respective prefix using IAM conditions:

```bash
STORAGE_ID=$(gcloud storage buckets describe gs://my-lakehouse \
  --format="value(name)")

# SlateDuck: catalog prefix only
gcloud storage buckets add-iam-policy-binding gs://my-lakehouse \
  --member="serviceAccount:slateduck-catalog@my-project.iam.gserviceaccount.com" \
  --role="roles/storage.objectAdmin" \
  --condition='title=CatalogOnly,expression=resource.name.startsWith("projects/_/buckets/my-lakehouse/objects/catalog/")'

# DuckDB writer: data prefix, write access
gcloud storage buckets add-iam-policy-binding gs://my-lakehouse \
  --member="serviceAccount:duckdb-writer@my-project.iam.gserviceaccount.com" \
  --role="roles/storage.objectAdmin" \
  --condition='title=DataWrite,expression=resource.name.startsWith("projects/_/buckets/my-lakehouse/objects/data/")'

# DuckDB reader: data prefix, read access
gcloud storage buckets add-iam-policy-binding gs://my-lakehouse \
  --member="serviceAccount:duckdb-reader@my-project.iam.gserviceaccount.com" \
  --role="roles/storage.objectViewer" \
  --condition='title=DataRead,expression=resource.name.startsWith("projects/_/buckets/my-lakehouse/objects/data/")'
```

## Azure Blob Storage: Managed Identity Isolation

### Three Managed Identities

```bash
az identity create --name slateduck-catalog --resource-group slateduck-rg
az identity create --name duckdb-writer --resource-group slateduck-rg
az identity create --name duckdb-reader --resource-group slateduck-rg

STORAGE_ID=$(az storage account show --name mylakehouse \
  --resource-group slateduck-rg --query id --output tsv)
```

### Scoped RBAC Assignments

```bash
# SlateDuck: catalog container access
CATALOG_SCOPE="${STORAGE_ID}/blobServices/default/containers/catalog"

SLATEDUCK_PRINCIPAL=$(az identity show --name slateduck-catalog \
  --resource-group slateduck-rg --query principalId --output tsv)

az role assignment create \
  --assignee-object-id $SLATEDUCK_PRINCIPAL \
  --assignee-principal-type ServicePrincipal \
  --role "Storage Blob Data Contributor" \
  --scope $CATALOG_SCOPE

# DuckDB writer: data container write access
DATA_SCOPE="${STORAGE_ID}/blobServices/default/containers/data"

WRITER_PRINCIPAL=$(az identity show --name duckdb-writer \
  --resource-group slateduck-rg --query principalId --output tsv)

az role assignment create \
  --assignee-object-id $WRITER_PRINCIPAL \
  --assignee-principal-type ServicePrincipal \
  --role "Storage Blob Data Contributor" \
  --scope $DATA_SCOPE

# DuckDB reader: data container read access
READER_PRINCIPAL=$(az identity show --name duckdb-reader \
  --resource-group slateduck-rg --query principalId --output tsv)

az role assignment create \
  --assignee-object-id $READER_PRINCIPAL \
  --assignee-principal-type ServicePrincipal \
  --role "Storage Blob Data Reader" \
  --scope $DATA_SCOPE
```

## Deploying with Isolated Credentials

### Kubernetes Example

In Kubernetes, create separate pods/deployments with different service accounts:

```yaml
# SlateDuck deployment with catalog credentials
apiVersion: apps/v1
kind: Deployment
metadata:
  name: slateduck
spec:
  template:
    spec:
      serviceAccountName: slateduck-sa  # Has catalog access
      containers:
        - name: slateduck
          image: ghcr.io/slateduck/slateduck:latest
          args: [serve, --catalog, s3://my-lakehouse/catalog/, --bind, 0.0.0.0:5432]
---
# DuckDB ingestion job with data writer credentials
apiVersion: batch/v1
kind: Job
metadata:
  name: data-ingestion
spec:
  template:
    spec:
      serviceAccountName: duckdb-writer-sa  # Has data write access
      containers:
        - name: ingestion
          image: duckdb/duckdb:latest
          # DuckDB connects to SlateDuck via PG wire for catalog,
          # and uses AWS credentials for direct S3 data writes
```

### Docker Compose Example

```yaml
services:
  slateduck:
    image: ghcr.io/slateduck/slateduck:latest
    command: serve --catalog s3://bucket/catalog/ --bind 0.0.0.0:5432
    environment:
      # Catalog-only credentials
      AWS_ACCESS_KEY_ID: ${SLATEDUCK_AWS_KEY}
      AWS_SECRET_ACCESS_KEY: ${SLATEDUCK_AWS_SECRET}
      AWS_REGION: us-east-1

  ingestion:
    image: my-pipeline:latest
    environment:
      # Data-write credentials
      AWS_ACCESS_KEY_ID: ${WRITER_AWS_KEY}
      AWS_SECRET_ACCESS_KEY: ${WRITER_AWS_SECRET}
      AWS_REGION: us-east-1
      # PG wire endpoint for catalog registration
      SLATEDUCK_ENDPOINT: slateduck:5432
```

## Network-Level Isolation

Beyond IAM/RBAC isolation, consider network-level controls:

**VPC/VNet placement.** Run SlateDuck and DuckDB within the same VPC/VNet, with the SlateDuck port (5432) accessible only within the VPC. This prevents external access to the catalog even if credentials are leaked — an attacker would also need network access.

**Security groups/firewall rules.** Restrict inbound connections to SlateDuck's port to only the IP ranges of known DuckDB clients:

```bash
# AWS Security Group: allow port 5432 only from the ingestion subnet
aws ec2 authorize-security-group-ingress \
  --group-id sg-slateduck \
  --protocol tcp \
  --port 5432 \
  --cidr 10.0.0.0/8

# Deny everything else
aws ec2 revoke-security-group-ingress \
  --group-id sg-slateduck \
  --protocol tcp \
  --port 5432 \
  --cidr 0.0.0.0/0
```

**Object storage VPC endpoints.** Use VPC endpoints for S3 (AWS) or Private Service Connect (GCS) to keep all catalog I/O within the private network. This prevents catalog SST files from transiting the public internet even though S3 is technically a public service.

## Auditing Credential Usage

Enable access logging to verify that the isolation is working and detect any unexpected access patterns:

```bash
# AWS: Enable S3 server access logging
aws s3api put-bucket-logging \
  --bucket my-lakehouse \
  --bucket-logging-status '{
    "LoggingEnabled": {
      "TargetBucket": "my-lakehouse-logs",
      "TargetPrefix": "s3-access-"
    }
  }'

# Alert if the catalog credential accesses the data prefix
# (this should never happen if isolation is configured correctly)
```

Set up monitoring rules that alert when:
- The SlateDuck IAM role accesses any path outside `catalog/`
- The DuckDB writer role accesses any path outside `data/`
- Any identity accesses the `catalog/` prefix directly (bypassing SlateDuck)

Unexpected cross-prefix access is a signal that either the IAM configuration is wrong or credentials have been misappropriated.

## Further Reading

- **[AWS S3 Deployment](aws-s3.md)** — Full AWS S3 deployment guide with IAM role setup
- **[GCS Deployment](gcs.md)** — Full GCS deployment guide with service account setup
- **[Azure Deployment](azure.md)** — Full Azure deployment guide with Managed Identity setup
- **[TLS and Authentication](tls.md)** — Securing the PG-wire channel between DuckDB and SlateDuck
- **[Concepts: Catalog vs Data](../concepts/catalog-vs-data.md)** — The architectural basis for the two-plane separation
