"""
Seismosis Historical Ingestion Service
=======================================

One-shot CLI that bulk-downloads the USGS ComCat earthquake catalog for the
Turkey region (bbox 33–45°N, 22–48°E, M ≥ 1.5) starting from 2016-01-01 and
upserts all events into seismology.historical_events in PostgreSQL.

Checkpoint resume
-----------------
If a previous run was interrupted the job reads the last successfully written
event_time from seismology.ingest_checkpoints and continues from there, so no
events need to be re-downloaded.

Observability
-------------
* Structured JSON logs via structlog (progress every 1 000 events)
* Prometheus metrics exposed on :9097/metrics

Usage
-----
    python -m historical_ingestion.main

Required environment variables
-------------------------------
    DATABASE_URL   — asyncpg-compatible DSN
                     e.g. postgresql://user:pass@localhost:5433/seismosis
Optional environment variables
-------------------------------
    LOG_LEVEL      — DEBUG | INFO | WARNING | ERROR  (default: INFO)
    METRICS_PORT   — Prometheus HTTP port            (default: 9097)
    START_DATE     — ISO-8601 date to begin from     (default: 2016-01-01)
    END_DATE       — ISO-8601 date to stop at        (default: today UTC)
"""
from __future__ import annotations

import asyncio
import logging
import os
import sys
import threading
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, HTTPServer

import aiohttp
import structlog
import structlog.types
from prometheus_client import CollectorRegistry, CONTENT_TYPE_LATEST, generate_latest

from .comcat import iter_turkey_events
from .db import Database
from .metrics import Metrics

_DEFAULT_START = "2016-01-01T00:00:00"
_DEFAULT_METRICS_PORT = 9097
_PROGRESS_INTERVAL = 1_000  # log progress every N events


# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------

def _configure_logging(level: str) -> None:
    structlog.configure(
        processors=[
            structlog.stdlib.add_log_level,
            structlog.processors.TimeStamper(fmt="iso"),
            structlog.processors.StackInfoRenderer(),
            structlog.processors.ExceptionRenderer(),
            structlog.processors.JSONRenderer(),
        ],
        wrapper_class=structlog.make_filtering_bound_logger(
            getattr(logging, level.upper(), logging.INFO)
        ),
        logger_factory=structlog.PrintLoggerFactory(),
    )


# ---------------------------------------------------------------------------
# Metrics HTTP server
# ---------------------------------------------------------------------------

def _start_metrics_server(port: int, prom_registry: CollectorRegistry) -> HTTPServer:
    """Start a daemon thread serving /metrics (Prometheus) and /health."""

    class _Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            if self.path == "/metrics":
                output = generate_latest(prom_registry)
                self.send_response(200)
                self.send_header("Content-Type", CONTENT_TYPE_LATEST)
                self.end_headers()
                self.wfile.write(output)
            elif self.path == "/health":
                body = b"ok"
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self.send_response(404)
                self.end_headers()

        def log_message(self, fmt: str, *args: object) -> None:
            pass  # suppress per-request access log noise

    server = HTTPServer(("0.0.0.0", port), _Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


# ---------------------------------------------------------------------------
# Core ingestion loop
# ---------------------------------------------------------------------------

async def run_ingestion(
    db: Database,
    metrics: Metrics,
    start_time: datetime,
    end_time: datetime,
    logger: structlog.types.BoundLogger,
) -> int:
    """
    Download all Turkey-region events in [start_time, end_time] and upsert
    them into PostgreSQL.

    Returns the total number of events ingested in this run.
    """
    # Check for an existing checkpoint so we can resume
    checkpoint = await db.read_checkpoint()
    if checkpoint is not None and checkpoint > start_time:
        logger.info(
            "checkpoint_found",
            resuming_from=checkpoint.isoformat(),
            configured_start=start_time.isoformat(),
        )
        cursor_start = checkpoint
    else:
        cursor_start = start_time

    logger.info(
        "ingestion_starting",
        start_time=cursor_start.isoformat(),
        end_time=end_time.isoformat(),
    )

    total_ingested = 0
    last_progress_log = 0
    wall_start = time.monotonic()

    connector = aiohttp.TCPConnector(limit=4)
    async with aiohttp.ClientSession(connector=connector) as session:
        async for page_events in iter_turkey_events(session, cursor_start, end_time):
            t0 = time.monotonic()
            try:
                await db.upsert_batch(page_events, total_ingested)
            except Exception as exc:
                logger.error(
                    "batch_upsert_failed",
                    error=str(exc),
                    page_size=len(page_events),
                )
                metrics.fetch_errors_total.inc()
                raise

            batch_duration = time.monotonic() - t0
            metrics.ingest_duration_seconds.observe(batch_duration)
            metrics.events_ingested_total.inc(len(page_events))

            total_ingested += len(page_events)

            # Log progress every _PROGRESS_INTERVAL events
            if total_ingested - last_progress_log >= _PROGRESS_INTERVAL:
                elapsed = time.monotonic() - wall_start
                rate = total_ingested / elapsed if elapsed > 0 else 0.0
                last_event_time = max(e.event_time for e in page_events)
                logger.info(
                    "progress",
                    ingested=total_ingested,
                    last_time=last_event_time.isoformat(),
                    rate_per_sec=round(rate, 1),
                )
                last_progress_log = total_ingested

    return total_ingested


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------

def main() -> None:
    log_level = os.environ.get("LOG_LEVEL", "INFO")
    _configure_logging(log_level)
    logger: structlog.types.BoundLogger = structlog.get_logger(__name__)

    database_url = os.environ.get("DATABASE_URL", "")
    if not database_url:
        logger.error("fatal_missing_env_var", var="DATABASE_URL")
        sys.exit(1)

    metrics_port = int(os.environ.get("METRICS_PORT", str(_DEFAULT_METRICS_PORT)))

    start_date_str = os.environ.get("START_DATE", _DEFAULT_START)
    try:
        start_time = datetime.fromisoformat(start_date_str).replace(tzinfo=timezone.utc)
    except ValueError as exc:
        logger.error("fatal_invalid_start_date", value=start_date_str, error=str(exc))
        sys.exit(1)

    end_date_str = os.environ.get("END_DATE", "")
    if end_date_str:
        try:
            end_time = datetime.fromisoformat(end_date_str).replace(tzinfo=timezone.utc)
        except ValueError as exc:
            logger.error("fatal_invalid_end_date", value=end_date_str, error=str(exc))
            sys.exit(1)
    else:
        end_time = datetime.now(tz=timezone.utc)

    prom_registry = CollectorRegistry()
    metrics = Metrics(prom_registry)
    _start_metrics_server(metrics_port, prom_registry)
    logger.info("metrics_server_started", port=metrics_port)

    logger.info("seismosis-historical-ingestion starting", start=start_time.isoformat(), end=end_time.isoformat())

    async def _run() -> int:
        db = await Database.connect(database_url)
        try:
            return await run_ingestion(db, metrics, start_time, end_time, logger)
        finally:
            await db.close()

    try:
        total = asyncio.run(_run())
    except Exception as exc:
        logger.error("fatal_ingestion_error", error=str(exc), exc_info=True)
        sys.exit(1)

    logger.info(
        "ingestion_complete",
        total_events_ingested=total,
        start=start_time.isoformat(),
        end=end_time.isoformat(),
    )
    sys.exit(0)


if __name__ == "__main__":
    main()
