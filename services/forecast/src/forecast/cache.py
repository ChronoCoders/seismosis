"""Redis cache for forecast results (GR analysis, etc.)."""
from __future__ import annotations

import json
from typing import Any

import redis
import structlog

log: structlog.stdlib.BoundLogger = structlog.get_logger(__name__)

_GR_KEY_PREFIX = "forecast:gr:"


class ForecastCache:
    """Thin Redis wrapper for caching forecast computation results.

    All methods are non-fatal: Redis errors are caught, logged at WARNING
    level, and the caller receives a sentinel value (None / no-op) so that
    the forecast pipeline continues without interruption when Redis is
    unavailable.
    """

    def __init__(self, redis_url: str) -> None:
        self._client: redis.Redis[str] = redis.Redis.from_url(
            redis_url, decode_responses=True
        )

    # ── GR analysis cache ────────────────────────────────────────────────────

    def set_gr_result(
        self,
        region_name: str,
        data: dict[str, Any],
        ttl_secs: int = 86400,
    ) -> None:
        """Cache a GR analysis result dict under *region_name*.

        The TTL defaults to 24 hours (86 400 s).  Redis errors are swallowed
        and logged at WARNING level — caching is non-fatal.
        """
        key = f"{_GR_KEY_PREFIX}{region_name}"
        try:
            self._client.setex(key, ttl_secs, json.dumps(data))
        except redis.exceptions.RedisError as exc:
            log.warning(
                "forecast_cache.set_gr_result.failed",
                region=region_name,
                error=str(exc),
            )

    def get_gr_result(self, region_name: str) -> dict[str, Any] | None:
        """Return the cached GR result for *region_name*, or None on miss/error."""
        key = f"{_GR_KEY_PREFIX}{region_name}"
        try:
            raw = self._client.get(key)
            if raw is None:
                return None
            return json.loads(raw)  # type: ignore[no-any-return]
        except redis.exceptions.RedisError as exc:
            log.warning(
                "forecast_cache.get_gr_result.failed",
                region=region_name,
                error=str(exc),
            )
            return None

    # ── Lifecycle ────────────────────────────────────────────────────────────

    def close(self) -> None:
        """Close the underlying Redis connection."""
        try:
            self._client.close()
        except redis.exceptions.RedisError as exc:
            log.warning("forecast_cache.close.failed", error=str(exc))
