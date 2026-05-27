# Deployment

Rocklake is designed to be deployed simply — it is a single static binary with no runtime dependencies beyond network access to your object storage provider. There is no database to provision, no cluster to manage, no sidecar containers to coordinate. You download the binary, point it at a bucket, and start accepting connections. The simplicity is intentional: a system that stores all state in object storage has fundamentally different operational characteristics than a traditional database, and the deployment model reflects this.

This section covers the full spectrum of deployment options — from running the binary directly on your laptop for development to production Kubernetes clusters with automated failover, serverless functions that scale to zero, and multi-region setups with cross-continental read replicas.

## Quick Decision Guide

| Scenario | Recommended Approach | Page |
|----------|---------------------|------|
| Local development | Binary on filesystem | [Binary](binary.md) |
| Single-team production | Docker on a small VM | [Docker](docker.md) |
| Platform team / multi-tenant | Kubernetes with Helm | [Kubernetes](kubernetes.md) |
| Event-driven / cost-sensitive | Lambda / serverless | [Lambda](lambda.md) |
| Global analytics team | Multi-region readers | [Multi-Region](multi-region.md) |
| Edge deployment | Fly.io | [Fly.io](fly-io.md) |

## Deployment Strategies

- **[Binary](binary.md)** — Running Rocklake directly on a VM or bare metal. The simplest deployment: download, configure, run. Covers systemd integration, resource limits, and log management.

- **[Docker](docker.md)** — Container images and Docker Compose setups. Production-ready container configuration with health checks, graceful shutdown, and volume management.

- **[Kubernetes](kubernetes.md)** — Helm charts, Deployments, and production configuration. Includes health probes, resource requests, RBAC, and integration with cloud IAM.

- **[Lambda / Serverless](lambda.md)** — Running Rocklake as a serverless function. Scale-to-zero deployments for cost-sensitive workloads with infrequent catalog access.

- **[Fly.io](fly-io.md)** — Deploying on Fly.io with global edge routing. Fast iteration and global distribution with minimal configuration.

## Configuration and Security

- **[Configuration](configuration.md)** — Complete reference for all server options: CLI flags, environment variables, and configuration file format. Covers storage paths, bind addresses, authentication, limits, and tuning.

- **[TLS](tls.md)** — Encrypting connections with TLS certificates. Covers certificate generation, renewal, mutual TLS, and integration with certificate managers.

- **[Networking](networking.md)** — Network topology, firewall rules, service discovery, and load balancing. How to expose Rocklake securely within a VPC or to the internet.

## Advanced Patterns

- **[High Availability](high-availability.md)** — Achieving uptime SLAs with writer failover. How the single-writer model interacts with health monitoring, and strategies for minimizing downtime during writer transitions.

- **[Multi-Region](multi-region.md)** — Cross-region read replicas and disaster recovery. Using object storage replication to serve readers in multiple regions with bounded staleness.

## Key Operational Characteristics

Understanding these characteristics helps make deployment decisions:

- **Stateless binary.** Rocklake stores all state in object storage. You can stop and restart it on a completely different machine and it resumes from where it left off.
- **Single writer.** Only one Rocklake instance can write to a given catalog at a time. Multiple readers can connect concurrently.
- **Fast startup.** Cold start time is 200–500ms (reading manifest + initial state from object storage). There is no recovery phase.
- **Low resource usage.** Typical memory footprint is 50–200 MB. CPU usage is negligible between requests. Disk is not used (all state is in object storage).
