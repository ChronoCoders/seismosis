"""
Seismosis Analysis Service
==========================

Consumes raw seismic events from earthquakes.raw and produces:
  • earthquakes.enriched  — ML-refined event with aftershock flag + risk estimate
  • earthquakes.alerts    — M ≥ threshold events with alert level
  • earthquakes.dead-letter — decode / produce failures (JSON)

Also writes enrichment columns to seismology.seismic_events (PostgreSQL)
and caches enriched events in Redis.

Processing pipeline per message
---------------------------------
1.  Avro decode (Confluent wire format via Schema Registry)
2.  ML magnitude refinement     (magnitude.refine_to_ml)
3.  ETAS aftershock detection   (aftershock.find_mainshock / classify_aftershock)
4.  Risk estimation             (risk.estimate_felt_radius_km / estimate_epicentral_mmi)
5.  Produce enriched event      → earthquakes.enriched
6.  Produce alert if M ≥ threshold → earthquakes.alerts
7.  Upsert to PostgreSQL        (non-fatal)
8.  Write Redis cache           (non-fatal)
9.  Commit Kafka offset

Failure modes
-------------
* Avro decode failure     → DLQ, offset committed
* Enriched produce failure → DLQ, offset committed
* Alert produce failure   → logged ERROR, processing continues
* DB write failure        → logged ERROR, metric incremented, continues
* Redis write failure     → logged WARNING, metric incremented, continues
"""
from __future__ import annotations

import logging
import signal
import threading
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, HTTPServer
from typing import Optional

import fastavro
import structlog
from prometheus_client import CollectorRegistry, CONTENT_TYPE_LATEST, generate_latest

from .aftershock import classify_aftershock, find_mainshock
from .avro_codec import AvroDecoder, AvroEncoder, SchemaRegistry
from .cache import Cache
from .config import Config
from .consumer import KafkaConsumer
from .db import Database
from .magnitude import refine_to_ml
from .metrics import Metrics
from .models import AlertEvent, EnrichedEvent, RawEvent
from .producer import KafkaProducer
from .risk import alert_level, estimate_epicentral_mmi, estimate_felt_radius_km
from .schema import ALERT_SCHEMA, ALERT_SUBJECT, ENRICHED_SCHEMA, ENRICHED_SUBJECT


# ─── Logging ──────────────────────────────────────────────────────────────────

def _configure_logging(level: str) -> None:
    structlog.configure(
        processors=[
            structlog.stdlib.filter_by_level,
            structlog.stdlib.add_log_level,
            structlog.stdlib.add_logger_name,
            structlog.processors.TimeStamper(fmt="iso"),
            structlog.processors.StackInfoRenderer(),
            structlog.processors.format_exc_info,
            structlog.processors.JSONRenderer(),
        ],
        wrapper_class=structlog.make_filtering_bound_logger(
            getattr(logging, level.upper(), logging.INFO)
        ),
        logger_factory=structlog.PrintLoggerFactory(),
    )


# ─── Metrics HTTP server ──────────────────────────────────────────────────────

def _start_metrics_server(port: int, prom_registry: CollectorRegistry) -> HTTPServer:
    """
    Start a daemon thread serving /metrics (Prometheus) and /health.
    """
    class _Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.path == "/metrics":
                output = generate_latest(prom_registry)
                self.send_response(200)
                self.send_header("Content-Type", CONTENT_TYPE_LATEST)
                self.end_headers()
                self.wfile.write(output)
            elif self.path == "/health":
                self.send_response(200)
                self.end_headers()
                self.wfile.write(b"ok")
            else:
                self.send_response(404)
                self.end_headers()

        def log_message(self, fmt, *args):
            pass  # suppress per-request access log noise

    server = HTTPServer(("0.0.0.0", port), _Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


# ─── Per-message processing ───────────────────────────────────────────────────

def process_message(
    raw_bytes: bytes,
    partition: int,
    offset: int,
    topic: str,
    decoder: AvroDecoder,
    enriched_encoder: AvroEncoder,
    alert_encoder: AvroEncoder,
    producer: KafkaProducer,
    db: Database,
    cache: Cache,
    metrics: Metrics,
    config: Config,
    logger,
) -> None:
    """
    Decode, enrich, and produce a single Kafka message.

    All side effects that are non-fatal (DB, Redis, alert produce) are
    guarded by try/except.  The only fatal paths are Avro decode and
    enriched produce failures — both route to the DLQ.
    """
    source_id: Optional[str] = None

    # ── 1. Avro decode ────────────────────────────────────────────────────
    try:
        record = decoder.decode(raw_bytes)
        event = RawEvent.from_avro_record(record)
        source_id = event.source_id
    except Exception as exc:
        logger.warning(
            "avro_decode_error",
            error=str(exc),
            partition=partition,
            offset=offset,
        )
        producer.produce_dead_letter(
            config.kafka_topic_dead_letter,
            key=source_id or "unknown",
            failure_reason="avro_decode_error",
            failure_detail=str(exc),
            original_topic=topic,
            partition=partition,
            offset=offset,
            source_id=source_id,
            raw_payload_hex=raw_bytes.hex(),
        )
        metrics.dead_letter_total.labels(failure_reason="avro_decode_error").inc()
        return

    # ── 2. ML magnitude refinement ────────────────────────────────────────
    ml_magnitude, ml_source = refine_to_ml(
        event.magnitude, event.magnitude_type, event.depth_km
    )
    metrics.magnitude_correction.labels(
        magnitude_type=event.magnitude_type
    ).observe(ml_magnitude - event.magnitude)

    # ── 3. ETAS aftershock detection ──────────────────────────────────────
    is_aftershock = False
    mainshock_source_id: Optional[str] = None
    mainshock_magnitude: Optional[float] = None
    mainshock_distance_km: Optional[float] = None
    mainshock_time_delta_hours: Optional[float] = None

    conn = db.get_conn()
    try:
        mainshock = find_mainshock(
            conn,
            magnitude=event.magnitude,
            latitude=event.latitude,
            longitude=event.longitude,
            event_time=event.event_time,
            max_window_days=config.aftershock_window_days,
            max_window_km=config.aftershock_distance_km,
        )
        if mainshock is not None:
            is_aftershock, delta_h, dist_km = classify_aftershock(
                event.magnitude, event.event_time, mainshock
            )
            if is_aftershock:
                mainshock_source_id = mainshock.source_id
                mainshock_magnitude = mainshock.magnitude
                mainshock_distance_km = dist_km
                mainshock_time_delta_hours = delta_h
                metrics.aftershocks_detected_total.inc()
    except Exception as exc:
        logger.warning(
            "aftershock_detection_error",
            source_id=source_id,
            error=str(exc),
        )
    finally:
        db.put_conn(conn)

    # ── 4. Risk estimation ────────────────────────────────────────────────
    felt_radius = estimate_felt_radius_km(ml_magnitude, event.depth_km)
    mmi = estimate_epicentral_mmi(ml_magnitude, event.depth_km)
    level = alert_level(ml_magnitude)

    now_ms = int(datetime.now(tz=timezone.utc).timestamp() * 1000)

    enriched = EnrichedEvent(
        source_id=event.source_id,
        source_network=event.source_network,
        event_time_ms=event.event_time_ms,
        latitude=event.latitude,
        longitude=event.longitude,
        depth_km=event.depth_km,
        magnitude=event.magnitude,
        magnitude_type=event.magnitude_type,
        region_name=event.region_name,
        quality_indicator=event.quality_indicator,
        raw_payload=event.raw_payload,
        ingested_at_ms=event.ingested_at_ms,
        pipeline_version=event.pipeline_version,
        ml_magnitude=ml_magnitude,
        ml_magnitude_source=ml_source,
        is_aftershock=is_aftershock,
        mainshock_source_id=mainshock_source_id,
        mainshock_magnitude=mainshock_magnitude,
        mainshock_distance_km=mainshock_distance_km,
        mainshock_time_delta_hours=mainshock_time_delta_hours,
        estimated_felt_radius_km=felt_radius,
        estimated_intensity_mmi=mmi,
        enriched_at_ms=now_ms,
        analysis_version=config.analysis_version,
    )

    # ── 5. Produce enriched event ─────────────────────────────────────────
    try:
        payload = enriched_encoder.encode(enriched.to_avro_record())
        producer.produce_enriched(
            config.kafka_topic_enriched,
            key=enriched.source_id,
            value=payload,
        )
        metrics.messages_enriched_total.labels(
            source_network=event.source_network
        ).inc()
    except Exception as exc:
        logger.error(
            "enriched_produce_error",
            source_id=source_id,
            error=str(exc),
        )
        producer.produce_dead_letter(
            config.kafka_topic_dead_letter,
            key=source_id,
            failure_reason="enriched_produce_error",
            failure_detail=str(exc),
            original_topic=topic,
            partition=partition,
            offset=offset,
            source_id=source_id,
            raw_payload_hex=raw_bytes.hex(),
        )
        metrics.dead_letter_total.labels(failure_reason="enriched_produce_error").inc()
        return

    # ── 6. Alert (M ≥ threshold) ──────────────────────────────────────────
    if level is not None and ml_magnitude >= config.alert_magnitude_threshold:
        alert_event = AlertEvent(
            source_id=enriched.source_id,
            event_time_ms=enriched.event_time_ms,
            latitude=enriched.latitude,
            longitude=enriched.longitude,
            depth_km=enriched.depth_km,
            magnitude=enriched.magnitude,
            ml_magnitude=enriched.ml_magnitude,
            region_name=enriched.region_name,
            estimated_intensity_mmi=enriched.estimated_intensity_mmi,
            estimated_felt_radius_km=enriched.estimated_felt_radius_km,
            is_aftershock=enriched.is_aftershock,
            alert_level=level,
            triggered_at_ms=now_ms,
        )
        try:
            alert_payload = alert_encoder.encode(alert_event.to_avro_record())
            producer.produce_alert(
                config.kafka_topic_alerts,
                key=enriched.source_id,
                value=alert_payload,
            )
            metrics.alerts_produced_total.labels(alert_level=level).inc()
            logger.info(
                "alert_produced",
                source_id=source_id,
                ml_magnitude=ml_magnitude,
                alert_level=level,
                mmi=mmi,
            )
        except Exception as exc:
            logger.error(
                "alert_produce_error",
                source_id=source_id,
                error=str(exc),
            )

    # ── 7. PostgreSQL upsert (non-fatal) ──────────────────────────────────
    try:
        db.upsert_enriched_event(enriched)
    except Exception as exc:
        logger.error("db_write_error", source_id=source_id, error=str(exc))
        metrics.db_write_errors_total.inc()

    # ── 8. Redis cache (non-fatal) ────────────────────────────────────────
    if not cache.set_event(enriched, ttl_secs=config.event_cache_ttl_secs):
        metrics.cache_write_errors_total.inc()

    logger.debug(
        "message_processed",
        source_id=source_id,
        ml_magnitude=ml_magnitude,
        ml_source=ml_source,
        is_aftershock=is_aftershock,
        mmi=mmi,
        felt_radius_km=felt_radius,
    )


# ─── Entry point ──────────────────────────────────────────────────────────────

def main() -> None:
    config = Config.from_env()
    _configure_logging(config.log_level)
    log = structlog.get_logger(__name__)

    log.info("seismosis-analysis starting", version=config.analysis_version)

    # ── Schema Registry + Avro setup ──────────────────────────────────────
    registry = SchemaRegistry(config.schema_registry_url)

    log.info("registering_schemas")
    enriched_schema_id = registry.register(ENRICHED_SUBJECT, ENRICHED_SCHEMA)
    alert_schema_id = registry.register(ALERT_SUBJECT, ALERT_SCHEMA)
    log.info(
        "schemas_registered",
        enriched_id=enriched_schema_id,
        alert_id=alert_schema_id,
    )

    enriched_parsed = fastavro.parse_schema(dict(ENRICHED_SCHEMA))
    alert_parsed = fastavro.parse_schema(dict(ALERT_SCHEMA))
    decoder = AvroDecoder(registry)
    enriched_encoder = AvroEncoder(enriched_parsed, enriched_schema_id)
    alert_encoder = AvroEncoder(alert_parsed, alert_schema_id)

    # ── Infrastructure ─────────────────────────────────────────────────────
    db = Database(config.database_url, config.db_pool_min, config.db_pool_max)
    cache = Cache(config.redis_url)
    consumer = KafkaConsumer(
        config.kafka_brokers, config.kafka_group_id, config.kafka_topic_raw
    )
    producer = KafkaProducer(config.kafka_brokers)

    # ── Prometheus ─────────────────────────────────────────────────────────
    prom_registry = CollectorRegistry()
    metrics = Metrics(prom_registry)
    _start_metrics_server(config.metrics_port, prom_registry)
    log.info("metrics_server_started", port=config.metrics_port)

    # ── Graceful shutdown ──────────────────────────────────────────────────
    shutdown = threading.Event()

    def _handle_signal(sig, _frame):
        log.info("shutdown_signal_received", signal=sig)
        shutdown.set()

    signal.signal(signal.SIGTERM, _handle_signal)
    signal.signal(signal.SIGINT, _handle_signal)

    log.info(
        "consume_loop_started",
        topic=config.kafka_topic_raw,
        group=config.kafka_group_id,
    )

    # ── Consume loop ───────────────────────────────────────────────────────
    while not shutdown.is_set():
        msg = consumer.poll(timeout=1.0)
        if msg is None:
            continue

        metrics.messages_consumed_total.labels(
            topic=config.kafka_topic_raw
        ).inc()

        t0 = time.monotonic()
        try:
            process_message(
                raw_bytes=msg.value(),
                partition=msg.partition(),
                offset=msg.offset(),
                topic=msg.topic(),
                decoder=decoder,
                enriched_encoder=enriched_encoder,
                alert_encoder=alert_encoder,
                producer=producer,
                db=db,
                cache=cache,
                metrics=metrics,
                config=config,
                logger=log,
            )
        except Exception as exc:
            log.error(
                "unexpected_processing_error",
                partition=msg.partition(),
                offset=msg.offset(),
                error=str(exc),
                exc_info=True,
            )
        finally:
            metrics.processing_duration_seconds.labels(
                topic=config.kafka_topic_raw
            ).observe(time.monotonic() - t0)
            # Always commit — DLQ is the safety net for unprocessable messages.
            consumer.commit(msg)

    # ── Drain and close ────────────────────────────────────────────────────
    log.info("shutting_down")
    consumer.close()
    producer.close()
    db.close()
    cache.close()
    log.info("seismosis-analysis stopped")


if __name__ == "__main__":
    main()
