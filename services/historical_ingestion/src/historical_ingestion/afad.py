"""
AFAD (Disaster and Emergency Management Authority) historical earthquake catalog.

Fetches Turkey region seismicity from:
    https://deprem.afad.gov.tr/apiv2/event/filter

The endpoint accepts a GET request with query parameters:
    start   — window start, "YYYY-MM-DD HH:MM:SS" (UTC)
    end     — window end,   "YYYY-MM-DD HH:MM:SS" (UTC)
    minmag  — minimum magnitude
    maxlat  — bounding box north edge
    minlat  — bounding box south edge
    maxlon  — bounding box east edge
    minlon  — bounding box west edge

Response: JSON array of event objects with fields:
    eventID, date (ISO UTC), latitude, longitude, depth,
    type (magnitude type), magnitude, location, province

Pagination strategy
-------------------
AFAD provides no cursor-based paging.  The client fetches in fixed 14-day
windows and advances the cursor to max(event_time) + 1 s after each non-empty
window.  Empty windows advance the cursor by the full window width so the
loop always terminates.

Rate limit: 2 requests/second (0.5 s sleep between requests).

Timezone
--------
The AFAD API returns event times in UTC (ISO format without timezone suffix).
"""
from __future__ import annotations

import asyncio
import json
import logging
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any, AsyncIterator

import aiohttp
import structlog

log: structlog.stdlib.BoundLogger = structlog.get_logger(__name__)

_AFAD_URL = "https://deprem.afad.gov.tr/apiv2/event/filter"

# Turkey region bounding box (covers mainland Turkey and adjacent seismogenic zones)
_MIN_LAT = 35.0
_MAX_LAT = 43.0
_MIN_LON = 25.0
_MAX_LON = 45.0
_MIN_MAG = 1.5

_WINDOW_DAYS = 14          # fetch in 14-day chunks
_REQUEST_INTERVAL_SECS = 0.5   # 2 req/s rate limit
_HTTP_TIMEOUT_SECS = 60.0

_MAX_RETRIES = 3
_BACKOFF_BASE_SECS = 2.0

_DATE_FORMATS = (
    "%Y-%m-%dT%H:%M:%S",
    "%Y.%m.%d %H:%M:%S",
    "%Y-%m-%d %H:%M:%S",
)

_HEADERS = {
    "User-Agent": "Seismosis/1.0 (earthquake research; contact altug@bytus.io)",
    "Accept": "application/json",
}


# ---------------------------------------------------------------------------
# Public event dataclass
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class AfadEvent:
    """A single parsed seismic event from the AFAD catalog."""

    source_id: str
    source_network: str
    event_time: datetime          # UTC
    latitude: float
    longitude: float
    depth_km: float
    magnitude: float
    magnitude_type: str
    region_name: str


# ---------------------------------------------------------------------------
# Internal helpers
# ---------------------------------------------------------------------------


def _parse_afad_date(date_str: str) -> datetime:
    """Parse an AFAD date string into a UTC-aware datetime."""
    cleaned = date_str.strip()
    for fmt in _DATE_FORMATS:
        try:
            dt = datetime.strptime(cleaned, fmt)
            return dt.replace(tzinfo=timezone.utc)
        except ValueError:
            continue
    raise ValueError(f"Cannot parse AFAD date: {date_str!r}")


def _parse_afad_event(item: dict[str, Any]) -> AfadEvent | None:
    """
    Parse a single AFAD event dict into an AfadEvent.

    Returns None and logs a debug message if any required field is missing or
    cannot be coerced — the caller skips None entries.
    """
    try:
        event_id = str(item.get("eventID") or "").strip()
        if not event_id:
            log.debug("afad_skip_missing_id", item_keys=list(item.keys()))
            return None

        date_raw = str(item.get("date") or "").strip()
        if not date_raw:
            log.debug("afad_skip_missing_date", event_id=event_id)
            return None

        event_time = _parse_afad_date(date_raw)

        lat_raw = item.get("latitude")
        lon_raw = item.get("longitude")
        if lat_raw is None or lon_raw is None:
            log.debug("afad_skip_missing_coords", event_id=event_id)
            return None

        latitude = float(str(lat_raw))
        longitude = float(str(lon_raw))
        depth_km = float(str(item.get("depth") or 0.0))
        magnitude = float(str(item.get("magnitude") or 0.0))

        # AFAD uses "type" for the magnitude scale (ML, Mw, mb, etc.)
        mag_type = str(item.get("type") or "ML").strip() or "ML"

        # "location" is the most readable description; fall back to province
        region_name = str(
            item.get("location") or item.get("province") or ""
        ).strip()

        return AfadEvent(
            source_id=f"afad:{event_id}",
            source_network="AFAD",
            event_time=event_time,
            latitude=latitude,
            longitude=longitude,
            depth_km=depth_km,
            magnitude=magnitude,
            magnitude_type=mag_type,
            region_name=region_name,
        )
    except (ValueError, TypeError, KeyError) as exc:
        log.warning(
            "afad_parse_error",
            error=str(exc),
            event_id=str(item.get("eventID") or "?"),
        )
        return None


async def _fetch_window(
    session: aiohttp.ClientSession,
    params: dict[str, str],
) -> list[dict[str, Any]]:
    """
    GET the AFAD API for one time window with exponential back-off retry.

    Uses SSL verification disabled — the AFAD server's certificate chain
    causes connection resets under strict verification from Docker containers.

    Returns the list of raw event dicts (may be empty).
    Raises aiohttp.ClientError after all retries are exhausted.
    """
    last_exc: Exception = RuntimeError("No attempts made")
    for attempt in range(_MAX_RETRIES):
        try:
            async with session.get(
                _AFAD_URL,
                params=params,
                headers=_HEADERS,
                ssl=False,  # AFAD cert causes connection resets under strict TLS
                timeout=aiohttp.ClientTimeout(total=_HTTP_TIMEOUT_SECS),
                allow_redirects=True,
            ) as resp:
                resp.raise_for_status()
                text = await resp.text()
                if not text.strip() or text.strip() in ("null", "[]"):
                    return []
                data: Any = json.loads(text)
                if isinstance(data, list):
                    return data  # type: ignore[return-value]
                # Some API gateway versions wrap the array
                if isinstance(data, dict):
                    for key in ("result", "data", "events", "items"):
                        if isinstance(data.get(key), list):
                            return data[key]  # type: ignore[return-value]
                log.warning(
                    "afad_unexpected_response_shape",
                    shape=type(data).__name__,
                )
                return []
        except (aiohttp.ClientError, asyncio.TimeoutError, json.JSONDecodeError) as exc:
            last_exc = exc
            delay = _BACKOFF_BASE_SECS * (2 ** attempt)
            log.warning(
                "afad_fetch_error",
                attempt=attempt + 1,
                max_retries=_MAX_RETRIES,
                delay_secs=delay,
                error=str(exc),
                window_start=params.get("start"),
            )
            await asyncio.sleep(delay)

    raise last_exc


# ---------------------------------------------------------------------------
# Public async generator
# ---------------------------------------------------------------------------


async def iter_afad_events(
    session: aiohttp.ClientSession,
    start_time: datetime,
    end_time: datetime,
) -> AsyncIterator[list[AfadEvent]]:
    """
    Async generator that pages through the AFAD catalog in 14-day windows.

    Yields one list of AfadEvent per non-empty window.  Empty windows advance
    the cursor by the full window width so the loop always terminates.

    Rate-limited to 2 requests/second via asyncio.sleep between windows.
    """
    window = timedelta(days=_WINDOW_DAYS)
    cursor = start_time

    while cursor < end_time:
        window_end = min(cursor + window, end_time)

        params: dict[str, str] = {
            "start":  cursor.strftime("%Y-%m-%d %H:%M:%S"),
            "end":    window_end.strftime("%Y-%m-%d %H:%M:%S"),
            "minmag": str(_MIN_MAG),
            "maxlat": str(_MAX_LAT),
            "minlat": str(_MIN_LAT),
            "maxlon": str(_MAX_LON),
            "minlon": str(_MIN_LON),
        }

        log.debug(
            "afad_fetching_window",
            start=params["start"],
            end=params["end"],
        )

        raw_items = await _fetch_window(session, params)
        events = [e for item in raw_items if (e := _parse_afad_event(item)) is not None]

        if events:
            yield events
            latest = max(e.event_time for e in events)
            cursor = latest.replace(microsecond=0) + timedelta(seconds=1)
            log.debug(
                "afad_window_done",
                count=len(events),
                next_cursor=cursor.isoformat(),
            )
        else:
            # No events in this window — advance past it to avoid stalling
            cursor = window_end + timedelta(seconds=1)
            log.debug("afad_empty_window", advancing_to=cursor.isoformat())

        await asyncio.sleep(_REQUEST_INTERVAL_SECS)
