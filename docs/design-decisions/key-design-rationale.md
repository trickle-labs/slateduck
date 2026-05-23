# Key Design Rationale

## Why Tag-Byte Prefix

Ensures prefix scan isolation. 256 possible values. No tag collision.

## Why Big-Endian Integers

Preserves numeric ordering in lexicographic byte comparison. Prefix scans return results in ID order.

## Why `begin_snapshot` in Key

Makes each version a distinct KV pair (immutability). Prefix scan returns all versions; MVCC filter selects correct one.

## Why `table_id` Before `column_id` in Stats

Enables dominant access pattern: `prune_files(table_id, column_id, predicate)` with single prefix scan.

## Why Counters Under `0xFE`

Can be read independently. Counter allocation and row insertion commit atomically in same transaction.
