"""Integration tests for rocklake Python bindings.

Run with: pytest bindings/python/tests/
Requires the rocklake wheel to be installed first (maturin develop).
"""

import os
import tempfile

import pytest


def import_rocklake():
    """Import the rocklake module, skip if not available."""
    try:
        import rocklake  # noqa: F401
        return rocklake
    except ImportError:
        pytest.skip("rocklake wheel not installed (run: maturin develop)")


def test_open_close(tmp_path):
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    cat.close()


def test_snapshot_id_fresh_catalog(tmp_path):
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    snap = cat.snapshot_id()
    assert snap == 0, f"expected 0, got {snap}"
    cat.close()


def test_current_snapshot(tmp_path):
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    snap = cat.current_snapshot()
    assert snap.snapshot_id == 0
    cat.close()


def test_list_schemas_empty(tmp_path):
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    schemas = cat.list_schemas(0)
    assert schemas == [], f"expected empty, got {schemas}"
    cat.close()


def test_list_tables_empty(tmp_path):
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    tables = cat.list_tables(1, 0)
    assert tables == []
    cat.close()


def test_list_data_files_empty(tmp_path):
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    files = cat.list_data_files(1, 0)
    assert files == []
    cat.close()


def test_data_file_to_dict(tmp_path):
    """list_data_files() returns objects compatible with polars/pandas."""
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    # No files — just verify the method exists and returns an empty list
    files = cat.list_data_files(1, 0)
    assert isinstance(files, list)
    cat.close()


def test_close_is_idempotent(tmp_path):
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    cat.close()
    cat.close()  # second close must not raise


def test_repr(tmp_path):
    rl = import_rocklake()
    cat = rl.RockLakeCatalog.open(str(tmp_path))
    assert "RockLakeCatalog" in repr(cat)
    cat.close()
