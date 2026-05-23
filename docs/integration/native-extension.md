# Native Extension

Strategy C embeds SlateDuck directly in DuckDB via `slateduck-ffi`.

## Usage

```sql
INSTALL slateduck;
LOAD slateduck;
ATTACH 'ducklake:slatedb:s3://bucket/catalogs/warehouse' AS lake;
```

## How It Works

```
DuckDB Catalog Interface (C++) -> slateduck-ffi (C ABI) -> slateduck-catalog (Rust) -> SlateDB -> S3
```

## ABI Versioning

`slateduck_abi_version()` returns compile-time constant. Extension refuses to load on mismatch.

## Equivalence

Both Strategy B and Strategy C produce identical results. The test suite verifies this.
