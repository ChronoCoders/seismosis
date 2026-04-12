"""Unit tests for domain model construction and Avro record serialisation."""
from __future__ import annotations

from datetime import datetime, timezone

import pytest

from analysis.models import AlertEvent, EnrichedEvent, MainshockInfo, RawEvent


# ─── Fixtures ──────────────────────────────────────────────────────────────────

_RAW_DICT: dict = {
    "source_id": "USGS:us7000test",
    "source_network": "USGS",
    "event_time_ms": 1_700_000_000_000,
    "latitude": 38.5,
    "longitude": 27.3,
    "depth_km": 10.0,
    "magnitude": 4.2,
    "magnitude_type": "ML",
    "region_name": "Aegean Sea",
    "quality_indicator": "B",
    "raw_payload": '{"id":"us7000test","mag":4.2}',
    "ingested_at_ms": 1_700_000_001_000,
    "pipeline_version": "0.1.0",
}

_ENRICHED_KWARGS: dict = {
    **_RAW_DICT,
    "ml_magnitude": 4.18,
    "ml_magnitude_source": "ml_depth_corrected",
    "is_aftershock": False,
    "mainshock_source_id": None,
    "mainshock_magnitude": None,
    "mainshock_distance_km": None,
    "mainshock_time_delta_hours": None,
    "estimated_felt_radius_km": 45.0,
    "estimated_intensity_mmi": 5.2,
    "enriched_at_ms": 1_700_000_002_000,
    "analysis_version": "0.1.0",
}


# ─── RawEvent ──────────────────────────────────────────────────────────────────

class TestRawEvent:
    def test_from_avro_record_basic_fields(self):
        ev = RawEvent.from_avro_record(_RAW_DICT)
        assert ev.source_id == "USGS:us7000test"
        assert ev.magnitude == pytest.approx(4.2)
        assert ev.depth_km == pytest.approx(10.0)
        assert ev.quality_indicator == "B"

    def test_from_avro_record_null_depth(self):
        ev = RawEvent.from_avro_record({**_RAW_DICT, "depth_km": None})
        assert ev.depth_km is None

    def test_from_avro_record_null_region(self):
        ev = RawEvent.from_avro_record({**_RAW_DICT, "region_name": None})
        assert ev.region_name is None

    def test_from_avro_record_datetime_timestamps(self):
        """fastavro returns datetime objects for timestamp-millis; must coerce to int."""
        dt = datetime(2023, 11, 14, 22, 13, 20, tzinfo=timezone.utc)
        ev = RawEvent.from_avro_record({**_RAW_DICT, "event_time_ms": dt})
        assert isinstance(ev.event_time_ms, int)
        assert ev.event_time_ms == int(dt.timestamp() * 1000)

    def test_event_time_property_is_utc(self):
        ev = RawEvent.from_avro_record(_RAW_DICT)
        assert isinstance(ev.event_time, datetime)
        assert ev.event_time.tzinfo == timezone.utc

    def test_immutability(self):
        ev = RawEvent.from_avro_record(_RAW_DICT)
        with pytest.raises((AttributeError, TypeError)):
            ev.magnitude = 9.9  # type: ignore[misc]


# ─── EnrichedEvent ─────────────────────────────────────────────────────────────

class TestEnrichedEvent:
    def test_to_avro_record_timestamp_fields_are_datetime(self):
        ev = EnrichedEvent(**_ENRICHED_KWARGS)
        rec = ev.to_avro_record()
        assert isinstance(rec["event_time_ms"], datetime)
        assert isinstance(rec["ingested_at_ms"], datetime)
        assert isinstance(rec["enriched_at_ms"], datetime)

    def test_to_avro_record_enrichment_fields(self):
        ev = EnrichedEvent(**_ENRICHED_KWARGS)
        rec = ev.to_avro_record()
        assert rec["ml_magnitude"] == pytest.approx(4.18)
        assert rec["is_aftershock"] is False
        assert rec["ml_magnitude_source"] == "ml_depth_corrected"

    def test_to_avro_record_null_optionals(self):
        ev = EnrichedEvent(
            **{**_ENRICHED_KWARGS, "depth_km": None, "region_name": None}
        )
        rec = ev.to_avro_record()
        assert rec["depth_km"] is None
        assert rec["region_name"] is None

    def test_to_avro_record_aftershock_fields(self):
        ev = EnrichedEvent(
            **{
                **_ENRICHED_KWARGS,
                "is_aftershock": True,
                "mainshock_source_id": "USGS:us6000main",
                "mainshock_magnitude": 6.1,
                "mainshock_distance_km": 35.5,
                "mainshock_time_delta_hours": 2.3,
            }
        )
        rec = ev.to_avro_record()
        assert rec["is_aftershock"] is True
        assert rec["mainshock_source_id"] == "USGS:us6000main"
        assert rec["mainshock_magnitude"] == pytest.approx(6.1)

    def test_raw_payload_json_property(self):
        ev = EnrichedEvent(**_ENRICHED_KWARGS)
        assert ev.raw_payload_json == {"id": "us7000test", "mag": 4.2}

    def test_event_time_property(self):
        ev = EnrichedEvent(**_ENRICHED_KWARGS)
        assert ev.event_time.tzinfo == timezone.utc


# ─── AlertEvent ────────────────────────────────────────────────────────────────

class TestAlertEvent:
    def _make_alert(self, **kwargs) -> AlertEvent:
        defaults = dict(
            source_id="USGS:us7000big",
            event_time_ms=1_700_000_000_000,
            latitude=38.5,
            longitude=27.3,
            depth_km=15.0,
            magnitude=5.8,
            ml_magnitude=5.72,
            region_name="Turkey",
            estimated_intensity_mmi=7.1,
            estimated_felt_radius_km=180.0,
            is_aftershock=False,
            alert_level="YELLOW",
            triggered_at_ms=1_700_000_001_000,
        )
        defaults.update(kwargs)
        return AlertEvent(**defaults)

    def test_to_avro_record_basic(self):
        rec = self._make_alert().to_avro_record()
        assert rec["alert_level"] == "YELLOW"
        assert rec["ml_magnitude"] == pytest.approx(5.72)
        assert isinstance(rec["event_time_ms"], datetime)

    def test_alert_level_variants(self):
        for level in ("YELLOW", "ORANGE", "RED"):
            rec = self._make_alert(alert_level=level).to_avro_record()
            assert rec["alert_level"] == level

    def test_null_depth_passes_through(self):
        rec = self._make_alert(depth_km=None).to_avro_record()
        assert rec["depth_km"] is None


# ─── MainshockInfo ─────────────────────────────────────────────────────────────

class TestMainshockInfo:
    def test_construction(self):
        dt = datetime.now(tz=timezone.utc)
        ms = MainshockInfo(
            source_id="USGS:us6000big",
            magnitude=6.5,
            event_time=dt,
            distance_km=42.0,
        )
        assert ms.magnitude == pytest.approx(6.5)
        assert ms.distance_km == pytest.approx(42.0)
