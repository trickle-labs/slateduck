"""
RockLake Python — 10-line example.

Usage:
    pip install rocklake
    python examples/quickstart.py /path/to/catalog
"""

import sys
from rocklake import RockLakeCatalog

catalog_path = sys.argv[1] if len(sys.argv) > 1 else "/tmp/demo-catalog"

cat = RockLakeCatalog.open(catalog_path)
snap = cat.snapshot_id()
schemas = cat.list_schemas(snap)
print(f"Snapshot {snap}: {len(schemas)} schema(s)")
for s in schemas:
    tables = cat.list_tables(s.schema_id, snap)
    print(f"  {s.schema_name}: {len(tables)} table(s)")
cat.close()
