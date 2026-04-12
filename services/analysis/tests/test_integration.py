"""
Integration tests for the analysis service pipeline.

These tests spin up real Redpanda (Kafka + Schema Registry), PostgreSQL
(PostGIS), and Redis containers via testcontainers and verify the full
pipeline end-to-end: raw event → enrich → produce → DB write → Redis cache.

All fixtures are session-scoped (containers started once per run).

Run with:
    pytest tests/test_integration.py -v

Skip reason: tests are skipped automatically when Docker/testcontainers
packages are unavailable.
"""
from __future__ import annotations

import io
import json
import struct
import time
from datetime import datetime, timezone
from typing import Generator

import fastavro
import psycopg2
import pytest

from analysis.avro_codec import AvroDecoder, AvroEncoder, SchemaRegistry
from analysis.cache import Cache
from analysis.config import Config
from analysis.db import Database
from analysis.magnitude import refine_to_ml
from analysis.main import process_message
from analysis.metrics import Metrics
from analysis.models import EnrichedEvent, RawEvent
from analysis.producer import KafkaProducer
from analysis.risk import alert_level, estimate_epicentral_mmi, estimate_felt_radius_km
from analysis.schema import (
    ALERT_SCHEMA,
    ALERT_SUBJECT,
    ENRICHED_SCHEMA,
    ENRICHED_SUBJECT,
)

# ─── Raw event Avro schema (mirrors ingestion/src/schema.rs) ──────────────────

_RAW_SCHEMA: dict = {
    "type": "record",
    "name": "RawEarthquakeEvent",
    "namespace": "com.seismosis",
    "fields": [
        {"name": "source_id", "type": "string"},
        {"name": "source_network", "type": "string"},
        {
            "name": "event_time_ms",
            "type": {"type": "long", "logicalType": "timestamp-millis"},
        },
        {"name": "latitude", "type": "double"},
        {"name": "longitude", "type": "double"},
        {"name": "depth_km", "type": ["null", "double"], "default": None},
        {"name": "magnitude", "type": "double"},
        {"name": "magnitude_type", "type": "string"},
        {"name": "region_name", "type": ["null", "string"], "default": None},
        {"name": "quality_indicator", "type": "string"},
        {"name": "raw_payload", "type": "string"},
        {
            "name": "ingested_at_ms",
            "type": {"type": "long", "logicalType": "timestamp-millis"},
        },
        {"name": "pipeline_version", "type": "string"},
    ],
}

_RAW_SUBJECT = "earthquakes.raw-value"

# ─── Helpers ──────────────────────────────────────────────────────────────────

def _encode_raw_event(event_dict: dict, schema_id: int) -> bytes:
    """Encode a raw event dict to Confluent wire format."""
    parsed = fastavro.parse_schema(dict(_RAW_SCHEMA))
    bio = io.BytesIO()
    fastavro.schemaless_writer(bio, parsed, event_dict)
    return bytes([0x00]) + struct.pack(">I", schema_id) + bio.getvalue()


def _sample_raw_record(now_ms: int) -> dict:
    return {
        "source_id": "USGS:integration_test_001",
        "source_network": "USGS",
        "event_time_ms": datetime.fromtimestamp(now_ms / 1000, tz=timezone.utc),
        "latitude": 38.5,
        "longitude": 27.3,
        "depth_km": 12.0,
        "magnitude": 4.2,
        "magnitude_type": "ML",
        "region_name": "Aegean Sea",
        "quality_indicator": "B",
        "raw_payload": json.dumps({"id": "integration_test_001", "net": "us"}),
        "ingested_at_ms": datetime.fromtimestamp(now_ms / 1000, tz=timezone.utc),
        "pipeline_version": "0.1.0",
    }


def _sample_large_raw_record(now_ms: int) -> dict:
    """M≥5.0 event that should trigger an alert."""
    rec = _sample_raw_record(now_ms)
    return {
        **rec,
        "source_id": "USGS:integration_test_big",
        "magnitude": 6.3,
        "magnitude_type": "Mw",
        "region_name": "Central Turkey",
    }


# ─── Fixtures ─────────────────────────────────────────────────────────────────

@pytest.fixture(scope="session")
def sr_client(redpanda_brokers_and_sr) -> SchemaRegistry:
    _, sr_url = redpanda_brokers_and_sr
    return SchemaRegistry(sr_url)


@pytest.fixture(scope="session")
def raw_schema_id(sr_client: SchemaRegistry) -> int:
    return sr_client.register(_RAW_SUBJECT, _RAW_SCHEMA)


@pytest.fixture(scope="session")
def enriched_schema_id(sr_client: SchemaRegistry) -> int:
    return sr_client.register(ENRICHED_SUBJECT, ENRICHED_SCHEMA)


@pytest.fixture(scope="session")
def alert_schema_id(sr_client: SchemaRegistry) -> int:
    return sr_client.register(ALERT_SUBJECT, ALERT_SCHEMA)


@pytest.fixture(scope="session")
def encoders(enriched_schema_id, alert_schema_id):
    enriched_parsed = fastavro.parse_schema(dict(ENRICHED_SCHEMA))
    alert_parsed = fastavro.parse_schema(dict(ALERT_SCHEMA))
    return (
        AvroEncoder(enriched_parsed, enriched_schema_id),
        AvroEncoder(alert_parsed, alert_schema_id),
    )


@pytest.fixture(scope="session")
def integration_config(redpanda_brokers_and_sr, postgres_dsn, redis_url) -> Config:
    brokers, sr_url = redpanda_brokers_and_sr
    return Config(
        kafka_brokers=brokers,
        kafka_topic_raw="earthquakes.raw",
        kafka_topic_enriched="earthquakes.enriched",
        kafka_topic_alerts="earthquakes.alerts",
        kafka_topic_dead_letter="earthquakes.dead-letter",
        kafka_group_id="seismosis-analysis-test",
        schema_registry_url=sr_url,
        database_url=postgres_dsn,
        db_pool_min=1,
        db_pool_max=3,
        redis_url=redis_url,
        event_cache_ttl_secs=300,
        metrics_port=19092,  # avoid clashing with any running service
        alert_magnitude_threshold=5.0,
        aftershock_window_days=30,
        aftershock_distance_km=100.0,
        analysis_version="0.1.0-test",
        log_level="WARNING",
    )


@pytest.fixture(scope="session")
def db(integration_config: Config) -> Generator[Database, None, None]:
    database = Database(
        integration_config.database_url,
        min_conn=1,
        max_conn=3,
    )
    yield database
    database.close()


@pytest.fixture(scope="session")
def cache(integration_config: Config) -> Generator[Cache, None, None]:
    c = Cache(integration_config.redis_url)
    yield c
    c.close()


@pytest.fixture(scope="session")
def producer(integration_config: Config) -> Generator[KafkaProducer, None, None]:
    p = KafkaProducer(integration_config.kafka_brokers)
    yield p
    p.close()


# ─── Tests ────────────────────────────────────────────────────────────────────

class TestProcessMessageIntegration:
    """End-to-end pipeline tests with real Redpanda, PostgreSQL, Redis."""

    def test_raw_event_enriched_and_written_to_db(
        self,
        redpanda_brokers_and_sr,
        sr_client,
        raw_schema_id,
        encoders,
        integration_config,
        db,
        cache,
        producer,
    ):
        from prometheus_client import CollectorRegistry
        enriched_encoder, alert_encoder = encoders
        decoder = AvroDecoder(sr_client)
        prom_registry = CollectorRegistry()
        metrics = Metrics(prom_registry)

        now_ms = int(datetime.now(tz=timezone.utc).timestamp() * 1000)
        raw_record = _sample_raw_record(now_ms)
        raw_bytes = _encode_raw_event(raw_record, raw_schema_id)

        process_message(
            raw_bytes=raw_bytes,
            partition=0,
            offset=0,
            topic=integration_config.kafka_topic_raw,
            decoder=decoder,
            enriched_encoder=enriched_encoder,
            alert_encoder=alert_encoder,
            producer=producer,
            db=db,
            cache=cache,
            metrics=metrics,
            config=integration_config,
            logger=__import__("structlog").get_logger(),
        )

        # ── Verify PostgreSQL row ─────────────────────────────────────────
        conn = psycopg2.connect(integration_config.database_url)
        try:
            with conn.cursor() as cur:
                cur.execute(
                    """
                    SELECT source_id, magnitude, ml_magnitude, is_aftershock,
                           estimated_felt_radius_km, estimated_intensity_mmi,
                           analysis_version
                    FROM seismology.seismic_events
                    WHERE source_id = %s
                    """,
                    ("USGS:integration_test_001",),
                )
                row = cur.fetchone()
        finally:
            conn.close()

        assert row is not None, "Row not found in seismic_events"
        assert float(row[1]) == pytest.approx(4.2, abs=0.05)  # magnitude
        assert row[2] is not None                              # ml_magnitude
        assert row[3] is not None                              # is_aftershock
        assert row[4] is not None                              # felt_radius
        assert row[5] is not None                              # mmi
        assert row[6] == "0.1.0-test"                         # analysis_version

        # ── Verify Redis cache ────────────────────────────────────────────
        cached = cache.get_event("USGS:integration_test_001")
        assert cached is not None
        assert cached["source_id"] == "USGS:integration_test_001"
        assert "ml_magnitude" in cached
        assert "is_aftershock" in cached

        # ── Verify metrics ────────────────────────────────────────────────
        assert (
            metrics.messages_enriched_total.labels(source_network="USGS")._value.get()
            == 1.0
        )

    def test_large_event_triggers_alert(
        self,
        redpanda_brokers_and_sr,
        sr_client,
        raw_schema_id,
        encoders,
        integration_config,
        db,
        cache,
        producer,
    ):
        from prometheus_client import CollectorRegistry
        enriched_encoder, alert_encoder = encoders
        decoder = AvroDecoder(sr_client)
        prom_registry = CollectorRegistry()
        metrics = Metrics(prom_registry)

        now_ms = int(datetime.now(tz=timezone.utc).timestamp() * 1000)
        raw_record = _sample_large_raw_record(now_ms)
        raw_bytes = _encode_raw_event(raw_record, raw_schema_id)

        process_message(
            raw_bytes=raw_bytes,
            partition=0,
            offset=1,
            topic=integration_config.kafka_topic_raw,
            decoder=decoder,
            enriched_encoder=enriched_encoder,
            alert_encoder=alert_encoder,
            producer=producer,
            db=db,
            cache=cache,
            metrics=metrics,
            config=integration_config,
            logger=__import__("structlog").get_logger(),
        )

        # A M6.3 Mw event should produce an alert
        total_alerts = sum(
            metrics.alerts_produced_total.labels(level)._value.get()
            for level in ("YELLOW", "ORANGE", "RED")
        )
        assert total_alerts >= 1


class TestDatabaseIntegration:
    """Direct Database upsert tests."""

    def _make_enriched(self, source_id: str) -> EnrichedEvent:
        now_ms = int(datetime.now(tz=timezone.utc).timestamp() * 1000)
        return EnrichedEvent(
            source_id=source_id,
            source_network="EMSC",
            event_time_ms=now_ms - 5000,
            latitude=39.1,
            longitude=29.2,
            depth_km=8.0,
            magnitude=3.8,
            magnitude_type="ML",
            region_name="Western Turkey",
            quality_indicator="C",
            raw_payload='{"id":"db_test"}',
            ingested_at_ms=now_ms - 1000,
            pipeline_version="0.1.0",
            ml_magnitude=3.76,
            ml_magnitude_source="ml_depth_corrected",
            is_aftershock=False,
            mainshock_source_id=None,
            mainshock_magnitude=None,
            mainshock_distance_km=None,
            mainshock_time_delta_hours=None,
            estimated_felt_radius_km=12.0,
            estimated_intensity_mmi=4.1,
            enriched_at_ms=now_ms,
            analysis_version="0.1.0-test",
        )

    def test_upsert_insert_then_update(self, db: Database, postgres_dsn: str):
        ev = self._make_enriched("EMSC:db_upsert_test")
        db.upsert_enriched_event(ev)

        # Upsert with different ml_magnitude to verify UPDATE path
        ev2 = EnrichedEvent(
            **{**ev.__dict__, "ml_magnitude": 3.80, "is_aftershock": True}
        )
        db.upsert_enriched_event(ev2)

        conn = psycopg2.connect(postgres_dsn)
        try:
            with conn.cursor() as cur:
                cur.execute(
                    "SELECT ml_magnitude, is_aftershock FROM seismology.seismic_events "
                    "WHERE source_id = %s",
                    ("EMSC:db_upsert_test",),
                )
                row = cur.fetchone()
        finally:
            conn.close()

        assert row is not None
        assert float(row[0]) == pytest.approx(3.80, abs=0.01)
        assert row[1] is True


class TestCacheIntegration:
    def test_set_and_get_roundtrip(self, cache: Cache):
        now_ms = int(datetime.now(tz=timezone.utc).timestamp() * 1000)
        ev = EnrichedEvent(
            source_id="USGS:cache_roundtrip",
            source_network="USGS",
            event_time_ms=now_ms,
            latitude=37.0,
            longitude=36.0,
            depth_km=5.0,
            magnitude=4.0,
            magnitude_type="ML",
            region_name=None,
            quality_indicator="A",
            raw_payload="{}",
            ingested_at_ms=now_ms,
            pipeline_version="0.1.0",
            ml_magnitude=3.97,
            ml_magnitude_source="ml_depth_corrected",
            is_aftershock=False,
            mainshock_source_id=None,
            mainshock_magnitude=None,
            mainshock_distance_km=None,
            mainshock_time_delta_hours=None,
            estimated_felt_radius_km=10.0,
            estimated_intensity_mmi=4.5,
            enriched_at_ms=now_ms,
            analysis_version="0.1.0-test",
        )
        assert cache.set_event(ev, ttl_secs=300) is True
        cached = cache.get_event("USGS:cache_roundtrip")
        assert cached is not None
        assert cached["ml_magnitude"] == pytest.approx(3.97)
        assert cached["is_aftershock"] is False
