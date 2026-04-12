"""Redis cache for enriched seismic events."""
from __future__ import annotations

import json
import logging
from typing import Optional

import redis as redis_lib

from .models import EnrichedEvent

log = logging.getLogger(__name__)

_KEY_PREFIX = "analysis:event:"


def event_key(source_id: str) -> str:
    """Return the Redis key for a single enriched event."""
    return f"{_KEY_PREFIX}{source_id}"


class Cache:
    """Thin Redis wrapper for writing enriched event payloads."""

    def __init__(self, url: str) -> None:
        self._client = redis_lib.Redis.from_url(url, decode_responses=True)

    def set_event(self, event: EnrichedEvent, ttl_secs: int) -> bool:
        """
        Store a JSON-serialised enriched event with *ttl_secs* expiry.

        Returns True on success.  Redis failures are non-fatal; callers
        should count the failure and continue.
        """
        key = event_key(event.source_id)
        payload = {
            "source_id": event.source_id,
            "source_network": event.source_network,
            "event_time_ms": event.event_time_ms,
            "latitude": event.latitude,
            "longitude": event.longitude,
            "depth_km": event.depth_km,
            "magnitude": event.magnitude,
            "magnitude_type": event.magnitude_type,
            "region_name": event.region_name,
            "quality_indicator": event.quality_indicator,
            "ml_magnitude": event.ml_magnitude,
            "ml_magnitude_source": event.ml_magnitude_source,
            "is_aftershock": event.is_aftershock,
            "mainshock_source_id": event.mainshock_source_id,
            "estimated_felt_radius_km": event.estimated_felt_radius_km,
            "estimated_intensity_mmi": event.estimated_intensity_mmi,
            "enriched_at_ms": event.enriched_at_ms,
            "analysis_version": event.analysis_version,
        }
        try:
            self._client.setex(key, ttl_secs, json.dumps(payload))
            return True
        except redis_lib.RedisError as exc:
            log.warning("redis SET failed: key=%s error=%s", key, exc)
            return False

    def get_event(self, source_id: str) -> Optional[dict]:
        """
        Retrieve a cached enriched event dict, or None on miss/error.

        Used by tests; not called in the hot path.
        """
        try:
            raw = self._client.get(event_key(source_id))
            return json.loads(raw) if raw is not None else None
        except redis_lib.RedisError as exc:
            log.warning("redis GET failed: source_id=%s error=%s", source_id, exc)
            return None

    def ping(self) -> bool:
        """Return True if Redis is reachable."""
        try:
            return bool(self._client.ping())
        except redis_lib.RedisError:
            return False

    def close(self) -> None:
        self._client.close()
