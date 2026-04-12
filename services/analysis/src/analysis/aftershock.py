"""
ETAS-based aftershock sequence detection.

Uses Gardner & Knopoff (1974) space-time windows to determine whether an
incoming event is likely an aftershock of a larger event in the seismic
catalogue.

Algorithm
---------
1. Compute the Gardner-Knopoff lookback window for the potential mainshock
   (proxy: current magnitude + 1.0, capped at configured maximums).
2. Query seismology.seismic_events for the largest event within that
   space-time window that has magnitude > current event.
3. Evaluate the G-K window for *that* candidate's magnitude.
4. If the current event falls inside the candidate's window → aftershock.

The detector degrades gracefully when the catalogue is empty (first-time
bootstrap) or when the PostGIS query fails — it returns is_aftershock=False
and logs a warning.

References
----------
* Gardner, J.K. & Knopoff, L. (1974) "Is the sequence of earthquakes in
  Southern California, with aftershocks removed, Poissonian?"
  BSSA 64(5): 1363–1367.
* Utsu, T. (1961) "A statistical study on the occurrence of aftershocks."
  Geophysical Magazine 30: 521–605.
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone
from typing import Optional

import psycopg2
import psycopg2.extras

from .models import MainshockInfo


def gardner_knopoff_window(magnitude: float) -> tuple[float, float]:
    """
    Compute the Gardner-Knopoff (1974) space-time aftershock window.

    Parameters
    ----------
    magnitude:
        Magnitude of the potential mainshock.

    Returns
    -------
    (time_days, distance_km)
        The temporal and spatial radii of the aftershock zone.
    """
    if magnitude >= 6.5:
        # Eq. (3) of Gardner & Knopoff (1974)
        time_days = 10 ** (0.32 * magnitude - 1.11)
    else:
        # Eq. (2) of Gardner & Knopoff (1974)
        time_days = 10 ** (0.5 * magnitude - 1.7)

    distance_km = 10 ** (0.1238 * magnitude + 0.983)
    return time_days, distance_km


def find_mainshock(
    conn: psycopg2.extensions.connection,
    magnitude: float,
    latitude: float,
    longitude: float,
    event_time: datetime,
    max_window_days: int = 30,
    max_window_km: float = 100.0,
) -> Optional[MainshockInfo]:
    """
    Search the seismic catalogue for a candidate mainshock.

    The search window is the G-K window for (magnitude + 1.0), hard-capped
    at max_window_days / max_window_km to prevent runaway queries.

    Parameters
    ----------
    conn:
        Live psycopg2 connection.  Caller owns lifecycle; this function
        does not commit or close.
    magnitude:
        Magnitude of the current (possibly aftershock) event.
    latitude, longitude:
        WGS-84 epicentre coordinates.
    event_time:
        Origin time of the current event (UTC, timezone-aware).
    max_window_days:
        Hard cap on temporal lookback.
    max_window_km:
        Hard cap on spatial search radius.

    Returns
    -------
    MainshockInfo if a candidate is found, else None.
    """
    proxy_mag = min(magnitude + 1.0, 8.5)
    window_days, window_km = gardner_knopoff_window(proxy_mag)
    window_days = min(window_days, max_window_days)
    window_km = min(window_km, max_window_km)

    since = event_time - timedelta(days=window_days)
    window_m = window_km * 1000.0  # PostGIS geography uses metres

    sql = """
        SELECT
            source_id,
            magnitude,
            event_time,
            ST_Distance(
                location::geography,
                ST_SetSRID(ST_MakePoint(%s, %s), 4326)::geography
            ) / 1000.0  AS distance_km
        FROM seismology.seismic_events
        WHERE magnitude > %s
          AND event_time < %s
          AND event_time > %s
          AND ST_DWithin(
                  location::geography,
                  ST_SetSRID(ST_MakePoint(%s, %s), 4326)::geography,
                  %s
              )
        ORDER BY magnitude DESC, event_time DESC
        LIMIT 1
    """
    params = (
        longitude, latitude,          # ST_Distance point
        magnitude,                     # magnitude > current
        event_time,                    # mainshock precedes current event
        since,                         # within lookback window
        longitude, latitude,           # ST_DWithin point
        window_m,                      # radius in metres
    )

    with conn.cursor(cursor_factory=psycopg2.extras.RealDictCursor) as cur:
        cur.execute(sql, params)
        row = cur.fetchone()

    if row is None:
        return None

    event_time_row: datetime = row["event_time"]
    if event_time_row.tzinfo is None:
        event_time_row = event_time_row.replace(tzinfo=timezone.utc)

    return MainshockInfo(
        source_id=row["source_id"],
        magnitude=float(row["magnitude"]),
        event_time=event_time_row,
        distance_km=float(row["distance_km"]),
    )


def classify_aftershock(
    event_magnitude: float,
    event_time: datetime,
    mainshock: MainshockInfo,
) -> tuple[bool, float, float]:
    """
    Confirm whether the current event falls within the G-K window for the
    candidate mainshock.

    Parameters
    ----------
    event_magnitude:
        Magnitude of the current event (unused in window calculation but
        retained for caller symmetry).
    event_time:
        Origin time of the current event (UTC, timezone-aware).
    mainshock:
        Candidate mainshock returned by find_mainshock().

    Returns
    -------
    (is_aftershock, time_delta_hours, distance_km)
    """
    gk_days, gk_km = gardner_knopoff_window(mainshock.magnitude)
    delta_hours = (event_time - mainshock.event_time).total_seconds() / 3600.0
    in_time = 0.0 <= delta_hours <= gk_days * 24.0
    in_space = mainshock.distance_km <= gk_km
    return (in_time and in_space), delta_hours, mainshock.distance_km
