FROM python:3.11-slim AS builder

WORKDIR /build

# confluent-kafka bundles librdkafka in its binary wheel on Linux.
# No extra system packages are needed for the build.
COPY pyproject.toml ./
COPY src/ ./src/

RUN pip install --no-cache-dir --prefix=/install .

# ─────────────────────────────────────────────────────────────────────────────

FROM python:3.11-slim

# wget: healthcheck.  No other runtime system deps required.
RUN apt-get update \
    && apt-get install -y --no-install-recommends wget \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -u 1001 -s /sbin/nologin analysis

WORKDIR /app

COPY --from=builder /install /usr/local
COPY src/ ./src/

RUN chown -R analysis:analysis /app

USER analysis

ENV PYTHONUNBUFFERED=1
ENV PYTHONDONTWRITEBYTECODE=1
ENV PYTHONPATH=/app/src

HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
    CMD wget -qO- "http://localhost:${METRICS_PORT:-9092}/health" || exit 1

CMD ["python", "-m", "analysis.main"]
