# Multi-Region

## S3 Cross-Region Replication

```mermaid
graph LR
    subgraph us-east-1
        W[Writer] --> B1[S3 Primary]
    end
    subgraph eu-west-1
        R1[Reader] --> B2[S3 Replica]
    end
    B1 -->|CRR| B2
```

Writer runs in primary region. Readers in other regions use the replica bucket. Replication lag: typically seconds.

## Considerations

- Replication lag means secondary readers may see stale data
- Writer MUST run in the primary bucket region
- CRR incurs data transfer charges
