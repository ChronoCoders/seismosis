"""
USGS ComCat API client for historical earthquake data.

Fetches the USGS FDSN event catalog for the Turkey region (bbox 33–45°N, 22–48°E)
in GeoJSON pages of up to 20 000 events, advancing the starttime window by 1 ms
after each page until 0 features are returned.

Rate limiting: at most 2 requests per second (asyncio.sleep between pages).
HTTP errors are retried with exponential back-off (3 retries, delays 2/4/8 s).
"""
from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import Any, AsyncIterator

import aiohttp
import structlog

log: structlog.stdlib.BoundLogger = structlog.get_logger(__name__)

_BASE_URL = "https://earthquake.usgs.gov/fdsnws/event/1/query"

# Turkey region bounding box
_MIN_LAT = 33.0
_MAX_LAT = 45.0
_MIN_LON = 22.0
_MAX_LON = 48.0
_MIN_MAG = 1.5
_PAGE_LIMIT = 20_000

# Rate limit: 2 requests/second → sleep 0.5 s between pages
_REQUEST_INTERVAL_SECS = 0.5

# Exponential back-off for HTTP errors
_MAX_RETRIES = 3
_BACKOFF_BASE_SECS = 2.0

# Map USGS network codes → canonical source_network names
_NETWORK_MAP: dict[str, str] = {
    "us": "USGS",
    "ak": "USGS-AK",
    "ci": "USGS-CI",
    "nc": "USGS-NC",
    "uu": "USGS-UU",
    "uw": "USGS-UW",
    "nn": "USGS-NN",
    "hv": "USGS-HV",
    "pr": "USGS-PR",
    "se": "USGS-SE",
    "ge": "GFZ",
    "emsc": "EMSC",
    "koeri": "KOERI",
    "afad": "AFAD",
}


@dataclass(frozen=True)
class ComCatEvent:
    """A single parsed seismic event from the USGS ComCat GeoJSON response."""

    source_id: str
    source_network: str
    event_time: datetime          # UTC
    latitude: float
    longitude: float
    depth_km: float
    magnitude: float
    magnitude_type: str
    region_name: str


def _map_network(net: str | None) -> str:
    """Map a USGS network code to a canonical source_network string."""
    if net is None:
        return "USGS"
    return _NETWORK_MAP.get(net.lower(), net.upper())


def _parse_feature(feature: dict[str, Any]) -> ComCatEvent:
    """
    Parse a single GeoJSON Feature into a ComCatEvent.

    The USGS GeoJSON schema guarantees:
      feature["id"]                    → str  (e.g. "us7000abcd")
      feature["properties"]["mag"]     → float | None
      feature["properties"]["place"]   → str | None
      feature["properties"]["time"]    → int  (epoch milliseconds)
      feature["properties"]["magType"] → str | None
      feature["properties"]["net"]     → str | None
      feature["geometry"]["coordinates"] → [lon, lat, depth_km]
    """
    props: dict[str, Any] = feature["properties"]
    coords: list[float] = feature["geometry"]["coordinates"]

    event_time_ms: int = int(props["time"])
    event_time = datetime.fromtimestamp(event_time_ms / 1000.0, tz=timezone.utc)

    source_id = f"usgs:{feature['id']}"
    source_network = _map_network(props.get("net"))
    longitude = float(coords[0])
    latitude = float(coords[1])
    depth_km = float(coords[2]) if coords[2] is not None else 0.0
    magnitude = float(props["mag"]) if props.get("mag") is not None else 0.0
    magnitude_type = str(props.get("magType") or "unknown")
    region_name = str(props.get("place") or "")

    return ComCatEvent(
        source_id=source_id,
        source_network=source_network,
        event_time=event_time,
        latitude=latitude,
        longitude=longitude,
        depth_km=depth_km,
        magnitude=magnitude,
        magnitude_type=magnitude_type,
        region_name=region_name,
    )


async def _fetch_page(
    session: aiohttp.ClientSession,
    start_time: datetime,
    end_time: datetime,
) -> list[dict[str, Any]]:
    """
    Fetch a single page from the USGS ComCat API with exponential back-off.

    Returns the list of GeoJSON Feature dicts (may be empty).
    Raises aiohttp.ClientError after all retries are exhausted.
    """
    params: dict[str, str] = {
        "format": "geojson",
        "minlatitude": str(_MIN_LAT),
        "maxlatitude": str(_MAX_LAT),
        "minlongitude": str(_MIN_LON),
        "maxlongitude": str(_MAX_LON),
        "minmagnitude": str(_MIN_MAG),
        "orderby": "time-asc",
        "limit": str(_PAGE_LIMIT),
        "starttime": start_time.strftime("%Y-%m-%dT%H:%M:%S.") + f"{start_time.microsecond // 1000:03d}",
        "endtime": end_time.strftime("%Y-%m-%dT%H:%M:%S.") + f"{end_time.microsecond // 1000:03d}",
    }

    last_exc: Exception = RuntimeError("No attempts made")
    for attempt in range(_MAX_RETRIES):
        try:
            async with session.get(_BASE_URL, params=params, timeout=aiohttp.ClientTimeout(total=60)) as resp:
                resp.raise_for_status()
                data: dict[str, Any] = await resp.json(content_type=None)
                features: list[dict[str, Any]] = data.get("features", [])
                return features
        except (aiohttp.ClientError, asyncio.TimeoutError) as exc:
            last_exc = exc
            delay = _BACKOFF_BASE_SECS * (2 ** attempt)
            log.warning(
                "comcat_fetch_error",
                attempt=attempt + 1,
                max_retries=_MAX_RETRIES,
                delay_secs=delay,
                error=str(exc),
                start_time=start_time.isoformat(),
            )
            await asyncio.sleep(delay)

    raise last_exc


async def iter_turkey_events(
    session: aiohttp.ClientSession,
    start_time: datetime,
    end_time: datetime,
) -> AsyncIterator[list[ComCatEvent]]:
    """
    Async generator that paginates through the USGS ComCat catalog.

    Yields one list of ComCatEvent per API page until the API returns 0 features.
    Advances the query window by 1 ms after each non-empty page so the next page
    starts strictly after the last returned event.

    The caller is responsible for tracking the cursor (last event time) for
    checkpoint resumption — pass the checkpoint time as *start_time*.

    Rate-limited to 2 requests/second via asyncio.sleep(_REQUEST_INTERVAL_SECS).
    """
    cursor = start_time

    while cursor < end_time:
        log.debug("comcat_fetching_page", start_time=cursor.isoformat(), end_time=end_time.isoformat())

        raw_features = await _fetch_page(session, cursor, end_time)

        if not raw_features:
            log.info("comcat_pagination_complete", final_cursor=cursor.isoformat())
            return

        events = [_parse_feature(f) for f in raw_features]

        yield events

        # Advance cursor to 1 ms after the latest event time in this page
        latest_time = max(e.event_time for e in events)
        cursor = latest_time + timedelta(milliseconds=1)

        log.debug(
            "comcat_page_done",
            page_size=len(events),
            next_cursor=cursor.isoformat(),
        )

        # Rate limit: at most 2 requests/second
        await asyncio.sleep(_REQUEST_INTERVAL_SECS)
