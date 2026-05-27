# Object-Store and Credential Isolation Spike — Phase 0

> Validates that separate IAM policies can isolate catalog access from data access.

## Setup

Two IAM policy sets tested against MinIO (S3-compatible):

### Policy: `catalog-only`
```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": ["s3:GetObject", "s3:PutObject", "s3:ListBucket", "s3:DeleteObject"],
    "Resource": [
      "arn:aws:s3:::bucket/catalogs/*",
      "arn:aws:s3:::bucket"
    ],
    "Condition": {
      "StringLike": {"s3:prefix": ["catalogs/*"]}
    }
  }]
}
```

### Policy: `data-only`
```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": ["s3:GetObject", "s3:PutObject", "s3:ListBucket", "s3:DeleteObject"],
    "Resource": [
      "arn:aws:s3:::bucket/data/*",
      "arn:aws:s3:::bucket"
    ],
    "Condition": {
      "StringLike": {"s3:prefix": ["data/*"]}
    }
  }]
}
```

## Results

| Test | Policy | Result | Notes |
|------|--------|--------|-------|
| Sidecar opens SlateDB at `catalogs/warehouse-a` | `catalog-only` | **PASS** | Full read/write to catalog prefix |
| Sidecar reads from `data/` prefix | `catalog-only` | **FAIL (expected)** | Access denied as designed |
| DuckDB reads Parquet at `data/warehouse-a/` | `data-only` | **PASS** | Full read access to data files |
| DuckDB writes to `catalogs/` prefix | `data-only` | **FAIL (expected)** | Access denied as designed |
| GC/maintenance reads both prefixes | Both policies | **PASS** | Requires union of both policies |

## SQLSTATE Mapping for Permission Failures

| Object-Store Error | SQLSTATE | Condition |
|-------------------|----------|-----------|
| `403 Forbidden` | `42501` | Insufficient privilege |
| `AccessDenied` | `42501` | IAM policy denial |
| `404 Not Found` | `3D000` | Catalog path does not exist (on init) |
| `404 Not Found` | `02000` | Data file not found (on read) |

## Operational Requirements

- **Sidecar process:** needs `catalog-only` policy (or broader)
- **DuckDB client:** needs `data-only` policy for Parquet read/write
- **GC/maintenance job:** needs both `catalog-only` and `data-only`
- **Excision job:** needs both policies plus `s3:DeleteObject`

## Security Model

```
┌─────────────┐     catalog-only      ┌──────────────────┐
│   Rocklake │ ◄──────────────────── │  s3://bucket/    │
│   Sidecar   │                       │  catalogs/       │
└─────────────┘                       └──────────────────┘

┌─────────────┐     data-only         ┌──────────────────┐
│   DuckDB    │ ◄──────────────────── │  s3://bucket/    │
│   Client    │                       │  data/           │
└─────────────┘                       └──────────────────┘
```

This confirms the security model: the sidecar never touches data files,
and DuckDB clients never touch catalog state directly.
