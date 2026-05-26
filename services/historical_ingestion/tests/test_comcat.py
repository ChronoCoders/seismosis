"""
Unit tests for historical_ingestion.comcat

Test 1 — pagination stops when API returns 0 features
Test 2 — source_id is correctly formed from a feature's id field
Test 3 — checkpoint time (epoch-ms property) is correctly parsed to UTC datetime
"""
from __future__ import annotations

import asyncio
from datetime import datetime, timezone
from typing import Any
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from historical_ingestion.comcat import (
    ComCatEvent,
    _parse_feature,
    iter_turkey_events,
)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_feature(
    feature_id: str = "us7000abcd",
    mag: float = 4.2,
    place: str = "10 km E of Ankara, Turkey",
    time_ms: int = 1_700_000_000_000,
    mag_type: str = "mw",
    net: str = "us",
    lon: float = 33.0,
    lat: float = 40.0,
    depth_km: float = 10.0,
) -> dict[str, Any]:
    """Return a minimal GeoJSON Feature dict matching the USGS ComCat schema."""
    return {
        "type": "Feature",
        "id": feature_id,
        "properties": {
            "mag": mag,
            "place": place,
            "time": time_ms,
            "magType": mag_type,
            "net": net,
        },
        "geometry": {
            "type": "Point",
            "coordinates": [lon, lat, depth_km],
        },
    }


def _make_geojson_response(features: list[dict[str, Any]]) -> dict[str, Any]:
    """Wrap a list of features into a GeoJSON FeatureCollection dict."""
    return {
        "type": "FeatureCollection",
        "features": features,
        "metadata": {"count": len(features)},
    }


# ---------------------------------------------------------------------------
# Test 1: pagination stops on empty response
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_pagination_stops_on_empty_features() -> None:
    """
    When the USGS API returns a FeatureCollection with 0 features the
    async generator must stop immediately and yield nothing.
    """
    start = datetime(2020, 1, 1, tzinfo=timezone.utc)
    end = datetime(2020, 12, 31, tzinfo=timezone.utc)

    empty_response = _make_geojson_response([])

    mock_resp = AsyncMock()
    mock_resp.raise_for_status = MagicMock()
    mock_resp.json = AsyncMock(return_value=empty_response)
    mock_resp.__aenter__ = AsyncMock(return_value=mock_resp)
    mock_resp.__aexit__ = AsyncMock(return_value=False)

    mock_session = MagicMock()
    mock_session.get = MagicMock(return_value=mock_resp)

    pages: list[list[ComCatEvent]] = []
    async for page in iter_turkey_events(mock_session, start, end):
        pages.append(page)

    assert pages == [], (
        "Expected no pages when API returns 0 features, "
        f"but got {len(pages)} page(s)"
    )


# ---------------------------------------------------------------------------
# Test 2: source_id is correctly formed
# ---------------------------------------------------------------------------

def test_source_id_correctly_formed() -> None:
    """
    The source_id of a parsed ComCatEvent must be 'usgs:<feature_id>',
    where <feature_id> is exactly the 'id' field of the GeoJSON Feature.
    """
    feature_id = "us7000abcd"
    feature = _make_feature(feature_id=feature_id)
    event = _parse_feature(feature)

    assert event.source_id == f"usgs:{feature_id}", (
        f"Expected source_id='usgs:{feature_id}', got '{event.source_id}'"
    )


# ---------------------------------------------------------------------------
# Test 3: checkpoint time is correctly parsed from epoch-ms
# ---------------------------------------------------------------------------

def test_checkpoint_time_correctly_parsed() -> None:
    """
    The 'time' property in USGS GeoJSON is epoch milliseconds (UTC).
    _parse_feature must convert it to a timezone-aware UTC datetime with
    millisecond precision.

    Reference: 1_700_000_000_000 ms = 2023-11-14T22:13:20+00:00
    """
    time_ms = 1_700_000_000_000
    expected_dt = datetime.fromtimestamp(time_ms / 1000.0, tz=timezone.utc)

    feature = _make_feature(time_ms=time_ms)
    event = _parse_feature(feature)

    assert event.event_time == expected_dt, (
        f"Expected event_time={expected_dt.isoformat()!r}, "
        f"got {event.event_time.isoformat()!r}"
    )
    assert event.event_time.tzinfo is not None, (
        "event_time must be timezone-aware (UTC)"
    )
    assert event.event_time.tzinfo == timezone.utc, (
        "event_time must carry the UTC timezone, "
        f"got tzinfo={event.event_time.tzinfo!r}"
    )
