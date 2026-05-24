# Deploying on AWS S3

Amazon S3 is the most widely used object-store backend for SlateDuck in production. It provides eleven-nines durability, tight IAM integration, multiple performance tiers (Standard, S3 Express One Zone), and the richest ecosystem of tooling for monitoring, lifecycle management, and replication. This guide covers everything you need to deploy SlateDuck against S3 in production: bucket configuration, IAM policy design, environment variable setup, performance tier selection, deployment patterns, cost optimization, and troubleshooting.

## Prerequisites

Before starting, you need:

- An AWS account with permission to create S3 buckets and IAM roles/policies
- The SlateDuck binary (see [Binary Deployment](binary.md) or [Docker](docker.md))
- AWS credentials with sufficient permissions for the initial setup
- The AWS CLI installed for bucket creation steps (or equivalent access through the AWS Console)

## Bucket Configuration

### Creating the Bucket

SlateDuck requires a dedicated prefix within an S3 bucket for catalog storage. You can share a bucket with your Parquet data files (using separate prefixes) or use a dedicated bucket for the catalog. Both approaches work; the tradeoffs are discussed in [Credential Isolation](credential-isolation.md).

Create a bucket in your preferred region:

```bash
aws s3api create-bucket \
  --bucket my-lakehouse \
  --region us-east-1

# For regions other than us-east-1, you need LocationConstraint:
aws s3api create-bucket \
  --bucket my-lakehouse \
  --region eu-west-1 \
  --create-bucket-configuration LocationConstraint=eu-west-1
```

### Recommended Bucket Settings

**Disable public access.** Catalog data should never be publicly accessible:

```bash
aws s3api put-public-access-block \
  --bucket my-lakehouse \
  --public-access-block-configuration \
  "BlockPublicAcls=true,IgnorePublicAcls=true,BlockPublicPolicy=true,RestrictPublicBuckets=true"
```

**Enable versioning (optional but recommended).** S3 versioning provides an additional layer of protection against accidental deletion of catalog objects:

```bash
aws s3api put-bucket-versioning \
  --bucket my-lakehouse \
  --versioning-configuration Status=Enabled
```

**Enable server-side encryption.** AWS SSE-S3 (AES-256) is applied at no additional cost and protects at-rest data against storage-level compromise:

```bash
aws s3api put-bucket-encryption \
  --bucket my-lakehouse \
  --server-side-encryption-configuration '{
    "Rules": [{
      "ApplyServerSideEncryptionByDefault": {
        "SSEAlgorithm": "AES256"
      },
      "BucketKeyEnabled": true
    }]
  }'
```

For higher compliance requirements, use SSE-KMS with a customer-managed key:

```bash
aws s3api put-bucket-encryption \
  --bucket my-lakehouse \
  --server-side-encryption-configuration '{
    "Rules": [{
      "ApplyServerSideEncryptionByDefault": {
        "SSEAlgorithm": "aws:kms",
        "KMSMasterKeyID": "arn:aws:kms:us-east-1:123456789012:key/your-key-id"
      },
      "BucketKeyEnabled": true
    }]
  }'
```

### Object Lifecycle Policies

By default, SlateDuck's immutability guarantee means old SST files are never deleted. To manage storage costs, you can configure S3 lifecycle rules that delete objects past a certain age. Be careful: this should only apply to catalog objects that SlateDuck has already explicitly GC'd and no longer references. Deleting objects that SlateDB still references will corrupt the catalog.

The safe approach is to first run `slateduck gc` to advance the retention horizon and compact old data, then use lifecycle rules to clean up orphaned objects. See [Garbage Collection and Retention](../operations/garbage-collection.md) for the correct sequence.

## IAM Configuration

### Minimal Permissions for SlateDuck

SlateDuck needs the following S3 permissions on the catalog prefix:

| Action | Purpose |
|--------|---------|
| `s3:GetObject` | Reading SST files and WAL segments |
| `s3:PutObject` | Writing WAL segments and new SST files |
| `s3:DeleteObject` | Compaction cleanup (deleting merged SST files) |
| `s3:ListBucket` | Manifest reconciliation and compaction planning |
| `s3:HeadObject` | Checking object existence without reading content |

Create an IAM policy:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "SlateDuckCatalogAccess",
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
      "Sid": "SlateDuckListBucket",
      "Effect": "Allow",
      "Action": "s3:ListBucket",
      "Resource": "arn:aws:s3:::my-lakehouse",
      "Condition": {
        "StringLike": {
          "s3:prefix": "catalog/*"
        }
      }
    }
  ]
}
```

Save this as `slateduck-catalog-policy.json` and create it:

```bash
aws iam create-policy \
  --policy-name SlateDuckCatalogAccess \
  --policy-document file://slateduck-catalog-policy.json
```

### IAM Role for EC2/ECS/EKS

For production deployments running on AWS infrastructure, use IAM roles rather than static credentials. This eliminates the need to distribute access keys and provides automatic credential rotation.

For EC2:

```bash
# Create the role
aws iam create-role \
  --role-name SlateDuckRole \
  --assume-role-policy-document '{
    "Version": "2012-10-17",
    "Statement": [{
      "Effect": "Allow",
      "Principal": {"Service": "ec2.amazonaws.com"},
      "Action": "sts:AssumeRole"
    }]
  }'

# Attach the policy
aws iam attach-role-policy \
  --role-name SlateDuckRole \
  --policy-arn arn:aws:iam::123456789012:policy/SlateDuckCatalogAccess

# Create an instance profile
aws iam create-instance-profile --instance-profile-name SlateDuckProfile
aws iam add-role-to-instance-profile \
  --instance-profile-name SlateDuckProfile \
  --role-name SlateDuckRole
```

For EKS (Kubernetes), use IRSA (IAM Roles for Service Accounts):

```bash
# Annotate the Kubernetes service account
kubectl annotate serviceaccount slateduck \
  -n slateduck-namespace \
  eks.amazonaws.com/role-arn=arn:aws:iam::123456789012:role/SlateDuckRole
```

### Separate Credentials for Catalog and Data

A critical security practice is to use separate IAM identities for the catalog plane (SlateDuck) and the data plane (DuckDB writing Parquet files). This limits the blast radius if either component is compromised. See [Credential Isolation](credential-isolation.md) for the complete guide; the short version is:

- SlateDuck's IAM identity: `s3:*` on `catalog/*` prefix only
- DuckDB writer's IAM identity: `s3:PutObject` on `data/*` prefix only
- DuckDB reader's IAM identity: `s3:GetObject` on `data/*` prefix only

## Environment Variables

With static credentials (development only — use IAM roles in production):

```bash
export AWS_REGION=us-east-1
export AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE
export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY
```

With an assumed role:

```bash
export AWS_REGION=us-east-1
export AWS_ROLE_ARN=arn:aws:iam::123456789012:role/SlateDuckRole
export AWS_WEB_IDENTITY_TOKEN_FILE=/var/run/secrets/eks.amazonaws.com/serviceaccount/token
```

When running on EC2 with an instance profile, no environment variables are needed — SlateDuck uses the Instance Metadata Service (IMDS) automatically.

## Starting SlateDuck

Point SlateDuck at your S3 catalog prefix:

```bash
slateduck serve \
  --catalog s3://my-lakehouse/catalog/ \
  --bind 0.0.0.0:5432
```

Expected startup output:

```
INFO  SlateDuck v0.8.0 starting
INFO  Storage backend: aws-s3
INFO  Catalog path: s3://my-lakehouse/catalog/
INFO  Opening SlateDB...
INFO  Catalog format version: v1
INFO  Next snapshot ID: 42
INFO  Writer epoch: 7
INFO  Listening on 0.0.0.0:5432
INFO  Ready to accept connections
```

If you see `ERROR Failed to open catalog: credential error`, your AWS credentials are not configured correctly. Run `aws s3 ls s3://my-lakehouse/catalog/` to verify credentials independently.

## Connecting DuckDB

Once SlateDuck is running, connect DuckDB using the `ducklake` extension:

```sql
-- Install the extension (one time)
INSTALL ducklake;
LOAD ducklake;

-- Connect to the catalog
ATTACH 'ducklake:host=localhost;port=5432' AS lake;

-- List schemas
SHOW ALL TABLES FROM lake;

-- Create a table
CREATE TABLE lake.analytics.events (
  event_id BIGINT,
  user_id BIGINT,
  event_type VARCHAR,
  created_at TIMESTAMP
);

-- Insert data
INSERT INTO lake.analytics.events VALUES
  (1, 42, 'page_view', '2024-06-01 10:00:00'),
  (2, 42, 'click', '2024-06-01 10:01:00');
```

## S3 Express One Zone

S3 Express One Zone is a high-performance storage class that provides 10× lower latency than S3 Standard at modestly higher cost. For SlateDuck, this translates to 3–10 ms catalog operations instead of 20–50 ms, which matters for:

- Applications where query startup latency is perceptible to end users
- High-frequency metadata operations (many short transactions per second)
- Writer fencing and takeover recovery, which completes in ~10 seconds on Express vs. ~30 seconds on Standard

Directory buckets (required for S3 Express) use a slightly different naming convention and API:

```bash
# Create a directory bucket (includes AZ suffix)
aws s3api create-bucket \
  --bucket my-lakehouse--use1-az4--x-s3 \
  --region us-east-1 \
  --create-bucket-configuration '{
    "Location": {"Type": "AvailabilityZone", "Name": "use1-az4"},
    "Bucket": {"Type": "Directory", "DataRedundancy": "SingleAvailabilityZone"}
  }'
```

Point SlateDuck at the directory bucket:

```bash
slateduck serve \
  --catalog s3express://my-lakehouse--use1-az4--x-s3/catalog/ \
  --bind 0.0.0.0:5432
```

!!! warning "Single AZ"
    S3 Express One Zone stores data in a single availability zone. This reduces the durability guarantee from eleven nines to nine nines (99.999999%) and means a full AZ outage makes the catalog unavailable until the AZ recovers. For most workloads this trade-off is acceptable; for systems requiring the highest possible durability, use S3 Standard.

## Path-Style vs. Virtual-Hosted-Style Addressing

By default, SlateDuck uses virtual-hosted-style addressing: `https://my-bucket.s3.amazonaws.com/key`. This is AWS's preferred style and works for all modern deployments. Path-style addressing (`https://s3.amazonaws.com/my-bucket/key`) is deprecated by AWS but may be required for:

- S3-compatible services that don't support virtual-hosted-style (MinIO in some configurations)
- Testing against local S3 emulators

To force path-style addressing:

```bash
slateduck serve \
  --catalog s3://my-lakehouse/catalog/ \
  --s3-path-style \
  --bind 0.0.0.0:5432
```

## Multi-Region and Cross-Region Replication

For the highest available catalog durability, enable S3 Cross-Region Replication (CRR):

```bash
# Create replication role
aws iam create-role \
  --role-name S3ReplicationRole \
  --assume-role-policy-document '{
    "Version": "2012-10-17",
    "Statement": [{
      "Effect": "Allow",
      "Principal": {"Service": "s3.amazonaws.com"},
      "Action": "sts:AssumeRole"
    }]
  }'

# Configure replication on the source bucket
aws s3api put-bucket-replication \
  --bucket my-lakehouse \
  --replication-configuration '{
    "Role": "arn:aws:iam::123456789012:role/S3ReplicationRole",
    "Rules": [{
      "ID": "CatalogReplication",
      "Status": "Enabled",
      "Filter": {"Prefix": "catalog/"},
      "Destination": {
        "Bucket": "arn:aws:s3:::my-lakehouse-dr",
        "ReplicationTime": {"Status": "Enabled", "Time": {"Minutes": 15}},
        "Metrics": {"Status": "Enabled", "EventThreshold": {"Minutes": 15}}
      }
    }]
  }'
```

SlateDuck always reads and writes from the primary bucket. The replica is used only for disaster recovery: if the primary region becomes unavailable, you can reconfigure SlateDuck to point at the replica bucket and resume operations. Note that any writes that occurred between the last replication and the failure will be lost — CRR is asynchronous.

## Cost Optimization

S3 costs for a typical SlateDuck catalog are modest but worth understanding:

**Storage costs.** A catalog tracking 100,000 Parquet files might occupy 500 MB of SST storage. At $0.023/GB/month (S3 Standard), this is about $0.01/month. Even after years of operation with full history retained, catalog storage costs are typically negligible.

**Request costs.** S3 Standard charges $0.0004 per 1,000 PUT requests and $0.0004 per 1,000 GET requests. A moderately active deployment making 1,000 catalog requests per hour generates about $0.29/month in request charges. For high-throughput deployments, these costs become more significant.

**Data transfer costs.** Data transfer between SlateDuck and S3 within the same region is free. Cross-region traffic is charged at standard AWS data transfer rates. Deploy SlateDuck in the same region as your bucket.

To minimize request costs at scale:
- Use SlateDB's block cache aggressively (increase `--block-cache-size`)
- Batch writes into larger transactions to reduce WAL PUT frequency
- Schedule compaction during off-peak hours to batch SST writes

## Monitoring and Troubleshooting

**Check if the catalog is accessible:**

```bash
aws s3 ls s3://my-lakehouse/catalog/ --recursive | head -20
```

You should see SlateDB SST files (`.sst` extension) and WAL segments (`.wal`). If the listing is empty on first startup, the catalog will be initialized when SlateDuck first opens it.

**Common errors and fixes:**

| Error | Cause | Fix |
|-------|-------|-----|
| `NoCredentialProviders` | No AWS credentials found | Set `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` or attach an IAM role |
| `AccessDenied` on PUT | Missing `s3:PutObject` permission | Add `s3:PutObject` to the IAM policy |
| `AccessDenied` on LIST | Missing `s3:ListBucket` permission | Add `s3:ListBucket` with the correct prefix condition |
| `NoSuchBucket` | Bucket does not exist or wrong region | Verify bucket name and `AWS_REGION` setting |
| `SlowDown` (503) | S3 request throttling | Reduce concurrent SlateDuck connections or distribute load |

**S3 request metrics.** Enable S3 server access logging or use AWS CloudWatch metrics to monitor request rates, error rates, and latency. Metrics to watch: `NumberOfObjects` (catalog size growth), `TotalRequestLatency` (S3-side latency), `4xxErrors` (configuration or permission issues).

## Further Reading

- **[Credential Isolation](credential-isolation.md)** — Separate IAM identities for catalog and data plane access
- **[Docker Deployment](docker.md)** — Running SlateDuck in containers on EC2 or ECS
- **[Kubernetes Deployment](kubernetes.md)** — Running SlateDuck on EKS with IRSA
- **[Object Store Durability](../concepts/object-store-durability.md)** — The conceptual basis for why object storage provides strong durability
- **[Performance: Latency Model](../performance/latency-model.md)** — Expected latency numbers for S3 Standard vs. S3 Express
