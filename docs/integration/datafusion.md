# DataFusion

The `slateduck-datafusion` crate provides a `CatalogProvider` for Apache DataFusion.

## Usage

```rust
use slateduck_datafusion::SlateDuckCatalogProvider;
use datafusion::prelude::*;

let ctx = SessionContext::new();
let provider = SlateDuckCatalogProvider::new(store);
ctx.register_catalog("lake", Arc::new(provider));
let df = ctx.sql("SELECT * FROM lake.analytics.events").await?;
```

## What It Provides

- List schemas and tables
- Get table schemas
- Predicate pushdown via file-column statistics

## What It Does Not Provide

Write operations. DataFusion integration is read-only.
