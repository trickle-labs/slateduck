# rocklake-client

Idiomatic async Rust client for the [RockLake](https://github.com/trickle-labs/rocklake) catalog.

## Usage

```rust
use rocklake_client::{CatalogClient, CatalogClientBuilder};

#[tokio::main]
async fn main() {
    let client = CatalogClientBuilder::new("file:///path/to/catalog")
        .build()
        .await
        .unwrap();

    let snapshot = client.snapshot_id().await.unwrap();
    println!("current snapshot: {snapshot}");

    let schemas = client.list_schemas(snapshot).await.unwrap();
    println!("schemas: {schemas:?}");

    client.close().await;
}
```

For synchronous contexts:

```rust
use rocklake_client::CatalogClientSync;

let client = CatalogClientSync::open("file:///path/to/catalog").unwrap();
let schemas = client.list_schemas(0).unwrap();
client.close();
```

## License

Apache-2.0
