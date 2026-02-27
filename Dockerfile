# Multi-stage build for websocket-server (amd64 / arm64)
# Build: docker buildx build --platform linux/amd64,linux/arm64 -t websocket-server:latest .

# -----------------------------------------------------------------------------
# Stage 1: build
# -----------------------------------------------------------------------------
FROM rust:1-bookworm AS builder

WORKDIR /app

# Cache dependencies: copy manifest and lockfile, then build a dummy binary
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN cargo build --release && rm -rf src

# Build real binary
COPY src ./src
RUN touch src/main.rs && cargo build --release

# -----------------------------------------------------------------------------
# Stage 2: runtime
# -----------------------------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/websocket-server /usr/local/bin/websocket-server

# Default port (override with -e PORT=... or env in run)
ENV PORT=8080
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/websocket-server"]
