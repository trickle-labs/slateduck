# Verify & Repair

## Verify

```bash
slateduck verify --catalog-path s3://bucket/catalogs/warehouse
```

Checks: magic headers, Protobuf decode, key ordering, counter consistency, MVCC consistency, referential integrity, snapshot monotonicity.

## Repair

```bash
slateduck repair --catalog-path s3://bucket/catalogs/warehouse --dry-run
slateduck repair --catalog-path s3://bucket/catalogs/warehouse
```

Repairs: counter drift, orphaned stats, missing hot-key. Never deletes user data.
