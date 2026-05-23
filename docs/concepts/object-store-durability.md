# Object-Store Durability

SlateDB uses object storage as both its write-ahead log (WAL) and sorted data store (SST files).

## Write Path

1. Writes accumulate in an in-memory memtable
2. When full, the memtable is flushed as a WAL segment (one PutObject)
3. Once PutObject succeeds, data is durable (S3: 11 nines)
4. Background compaction merges WAL segments into sorted SST files

## Durability Guarantee

A committed transaction is durable once the WAL PutObject succeeds. Even if the process crashes immediately after, the data is recoverable.

## Consistency Model

S3 provides strong read-after-write consistency (since December 2020). A successful PutObject is immediately readable.

## The Cost

Every commit has at least one round-trip of object-store latency (30-60 ms on S3 Standard).
