"""Forecast service orchestrator and HTTP health/metrics server.

Runs three periodic tasks:
  - Gutenberg-Richter grid analysis  (default every 6 h)
  - ETAS aftershock forecasting       (default every 1 h)
  - Seismicity classification         (default every 6 h)

Exposes /health and /metrics on METRICS_PORT (default 9098).
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
import threading
import time
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Any

import structlog
import structlog.types
from prometheus_client import (
    CollectorRegistry,
    CONTENT_TYPE_LATEST,
    generate_latest,
)

from .cache import ForecastCache
from .classifier import SeismicClassifier
from .db import CatalogEvent, Database
from .etas import forecast as etas_forecast
from .producer import ForecastProducer
from .gutenberg_richter import (
    GrResult,
    analyze_catalog,
    build_grid_cells,
    cell_to_wkt,
)
from .metrics import Metrics

# ---------------------------------------------------------------------------
# Service config dataclass
# ---------------------------------------------------------------------------


@dataclass
class ServiceConfig:
    database_url: str
    metrics_port: int
    log_level: str
    gr_interval_hours: float
    etas_interval_hours: float
    classifier_interval_hours: float
    min_lat: float
    max_lat: float
    min_lon: float
    max_lon: float
    min_mag: float
    etas_min_mag: float
    horizon_days: int
    kafka_brokers: str = ""
    schema_registry_url: str = ""
    redis_url: str = ""


def _load_config() -> ServiceConfig:
    return ServiceConfig(
        database_url=os.environ["DATABASE_URL"],
        metrics_port=int(os.environ.get("METRICS_PORT", "9098")),
        log_level=os.environ.get("LOG_LEVEL", "INFO"),
        gr_interval_hours=float(os.environ.get("GR_INTERVAL_HOURS", "6")),
        etas_interval_hours=float(os.environ.get("ETAS_INTERVAL_HOURS", "1")),
        classifier_interval_hours=float(os.environ.get("CLASSIFIER_INTERVAL_HOURS", "6")),
        min_lat=float(os.environ.get("MIN_LAT", "33.0")),
        max_lat=float(os.environ.get("MAX_LAT", "45.0")),
        min_lon=float(os.environ.get("MIN_LON", "22.0")),
        max_lon=float(os.environ.get("MAX_LON", "48.0")),
        min_mag=float(os.environ.get("MIN_MAG", "1.5")),
        etas_min_mag=float(os.environ.get("ETAS_MIN_MAG", "4.0")),
        horizon_days=int(os.environ.get("HORIZON_DAYS", "30")),
        kafka_brokers=os.environ.get("KAFKA_BROKERS", ""),
        schema_registry_url=os.environ.get("SCHEMA_REGISTRY_URL", ""),
        redis_url=os.environ.get("REDIS_URL", ""),
    )


# ---------------------------------------------------------------------------
# HTTP server (health + metrics)
# ---------------------------------------------------------------------------


def _make_handler(registry: CollectorRegistry) -> type[BaseHTTPRequestHandler]:
    class _Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802
            if self.path == "/health":
                body = b'{"status":"ok"}'
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif self.path == "/metrics":
                output: bytes = generate_latest(registry)
                self.send_response(200)
                self.send_header("Content-Type", CONTENT_TYPE_LATEST)
                self.send_header("Content-Length", str(len(output)))
                self.end_headers()
                self.wfile.write(output)
            else:
                self.send_response(404)
                self.end_headers()

        def log_message(self, format: str, *args: Any) -> None:
            # Suppress default stderr logging from BaseHTTPRequestHandler
            pass

    return _Handler


def start_metrics_server(port: int, registry: CollectorRegistry) -> HTTPServer:
    """Start the HTTP server in a daemon thread and return the server instance."""
    server = HTTPServer(("0.0.0.0", port), _make_handler(registry))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


# ---------------------------------------------------------------------------
# GR task
# ---------------------------------------------------------------------------


async def run_gr_analysis(
    db: Database,
    metrics: Metrics,
    logger: Any,
    config: ServiceConfig,
) -> None:
    log = logger.bind(task="gr_analysis")
    log.info("gr_analysis.start")

    try:
        count: int = await db.count_historical_events()
        if count < 30:
            log.warning("gr_analysis.skip", reason="insufficient_events", count=count)
            return

        now: datetime = datetime.now(tz=timezone.utc)
        catalog_start: datetime = datetime(2010, 1, 1, tzinfo=timezone.utc)

        events: list[CatalogEvent] = await db.get_catalog(
            config.min_lat, config.max_lat,
            config.min_lon, config.max_lon,
            config.min_mag,
            catalog_start, now,
            limit=200_000,
        )

        if len(events) < 30:
            log.warning("gr_analysis.skip", reason="insufficient_catalog", count=len(events))
            return

        times: list[datetime] = [e.event_time for e in events]
        lats: list[float] = [e.latitude for e in events]
        lons: list[float] = [e.longitude for e in events]
        mags: list[float] = [e.magnitude for e in events]
        actual_start: datetime = min(times)
        actual_end: datetime = max(times)

        # Global / region-level analysis
        try:
            global_result: GrResult | None = analyze_catalog(
                times, lats, lons, mags,
                actual_start, actual_end,
                "Turkey Region",
            )
            if global_result is not None:
                await db.upsert_gr_analysis(
                    region_name="Turkey Region",
                    grid_cell_wkt=None,
                    b_value=global_result.b_value,
                    b_std=global_result.b_std,
                    a_value=global_result.a_value,
                    mc=global_result.mc,
                    n_events=global_result.n_events,
                    catalog_start=global_result.catalog_start,
                    catalog_end=global_result.catalog_end,
                    model_version="gr-aki-utsu-maxcurv-v1",
                )
                log.info("gr_analysis.global_written", b=global_result.b_value)
        except Exception as exc:
            metrics.gr_errors.inc()
            log.error("gr_analysis.global_error", error=str(exc))

        # Grid analysis
        cells: list[tuple[float, float, float, float]] = build_grid_cells(
            config.min_lat, config.max_lat,
            config.min_lon, config.max_lon,
        )

        for lat_min, lat_max, lon_min, lon_max in cells:
            try:
                cell_events: list[tuple[datetime, float, float, float]] = [
                    (t, la, lo, m)
                    for t, la, lo, m in zip(times, lats, lons, mags)
                    if lat_min <= la < lat_max and lon_min <= lo < lon_max
                ]
                if len(cell_events) < 30:
                    continue

                c_times: list[datetime] = [e[0] for e in cell_events]
                c_lats: list[float] = [e[1] for e in cell_events]
                c_lons: list[float] = [e[2] for e in cell_events]
                c_mags: list[float] = [e[3] for e in cell_events]
                cell_start: datetime = min(c_times)
                cell_end: datetime = max(c_times)

                cell_result: GrResult | None = analyze_catalog(
                    c_times, c_lats, c_lons, c_mags,
                    cell_start, cell_end,
                )
                if cell_result is None:
                    continue

                wkt: str = cell_to_wkt(lat_min, lat_max, lon_min, lon_max)
                region_label: str = (
                    f"{lat_min:.1f}-{lat_max:.1f}N {lon_min:.1f}-{lon_max:.1f}E"
                )
                await db.upsert_gr_analysis(
                    region_name=region_label,
                    grid_cell_wkt=wkt,
                    b_value=cell_result.b_value,
                    b_std=cell_result.b_std,
                    a_value=cell_result.a_value,
                    mc=cell_result.mc,
                    n_events=cell_result.n_events,
                    catalog_start=cell_result.catalog_start,
                    catalog_end=cell_result.catalog_end,
                    model_version="gr-aki-utsu-maxcurv-v1",
                )
                metrics.gr_cells_computed.inc()

            except Exception as exc:
                metrics.gr_errors.inc()
                log.error(
                    "gr_analysis.cell_error",
                    cell=f"{lat_min:.1f}-{lat_max:.1f}N {lon_min:.1f}-{lon_max:.1f}E",
                    error=str(exc),
                )

        log.info("gr_analysis.complete", cells_total=len(cells))

    except Exception as exc:
        metrics.gr_errors.inc()
        log.error("gr_analysis.fatal_error", error=str(exc))


# ---------------------------------------------------------------------------
# ETAS task
# ---------------------------------------------------------------------------


async def run_etas_forecasts(
    db: Database,
    metrics: Metrics,
    logger: Any,
    config: ServiceConfig,
) -> None:
    log = logger.bind(task="etas_forecasts")
    log.info("etas_forecasts.start")

    try:
        mainshocks: list[CatalogEvent] = await db.get_recent_mainshocks(
            min_mag=config.etas_min_mag,
            lookback_days=30,
        )

        if not mainshocks:
            log.info("etas_forecasts.no_mainshocks")
            return

        now: datetime = datetime.now(tz=timezone.utc)
        computed: int = 0

        for mainshock in mainshocks:
            try:
                start_window: datetime = mainshock.event_time - timedelta(days=365)
                nearby: list[CatalogEvent] = await db.get_nearby_catalog(
                    center_lon=mainshock.longitude,
                    center_lat=mainshock.latitude,
                    radius_m=300_000.0,
                    min_mag=1.5,
                    start_time=start_window,
                    end_time=now,
                )

                catalog_times = [e.event_time for e in nearby]
                catalog_mags = [e.magnitude for e in nearby]
                catalog_lats = [e.latitude for e in nearby]
                catalog_lons = [e.longitude for e in nearby]

                result = etas_forecast(
                    mainshock_time=mainshock.event_time,
                    mainshock_mag=mainshock.magnitude,
                    catalog_times=catalog_times,
                    catalog_mags=catalog_mags,
                    horizon_days=config.horizon_days,
                    catalog_lats=catalog_lats,
                    catalog_lons=catalog_lons,
                    mainshock_lat=mainshock.latitude,
                    mainshock_lon=mainshock.longitude,
                )

                params_snapshot: dict[str, float] = {
                    "mu": result.params.mu,
                    "K": result.params.K,
                    "alpha": result.params.alpha,
                    "c": result.params.c,
                    "p": result.params.p,
                }

                spatial_heatmap_json: str | None = (
                    json.dumps(result.spatial_heatmap)
                    if result.spatial_heatmap is not None
                    else None
                )

                await db.upsert_etas_forecast(
                    mainshock_source_id=mainshock.source_id,
                    horizon_days=config.horizon_days,
                    min_magnitude=1.0,
                    expected_count=result.expected_count,
                    p_at_least_one=result.p_at_least_one,
                    p_exceedance_json=json.dumps(result.p_exceedance),
                    daily_rates_json=json.dumps(result.daily_rates),
                    params_snapshot_json=json.dumps(params_snapshot),
                    model_version=result.model_version,
                    spatial_heatmap_json=spatial_heatmap_json,
                )
                metrics.etas_computed.inc()
                computed += 1

            except Exception as exc:
                metrics.etas_errors.inc()
                log.error(
                    "etas_forecasts.event_error",
                    source_id=mainshock.source_id,
                    error=str(exc),
                )

        log.info("etas_forecasts.complete", computed=computed, total=len(mainshocks))

    except Exception as exc:
        metrics.etas_errors.inc()
        log.error("etas_forecasts.fatal_error", error=str(exc))


# ---------------------------------------------------------------------------
# Classifier task
# ---------------------------------------------------------------------------


async def run_classifier(
    db: Database,
    metrics: Metrics,
    logger: Any,
    classifier: SeismicClassifier,
    config: ServiceConfig,
) -> None:
    log = logger.bind(task="classifier")
    log.info("classifier.start")

    try:
        now: datetime = datetime.now(tz=timezone.utc)
        catalog_start: datetime = datetime(2010, 1, 1, tzinfo=timezone.utc)

        # Attempt to train / retrain on historical catalog
        try:
            train_events: list[CatalogEvent] = await db.get_catalog(
                config.min_lat, config.max_lat,
                config.min_lon, config.max_lon,
                min_mag=1.5,
                start_time=catalog_start,
                end_time=now,
                limit=50_000,
            )

            if len(train_events) >= 50:
                depths: list[float] = [e.depth_km for e in train_events]
                mags: list[float] = [e.magnitude for e in train_events]
                lats: list[float] = [e.latitude for e in train_events]
                lons: list[float] = [e.longitude for e in train_events]

                n_train: int = classifier.train(depths, mags, lats, lons)

                if n_train > 0:
                    trained_at: datetime = datetime.now(tz=timezone.utc)
                    await db.deactivate_model_type("seismicity_classifier")
                    await db.upsert_model_registry(
                        model_type="seismicity_classifier",
                        version="hgb-classifier-v1",
                        trained_at=trained_at,
                        n_train=n_train,
                        metrics_json=json.dumps({"n_train": n_train}),
                        artifact_path="",
                        is_active=True,
                    )
                    log.info("classifier.trained", n_train=n_train)
            else:
                log.info(
                    "classifier.train_skip",
                    reason="insufficient_catalog",
                    count=len(train_events),
                )

        except Exception as exc:
            metrics.classifier_errors.inc()
            log.error("classifier.train_error", error=str(exc))

        # Classify unclassified events
        try:
            unclassified: list[CatalogEvent] = await db.get_unclassified_events(limit=500)

            if not unclassified:
                log.info("classifier.no_unclassified")
                return

            src_ids: list[str] = [e.source_id for e in unclassified]
            uc_depths: list[float] = [e.depth_km for e in unclassified]
            uc_mags: list[float] = [e.magnitude for e in unclassified]
            uc_lats: list[float] = [e.latitude for e in unclassified]
            uc_lons: list[float] = [e.longitude for e in unclassified]

            results = classifier.predict(src_ids, uc_depths, uc_mags, uc_lats, uc_lons)

            classifications: list[tuple[str, str, float]] = [
                (r.source_id, r.event_class, r.confidence) for r in results
            ]
            await db.update_event_classifications(classifications)
            metrics.classifier_events.inc(len(results))
            log.info("classifier.classified", count=len(results))

        except Exception as exc:
            metrics.classifier_errors.inc()
            log.error("classifier.predict_error", error=str(exc))

    except Exception as exc:
        metrics.classifier_errors.inc()
        log.error("classifier.fatal_error", error=str(exc))


# ---------------------------------------------------------------------------
# Main loop
# ---------------------------------------------------------------------------


async def main_loop(
    db: Database,
    metrics: Metrics,
    logger: Any,
    config: ServiceConfig,
    producer: ForecastProducer | None = None,
) -> None:
    classifier = SeismicClassifier()

    last_gr: float = 0.0
    last_etas: float = 0.0
    last_classifier: float = 0.0

    gr_interval_s: float = config.gr_interval_hours * 3600.0
    etas_interval_s: float = config.etas_interval_hours * 3600.0
    classifier_interval_s: float = config.classifier_interval_hours * 3600.0

    log = logger.bind(component="main_loop")
    log.info("main_loop.start")

    while True:
        cycle_start: float = time.monotonic()

        now_mono: float = time.monotonic()

        # Run GR if interval elapsed (or first run)
        if now_mono - last_gr >= gr_interval_s:
            with metrics.cycle_duration.time():
                await run_gr_analysis(db, metrics, logger, config)
            last_gr = time.monotonic()

        # Run ETAS if interval elapsed (or first run)
        if now_mono - last_etas >= etas_interval_s:
            await run_etas_forecasts(db, metrics, logger, config)
            last_etas = time.monotonic()

        # Run classifier if interval elapsed (or first run)
        if now_mono - last_classifier >= classifier_interval_s:
            await run_classifier(db, metrics, logger, classifier, config)
            last_classifier = time.monotonic()

        # Drain the producer send buffer at the end of every cycle so messages
        # are not held in the librdkafka queue indefinitely.  This is a no-op
        # when the producer is disabled (confluent_kafka not installed or
        # KAFKA_BROKERS not set).
        if producer is not None:
            producer.flush(timeout_secs=5.0)

        elapsed: float = time.monotonic() - cycle_start
        sleep_s: float = max(0.0, 60.0 - elapsed)
        await asyncio.sleep(sleep_s)


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> None:
    config: ServiceConfig
    try:
        config = _load_config()
    except KeyError as exc:
        print(f"Missing required environment variable: {exc}", file=sys.stderr)
        sys.exit(1)

    # Configure structlog
    _level_map: dict[str, int] = {"DEBUG": 10, "INFO": 20, "WARNING": 30, "ERROR": 40}
    log_level_int: int = _level_map.get(config.log_level.upper(), 20)
    structlog.configure(
        processors=[
            structlog.stdlib.add_log_level,
            structlog.processors.TimeStamper(fmt="iso"),
            structlog.processors.StackInfoRenderer(),
            structlog.processors.ExceptionRenderer(),
            structlog.processors.JSONRenderer(),
        ],
        wrapper_class=structlog.make_filtering_bound_logger(log_level_int),
        logger_factory=structlog.PrintLoggerFactory(),
    )
    logger: structlog.types.BoundLogger = structlog.get_logger("forecast")

    # Prometheus registry
    registry = CollectorRegistry()
    metrics = Metrics(registry)

    # Start HTTP server
    start_metrics_server(config.metrics_port, registry)
    logger.info("metrics_server.started", port=config.metrics_port)

    # Initialise Kafka producer (optional — no-op if KAFKA_BROKERS is empty)
    producer: ForecastProducer | None = None
    if config.kafka_brokers:
        producer = ForecastProducer(
            kafka_brokers=config.kafka_brokers,
            schema_registry_url=config.schema_registry_url,
        )
    else:
        logger.info("forecast_producer.skipped", reason="KAFKA_BROKERS not set")

    # Initialise Redis cache (non-fatal if Redis is unavailable)
    cache: ForecastCache | None = None
    if config.redis_url:
        cache = ForecastCache(config.redis_url)
        logger.info("forecast_cache.initialised", redis_url=config.redis_url)
    else:
        logger.info("forecast_cache.disabled", reason="REDIS_URL not set")

    # Connect DB and run
    async def _run() -> None:
        db: Database = await Database.connect(config.database_url)
        logger.info("database.connected")
        try:
            await main_loop(db, metrics, logger, config, producer)
        finally:
            await db.close()
            logger.info("database.closed")
            if cache is not None:
                cache.close()
                logger.info("forecast_cache.closed")

    try:
        asyncio.run(_run())
    except KeyboardInterrupt:
        logger.info("forecast.shutdown")
    finally:
        if producer is not None:
            producer.close()


if __name__ == "__main__":
    main()
