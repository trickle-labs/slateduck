# Single Writer, Many Readers

SlateDuck follows a strict single-writer, many-readers concurrency model.

## The Writer

One process holds the writer lease. It can create snapshots, register files, alter schemas, and commit transactions. SlateDB's fencing mechanism rejects competing writers.

## The Readers

Any number of processes can read concurrently. Each reader sees a consistent point-in-time view. Readers never coordinate with the writer or each other.

## Why This Works

Lakehouse catalogs have an asymmetric workload:

- Writes are infrequent (one snapshot per batch of files)
- Reads are frequent (every query needs metadata)
- A typical deployment: 1 writer, 10-100 readers

The single-writer model perfectly matches this asymmetry.
