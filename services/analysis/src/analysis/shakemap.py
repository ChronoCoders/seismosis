"""
ShakeMap enrichment for the Seismosis analysis service.

Fetches the USGS ShakeMap intensity GeoJSON for M≥3.5 USGS events and
writes the raw FeatureCollection to seismology.seismic_events.shakemap.

Fetch endpoint
--------------
    https://earthquake.usgs.gov/earthquakes/eventpage/{usgs_event_id}/shakemap/intensity.geojson

Filtering
---------
    * magnitude >= 3.5
    * source_network == 'USGS'  (case-insensitive)
    * usgs_event_id extracted from source_id: "usgs:us7000xyz" → "us7000xyz"

Retry policy
------------
    Up to 3 attempts with linear backoff: 90 s, 180 s, 270 s.
    ShakeMap data is typically processed 5–15 minutes after an event, so a
    small number of retries with moderate delays prevents noise from events
    that never have ShakeMap products without burning CPU in a tight loop.
    After 3 failed attempts the function returns None (non-fatal for pipeline).

HTTP
----
    Uses urllib.request / http.client only — no extra dependencies.
"""
from __future__ import annotations

import json
import time
import urllib.error
import urllib.request
from typing import Any, Optional

import psycopg2.extensions
import psycopg2.extras
import structlog

log: structlog.stdlib.BoundLogger = structlog.get_logger(__name__)

_SHAKEMAP_URL_TEMPLATE = (
    "https://earthquake.usgs.gov/earthquakes/eventpage"
    "/{usgs_event_id}/shakemap/intensity.geojson"
)

_MIN_MAGNITUDE: float = 3.5
_MAX_RETRIES: int = 3
_BACKOFF_BASE_SECS: float = 90.0
_HTTP_TIMEOUT_SECS: float = 30.0


# ---------------------------------------------------------------------------
# Public helpers
# ---------------------------------------------------------------------------


def should_fetch_shakemap(magnitude: float, source_network: str) -> bool:
    """
    Return True when ShakeMap fetching is applicable for this event.

    Parameters
    ----------
    magnitude:
        Calibrated ML magnitude.
    source_network:
        Source network string from the event (e.g. "USGS", "EMSC").
    """
    return magnitude >= _MIN_MAGNITUDE and source_network.upper() == "USGS"


def parse_usgs_event_id(source_id: str) -> Optional[str]:
    """
    Extract the USGS event ID from a canonical source_id.

    Expects the format ``usgs:{event_id}`` (case-insensitive prefix).
    Returns None if the source_id does not match the expected format.

    Examples
    --------
    >>> parse_usgs_event_id("usgs:us7000xyz")
    'us7000xyz'
    >>> parse_usgs_event_id("emsc:20240101_0000001")
    None
    """
    lower = source_id.lower()
    if not lower.startswith("usgs:"):
        return None
    event_id = source_id[5:]  # strip the "usgs:" prefix (preserve original case)
    if not event_id:
        return None
    return event_id


def fetch_shakemap(
    usgs_event_id: str,
    *,
    _sleep_fn: Any = time.sleep,  # injectable for tests
) -> Optional[dict[str, Any]]:
    """
    Fetch the USGS ShakeMap intensity GeoJSON for *usgs_event_id*.

    Retries up to 3 times with 90 s / 180 s / 270 s backoff.  Returns the
    parsed FeatureCollection dict on success, None after all retries fail.

    Parameters
    ----------
    usgs_event_id:
        Bare USGS event identifier (e.g. "us7000xyz") — without "usgs:" prefix.
    _sleep_fn:
        Callable used to sleep between retries.  Overridable in tests to avoid
        real delays.

    Returns
    -------
    dict[str, Any] | None
        Parsed GeoJSON FeatureCollection, or None when unavailable.
    """
    url = _SHAKEMAP_URL_TEMPLATE.format(usgs_event_id=usgs_event_id)
    logger = log.bind(usgs_event_id=usgs_event_id, url=url)

    for attempt in range(1, _MAX_RETRIES + 1):
        try:
            req = urllib.request.Request(
                url,
                headers={"Accept": "application/json"},
            )
            with urllib.request.urlopen(req, timeout=_HTTP_TIMEOUT_SECS) as resp:  # noqa: S310
                raw = resp.read()

            geojson: dict[str, Any] = json.loads(raw)
            logger.info(
                "shakemap_fetched",
                attempt=attempt,
                feature_count=len(geojson.get("features", [])),
            )
            return geojson

        except urllib.error.HTTPError as exc:
            if exc.code == 404:
                # ShakeMap not yet available or not produced for this event.
                logger.info(
                    "shakemap_not_found",
                    attempt=attempt,
                    http_status=exc.code,
                )
            else:
                logger.warning(
                    "shakemap_http_error",
                    attempt=attempt,
                    http_status=exc.code,
                    error=str(exc),
                )
        except (urllib.error.URLError, OSError, json.JSONDecodeError, ValueError) as exc:
            logger.warning(
                "shakemap_fetch_error",
                attempt=attempt,
                error=str(exc),
            )

        if attempt < _MAX_RETRIES:
            delay = _BACKOFF_BASE_SECS * attempt
            logger.debug("shakemap_retry_backoff", delay_secs=delay, next_attempt=attempt + 1)
            _sleep_fn(delay)

    logger.warning("shakemap_all_retries_exhausted", max_retries=_MAX_RETRIES)
    return None


def store_shakemap(
    conn: psycopg2.extensions.connection,
    source_id: str,
    geojson: dict[str, Any],
) -> None:
    """
    Persist *geojson* into seismology.seismic_events.shakemap for *source_id*.

    Uses a targeted UPDATE so this write never interferes with INSERT/upsert
    operations from the storage service or the main enrichment path.  The
    UPDATE is a no-op (zero rows affected) if the row doesn't exist yet;
    callers should treat that as non-fatal and rely on eventual consistency —
    the storage service will write the row shortly after ingestion.

    Parameters
    ----------
    conn:
        Active psycopg2 connection.  The caller is responsible for lifecycle.
    source_id:
        Canonical deduplication key for the event (e.g. "usgs:us7000xyz").
    geojson:
        Raw GeoJSON FeatureCollection dict from USGS.

    Raises
    ------
    psycopg2.Error
        Propagated to caller, which should treat this as non-fatal.
    """
    with conn.cursor() as cur:
        cur.execute(
            """
            UPDATE seismology.seismic_events
               SET shakemap = %s
             WHERE source_id = %s
            """,
            (psycopg2.extras.Json(geojson), source_id),
        )
    conn.commit()

    log.debug("shakemap_stored", source_id=source_id)


def enrich_with_shakemap(
    conn: psycopg2.extensions.connection,
    source_id: str,
    magnitude: float,
    source_network: str,
    *,
    _sleep_fn: Any = time.sleep,  # injectable for tests
) -> bool:
    """
    High-level entry point: fetch and store ShakeMap data when applicable.

    Checks eligibility, fetches the GeoJSON (with retries), then writes it
    to the database.  All failures are logged and swallowed — ShakeMap
    enrichment is non-fatal for the processing pipeline.

    Parameters
    ----------
    conn:
        Checked-out psycopg2 connection.  Caller retains ownership.
    source_id:
        Canonical event identifier (e.g. "usgs:us7000xyz").
    magnitude:
        Calibrated ML magnitude used for the M≥3.5 gate.
    source_network:
        Source network string (e.g. "USGS").
    _sleep_fn:
        Overridable sleep callable for unit tests.

    Returns
    -------
    bool
        True if ShakeMap data was successfully fetched and stored, False
        otherwise (ineligible, not found, or any error).
    """
    if not should_fetch_shakemap(magnitude, source_network):
        return False

    usgs_event_id = parse_usgs_event_id(source_id)
    if usgs_event_id is None:
        log.warning(
            "shakemap_bad_source_id",
            source_id=source_id,
            reason="could not extract USGS event ID from source_id",
        )
        return False

    geojson = fetch_shakemap(usgs_event_id, _sleep_fn=_sleep_fn)
    if geojson is None:
        return False

    try:
        store_shakemap(conn, source_id, geojson)
    except Exception as exc:
        log.error(
            "shakemap_store_error",
            source_id=source_id,
            error=str(exc),
        )
        try:
            conn.rollback()
        except Exception:  # noqa: BLE001
            pass
        return False

    return True
