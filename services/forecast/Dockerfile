# ---- builder stage ----
FROM python:3.11-slim AS builder

WORKDIR /build

COPY pyproject.toml ./
COPY src/ ./src/

RUN pip install --upgrade pip --no-cache-dir \
 && pip install --no-cache-dir --prefix=/install .

# ---- runtime stage ----
FROM python:3.11-slim

# Install wget for HEALTHCHECK
RUN apt-get update \
 && apt-get install -y --no-install-recommends wget \
 && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN groupadd --gid 1001 forecast \
 && useradd --uid 1001 --gid forecast --no-create-home --shell /usr/sbin/nologin forecast

COPY --from=builder /install /usr/local
COPY --from=builder /build/src /app/src

WORKDIR /app

ENV PYTHONPATH=/app/src \
    PYTHONUNBUFFERED=1

EXPOSE 9098

HEALTHCHECK --interval=30s --timeout=10s --start-period=15s --retries=3 \
    CMD wget --quiet --tries=1 --spider http://localhost:9098/health || exit 1

USER forecast

CMD ["python", "-m", "forecast.main"]
