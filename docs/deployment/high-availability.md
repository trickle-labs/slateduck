# High Availability

## Writer HA

Single-replica with fast restart:

- Kubernetes: `Recreate` strategy, 5-15s restart
- ECS: circuit-breaker, 10-30s restart
- Fly.io: auto-restart, 3-10s

During restart, reads continue; writes fail until new writer is ready.

## Reader HA

Readers are stateless — deploy multiple replicas behind a load balancer.

## Recovery Time

| Failure | Recovery | Impact |
|---------|----------|--------|
| Writer crash | 5-30s | Writes fail; reads OK |
| Reader crash | Instant | No impact (others serve) |
| S3 outage | Minutes-hours | All ops fail |
