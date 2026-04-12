"""Domain models for raw and enriched seismic events."""
from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Optional


# ─── Helpers ──────────────────────────────────────────────────────────────────

def _to_ms(v: int | datetime) -> int:
    """Coerce a fastavro timestamp-millis value (datetime or int) to epoch ms."""
    if isinstance(v, datetime):
        return int(v.timestamp() * 1000)
    return int(v)


def _from_ms(ms: int) -> datetime:
    return datetime.fromtimestamp(ms / 1000.0, tz=timezone.utc)


# ─── Raw event (earthquakes.raw consumer) ─────────────────────────────────────

@dataclass(frozen=True)
class RawEvent:
    """Deserialised record from the earthquakes.raw Avro topic."""

    source_id: str
    source_network: str
    event_time_ms: int          # epoch ms UTC
    latitude: float
    longitude: float
    depth_km: Optional[float]
    magnitude: float
    magnitude_type: str
    region_name: Optional[str]
    quality_indicator: str      # A | B | C | D
    raw_payload: str            # original source JSON, carried as string
    ingested_at_ms: int         # epoch ms UTC
    pipeline_version: str

    @property
    def event_time(self) -> datetime:
        return _from_ms(self.event_time_ms)

    @classmethod
    def from_avro_record(cls, record: dict[str, Any]) -> RawEvent:
        """
        Build a RawEvent from a dict decoded by fastavro.

        fastavro returns datetime objects for timestamp-millis logical types;
        _to_ms normalises both int and datetime inputs.
        """
        return cls(
            source_id=record["source_id"],
            source_network=record["source_network"],
            event_time_ms=_to_ms(record["event_time_ms"]),
            latitude=float(record["latitude"]),
            longitude=float(record["longitude"]),
            depth_km=record.get("depth_km"),
            magnitude=float(record["magnitude"]),
            magnitude_type=record["magnitude_type"],
            region_name=record.get("region_name"),
            quality_indicator=record["quality_indicator"],
            raw_payload=record["raw_payload"],
            ingested_at_ms=_to_ms(record["ingested_at_ms"]),
            pipeline_version=record["pipeline_version"],
        )


# ─── Aftershock detection intermediate ────────────────────────────────────────

@dataclass(frozen=True)
class MainshockInfo:
    """Candidate mainshock returned by the aftershock detector."""

    source_id: str
    magnitude: float
    event_time: datetime
    distance_km: float


# ─── Enriched event (earthquakes.enriched producer) ───────────────────────────

@dataclass(frozen=True)
class EnrichedEvent:
    """
    Fully enriched seismic event for the earthquakes.enriched topic.

    Carries all raw fields through unchanged plus analysis enrichment.
    """

    # ── Carried through from raw ───────────────────────────────────────────
    source_id: str
    source_network: str
    event_time_ms: int
    latitude: float
    longitude: float
    depth_km: Optional[float]
    magnitude: float
    magnitude_type: str
    region_name: Optional[str]
    quality_indicator: str
    raw_payload: str
    ingested_at_ms: int
    pipeline_version: str

    # ── Analysis enrichment ────────────────────────────────────────────────
    ml_magnitude: float
    ml_magnitude_source: str
    is_aftershock: bool
    mainshock_source_id: Optional[str]
    mainshock_magnitude: Optional[float]
    mainshock_distance_km: Optional[float]
    mainshock_time_delta_hours: Optional[float]
    estimated_felt_radius_km: float
    estimated_intensity_mmi: float
    enriched_at_ms: int
    analysis_version: str

    @property
    def event_time(self) -> datetime:
        return _from_ms(self.event_time_ms)

    @property
    def raw_payload_json(self) -> dict[str, Any]:
        return json.loads(self.raw_payload)

    def to_avro_record(self) -> dict[str, Any]:
        """
        Serialise to a dict for fastavro schemaless_writer.

        Timestamp fields must be datetime objects so fastavro applies the
        correct timestamp-millis encoding.
        """
        def _opt_float(v: Optional[float]) -> Optional[float]:
            return float(v) if v is not None else None

        return {
            "source_id": self.source_id,
            "source_network": self.source_network,
            "event_time_ms": _from_ms(self.event_time_ms),
            "latitude": self.latitude,
            "longitude": self.longitude,
            "depth_km": _opt_float(self.depth_km),
            "magnitude": self.magnitude,
            "magnitude_type": self.magnitude_type,
            "region_name": self.region_name,
            "quality_indicator": self.quality_indicator,
            "raw_payload": self.raw_payload,
            "ingested_at_ms": _from_ms(self.ingested_at_ms),
            "pipeline_version": self.pipeline_version,
            "ml_magnitude": self.ml_magnitude,
            "ml_magnitude_source": self.ml_magnitude_source,
            "is_aftershock": self.is_aftershock,
            "mainshock_source_id": self.mainshock_source_id,
            "mainshock_magnitude": _opt_float(self.mainshock_magnitude),
            "mainshock_distance_km": _opt_float(self.mainshock_distance_km),
            "mainshock_time_delta_hours": _opt_float(self.mainshock_time_delta_hours),
            "estimated_felt_radius_km": self.estimated_felt_radius_km,
            "estimated_intensity_mmi": self.estimated_intensity_mmi,
            "enriched_at_ms": _from_ms(self.enriched_at_ms),
            "analysis_version": self.analysis_version,
        }


# ─── Alert event (earthquakes.alerts producer) ────────────────────────────────

@dataclass(frozen=True)
class AlertEvent:
    """Alert record produced for M ≥ threshold seismic events."""

    source_id: str
    event_time_ms: int
    latitude: float
    longitude: float
    depth_km: Optional[float]
    magnitude: float
    ml_magnitude: float
    region_name: Optional[str]
    estimated_intensity_mmi: float
    estimated_felt_radius_km: float
    is_aftershock: bool
    alert_level: str            # "YELLOW" | "ORANGE" | "RED"
    triggered_at_ms: int

    def to_avro_record(self) -> dict[str, Any]:
        return {
            "source_id": self.source_id,
            "event_time_ms": _from_ms(self.event_time_ms),
            "latitude": self.latitude,
            "longitude": self.longitude,
            "depth_km": self.depth_km,
            "magnitude": self.magnitude,
            "ml_magnitude": self.ml_magnitude,
            "region_name": self.region_name,
            "estimated_intensity_mmi": self.estimated_intensity_mmi,
            "estimated_felt_radius_km": self.estimated_felt_radius_km,
            "is_aftershock": self.is_aftershock,
            "alert_level": self.alert_level,
            "triggered_at_ms": _from_ms(self.triggered_at_ms),
        }
