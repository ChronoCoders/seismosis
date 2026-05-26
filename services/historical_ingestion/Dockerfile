FROM python:3.11-slim AS builder

WORKDIR /build

# aiohttp requires no extra system packages on Linux slim images.
# asyncpg bundles its C extension in the binary wheel.
COPY pyproject.toml ./
COPY src/ ./src/

RUN pip install --no-cache-dir --prefix=/install .

# ─────────────────────────────────────────────────────────────────────────────

FROM python:3.11-slim

# wget: healthcheck.  No other runtime system deps required.
RUN apt-get update \
    && apt-get install -y --no-install-recommends wget \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -u 1001 -s /sbin/nologin historical-ingestion

WORKDIR /app

COPY --from=builder /install /usr/local
COPY src/ ./src/

RUN chown -R historical-ingestion:historical-ingestion /app

USER historical-ingestion

ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1
ENV PYTHONPATH=/app/src

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD wget -qO- "http://localhost:${METRICS_PORT:-9097}/health" || exit 1

CMD ["python", "-m", "historical_ingestion.main"]
