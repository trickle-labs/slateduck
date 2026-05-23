# TLS

SlateDuck does not implement TLS natively. Use a TLS-terminating proxy.

## Why No Native TLS?

1. Certificate management is better handled by infrastructure
2. DuckDB supports `sslmode=disable` for local connections
3. In Kubernetes, mTLS is handled by the service mesh

## Options

- **Envoy sidecar** — TCP proxy with TLS termination
- **HAProxy** — TCP frontend with SSL offloading
- **AWS NLB** — TLS listener forwarding to port 5432
- **Service mesh** — Istio/Linkerd automatic mTLS
