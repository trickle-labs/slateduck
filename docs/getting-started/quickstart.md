# Quickstart (Local)

This guide gets SlateDuck running on your machine with the local filesystem backend in under 5 minutes.

## Prerequisites

- Rust toolchain (1.75+): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- DuckDB (1.2.2+): [duckdb.org/docs/installation](https://duckdb.org/docs/installation)

## Build

```bash
git clone https://github.com/geir-gronmo/slateduck.git
cd slateduck
cargo build --release
```

## Start the Sidecar

```bash
./target/release/slateduck serve --catalog-path /tmp/my-lakehouse
```

The sidecar starts listening on `localhost:5432`.

## Connect from DuckDB

```sql
INSTALL ducklake;
INSTALL postgres;
LOAD ducklake;
LOAD postgres;
ATTACH 'ducklake:postgres:host=localhost port=5432 dbname=warehouse' AS lake;
USE lake;

CREATE TABLE events (id BIGINT, event_type VARCHAR, ts TIMESTAMP);
INSERT INTO events VALUES (1, 'click', '2024-01-01 10:00:00');
SELECT * FROM events WHERE event_type = 'click';
```

## What Just Happened?

1. `slateduck serve` initialized an empty catalog in `/tmp/my-lakehouse/`
2. DuckDB connected via the PG wire protocol
3. `CREATE TABLE` registered the table schema in the catalog
4. `INSERT` created a Parquet file and registered it in the catalog
5. `SELECT` looked up which files contain data, then DuckDB read them directly
