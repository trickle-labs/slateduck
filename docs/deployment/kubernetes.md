# Kubernetes Deployment

## Writer (Single Replica)

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: slateduck-writer
spec:
  replicas: 1
  strategy:
    type: Recreate
  selector:
    matchLabels:
      app: slateduck
  template:
    spec:
      containers:
        - name: slateduck
          image: slateduck:latest
          args: ["--catalog-path", "s3://bucket/catalogs/warehouse"]
          ports:
            - containerPort: 5432
            - containerPort: 9090
          readinessProbe:
            tcpSocket:
              port: 5432
          resources:
            requests:
              memory: 128Mi
              cpu: 100m
---
apiVersion: v1
kind: Service
metadata:
  name: slateduck
spec:
  selector:
    app: slateduck
  ports:
    - port: 5432
      targetPort: 5432
```

Use IRSA for IAM in EKS. Use Workload Identity in GKE.
