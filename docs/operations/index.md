# Operations

This section covers the operational procedures for running Rocklake in production. A well-operated Rocklake deployment needs minimal intervention — the system is designed to run unattended for weeks or months — but when you do need to interact with it (cleaning up old data, investigating a problem, performing a backup), having clear procedures matters.

Each guide provides step-by-step instructions, explains what happens internally during each operation, and gives guidance on when to use it and what can go wrong.

## Quick Reference

| Task | Frequency | Impact | Page |
|------|-----------|--------|------|
| Monitor metrics | Continuous | None | [Monitoring](monitoring.md) |
| Review logs | As needed | None | [Logging](logging.md) |
| Health check | Every 5–10s (automated) | None | [Health Checks](health-checks.md) |
| Garbage collection | Daily or weekly | Brief pause for GC scan | [Garbage Collection](garbage-collection.md) |
| Verify integrity | Weekly or after incidents | Read-only scan | [Verify & Repair](verify-repair.md) |
| Upgrade version | Per release cycle | Brief restart | [Upgrades](upgrades.md) |
| Backup catalog | Before major changes | None (read-only export) | [Backup & Restore](backup-restore.md) |

## Routine Operations

- **[Monitoring](monitoring.md)** — Prometheus metrics, Grafana dashboards, and alerting rules. What to watch, what to alert on, and what is normal versus concerning.

- **[Logging](logging.md)** — Configuring log levels, structured JSON output, log aggregation, and using logs to debug session-level issues.

- **[Garbage Collection](garbage-collection.md)** — Managing catalog growth by removing expired snapshots and unreferenced data. Retention policies, scheduling, and safety guarantees.

- **[Health Checks](health-checks.md)** — Verifying operational readiness at the TCP, protocol, and semantic levels. Integration with load balancers, Kubernetes probes, and monitoring systems.

## Data Management

- **[Backup & Restore](backup-restore.md)** — NDJSON export, point-in-time snapshots, and disaster recovery. How to create portable backups and restore to a new storage location.

- **[Export](export.md)** — Extracting catalog metadata for migration to other systems, compliance audits, or offline analysis.

- **[Excision](excision.md)** — Physical deletion of historical data for compliance (GDPR right-to-erasure) or cost control. The nuclear option when GC retention is not enough.

- **[Inspect](inspect.md)** — Examining internal catalog state: key-value pairs, MVCC versions, transaction history, and storage layout.

## Maintenance

- **[Verify & Repair](verify-repair.md)** — Integrity checks (checksums, cross-references, orphan detection) and conservative repair operations.

- **[Upgrades](upgrades.md)** — Version upgrades, format migrations, rollback procedures, and compatibility guarantees.

- **[Troubleshooting](troubleshooting.md)** — Common problems, their root causes, and step-by-step resolution procedures.
