# -------------------------------------------------------------------
# Stage 1: Build the ephemera client binary
# -------------------------------------------------------------------
FROM rust:1.89-bookworm AS builder

WORKDIR /build

# Install build dependencies (SQLite bundled via rusqlite, so minimal extras)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first for layer caching
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build the client binary in release mode
RUN cargo build --release -p ephemera-client

# -------------------------------------------------------------------
# Stage 2: Minimal runtime image
# -------------------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl jq \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ephemera /usr/local/bin/ephemera

# Data directory — mount or use a named volume
ENV EPHEMERA_DATA_DIR=/data
# Bind to all interfaces so Docker port mapping works
ENV EPHEMERA_HTTP_ADDR=0.0.0.0

RUN mkdir -p /data

EXPOSE 3500

CMD ["ephemera"]
