# ============================================================
# Seismosis WebSocket Service — Multi-stage Dockerfile
# ============================================================
# Build context: services/ (Cargo workspace root)
#
# Stage 1 (builder): Compile the Rust binary with rdkafka's
#                    vendored librdkafka (requires cmake).
# Stage 2 (runtime): Minimal Debian image; no Rust toolchain.
# ============================================================

# ── Stage 1: Builder ─────────────────────────────────────────
FROM rust:1-slim-bookworm AS builder

WORKDIR /build

# Build dependencies for rdkafka cmake-build + reqwest rustls-tls
RUN apt-get update && apt-get install -y --no-install-recommends \
    cmake \
    make \
    gcc \
    g++ \
    pkg-config \
    libssl-dev \
    libcurl4-openssl-dev \
    && rm -rf /var/lib/apt/lists/*

# ── Dependency caching layer ──────────────────────────────────
# Copy workspace manifest + lock first, then all member manifests.
# A dummy main.rs per member lets Cargo compile all workspace deps
# before we copy real source; only changes to Cargo files bust this layer.
COPY Cargo.toml Cargo.lock ./
COPY ingestion/Cargo.toml ingestion/
COPY storage/Cargo.toml   storage/
COPY api/Cargo.toml       api/
COPY websocket/Cargo.toml websocket/

RUN mkdir -p ingestion/src storage/src api/src websocket/src \
    && echo 'fn main() {}' > ingestion/src/main.rs \
    && echo 'fn main() {}' > storage/src/main.rs \
    && echo 'fn main() {}' > api/src/main.rs \
    && echo 'fn main() {}' > websocket/src/main.rs

RUN cargo build --release --locked -p seismosis-websocket
# Remove dummy artifacts; leave compiled deps in place for the real build.
RUN rm -f target/release/seismosis-websocket \
          target/release/deps/seismosis_websocket*

# ── Real build ────────────────────────────────────────────────
COPY websocket/src/ websocket/src/
RUN cargo build --release --locked -p seismosis-websocket

# ── Stage 2: Runtime ─────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/* \
    && update-ca-certificates

# Non-root user
RUN useradd --system --no-create-home --shell /usr/sbin/nologin seismosis

COPY --from=builder \
    /build/target/release/seismosis-websocket \
    /usr/local/bin/seismosis-websocket

# WebSocket connections port
EXPOSE 9093
# Prometheus metrics port (internal)
EXPOSE 9094

USER seismosis

ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/seismosis-websocket"]
