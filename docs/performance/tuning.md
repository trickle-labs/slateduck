# Tuning

## Profiles

### `default`
Balanced. Block size 4 KB, bloom 10 bits/key, L0 threshold 4.

### `high_ingest`
Write-heavy. Block size 8 KB, L0 threshold 8, aggressive tombstone merge.

### `read_heavy`
Read-heavy. Bloom 14 bits/key, L0 threshold 2, block cache 64 MB.

## Individual Settings

| Setting | Default | Effect |
|---------|---------|--------|
| Block size | 4 KB | Larger = less metadata overhead, more read amplification |
| Bloom bits/key | 10 | Higher = fewer false positives |
| L0 threshold | 4 | Higher = more write throughput, worse reads |
| Block cache | 16 MB | Larger = fewer object-store reads |
