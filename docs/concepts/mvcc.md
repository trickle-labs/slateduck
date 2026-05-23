# MVCC

Multi-Version Concurrency Control (MVCC) enables snapshot isolation, time travel, and immutability to coexist.

## How It Works

Versioned tables have two version fields:

- `begin_snapshot`: When this version became visible
- `end_snapshot`: When this version was superseded (NULL if current)

A row is **visible** at target snapshot `T` when:

```
begin_snapshot <= T AND (end_snapshot IS NULL OR T < end_snapshot)
```

## Example

| Version | begin_snapshot | end_snapshot | Visible at 5? | Visible at 9? |
|---------|---------------|--------------|---------------|---------------|
| v1 | 3 | 7 | Yes | No |
| v2 | 7 | NULL | No | Yes |

## Why Application-Level MVCC?

SlateDB is a simple KV store without built-in MVCC. SlateDuck implements MVCC by including `begin_snapshot` in the key (making each version a distinct KV pair) and applying the visibility filter after every prefix scan.
