# Your First Lakehouse

The quickstart guides showed you how to start Rocklake and create a table. But a single table is not a lakehouse. A lakehouse is an evolving system — schemas change as business requirements shift, data accumulates across dozens or hundreds of tables, multiple teams query the same catalog concurrently, and operational tasks like garbage collection and inspection become part of the daily rhythm. This tutorial builds a realistic lakehouse from scratch and exercises every major feature of Rocklake along the way.

By the end of this guide, you will have created a multi-table schema, loaded data, evolved the schema over time, performed time travel queries, run garbage collection, inspected internal state, and configured retention policies. More importantly, you will have developed the mental model of how Rocklake behaves in production — not as a toy with one table, but as a system managing real-world complexity.

## The Scenario

You are building the analytics infrastructure for a mid-size e-commerce company called "TechMart." The company sells electronics online and has three main data domains:

- **Customers** — who buys products, their contact information, and when they joined
- **Products** — the catalog of items for sale with pricing and categories
- **Orders** — purchase transactions linking customers to products

Over the next several months, the business will evolve: new columns will be needed, tables will be renamed, new data will arrive continuously, and various teams will need to query the catalog at different points in time for auditing and debugging purposes.

## Setting Up the Environment

Start Rocklake with local storage for this tutorial. Everything you learn here applies identically to cloud storage — the only difference is the `--storage` path:

```bash
rocklake serve --catalog file:///tmp/techmart-lakehouse --bind 127.0.0.1:5432
```

In a separate terminal, open DuckDB and connect:

```sql
INSTALL ducklake;
LOAD ducklake;
ATTACH '' AS techmart (TYPE ducklake, PG 'host=127.0.0.1 port=5432');
USE techmart;
```

You are now connected to an empty catalog. The initial snapshot (snapshot 1) has been created automatically and represents the "empty catalog" state.

## Phase 1: Building the Initial Schema

A well-organized lakehouse separates concerns into schemas. Let's create two: one for raw ingested data and one for curated analytics tables:

```sql
-- Create schemas (snapshot 2)
CREATE SCHEMA raw;
CREATE SCHEMA analytics;
```

Now create the initial tables in the `raw` schema:

```sql
-- Customer master data (snapshot 3)
CREATE TABLE raw.customers (
    customer_id BIGINT,
    email VARCHAR,
    full_name VARCHAR,
    signup_date DATE,
    country VARCHAR
);

-- Product catalog (snapshot 4)
CREATE TABLE raw.products (
    product_id BIGINT,
    name VARCHAR,
    category VARCHAR,
    price DECIMAL(10, 2),
    in_stock BOOLEAN
);

-- Order transactions (snapshot 5)
CREATE TABLE raw.orders (
    order_id BIGINT,
    customer_id BIGINT,
    order_date TIMESTAMP,
    status VARCHAR,
    total_amount DECIMAL(12, 2)
);

-- Order line items (snapshot 6)
CREATE TABLE raw.order_items (
    item_id BIGINT,
    order_id BIGINT,
    product_id BIGINT,
    quantity INTEGER,
    unit_price DECIMAL(10, 2)
);
```

Each DDL statement creates a new snapshot. You can verify this:

```sql
SELECT snapshot_id, snapshot_time FROM ducklake_snapshots() ORDER BY snapshot_id;
```

You should see 6 snapshots, each representing a state transition in the catalog. Snapshot 1 is the empty catalog, snapshot 2 has the schemas, and snapshots 3–6 each add a table.

### What Happened Internally

When you created `raw.customers`, Rocklake:

1. Allocated a unique `schema_id` for `raw` and a unique `table_id` for `customers`
2. Created column rows for each of the 5 columns, each with a unique `column_id`
3. Set `begin_snapshot` on every new row to the current snapshot ID
4. Recorded the new snapshot in `ducklake_snapshot` with the current timestamp
5. Wrote all of this as a single atomic batch to SlateDB

The entire operation — even though it involves multiple catalog rows — is a single atomic write. If the process crashes mid-way, nothing is visible to any reader.

## Phase 2: Loading Initial Data

In a real lakehouse, data arrives from external systems — CSV exports, API responses, streaming events. DuckDB makes it easy to load data from many sources. For this tutorial, we'll use inline VALUES to keep things self-contained:

```sql
-- Load customer data (snapshot 7)
INSERT INTO raw.customers VALUES
    (1, 'alice@techmart.com', 'Alice Johnson', '2024-01-05', 'US'),
    (2, 'bob@techmart.com', 'Bob Chen', '2024-01-12', 'CA'),
    (3, 'carol@techmart.com', 'Carol Okafor', '2024-02-01', 'UK'),
    (4, 'dave@techmart.com', 'Dave Mueller', '2024-02-15', 'DE'),
    (5, 'eve@techmart.com', 'Eve Santos', '2024-03-01', 'BR');

-- Load product catalog (snapshot 8)
INSERT INTO raw.products VALUES
    (101, 'Wireless Headphones', 'Audio', 89.99, true),
    (102, 'Mechanical Keyboard', 'Peripherals', 149.99, true),
    (103, '4K Monitor', 'Displays', 449.99, true),
    (104, 'USB-C Hub', 'Accessories', 59.99, true),
    (105, 'Webcam HD', 'Video', 79.99, false);

-- Load order data (snapshot 9)
INSERT INTO raw.orders VALUES
    (1001, 1, '2024-03-01 10:30:00', 'completed', 239.98),
    (1002, 1, '2024-03-15 14:22:00', 'completed', 149.99),
    (1003, 2, '2024-03-20 09:15:00', 'shipped', 509.98),
    (1004, 3, '2024-04-01 16:45:00', 'completed', 89.99),
    (1005, 4, '2024-04-10 11:00:00', 'pending', 209.98);

-- Load order items (snapshot 10)
INSERT INTO raw.order_items VALUES
    (1, 1001, 101, 1, 89.99),
    (2, 1001, 102, 1, 149.99),
    (3, 1002, 102, 1, 149.99),
    (4, 1003, 103, 1, 449.99),
    (5, 1003, 104, 1, 59.99),
    (6, 1004, 101, 1, 89.99),
    (7, 1005, 104, 2, 59.99),
    (8, 1005, 101, 1, 89.99);
```

Each INSERT causes DuckDB to write one or more Parquet files and register them with Rocklake. The catalog now stores not just the schema but also the metadata for each data file: its path, row count, file size, and column-level statistics (min/max values) that enable predicate pushdown during query planning.

Let's verify the data is accessible:

```sql
SELECT c.full_name, COUNT(o.order_id) as total_orders, SUM(o.total_amount) as lifetime_value
FROM raw.customers c
JOIN raw.orders o ON c.customer_id = o.customer_id
GROUP BY c.full_name
ORDER BY lifetime_value DESC;
```

You should see Alice at the top with 2 orders totaling $389.97, followed by Bob with 1 order at $509.98 (a single expensive order for a monitor and hub).

## Phase 3: Schema Evolution

Two months pass. The business has new requirements:

1. The marketing team needs a `phone` column on customers for SMS campaigns
2. The operations team wants to track `shipping_address` on orders
3. The product team realized `in_stock` should be an integer (stock quantity) not a boolean
4. Analytics wants to rename `raw.orders` to `raw.order_transactions` for clarity

Let's evolve the schema:

```sql
-- Add phone column (snapshot 11)
ALTER TABLE raw.customers ADD COLUMN phone VARCHAR;

-- Add shipping address (snapshot 12)
ALTER TABLE raw.orders ADD COLUMN shipping_address VARCHAR;

-- The product team's request is more involved: DuckLake doesn't support ALTER COLUMN TYPE,
-- so we'll add a new column and keep the old one for backward compatibility (snapshot 13)
ALTER TABLE raw.products ADD COLUMN stock_quantity INTEGER;

-- Rename the table (snapshot 14)
ALTER TABLE raw.orders RENAME TO raw.order_transactions;
```

Now update some data with the new columns:

```sql
-- Update customer phone numbers (snapshot 15)
UPDATE raw.customers SET phone = '+1-555-0101' WHERE customer_id = 1;
UPDATE raw.customers SET phone = '+1-416-555-0102' WHERE customer_id = 2;

-- Add stock quantities (snapshot 16)
UPDATE raw.products SET stock_quantity = 150 WHERE product_id = 101;
UPDATE raw.products SET stock_quantity = 75 WHERE product_id = 102;
UPDATE raw.products SET stock_quantity = 30 WHERE product_id = 103;
UPDATE raw.products SET stock_quantity = 200 WHERE product_id = 104;
UPDATE raw.products SET stock_quantity = 0 WHERE product_id = 105;
```

### What Happened to the Schema

Let's look at what Rocklake did internally during these schema changes:

- **ADD COLUMN** created a new `ColumnRow` with `begin_snapshot = current`. Queries at older snapshots do not see this column because their snapshot ID is less than the column's `begin_snapshot`.

- **RENAME TABLE** created a new version of the table name: the old `TableRow` received an `end_snapshot`, and a new `TableRow` was created with the new name and `begin_snapshot = current`. Queries at older snapshots still see the old name.

- **UPDATE** in DuckLake creates a delete file (marking old rows as removed) and a new data file (with updated values). The catalog registers both files with appropriate snapshot IDs.

The critical insight: nothing was overwritten. Every previous state of the catalog is still accessible because all changes are additive.

## Phase 4: Time Travel

Now let's use time travel to see the catalog at different points in history. First, find out what snapshots exist:

```sql
SELECT snapshot_id, snapshot_time FROM ducklake_snapshots() ORDER BY snapshot_id;
```

### Querying Before Schema Evolution

To see the catalog as it was before any schema changes (before snapshot 11), open a new DuckDB connection with a snapshot parameter:

```sql
-- In a new DuckDB session:
LOAD ducklake;
ATTACH '' AS techmart_v10 (TYPE ducklake, PG 'host=127.0.0.1 port=5432', SNAPSHOT '10');

-- The table is still called "orders" (not "order_transactions")
SELECT * FROM techmart_v10.raw.orders LIMIT 2;

-- The customers table has only 5 columns (no phone)
SELECT column_name FROM information_schema.columns
WHERE table_schema = 'raw' AND table_name = 'customers';
```

This is invaluable for debugging. If a report produced different results last month, you can query at the exact snapshot that was current at that time and see exactly what the catalog looked like.

### Reproducible Analytics

Time travel enables reproducible analytics. Suppose you ran a quarterly report at snapshot 10:

```sql
-- Reproduce the exact same query at the exact same catalog state
ATTACH '' AS q1_report (TYPE ducklake, PG 'host=127.0.0.1 port=5432', SNAPSHOT '10');

SELECT 
    p.category,
    COUNT(DISTINCT oi.order_id) as orders,
    SUM(oi.quantity * oi.unit_price) as revenue
FROM q1_report.raw.order_items oi
JOIN q1_report.raw.products p ON oi.product_id = p.product_id
GROUP BY p.category
ORDER BY revenue DESC;
```

No matter how much the schema has evolved since snapshot 10, this query returns the same results it would have returned on the day the snapshot was created. The catalog state at snapshot 10 is immutable — it cannot be changed by any subsequent operation.

## Phase 5: Building Analytics Views

Now let's use the `analytics` schema for curated views:

```sql
-- Create a customer lifetime value view (snapshot 17)
CREATE VIEW analytics.customer_ltv AS
SELECT 
    c.customer_id,
    c.full_name,
    c.country,
    COUNT(o.order_id) as total_orders,
    SUM(o.total_amount) as lifetime_value,
    MIN(o.order_date) as first_order,
    MAX(o.order_date) as last_order
FROM raw.customers c
LEFT JOIN raw.order_transactions o ON c.customer_id = o.customer_id
GROUP BY c.customer_id, c.full_name, c.country;

-- Create a product performance view (snapshot 18)
CREATE VIEW analytics.product_performance AS
SELECT 
    p.product_id,
    p.name as product_name,
    p.category,
    p.price as current_price,
    COUNT(oi.item_id) as times_ordered,
    SUM(oi.quantity) as total_units_sold,
    SUM(oi.quantity * oi.unit_price) as total_revenue
FROM raw.products p
LEFT JOIN raw.order_items oi ON p.product_id = oi.product_id
GROUP BY p.product_id, p.name, p.category, p.price;
```

Views in DuckLake are stored as SQL text in the catalog. They do not materialize data — DuckDB expands them at query time. But they are versioned like everything else: if you drop or modify a view, the old definition is still visible at older snapshots.

```sql
-- Query the analytics views
SELECT * FROM analytics.customer_ltv ORDER BY lifetime_value DESC;
SELECT * FROM analytics.product_performance ORDER BY total_revenue DESC;
```

## Phase 6: Continuous Data Loading

In production, data arrives continuously. Let's simulate a new batch of orders:

```sql
-- New orders arrive (snapshot 19)
INSERT INTO raw.order_transactions VALUES
    (1006, 5, '2024-04-15 08:30:00', 'completed', 149.99, '123 Main St, São Paulo'),
    (1007, 1, '2024-04-20 19:00:00', 'shipped', 449.99, '456 Oak Ave, Portland'),
    (1008, 3, '2024-04-25 12:15:00', 'pending', 209.98, '789 High St, London');

-- New order items (snapshot 20)
INSERT INTO raw.order_items VALUES
    (9, 1006, 102, 1, 149.99),
    (10, 1007, 103, 1, 449.99),
    (11, 1008, 104, 2, 59.99),
    (12, 1008, 101, 1, 89.99);

-- A new customer signs up (snapshot 21)
INSERT INTO raw.customers VALUES
    (6, 'frank@techmart.com', 'Frank Yamamoto', '2024-04-20', 'JP', '+81-3-5555-0106');
```

Each batch of inserts creates a new snapshot. The catalog now contains 21 snapshots, each representing a distinct state of the lakehouse. Any analytics query can be pinned to any of these 21 states.

## Phase 7: Garbage Collection

After several months of operation, the catalog has accumulated many snapshots. Most are no longer needed for auditing — you only need the last 30 days of history. Let's configure garbage collection:

```bash
# Check current state
rocklake inspect --catalog file:///tmp/techmart-lakehouse
```

This shows you the current snapshot count, storage usage, and retention configuration.

```bash
# Advance the retention horizon to 30 days
rocklake gc advance --catalog file:///tmp/techmart-lakehouse --retain-days 30
```

After this command:

- Snapshots older than 30 days are no longer queryable via time travel
- The catalog data is still physically present (nothing has been deleted)
- Future queries specifying a snapshot older than the horizon receive an error

If you also want to reclaim storage (optional and irreversible):

```bash
# Physically remove superseded rows older than the horizon
rocklake excise --catalog file:///tmp/techmart-lakehouse
```

Excision deletes key-value pairs from SlateDB that are no longer needed — old versions of columns that have been superseded, data file entries that have been deleted, and snapshot records older than the horizon. This is the only destructive operation in Rocklake, and it requires explicit invocation.

### Pinning Important Snapshots

Before running GC, you might want to pin specific snapshots that should never be garbage collected — for example, the snapshot at the end of each fiscal quarter:

```bash
rocklake pin-snapshot --catalog file:///tmp/techmart-lakehouse --snapshot-id 10 --reason "Q1 2024 close"
rocklake pin-snapshot --catalog file:///tmp/techmart-lakehouse --snapshot-id 18 --reason "Q2 2024 close"
```

Pinned snapshots are protected from both horizon advancement and excision. They remain queryable indefinitely until explicitly unpinned.

## Phase 8: Inspecting the Catalog

At any point, you can inspect the internal state of the catalog to understand its health and contents:

```bash
rocklake inspect --catalog file:///tmp/techmart-lakehouse
```

Expected output (approximately):

```
Rocklake Catalog Inspector
============================
Storage:        file:///tmp/techmart-lakehouse
Format version: 8
Writer epoch:   1
Current snapshot: 21
Retention from: 0 (no GC applied)
Pinned snapshots: 10, 18

Schemas:    2 (raw, analytics)
Tables:     4 (raw.customers, raw.products, raw.order_transactions, raw.order_items)
Views:      2 (analytics.customer_ltv, analytics.product_performance)
Columns:    25 (across all tables)
Data files: 12
Delete files: 5

Storage breakdown:
  Manifest:  1.2 KB
  WAL:       15.4 KB
  SST files: 48.7 KB
  Total:     65.3 KB
```

The catalog for this tutorial — with 4 tables, 2 views, 21 snapshots, and all their historical versions — occupies about 65 KB. This illustrates why catalog storage cost is never a concern: even a catalog tracking thousands of tables and millions of data files typically occupies less than 100 MB.

## What You Have Learned

This tutorial exercised the complete lifecycle of a Rocklake lakehouse:

1. **Schema design** — Creating schemas to organize tables by domain (raw vs. analytics)
2. **Table creation** — Each DDL creates a new snapshot, tables are fully versioned from birth
3. **Data loading** — INSERT writes Parquet files and registers them with the catalog including statistics
4. **Schema evolution** — ADD COLUMN, RENAME TABLE, and UPDATE all create versioned state without destroying history
5. **Time travel** — Any historical snapshot is directly queryable by specifying its ID in the ATTACH statement
6. **Reproducible analytics** — Pinning queries to specific snapshots guarantees identical results over time
7. **Views** — SQL views are catalog objects, versioned and time-travel-capable like everything else
8. **Garbage collection** — Two-phase process (advance horizon, then optionally excise) with full operator control
9. **Snapshot pinning** — Protect important snapshots from GC for compliance or audit requirements
10. **Inspection** — Full visibility into catalog internals at any time

The key mental model: Rocklake is an append-only catalog where every mutation creates a new version. History accumulates naturally. You choose how much history to keep through retention policies. Readers can query any retained snapshot without coordination with the writer.

## Next Steps

You now have a solid foundation for working with Rocklake. Here are the recommended paths depending on your role:

- **For architects:** [Concepts](../concepts/index.md) — Deep understanding of immutability, MVCC, time travel, and scale-out
- **For operators:** [Operations](../operations/index.md) — Production operational procedures for GC, monitoring, backup, and troubleshooting
- **For developers:** [Architecture](../architecture/index.md) — How the Rust crates fit together and where to contribute
- **For deployers:** [Deployment](../deployment/index.md) — Docker, Kubernetes, Lambda, and multi-region patterns
