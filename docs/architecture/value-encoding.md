# Value Encoding

All values are Protobuf-encoded messages prefixed with a 4-byte magic header.

## Format

```
[magic: 4 bytes "SDKV"] [protobuf message: variable]
```

The `SDKV` magic provides corruption detection and format identification.

## Schema Evolution

Protobuf's field-number encoding provides forward and backward compatibility:

- Adding a field: old readers skip unknown field numbers
- Removing a field: new readers ignore missing fields
- Changing a type: not supported (use new field number)

## Message Types

| Tag | Message Type |
|-----|-------------|
| `0x03` | `SnapshotValue` |
| `0x05` | `TableValue` |
| `0x06` | `ColumnValue` |
| `0x07` | `DataFileValue` |
| `0x09` | `FileColumnStatsValue` |

Fields in the key are NOT duplicated in the value.
