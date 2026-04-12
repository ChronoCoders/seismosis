"""Runtime configuration loaded from environment variables."""
from __future__ import annotations

import os
from dataclasses import dataclass


@dataclass(frozen=True)
class Config:
    # ── Kafka ──────────────────────────────────────────────────────────────
    kafka_brokers: str
    kafka_topic_raw: str
    kafka_topic_enriched: str
    kafka_topic_alerts: str
    kafka_topic_dead_letter: str
    kafka_group_id: str
    # ── Schema Registry ────────────────────────────────────────────────────
    schema_registry_url: str
    # ── PostgreSQL ─────────────────────────────────────────────────────────
    database_url: str
    db_pool_min: int
    db_pool_max: int
    # ── Redis ──────────────────────────────────────────────────────────────
    redis_url: str
    event_cache_ttl_secs: int
    # ── Metrics HTTP server ────────────────────────────────────────────────
    metrics_port: int
    # ── Analysis parameters ────────────────────────────────────────────────
    alert_magnitude_threshold: float
    aftershock_window_days: int
    aftershock_distance_km: float
    analysis_version: str
    # ── Logging ────────────────────────────────────────────────────────────
    log_level: str

    @classmethod
    def from_env(cls) -> Config:
        return cls(
            kafka_brokers=os.environ.get("KAFKA_BROKERS", "redpanda:9092"),
            kafka_topic_raw=os.environ.get("KAFKA_TOPIC_RAW", "earthquakes.raw"),
            kafka_topic_enriched=os.environ.get("KAFKA_TOPIC_ENRICHED", "earthquakes.enriched"),
            kafka_topic_alerts=os.environ.get("KAFKA_TOPIC_ALERTS", "earthquakes.alerts"),
            kafka_topic_dead_letter=os.environ.get(
                "KAFKA_TOPIC_DEAD_LETTER", "earthquakes.dead-letter"
            ),
            kafka_group_id=os.environ.get("KAFKA_GROUP_ID", "seismosis-analysis-group"),
            schema_registry_url=os.environ.get("SCHEMA_REGISTRY_URL", "http://redpanda:8081"),
            database_url=os.environ.get(
                "DATABASE_URL",
                "postgresql://seismosis:seismosis@postgres:5432/seismosis",
            ),
            db_pool_min=int(os.environ.get("DB_POOL_MIN", "2")),
            db_pool_max=int(os.environ.get("DB_POOL_MAX", "10")),
            redis_url=os.environ.get("REDIS_URL", "redis://redis:6379"),
            event_cache_ttl_secs=int(os.environ.get("EVENT_CACHE_TTL_SECS", "3600")),
            metrics_port=int(os.environ.get("METRICS_PORT", "9092")),
            alert_magnitude_threshold=float(
                os.environ.get("ALERT_MAGNITUDE_THRESHOLD", "5.0")
            ),
            aftershock_window_days=int(os.environ.get("AFTERSHOCK_WINDOW_DAYS", "30")),
            aftershock_distance_km=float(
                os.environ.get("AFTERSHOCK_DISTANCE_KM", "100.0")
            ),
            analysis_version=os.environ.get("ANALYSIS_VERSION", "0.1.0"),
            log_level=os.environ.get("LOG_LEVEL", "INFO"),
        )
