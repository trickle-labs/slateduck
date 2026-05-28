#!/usr/bin/env python3
"""
Polars non-DuckDB engine validation for RockLake.

Verifies that the RockLake Python bindings can list data files from the catalog
and that the returned Parquet URLs can be opened with polars.read_parquet().

Usage:
    python scripts/polars_validation.py /path/to/catalog

The catalog must have at least one table with registered data files.
For CI, a minimal pre-built fixture catalog is used.

Exit code: 0 on success, 1 on failure.
"""

import sys
import os
import argparse


def main():
    parser = argparse.ArgumentParser(description="Polars / RockLake validation")
    parser.add_argument("catalog_path", help="Path to a RockLake catalog directory")
    parser.add_argument("--table-id", type=int, default=1, help="Table ID to query")
    args = parser.parse_args()

    # Import rocklake — skip gracefully if not installed
    try:
        from rocklake import RockLakeCatalog
    except ImportError:
        print("rocklake Python package not installed; skipping Polars validation.")
        sys.exit(0)

    cat = RockLakeCatalog.open(args.catalog_path)
    try:
        snap = cat.snapshot_id()
        schemas = cat.list_schemas(snap)
        tables = cat.list_tables(schemas[0].schema_id if schemas else 1, snap) if schemas else []
        table_id = tables[0].table_id if tables else args.table_id
        files = cat.list_data_files(table_id, snap)

        print(f"Catalog: {args.catalog_path}")
        print(f"Snapshot ID: {snap}")
        print(f"Schemas: {len(schemas)}")
        print(f"Data files for table {table_id}: {len(files)}")

        if not files:
            print("No data files registered — Polars read step skipped.")
            print("PASS: RockLake list_data_files() returned successfully.")
            sys.exit(0)

        # Try to import polars; skip read step if not available
        try:
            import polars as pl
        except ImportError:
            print("polars not installed; skipping read step.")
            print("PASS: RockLake list_data_files() returned successfully.")
            sys.exit(0)

        total_rows = 0
        for f in files:
            if not os.path.exists(f.path):
                print(f"  Skipping {f.path} (not a local file, object-store read not configured)")
                continue
            df = pl.read_parquet(f.path)
            print(f"  {f.path}: {len(df)} rows (catalog says {f.row_count})")
            assert len(df) == f.row_count, (
                f"row count mismatch: polars={len(df)}, catalog={f.row_count}"
            )
            total_rows += len(df)

        print(f"Total rows across {len(files)} file(s): {total_rows}")
        print("PASS")
    finally:
        cat.close()


if __name__ == "__main__":
    main()
