# Catalog vs. Data Plane

SlateDuck operates exclusively in the **catalog plane**.

## The Catalog Plane

The catalog plane answers: *what data exists, where is it stored, and how is it organized?*

It stores: table/schema definitions, column names and types, file registrations, column statistics, snapshot history, partition definitions.

The catalog is small — typically megabytes, occasionally gigabytes.

## The Data Plane

The data plane stores actual data — Parquet files containing the rows that users query. DuckDB reads and writes these files directly. SlateDuck never sees data content.

## Why the Separation Matters

1. SlateDuck's performance depends on catalog size, not data size
2. DuckDB reads data directly from storage with no proxy overhead
3. The catalog can be backed up and time-traveled independently of the data
4. Catalog corruption does not corrupt data (Parquet files are still intact)
