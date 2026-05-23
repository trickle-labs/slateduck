# Key Layout

Every key follows: `[tag: 1 byte] [field_1: N bytes] [field_2: M bytes] ...`

## Tag Byte Ranges

| Range | Purpose |
|-------|---------|
| `0x01`-`0x1C` | DuckLake catalog tables (28 tables) |
| `0x1D`-`0xFB` | Reserved for future tables |
| `0xFC` | Secondary indexes |
| `0xFD` | Dynamic inlined rows |
| `0xFE` | Counters |
| `0xFF` | System keys |

## Key Definitions (Selected)

### `ducklake_snapshot` (Tag `0x03`)
```
0x03 | snapshot_id: u64 BE
```

### `ducklake_table` (Tag `0x05`)
```
0x05 | schema_id: u64 BE | table_id: u64 BE | begin_snapshot: u64 BE
```

### `ducklake_column` (Tag `0x06`)
```
0x06 | table_id: u64 BE | column_id: u64 BE | begin_snapshot: u64 BE
```

### `ducklake_data_file` (Tag `0x07`)
```
0x07 | table_id: u64 BE | data_file_id: u64 BE | begin_snapshot: u64 BE
```

### `ducklake_file_column_stats` (Tag `0x09`)
```
0x09 | table_id: u64 BE | column_id: u64 BE | data_file_id: u64 BE
```

## Encoding Rules

1. All integers are unsigned, big-endian
2. Variable-length fields are length-prefixed (u16 BE)
3. All keys are self-delimiting
