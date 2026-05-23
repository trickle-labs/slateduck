# Docker Deployment

## Dockerfile

```dockerfile
FROM rust:1.75-slim AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/slateduck /usr/local/bin/
EXPOSE 5432 9090
ENTRYPOINT ["slateduck", "serve"]
```

## Docker Compose

```yaml
version: "3.8"
services:
  slateduck:
    build: .
    ports:
      - "5432:5432"
      - "9090:9090"
    environment:
      AWS_REGION: us-east-1
      AWS_ACCESS_KEY_ID: ${AWS_ACCESS_KEY_ID}
      AWS_SECRET_ACCESS_KEY: ${AWS_SECRET_ACCESS_KEY}
    command: ["--catalog-path", "s3://my-bucket/catalogs/warehouse"]
    restart: unless-stopped
```
