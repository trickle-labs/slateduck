# AWS Lambda Deployment

SlateDuck readers can run in Lambda for serverless catalog access.

## Architecture

```mermaid
graph LR
    API[API Gateway] --> L[Lambda Reader]
    L --> S3[S3 / SlateDB]
    W[Writer on ECS] --> S3
```

## Cold Start

- Runtime initialization: ~50 ms
- First SlateDB read: ~40 ms
- Total cold start: ~100 ms
- Warm invocations: ~40-60 ms

## When to Use

- Infrequent, bursty read workloads
- Cost optimization (pay per invocation)
- Multi-tenant isolation
