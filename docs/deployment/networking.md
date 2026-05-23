# Networking

## Recommended Architecture

```mermaid
graph TB
    subgraph VPC
        subgraph Private Subnet
            SD[SlateDuck Writer]
            S3E[S3 VPC Endpoint]
        end
        subgraph Public Subnet
            NLB[Network Load Balancer]
        end
    end
    DDB[DuckDB Clients] --> NLB
    NLB --> SD
    SD --> S3E
```

## Security Groups

| Direction | Port | Source | Purpose |
|-----------|------|--------|---------|
| Inbound | 5432 | NLB / VPC CIDR | PG connections |
| Inbound | 9090 | Monitoring SG | Metrics scrape |
| Outbound | 443 | S3 VPC Endpoint | Object-store access |

Use VPC endpoints for S3 access. No public IP on the SlateDuck instance.
