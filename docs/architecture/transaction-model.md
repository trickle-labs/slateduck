# Transaction Model

Every catalog mutation is atomic via SlateDB's `DbTransaction`.

## Transaction Lifecycle

```mermaid
sequenceDiagram
    participant W as Writer
    participant DB as SlateDB
    participant S3 as Object Store
    W->>DB: begin_transaction()
    W->>DB: get(counter_key)
    DB-->>W: current_value
    W->>W: compute mutations
    W->>DB: put(keys, values)
    W->>DB: commit()
    DB->>S3: PutObject(WAL segment)
    S3-->>DB: 200 OK
    DB-->>W: success
```

## Batch Size Limits

Maximum transaction batch: 64 MiB. Exceeding returns `SQLSTATE 54001`.
