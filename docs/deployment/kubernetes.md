# Kubernetes Deployment

Kubernetes is the recommended deployment platform for production RockLake instances that need automated restarts, health monitoring, and integration with cloud IAM. Because RockLake stores all state in object storage, it runs as a stateless Deployment — no PersistentVolumes, no StatefulSets, no operator required. This makes it one of the simplest database-adjacent services to deploy on Kubernetes.

This page covers complete deployment manifests, scaling patterns, IAM integration for all three major clouds, health probing strategies, and operational practices for running RockLake reliably in a cluster.

## Why Deployment, Not StatefulSet

Traditional databases on Kubernetes need StatefulSets for stable network identifiers and persistent storage. RockLake needs neither:

- **No persistent storage.** All data lives in object storage. The pod can be killed and rescheduled on any node without data loss.
- **No stable identity.** The writer acquires its epoch on startup; it does not need to be "pod-0" or have a fixed hostname.
- **Fast recovery.** A rescheduled pod starts accepting connections in <1 second after the image is pulled.

This means standard Deployment semantics apply: rolling updates, pod disruption budgets, and horizontal pod autoscaling (for readers) all work as expected.

## Core Manifests

### Namespace and ConfigMap

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: rocklake
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: rocklake-config
  namespace: rocklake
data:
  storage: "s3://my-lakehouse-bucket/catalog/"
  region: "us-east-1"
  max-sessions: "100"
  log-format: "json"
  log-level: "info"
```

### Secret (for Password Authentication)

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: rocklake-auth
  namespace: rocklake
type: Opaque
stringData:
  password: "your-secure-password-here"
```

### Writer Deployment

The primary (writer) deployment — always exactly 1 replica:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rocklake-writer
  namespace: rocklake
  labels:
    app.kubernetes.io/name: rocklake
    app.kubernetes.io/component: writer
spec:
  replicas: 1
  strategy:
    type: Recreate  # Ensure old pod is gone before new one starts (single-writer)
  selector:
    matchLabels:
      app.kubernetes.io/name: rocklake
      app.kubernetes.io/component: writer
  template:
    metadata:
      labels:
        app.kubernetes.io/name: rocklake
        app.kubernetes.io/component: writer
      annotations:
        prometheus.io/scrape: "true"
        prometheus.io/port: "9090"
    spec:
      serviceAccountName: rocklake
      terminationGracePeriodSeconds: 60
      securityContext:
        runAsNonRoot: true
        runAsUser: 65534
        fsGroup: 65534
      containers:
        - name: rocklake
          image: ghcr.io/trickle-labs/rocklake:latest  # pin to a specific release tag in production, e.g. 0.46.0
          ports:
            - containerPort: 5432
              name: pgwire
              protocol: TCP
          args:
            - "--storage"
            - "$(ROCKLAKE_STORAGE)"
            - "--bind"
            - "0.0.0.0:5432"
            - "--max-sessions"
            - "$(ROCKLAKE_MAX_SESSIONS)"
            - "--log-format"
            - "$(ROCKLAKE_LOG_FORMAT)"
            - "--log-level"
            - "$(ROCKLAKE_LOG_LEVEL)"
            - "--auth-user"
            - "ducklake"
          env:
            - name: ROCKLAKE_STORAGE
              valueFrom:
                configMapKeyRef:
                  name: rocklake-config
                  key: storage
            - name: ROCKLAKE_MAX_SESSIONS
              valueFrom:
                configMapKeyRef:
                  name: rocklake-config
                  key: max-sessions
            - name: ROCKLAKE_LOG_FORMAT
              valueFrom:
                configMapKeyRef:
                  name: rocklake-config
                  key: log-format
            - name: ROCKLAKE_LOG_LEVEL
              valueFrom:
                configMapKeyRef:
                  name: rocklake-config
                  key: log-level
            - name: AWS_REGION
              valueFrom:
                configMapKeyRef:
                  name: rocklake-config
                  key: region
            - name: ROCKLAKE_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: rocklake-auth
                  key: password
          resources:
            requests:
              memory: "128Mi"
              cpu: "100m"
            limits:
              memory: "512Mi"
              cpu: "2000m"
          livenessProbe:
            tcpSocket:
              port: pgwire
            initialDelaySeconds: 5
            periodSeconds: 10
            failureThreshold: 3
          readinessProbe:
            tcpSocket:
              port: pgwire
            initialDelaySeconds: 5
            periodSeconds: 5
            failureThreshold: 2
          startupProbe:
            tcpSocket:
              port: pgwire
            initialDelaySeconds: 2
            periodSeconds: 2
            failureThreshold: 15
```

### Reader Deployment (Optional)

For read-heavy workloads, deploy additional read-only replicas that scale horizontally:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rocklake-reader
  namespace: rocklake
  labels:
    app.kubernetes.io/name: rocklake
    app.kubernetes.io/component: reader
spec:
  replicas: 3
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: rocklake
      app.kubernetes.io/component: reader
  template:
    metadata:
      labels:
        app.kubernetes.io/name: rocklake
        app.kubernetes.io/component: reader
    spec:
      serviceAccountName: rocklake
      terminationGracePeriodSeconds: 30
      securityContext:
        runAsNonRoot: true
        runAsUser: 65534
      containers:
        - name: rocklake
          image: ghcr.io/trickle-labs/rocklake:latest  # pin to a specific release tag in production, e.g. 0.46.0
          ports:
            - containerPort: 5432
              name: pgwire
          args:
            - "--storage"
            - "$(ROCKLAKE_STORAGE)"
            - "--bind"
            - "0.0.0.0:5432"
            - "--read-only"
            - "--auth-user"
            - "ducklake"
          env:
            - name: ROCKLAKE_STORAGE
              valueFrom:
                configMapKeyRef:
                  name: rocklake-config
                  key: storage
            - name: AWS_REGION
              valueFrom:
                configMapKeyRef:
                  name: rocklake-config
                  key: region
            - name: ROCKLAKE_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: rocklake-auth
                  key: password
          resources:
            requests:
              memory: "64Mi"
              cpu: "50m"
            limits:
              memory: "256Mi"
              cpu: "1000m"
          readinessProbe:
            tcpSocket:
              port: pgwire
            periodSeconds: 5
```

### Services

```yaml
apiVersion: v1
kind: Service
metadata:
  name: rocklake-writer
  namespace: rocklake
  labels:
    app.kubernetes.io/name: rocklake
    app.kubernetes.io/component: writer
spec:
  selector:
    app.kubernetes.io/name: rocklake
    app.kubernetes.io/component: writer
  ports:
    - port: 5432
      targetPort: pgwire
      protocol: TCP
  type: ClusterIP
---
apiVersion: v1
kind: Service
metadata:
  name: rocklake-reader
  namespace: rocklake
  labels:
    app.kubernetes.io/name: rocklake
    app.kubernetes.io/component: reader
spec:
  selector:
    app.kubernetes.io/name: rocklake
    app.kubernetes.io/component: reader
  ports:
    - port: 5432
      targetPort: pgwire
      protocol: TCP
  type: ClusterIP
```

Connect from other pods:

```sql
-- Writer (DDL, DML, and queries)
ATTACH 'ducklake:host=rocklake-writer.rocklake.svc.cluster.local;port=5432;user=ducklake;password=...' AS lake;

-- Reader (queries only, load-balanced across replicas)
ATTACH 'ducklake:host=rocklake-reader.rocklake.svc.cluster.local;port=5432;user=ducklake;password=...' AS lake;
```

## IAM Integration

### AWS: IAM Roles for Service Accounts (IRSA)

On EKS, use IRSA to provide S3 credentials without static keys:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: rocklake
  namespace: rocklake
  annotations:
    eks.amazonaws.com/role-arn: arn:aws:iam::123456789012:role/rocklake-s3-access
```

The IAM role policy:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "s3:GetObject",
        "s3:PutObject",
        "s3:DeleteObject",
        "s3:ListBucket"
      ],
      "Resource": [
        "arn:aws:s3:::my-lakehouse-bucket",
        "arn:aws:s3:::my-lakehouse-bucket/catalog/*"
      ]
    }
  ]
}
```

Trust policy (allow the service account to assume the role):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "Federated": "arn:aws:iam::123456789012:oidc-provider/oidc.eks.us-east-1.amazonaws.com/id/EXAMPLE"
      },
      "Action": "sts:AssumeRoleWithWebIdentity",
      "Condition": {
        "StringEquals": {
          "oidc.eks.us-east-1.amazonaws.com/id/EXAMPLE:sub": "system:serviceaccount:rocklake:rocklake"
        }
      }
    }
  ]
}
```

### GCP: Workload Identity

On GKE, use Workload Identity to bind a Kubernetes service account to a GCP service account:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: rocklake
  namespace: rocklake
  annotations:
    iam.gke.io/gcp-service-account: rocklake@my-project.iam.gserviceaccount.com
```

Grant the GCP service account `roles/storage.objectAdmin` on the bucket.

### Azure: Workload Identity

On AKS with Azure AD Workload Identity:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: rocklake
  namespace: rocklake
  annotations:
    azure.workload.identity/client-id: "12345678-1234-1234-1234-123456789012"
  labels:
    azure.workload.identity/use: "true"
```

## Horizontal Pod Autoscaler (Readers)

Scale reader replicas based on CPU or connection count:

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: rocklake-reader
  namespace: rocklake
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: rocklake-reader
  minReplicas: 2
  maxReplicas: 10
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70
```

## Pod Disruption Budget

Protect the writer from accidental eviction during node maintenance:

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: rocklake-writer
  namespace: rocklake
spec:
  maxUnavailable: 0
  selector:
    matchLabels:
      app.kubernetes.io/name: rocklake
      app.kubernetes.io/component: writer
```

For readers, allow one replica to be unavailable during rolling updates:

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: rocklake-reader
  namespace: rocklake
spec:
  maxUnavailable: 1
  selector:
    matchLabels:
      app.kubernetes.io/name: rocklake
      app.kubernetes.io/component: reader
```

## Network Policy

Restrict which pods can connect to RockLake:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: rocklake-ingress
  namespace: rocklake
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: rocklake
  policyTypes:
    - Ingress
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              rocklake-access: "true"
      ports:
        - protocol: TCP
          port: 5432
```

Label namespaces that should have access:

```bash
kubectl label namespace analytics rocklake-access=true
```

## GC CronJob

Schedule garbage collection as a Kubernetes CronJob to clean up expired snapshots and compacted files:

```yaml
apiVersion: batch/v1
kind: CronJob
metadata:
  name: rocklake-gc
  namespace: rocklake
spec:
  schedule: "0 3 * * *"
  concurrencyPolicy: Forbid
  successfulJobsHistoryLimit: 3
  failedJobsHistoryLimit: 5
  jobTemplate:
    spec:
      backoffLimit: 3
      activeDeadlineSeconds: 3600
      template:
        spec:
          serviceAccountName: rocklake
          securityContext:
            runAsNonRoot: true
            runAsUser: 65534
          containers:
            - name: gc
              image: ghcr.io/trickle-labs/rocklake:latest  # pin to a specific release tag in production, e.g. 0.46.0
              command:
                - "rocklake"
                - "gc"
                - "--storage"
                - "s3://my-lakehouse-bucket/catalog/"
                - "--retain-days"
                - "30"
              env:
                - name: AWS_REGION
                  value: us-east-1
              resources:
                requests:
                  memory: "64Mi"
                  cpu: "50m"
                limits:
                  memory: "256Mi"
                  cpu: "500m"
          restartPolicy: OnFailure
```

## Rolling Updates

Because RockLake is stateless, rolling updates are straightforward:

```bash
# Update image tag
kubectl set image deployment/rocklake-writer \
  rocklake=ghcr.io/trickle-labs/rocklake:0.46.0 \
  -n rocklake

# Watch rollout
kubectl rollout status deployment/rocklake-writer -n rocklake
```

For the writer deployment (`strategy: Recreate`), Kubernetes terminates the old pod before starting the new one. This ensures no split-brain scenario — only one writer exists at any time. The brief downtime (typically 2–5 seconds) is acceptable because RockLake starts fast and clients retry automatically.

## Monitoring with Prometheus

Add a ServiceMonitor for Prometheus Operator:

```yaml
apiVersion: monitoring.coreos.com/v1
kind: ServiceMonitor
metadata:
  name: rocklake
  namespace: rocklake
spec:
  selector:
    matchLabels:
      app.kubernetes.io/name: rocklake
  endpoints:
    - port: metrics
      interval: 15s
```

## Troubleshooting

### Pod stuck in CrashLoopBackOff

Check logs: `kubectl logs -n rocklake deploy/rocklake-writer`. Common causes:

- **Storage access denied:** IAM role not configured or trust policy incorrect
- **Invalid storage path:** Bucket does not exist or prefix is misspelled
- **Port conflict:** Another service using port 5432 in the pod

### Pod healthy but clients cannot connect

- Verify the Service selector matches pod labels
- Check NetworkPolicy allows ingress from client namespace
- Ensure DNS resolution works: `kubectl exec -it debug -- nslookup rocklake-writer.rocklake.svc.cluster.local`

### Writer epoch conflict

If you accidentally run two writer deployments against the same storage, the second one will be fenced and crash. Ensure only one writer Deployment exists per catalog.

## Resource Tuning

### Memory Sizing

RockLake's memory usage is predictable:

| Component | Memory | Notes |
|-----------|--------|-------|
| Base process | ~30 MB | Binary, runtime, static allocations |
| SlateDB block cache | 10–50 MB | Configurable, caches hot SST blocks |
| Per-session state | ~1 MB each | Grows with concurrent connections |
| Protobuf decode buffers | ~5 MB | Temporary allocations during query processing |

**Formula:** `Total ≈ 50 MB + (sessions × 1 MB) + block_cache_size`

For a deployment with 50 max sessions and a 20 MB block cache: request 128 Mi, limit 256 Mi.

### CPU Sizing

RockLake's writer path is single-threaded (all writes serialize through the writer). The reader path can use multiple threads for concurrent sessions. For most deployments:

- **1 writer pod:** 100m request, 500m limit (burst for write batches)
- **Reader pods:** 100m request, 250m limit each (read-only workload is lighter)

### Horizontal Pod Autoscaling (Readers)

For read-heavy workloads, autoscale reader pods based on connection count:

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: rocklake-reader
  namespace: rocklake
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: rocklake-reader
  minReplicas: 2
  maxReplicas: 10
  metrics:
    - type: Pods
      pods:
        metric:
          name: rocklake_active_sessions
        target:
          type: AverageValue
          averageValue: "20"
```

This scales out when the average session count per pod exceeds 20, ensuring each pod handles a manageable number of concurrent connections.

## Network Policies

Lock down RockLake's network access for defense in depth:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: rocklake-writer
  namespace: rocklake
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/name: rocklake
      app.kubernetes.io/component: writer
  policyTypes:
    - Ingress
    - Egress
  ingress:
    # Allow connections from DuckDB client pods
    - from:
        - namespaceSelector:
            matchLabels:
              rocklake-access: "true"
      ports:
        - protocol: TCP
          port: 5432
    # Allow Prometheus scraping
    - from:
        - namespaceSelector:
            matchLabels:
              name: monitoring
      ports:
        - protocol: TCP
          port: 9090
  egress:
    # Allow access to S3 (HTTPS)
    - to:
        - ipBlock:
            cidr: 0.0.0.0/0
      ports:
        - protocol: TCP
          port: 443
    # Allow DNS resolution
    - to:
        - namespaceSelector: {}
      ports:
        - protocol: UDP
          port: 53
```

## Upgrade Strategy

For Kubernetes deployments, use the `Recreate` strategy (not `RollingUpdate`) for the writer pod:

```yaml
strategy:
  type: Recreate
```

This ensures the old writer pod is fully terminated before the new one starts, preventing epoch conflicts. The downtime is typically 2–5 seconds (old pod termination + new pod startup).

For reader pods, `RollingUpdate` is safe since readers do not hold exclusive leases:

```yaml
strategy:
  type: RollingUpdate
  rollingUpdate:
    maxSurge: 1
    maxUnavailable: 0
```

## Further Reading

- **[Docker](docker.md)** — Simpler container deployment without orchestration
- **[High Availability](high-availability.md)** — Failover strategies for uptime SLAs
- **[Multi-Region](multi-region.md)** — Cross-region reader distribution
- **[Configuration](configuration.md)** — Full configuration reference

---

## Reader Fleet (v0.47.0+)

> Read-scale-out: deploy a fleet of zero-epoch readers that can serve analytics
> queries without interfering with the writer.

### Reader Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rocklake-reader
  labels:
    app: rocklake
    role: reader
spec:
  replicas: 3
  selector:
    matchLabels:
      app: rocklake
      role: reader
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 0
  template:
    metadata:
      labels:
        app: rocklake
        role: reader
    spec:
      terminationGracePeriodSeconds: 60
      containers:
        - name: rocklake
          image: ghcr.io/trickle/rocklake:v0.47.0
          args:
            - serve
            - --catalog
            - $(ROCKLAKE_CATALOG)
            - --read-only
            - --bind
            - "0.0.0.0:5432"
            - --metrics-port
            - "9100"
            - --idle-connection-timeout
            - "60"
            - --drain-timeout
            - "30"
          env:
            - name: ROCKLAKE_CATALOG
              valueFrom:
                secretKeyRef:
                  name: rocklake-config
                  key: catalog_url
          ports:
            - name: pg
              containerPort: 5432
            - name: metrics
              containerPort: 9100
          livenessProbe:
            tcpSocket:
              port: pg
            initialDelaySeconds: 5
            periodSeconds: 10
          readinessProbe:
            tcpSocket:
              port: pg
            initialDelaySeconds: 3
            periodSeconds: 5
          resources:
            requests:
              cpu: "250m"
              memory: "256Mi"
            limits:
              cpu: "2"
              memory: "1Gi"
```

### HorizontalPodAutoscaler

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: rocklake-reader-hpa
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: rocklake-reader
  minReplicas: 2
  maxReplicas: 20
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 60
```

### PodDisruptionBudget

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: rocklake-reader-pdb
spec:
  selector:
    matchLabels:
      app: rocklake
      role: reader
  minAvailable: 1
```

### Reader Service

```yaml
apiVersion: v1
kind: Service
metadata:
  name: rocklake-reader
spec:
  selector:
    app: rocklake
    role: reader
  ports:
    - name: pg
      port: 5432
      targetPort: pg
  type: ClusterIP
```
