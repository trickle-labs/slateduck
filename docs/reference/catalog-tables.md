# Catalog Tables

All 28 DuckLake v1.0 catalog tables:

| Tag | Table | Type |
|-----|-------|------|
| `0x01` | `ducklake_catalog` | Singleton |
| `0x02` | `ducklake_schema` | Versioned |
| `0x03` | `ducklake_snapshot` | Append-only |
| `0x04` | `ducklake_snapshot_changes` | Append-only |
| `0x05` | `ducklake_table` | Versioned |
| `0x06` | `ducklake_column` | Versioned |
| `0x07` | `ducklake_data_file` | Versioned |
| `0x08` | `ducklake_delete_file` | Versioned |
| `0x09` | `ducklake_file_column_stats` | Versioned |
| `0x0A` | `ducklake_table_stats` | Point-write |
| `0x0B` | `ducklake_metadata` | Point-write |
| `0x0C` | `ducklake_partition_key` | Versioned |
| `0x0D` | `ducklake_partition` | Versioned |
| `0x0E` | `ducklake_secret` | Point-write |
| `0x0F` | `ducklake_table_function` | Versioned |
| `0x10` | `ducklake_scalar_function` | Versioned |
| `0x11` | `ducklake_macro` | Versioned |
| `0x12` | `ducklake_index` | Versioned |
| `0x13` | `ducklake_view` | Versioned |
| `0x14` | `ducklake_table_sort_order` | Versioned |
| `0x15` | `ducklake_table_column_mapping` | Versioned |
| `0x16` | `ducklake_inlined_data_insert` | Versioned |
| `0x17` | `ducklake_inlined_data_delete` | Versioned |
| `0x18` | `ducklake_tag` | Point-write |
| `0x19` | `ducklake_extension` | Versioned |
| `0x1A` | `ducklake_transaction` | Append-only |
| `0x1B` | `ducklake_type` | Versioned |
| `0x1C` | `ducklake_comment` | Point-write |

## Types

- **Singleton:** One row per catalog
- **Versioned:** MVCC with begin/end snapshot
- **Append-only:** Never modified after creation
- **Point-write:** Simple KV, no versioning
