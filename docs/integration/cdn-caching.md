# CDN Caching for Catalog Keys

RockLake's storage design provides a strong **key immutability guarantee** that
makes every catalog-data key safely cacheable by a CDN or HTTP cache.

## The Immutability Property

> **Committed catalog facts are never physically deleted by normal operation
> and are always readable at the `dl_snapshot_id` at which they were written.**

This means:

- A key written at snapshot *N* has the same value forever.
- `GET /catalog/<key>` can be cached indefinitely using the key itself as the
  cache-control identifier.
- No cache invalidation is needed for historical snapshot reads.

## CloudFront Distribution

Example CloudFront distribution configuration for caching catalog keys:

```yaml
# CloudFront distribution (pseudocode / Terraform-style)
distribution:
  origin:
    domain: my-rocklake-bucket.s3express.us-east-1.amazonaws.com
    path_prefix: /catalog/

  cache_behaviour:
    path_pattern: /catalog/data/*   # immutable catalog-data keys
    ttl:
      default: 86400                # 1 day
      max: 31536000                 # 1 year
    cache_key_policy:
      headers: []                   # no Vary headers needed
      query_strings: none
      cookies: none
    compress: true

  cache_behaviour:
    path_pattern: /catalog/system/*  # writer-epoch, retain-from, etc.
    ttl:
      default: 0                     # do NOT cache mutable system keys
      max: 0
```

## Cache-Control Key

Use the **SlateDB checkpoint generation** as the `ETag` or `Cache-Control` key
for cache validation:

```
Cache-Control: public, max-age=31536000, immutable
ETag: "<checkpoint-generation>"
```

For system keys (writer epoch, retain-from, hot-key) always use
`Cache-Control: no-store` because these are mutable infrastructure keys.

## Lambda@Edge Origin Logic

```javascript
// Lambda@Edge origin-request handler
exports.handler = async (event) => {
    const request = event.Records[0].cf.request;
    const uri = request.uri;

    // Immutable catalog-data keys: 0x01–0x1F prefix range
    if (uri.match(/\/catalog\/[\x01-\x1f]/)) {
        // Safe to cache indefinitely
        return {
            ...request,
            headers: {
                ...request.headers,
                'cache-control': [{ value: 'public, max-age=31536000, immutable' }],
            },
        };
    }

    // System keys (0xFF prefix): never cache
    if (uri.match(/\/catalog\/\xff/)) {
        return {
            ...request,
            headers: {
                ...request.headers,
                'cache-control': [{ value: 'no-store' }],
            },
        };
    }

    return request;
};
```

## Key Prefix Reference

| Prefix range | Type | Cacheable? |
|---|---|---|
| `0x01`–`0x1F` | DuckLake catalog-data (MVCC rows) | Yes — immutable once written |
| `0x21`–`0x25` | Secondary indexes, leases, encryption | Yes — immutable once written |
| `0xFC`–`0xFD` | Internal indexes, inlined rows | Yes — immutable once written |
| `0xFE` | Counters | No — mutable |
| `0xFF` | System keys (epoch, retain-from, checkpoint pins) | No — mutable |

## Verifying Immutability

The test `catalog_data_keys_are_immutable_after_1000_writes` in
`crates/rocklake-catalog/tests/v043_scale_tests.rs` verifies this property
across 1000 write cycles.

## Related

- [Lambda Reader Pattern](lambda-reader.md)
- [S3 Express Validation](../performance/s3-express-validation.md)
- [Key Layout](../architecture/key-layout.md)
