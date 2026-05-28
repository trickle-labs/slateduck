# Embedded Client Library

RockLake ships a universal embedded client library that lets any language
ecosystem read and write the catalog without running a PG-wire sidecar.
DuckDB is a first-class consumer, but the library is intentionally
language-neutral.

## Deployment Options

| Option | Use Case |
|--------|----------|
| **Strategy B — PG-wire Sidecar** | DuckDB, psql, any Postgres-compatible client |
| **Embedded Client Library** *(this page)* | Rust, Python, Go, Node.js, any language with C FFI |
| **Native DuckDB Extension** (v0.36.0) | `ATTACH 'ducklake:slatedb:...' AS lake` — no sidecar |

The embedded library exposes a stable C ABI (`rocklake.h`) that all language
bindings wrap.  See [docs/reference/c-api.md](../reference/c-api.md) for the
full function reference.

---

## Rust

The `rocklake-client` crate is the idiomatic Rust entry point.  It wraps the
`rocklake-catalog` internals with an async-first API.

### Dependency

```toml
[dependencies]
rocklake-client = "0.35"
```

### Async API

```rust
use rocklake_client::CatalogClientBuilder;

#[tokio::main]
async fn main() {
    let client = CatalogClientBuilder::new("file:///path/to/catalog")
        .build()
        .await
        .unwrap();

    let snap = client.snapshot_id().await.unwrap();
    let schemas = client.list_schemas(snap).await.unwrap();

    for schema in &schemas {
        println!("schema: {}", schema.schema_name);
        let tables = client.list_tables(schema.schema_id, snap).await.unwrap();
        for table in &tables {
            let files = client.list_data_files(table.table_id, snap).await.unwrap();
            println!("  table {} → {} data files", table.table_name, files.len());
        }
    }

    client.close().await;
}
```

### Sync API

For contexts that cannot use async Rust (C extensions, Python GIL-holding code):

```rust
use rocklake_client::CatalogClientSync;

let client = CatalogClientSync::open("file:///path/to/catalog").unwrap();
let schemas = client.list_schemas(0).unwrap();
println!("{} schemas", schemas.len());
client.close();
```

---

## Python

Install the `rocklake` wheel from PyPI or build from source with `maturin`.

### Install

```sh
pip install rocklake
```

### Build from source

```sh
cd bindings/python
pip install maturin
maturin develop
```

### Usage

```python
from rocklake import RockLakeCatalog

cat = RockLakeCatalog.open("/path/to/catalog")

snap = cat.snapshot_id()
schemas = cat.list_schemas(snap)

for schema in schemas:
    tables = cat.list_tables(schema.schema_id, snap)
    for table in tables:
        files = cat.list_data_files(table.table_id, snap)
        print(f"{table.table_name}: {len(files)} data files")

cat.close()
```

### Polars Integration

`list_data_files()` returns objects with a `.to_dict()` method compatible with
`polars.from_dicts()`:

```python
import polars as pl
from rocklake import RockLakeCatalog

cat = RockLakeCatalog.open("/path/to/catalog")
snap = cat.snapshot_id()

# Get data file list
files = cat.list_data_files(table_id=1, snapshot_id=snap)

# Build a DataFrame of catalog metadata
meta_df = pl.from_dicts([f.to_dict() for f in files])

# Read actual Parquet data
parquet_df = pl.read_parquet([f.path for f in files])
print(parquet_df.head())

cat.close()
```

---

## Go

Install via `go get`:

```sh
go get github.com/trickle-labs/rocklake-go
```

### Prerequisites

- A pre-built `librocklake_ffi.a` static library for your platform (distributed
  as a GitHub release asset) **or** a local Rust build (`cargo build -p rocklake-ffi`).
- `cgo` enabled (default).

### Usage

```go
package main

import (
    "fmt"
    "log"

    rocklake "github.com/trickle-labs/rocklake-go"
)

func main() {
    cat, err := rocklake.Open("/path/to/catalog")
    if err != nil {
        log.Fatal(err)
    }
    defer cat.Close()

    snap, err := cat.SnapshotID()
    if err != nil {
        log.Fatal(err)
    }

    schemas, err := cat.ListSchemas(snap)
    if err != nil {
        log.Fatal(err)
    }

    for _, s := range schemas {
        fmt.Printf("schema: %s\n", s.SchemaName)
        tables, _ := cat.ListTables(s.SchemaID, snap)
        for _, t := range tables {
            files, _ := cat.ListDataFiles(t.TableID, snap)
            fmt.Printf("  table %s → %d files\n", t.TableName, len(files))
        }
    }
}
```

---

## Node.js

```sh
npm install @rocklake/client
```

### Usage

```js
const { Catalog } = require('@rocklake/client');

const cat = Catalog.open('/path/to/catalog');

const snap = cat.snapshotId();
const schemas = cat.listSchemas(snap);

for (const schema of schemas) {
    const tables = cat.listTables(schema.schemaId, snap);
    for (const table of tables) {
        const files = cat.listDataFiles(table.tableId, snap);
        console.log(`${table.tableName}: ${files.length} data files`);
    }
}

cat.close();
```

TypeScript type declarations are included (`index.d.ts`).

---

## Non-DuckDB Engine Matrix

| Engine | Integration Path | Status |
|--------|-----------------|--------|
| **Polars** (Python) | `list_data_files()` → `polars.read_parquet()` | ✅ Validated |
| **DataFusion** (Rust) | `rocklake-client` → `list_data_files()` | ✅ Validated |
| **Spark** (PySpark) | Python bindings → `list_data_files()` → `spark.read.parquet()` | Documented |
| **Trino** | Python/Go bindings → `list_data_files()` → Trino catalog connector | Documented |

### Spark

```python
from rocklake import RockLakeCatalog
from pyspark.sql import SparkSession

cat = RockLakeCatalog.open("/path/to/catalog")
snap = cat.snapshot_id()
files = cat.list_data_files(table_id=1, snapshot_id=snap)

spark = SparkSession.builder.getOrCreate()
df = spark.read.parquet(*[f.path for f in files])
df.show()

cat.close()
```

### Trino

For Trino and other JVM-based engines, use the Python or Go bindings to retrieve
the list of Parquet files and register them as external tables, or use the
PG-wire sidecar (Strategy B) which provides a standard PostgreSQL interface that
Trino can query via the `postgresql` connector.

---

## Object-Store URL Format

| Backend | Example URI |
|---------|-------------|
| Local filesystem | `file:///absolute/path` or bare path |
| Amazon S3 | `s3://bucket/prefix` |
| Google Cloud Storage | `gs://bucket/prefix` |
| Azure Blob Storage | `az://container/prefix` |

S3 / GCS / Azure credentials are resolved from environment variables following
the standard `object_store` crate conventions (AWS_ACCESS_KEY_ID, etc.).

---

## Versioning Policy

The C ABI (`ROCKLAKE_ABI_VERSION`) follows semver major bumps for
breaking changes.  Language binding packages follow the RockLake workspace
version.

When `ROCKLAKE_ABI_VERSION` changes, the old constant is kept as a deprecated
alias for one release cycle before removal.

---

## See Also

- [C API Reference](../reference/c-api.md)
- [Architecture: FFI Safety](../architecture/ffi-safety.md)
- [Native DuckDB Extension (v0.36.0)](native-extension.md)
