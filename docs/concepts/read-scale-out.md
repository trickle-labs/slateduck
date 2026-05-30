# Read Scale-Out

> **Since:** v0.47.0

RockLake supports horizontally-scalable read fleets through **read-only catalog access**.
A read-only instance skips the CAS writer-epoch acquisition entirely, meaning any number
of reader pods can open the same catalog concurrently with **zero write conflicts** in the
underlying SlateDB key-value store.

## Architecture

```
                        ┌──────────────────────────────────────────┐
                        │           Object Storage (S3/GCS/AZ)      │
                        │           SlateDB SSTs + catalog data      │
                        └────────────────┬─────────────────────────┘
                                         │ read-only
              ┌──────────────────────────┼──────────────────────────┐
              │                          │                          │
       ┌──────┴──────┐           ┌───────┴─────┐           ┌────────┴────┐
       │  reader-0   │           │  reader-1   │           │  reader-N   │
       │  (no epoch) │           │  (no epoch) │           │  (no epoch) │
       └─────────────┘           └─────────────┘           └─────────────┘
              │ writes rejected (SQLSTATE 25006)
       ┌──────┴──────┐
       │   writer-0  │   ← single writer, epoch CAS acquired on open
       └─────────────┘
```

## How It Works

When `--read-only` is passed to `rocklake serve` (or `--mode reader`), the server calls
`CatalogStore::open_without_epoch()` instead of `CatalogStore::open()`. This skips the
`SYSTEM_WRITER_EPOCH` CAS key increment, leaving the SlateDB write log clean for the
actual writer replica.

Reader replicas still observe snapshot isolation: they read the snapshot that was latest
at the time of their last `refresh()` call (or at `open()`).

## Quick Start

### CLI

```bash
# Writer
rocklake serve --catalog s3://my-bucket/catalog --bind 0.0.0.0:5432

# Reader fleet (any number of pods)
rocklake serve --catalog s3://my-bucket/catalog --read-only --bind 0.0.0.0:5432
```

### Rust

```rust
use rocklake_catalog::{CatalogStore, OpenOptions};

// Open a read-only catalog (no writer epoch conflict)
let cat = CatalogStore::open_readonly(opts).await?;
let reader = cat.reader()?;
let schemas = reader.list_schemas().await?;
```

Using the high-level client:

```rust
use rocklake_client::CatalogClientBuilder;

let client = CatalogClientBuilder::new("s3://my-bucket/catalog")
    .build_readonly()
    .await?;
client.refresh().await?;          // advance to latest snapshot
let schemas = client.list_schemas().await?;
```

### Python

```python
from rocklake import RockLakeCatalog

cat = RockLakeCatalog.open_readonly("/path/to/catalog")
schemas = cat.list_schemas(0)
```

### Go

```go
cat, err := rocklake.OpenReadOnly("/path/to/catalog")
if err != nil { log.Fatal(err) }
defer cat.Close()
```

### Node.js

```js
const { Catalog } = require('rocklake');
const cat = Catalog.openReadonly('/path/to/catalog');
```

### Java

```java
try (RockLakeCatalog cat = RockLakeCatalog.openReadOnly("/path/to/catalog")) {
    long snap = cat.getSnapshot();
    List<DataFileRow> files = cat.listDataFiles("my_table");
}
```

## Connection Management

Reader fleet instances support two new server flags:

| Flag | Default | Description |
|------|---------|-------------|
| `--idle-connection-timeout <secs>` | 60 | Close idle connections after N seconds |
| `--drain-timeout <secs>` | 30 | Maximum wait for in-flight queries on SIGTERM |

## Snapshot Refresh

Readers cache the snapshot ID at open time. Call `refresh()` to advance to the latest
committed snapshot without restarting the process:

```rust
// Rust — ReadOnlyCatalog
let new_snap = cat.refresh().await?;

// Python
cat.refresh()

// Go — not yet exposed; re-open instead
```

## Kubernetes Deployment

See [Kubernetes Deployment](../deployment/kubernetes.md#reader-fleet) for a production-
ready Deployment manifest with HPA and PodDisruptionBudget.

## Guarantees and Limitations

| Property | Value |
|----------|-------|
| Writer-epoch conflicts on open | **0** |
| Snapshot isolation | **Per `refresh()` call** |
| Writes via reader | **Rejected** (SQLSTATE 25006) |
| Maximum simultaneous readers | Unlimited (bounded only by object-store API rate limits) |
| `refresh()` writes to object store | **None** |
