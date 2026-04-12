# ============================================================
# Seismosis Storage Service — Multi-stage Dockerfile
# ============================================================
# Build context: services/ (Cargo workspace root)
#
# Stage 1 (builder): Compile the Rust binary with rdkafka's
#                    vendored librdkafka (requires cmake) and
#                    sqlx offline query cache (SQLX_OFFLINE=true).
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
COPY Cargo.toml Cargo.lock ./
COPY ingestion/Cargo.toml ingestion/
COPY storage/Cargo.toml   storage/
COPY api/Cargo.toml       api/

RUN mkdir -p ingestion/src storage/src api/src \
    && echo 'fn main() {}' > ingestion/src/main.rs \
    && echo 'fn main() {}' > storage/src/main.rs \
    && echo 'fn main() {}' > api/src/main.rs

ENV SQLX_OFFLINE=true
RUN cargo build --release --locked -p seismosis-storage
RUN rm -f target/release/seismosis-storage \
          target/release/deps/seismosis_storage*

# ── Real build ────────────────────────────────────────────────
COPY storage/src/  storage/src/
COPY .sqlx/        .sqlx/

RUN cargo build --release --locked -p seismosis-storage

# ── Stage 2: Runtime ─────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    wget \
    && rm -rf /var/lib/apt/lists/* \
    && update-ca-certificates

RUN useradd --system --no-create-home --shell /usr/sbin/nologin seismosis

COPY --from=builder \
    /build/target/release/seismosis-storage \
    /usr/local/bin/seismosis-storage

# Prometheus metrics port
EXPOSE 9090

USER seismosis

ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/seismosis-storage"]
