# Health Checks

Health checks verify that a Rocklake instance is operational and capable of serving requests. They are the foundation of automated availability — without health checks, your orchestrator cannot detect failures, your load balancer cannot route around unhealthy instances, and your monitoring cannot alert on degradation. A proper health check strategy distinguishes between "the process is running" (liveness), "the process can serve traffic" (readiness), and "the system is fully healthy" (deep check).

This page covers the three tiers of health checking, integration with every major orchestration platform, diagnostic endpoints, and patterns for using health checks effectively without creating false positives.

## Health Check Tiers

Rocklake supports three levels of health verification, each with different cost and confidence:

| Tier | What It Checks | Latency | When to Use |
|------|---------------|---------|-------------|
| **Liveness** | Process is alive, not deadlocked | <1ms | Kubernetes liveness probe, systemd watchdog |
| **Readiness** | Can accept and process requests | 5–20ms | Load balancer health, Kubernetes readiness |
| **Deep** | Full catalog integrity | 100ms–5s | Post-deployment verification, incident response |

### Tier 1: Liveness (TCP Check)

The simplest health check: can a TCP connection be established to Rocklake's port?

```bash
# Using nc (netcat)
nc -z localhost 5432

# Using bash built-in
cat < /dev/tcp/localhost/5432

# Using pg_isready
pg_isready -h localhost -p 5432
```

A successful TCP connection proves:

- The process is running
- The network listener is active
- The event loop is not completely deadlocked

It does NOT prove:

- Object storage is reachable
- The catalog is accessible
- Authentication works
- Operations will succeed

**Use for:** Kubernetes liveness probes, process-level monitoring.

**Failure response:** Restart the process.

### Tier 2: Readiness (Protocol Check)

A readiness check verifies that Rocklake can complete a full PG-wire handshake and execute a trivial query:

```bash
# Using psql (full protocol roundtrip)
psql -h localhost -p 5432 -c "SELECT 1" -t -A

# Using pg_isready with authentication
pg_isready -h localhost -p 5432 -U ducklake
```

A successful readiness check proves:

- The PG-wire listener accepts connections
- Authentication succeeds (if configured)
- The query execution path works
- The catalog can be read (at minimum, the version query)

**Use for:** Load balancer health checks, Kubernetes readiness probes.

**Failure response:** Remove from load balancer, stop routing traffic to this instance.

### Tier 3: Deep (Catalog Integrity)

A deep health check verifies that the entire catalog is accessible and internally consistent:

```bash
# Full catalog inspection
rocklake inspect --catalog s3://bucket/catalog/ --verify

# Quick version (manifest only)
rocklake inspect --catalog s3://bucket/catalog/ --manifest-only
```

A successful deep check proves:

- Object storage is reachable and credentials are valid
- The manifest is readable and well-formed
- Key ranges are consistent
- Writer epoch is valid
- Retention horizon is set correctly

**Use for:** Post-deployment smoke tests, periodic integrity verification, incident investigation.

**Failure response:** Investigate specific failure. Do NOT use for automated restarts (too slow, may have false negatives during normal compaction).

## Custom Health Endpoint

Rocklake exposes a dedicated HTTP health endpoint on the metrics port:

```bash
# Enable health endpoint
rocklake serve --catalog s3://bucket/catalog/ --metrics-bind 0.0.0.0:9090
```

Endpoints:

| Path | Method | Response | Description |
|------|--------|----------|-------------|
| `/health/live` | GET | 200 or 503 | Liveness (process is running) |
| `/health/ready` | GET | 200 or 503 | Readiness (can serve requests) |
| `/health/startup` | GET | 200 or 503 | Startup complete (initial catalog loaded) |

Response body (JSON):

```json
{
  "status": "healthy",
  "checks": {
    "process": "ok",
    "storage": "ok",
    "writer_epoch": 42,
    "sessions": {"active": 5, "max": 100},
    "uptime_seconds": 86400
  }
}
```

Unhealthy response (503):

```json
{
  "status": "unhealthy",
  "checks": {
    "process": "ok",
    "storage": "error: connection refused",
    "writer_epoch": null,
    "sessions": {"active": 0, "max": 100},
    "uptime_seconds": 3
  }
}
```

## Kubernetes Integration

### Standard Probes

```yaml
containers:
  - name: rocklake
    ports:
      - containerPort: 5432
        name: pgwire
      - containerPort: 9090
        name: metrics
    startupProbe:
      httpGet:
        path: /health/startup
        port: metrics
      initialDelaySeconds: 2
      periodSeconds: 2
      failureThreshold: 15
      # Allows 30s for initial catalog load
    livenessProbe:
      httpGet:
        path: /health/live
        port: metrics
      periodSeconds: 10
      failureThreshold: 3
      # Restarts after 30s of unresponsiveness
    readinessProbe:
      httpGet:
        path: /health/ready
        port: metrics
      periodSeconds: 5
      failureThreshold: 2
      # Removed from Service after 10s of failures
```

### Why Three Probes?

- **Startup probe:** Gives Rocklake time to read the manifest on first boot. Without this, the liveness probe might kill the pod before it finishes starting.
- **Liveness probe:** Detects deadlocks and zombie processes. Triggers restart.
- **Readiness probe:** Detects temporary issues (storage blip, connection exhaustion). Removes from load balancer without killing.

### Tuning Probe Timing

| Parameter | Aggressive | Conservative | Recommendation |
|-----------|-----------|--------------|----------------|
| Liveness period | 5s | 30s | 10s |
| Liveness threshold | 2 | 5 | 3 |
| Readiness period | 3s | 15s | 5s |
| Readiness threshold | 1 | 3 | 2 |
| Startup period | 1s | 5s | 2s |
| Startup threshold | 10 | 30 | 15 |

Aggressive probes detect failures faster but risk false positives (restarting a healthy pod during a brief GC pause or storage latency spike). Conservative probes are safer but detect failures slower.

## Load Balancer Health Checks

### AWS NLB

```json
{
  "HealthCheckProtocol": "TCP",
  "HealthCheckPort": "5432",
  "HealthCheckIntervalSeconds": 10,
  "HealthyThresholdCount": 2,
  "UnhealthyThresholdCount": 3
}
```

### AWS ALB (via HTTP endpoint)

If using an ALB with TCP pass-through via a target group:

```json
{
  "HealthCheckProtocol": "HTTP",
  "HealthCheckPort": "9090",
  "HealthCheckPath": "/health/ready",
  "HealthCheckIntervalSeconds": 10,
  "HealthyThresholdCount": 2,
  "UnhealthyThresholdCount": 3
}
```

### GCP Health Check

```bash
gcloud compute health-checks create tcp rocklake-health \
    --port=5432 \
    --check-interval=10s \
    --timeout=5s \
    --healthy-threshold=2 \
    --unhealthy-threshold=3
```

### Azure Load Balancer

```bash
az network lb probe create \
    --resource-group rg-analytics \
    --lb-name rocklake-lb \
    --name rocklake-probe \
    --protocol tcp \
    --port 5432 \
    --interval 10 \
    --threshold 3
```

## systemd Watchdog

For bare-metal deployments, integrate with systemd's watchdog:

```ini
[Service]
Type=notify
WatchdogSec=30
```

Rocklake sends `WATCHDOG=1` notifications to systemd at regular intervals. If the notification stops (process deadlocked, event loop blocked), systemd kills and restarts the process after `WatchdogSec` seconds.

## Health Check Patterns

### Read-Only Instance Health

For read-only replicas, the readiness check should verify that the catalog snapshot is recent enough:

```bash
# Check that the catalog is not too stale (replica lag)
rocklake inspect --catalog s3://bucket/catalog/ --check-freshness 300
# Fails if the latest snapshot is older than 300 seconds
```

### Writer Instance Health

For the writer, verify that the write path is functional:

```bash
# Attempt a no-op write (touches the heartbeat key)
psql -h localhost -p 5432 -c "SELECT rocklake_heartbeat()"
```

### Composite Health (External Monitor)

For sophisticated health monitoring, combine multiple checks:

```python
import subprocess
import requests
import sys

def check_health():
    # Tier 1: Process alive
    result = subprocess.run(["pg_isready", "-h", "localhost", "-p", "5432"], 
                          capture_output=True, timeout=5)
    if result.returncode != 0:
        return "CRITICAL: Process not responding"
    
    # Tier 2: Metrics endpoint
    try:
        resp = requests.get("http://localhost:9090/health/ready", timeout=5)
        if resp.status_code != 200:
            return f"WARNING: Readiness check failed: {resp.json()}"
    except requests.Timeout:
        return "WARNING: Metrics endpoint timeout"
    
    # Tier 3: Check epoch stability
    metrics = requests.get("http://localhost:9090/metrics").text
    # Parse and check epoch changes...
    
    return "OK"

status = check_health()
print(status)
sys.exit(0 if "OK" in status else 2 if "CRITICAL" in status else 1)
```

## Avoiding False Positives

Health checks that are too sensitive cause unnecessary restarts. Common sources of false positives and mitigations:

| False Positive Source | Mitigation |
|----------------------|------------|
| Brief storage latency spike | Increase failure threshold (3+) |
| GC pause blocks event loop | Use TCP check (not query-based) for liveness |
| Cold cache after restart | Startup probe with generous timeout |
| Network partition (<5s) | Multi-probe average, not single-check failure |

### The Golden Rule

Liveness probes should be **very permissive** (restart only on clear death). Readiness probes should be **moderately strict** (remove from LB on any concern). Deep checks should be **informational only** (alert, don't auto-remediate).

## Monitoring Health Check Results

Track health check outcomes as metrics:

```yaml
- alert: RocklakeHealthCheckFlapping
  expr: changes(kube_pod_container_status_restarts_total{container="rocklake"}[1h]) > 3
  labels:
    severity: warning
  annotations:
    summary: "Rocklake pod restarting frequently — health check may be too aggressive"
```

## Health Check Anti-Patterns

Understanding what NOT to do is as important as knowing the correct patterns:

### Anti-Pattern: Full Catalog Scan in Readiness Probe

Never use `rocklake inspect` (which counts all entities) as a readiness probe. This scans potentially thousands of keys and can take seconds on large catalogs. If the probe timeout is shorter than the scan time, the probe fails, the pod restarts, and the new pod also fails — creating a restart loop.

### Anti-Pattern: Write Operations in Liveness Probes

Do not use write-based health checks (like `rocklake_heartbeat()`) for liveness probes. If the writer lease is temporarily contended during an upgrade or network partition, write failures would trigger a restart that makes the situation worse. Reserve write checks for deep checks only.

### Anti-Pattern: Checking Dependencies in Liveness

A liveness probe should only check whether the process itself is healthy. If you include object storage reachability in the liveness probe, a transient S3 outage will cause Kubernetes to restart all your pods — exactly when you need them to survive and retry. Check storage reachability in the readiness probe instead (stops traffic routing) but not in liveness (which restarts the pod).

### Anti-Pattern: Synchronous Checks in Hot Path

If your health check runs a SQL query against Rocklake, ensure it has its own dedicated connection rather than competing with production traffic. A health check that times out because the connection pool is exhausted creates misleading "unhealthy" signals when the system is actually just busy.

## Incident Response Using Health Checks

During an incident, health checks provide the first signal. Here is how to use them diagnostically:

### Triage Decision Tree

```
Health Check Failed
├── TCP Liveness Failed?
│   ├── Yes → Process is dead. Check OOM kills, segfaults, disk full.
│   └── No → Process is alive but degraded.
│       ├── Readiness Failed?
│       │   ├── Yes → Can't serve requests. Check storage connectivity.
│       │   │   ├── Storage reachable? → Check writer epoch/fencing.
│       │   │   └── Storage unreachable → Network/IAM/storage issue.
│       │   └── No → Readiness passes but clients report errors.
│       │       └── Check error codes — likely specific SQL failures.
│       └── Deep Check Failed?
│           └── Run rocklake verify for detailed integrity report.
```

### Health Degradation Timeline

When investigating past incidents, correlate health check state transitions with other events:

```bash
# Kubernetes: Get pod condition history
kubectl get events --field-selector involvedObject.name=rocklake-0 --sort-by='.lastTimestamp'

# systemd: Check restart history
journalctl -u rocklake | grep -E "Started|Stopped|Failed"

# Docker: Container state history
docker inspect rocklake --format '{{json .State}}' | jq '.Health.Log[-5:]'
```

## Custom Health Endpoints

For teams that need health information beyond what standard probes provide, Rocklake's metrics endpoint exposes a structured health summary:

```bash
curl -s http://localhost:9090/health | jq
```

```json
{
  "status": "healthy",
  "checks": {
    "process": {"status": "pass", "uptime_seconds": 86400},
    "storage": {"status": "pass", "last_read_ms": 12},
    "writer": {"status": "pass", "epoch": 3, "fenced": false},
    "catalog": {"status": "pass", "latest_snapshot": 1247}
  },
  "version": "0.8.0",
  "started_at": "2024-12-15T10:00:00Z"
}
```

This endpoint is useful for building custom dashboards or integration with monitoring systems that prefer JSON over Prometheus text format.

## Further Reading

- **[Monitoring](monitoring.md)** — Metrics-based observability
- **[Troubleshooting](troubleshooting.md)** — Diagnosing health check failures
- **[Deployment: Kubernetes](../deployment/kubernetes.md)** — Full probe configuration
- **[Deployment: High Availability](../deployment/high-availability.md)** — Health checks in HA context
