# Excision

Irreversible physical deletion for compliance (GDPR, CCPA).

## Commands

```bash
# Plan (dry run)
slateduck excise plan --catalog-path s3://bucket/catalogs/warehouse \
  --table analytics.users --snapshot-range 1-50

# Execute (irreversible!)
slateduck excise apply --catalog-path s3://bucket/catalogs/warehouse \
  --table analytics.users --snapshot-range 1-50 \
  --reason "GDPR erasure request #12345"
```

## Audit Trail

Every excision produces a signed, timestamped audit record. The fact of deletion is preserved; the deleted data is not.

!!! warning
    Excision cannot be undone. Verify scope before executing.
