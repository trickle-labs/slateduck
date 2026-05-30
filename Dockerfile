# syntax=docker/dockerfile:1
# RockLake multi-stage Dockerfile.
#
# Build:  docker build -t rocklake:latest .
# Run:    docker run -p 5432:5432 -e ROCKLAKE_CATALOG=s3://my-bucket/catalog rocklake:latest serve
#
# The image is published to ghcr.io/trickle-labs/rocklake:{version} via the
# release workflow.

# ─── Build stage ─────────────────────────────────────────────────────────────
FROM rust:1.93-slim-bookworm AS builder

# Install C build tools required by some crate build scripts.
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependencies before copying source.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build only the rocklake binary in release mode.
RUN cargo build --release -p rocklake-pgwire --bin rocklake

# ─── Runtime stage ───────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

# Install runtime CA certificates (needed for HTTPS/S3 TLS).
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user.
RUN useradd --system --no-create-home --uid 10000 rocklake

COPY --from=builder /build/target/release/rocklake /usr/local/bin/rocklake

# Default port for the PG-Wire sidecar.
EXPOSE 5432
# Default port for the Prometheus metrics endpoint.
EXPOSE 9090

USER rocklake

# Health check using `rocklake diagnose --quick` (non-zero exit = unhealthy).
HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
    CMD rocklake diagnose --catalog "${ROCKLAKE_CATALOG}" 2>/dev/null || exit 1

# Default command: start the PG-Wire sidecar using ROCKLAKE_CATALOG env var.
ENTRYPOINT ["rocklake"]
CMD ["serve", "--catalog", "${ROCKLAKE_CATALOG}"]

# Image labels (OCI Annotations).
LABEL org.opencontainers.image.title="RockLake"
LABEL org.opencontainers.image.description="Serverless lakehouse catalog backed by SlateDB"
LABEL org.opencontainers.image.url="https://github.com/trickle-labs/rocklake"
LABEL org.opencontainers.image.source="https://github.com/trickle-labs/rocklake"
LABEL org.opencontainers.image.licenses="Apache-2.0"
