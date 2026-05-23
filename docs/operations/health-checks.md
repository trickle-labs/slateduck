# Health Checks

## Liveness

```bash
nc -z localhost 5432
```

## Readiness

```bash
pg_isready -h localhost -p 5432
```

## Kubernetes Probes

```yaml
readinessProbe:
  tcpSocket:
    port: 5432
  initialDelaySeconds: 5
livenessProbe:
  tcpSocket:
    port: 5432
  initialDelaySeconds: 10
```
