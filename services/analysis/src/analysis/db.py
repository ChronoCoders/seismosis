"""
PostgreSQL writer for enriched seismic events.

The analysis service uses INSERT … ON CONFLICT (source_id) DO UPDATE so the
operation is idempotent regardless of whether the Rust storage service has
already written the raw row:

* INSERT path  — fires when analysis runs before storage.  All available
  fields (raw + enrichment) are written in one shot.
* UPDATE path  — fires when storage already owns the row.  Only enrichment
  columns are updated; raw columns written by storage are never overwritten.

The DB pool is thread-safe (ThreadedConnectionPool) because the Prometheus
metrics HTTP server runs in a daemon thread that may occasionally call
get_conn() for health checks.
"""
from __future__ import annotations

import logging
from datetime import datetime, timezone

import psycopg2
import psycopg2.extras
import psycopg2.pool

from .models import EnrichedEvent

log = logging.getLogger(__name__)


class Database:
    """Thread-safe PostgreSQL connection pool for the analysis service."""

    def __init__(self, url: str, min_conn: int = 2, max_conn: int = 10) -> None:
        self._pool = psycopg2.pool.ThreadedConnectionPool(min_conn, max_conn, url)

    def upsert_enriched_event(self, event: EnrichedEvent) -> None:
        """
        Upsert *event* into seismology.seismic_events.

        Raises psycopg2.Error on database failure (caller should count the
        error and continue — this write is non-fatal for the pipeline).
        """
        conn = self._pool.getconn()
        try:
            with conn.cursor() as cur:
                cur.execute(
                    """
                    INSERT INTO seismology.seismic_events (
                        source_id,       source_network,  event_time,
                        magnitude,       magnitude_type,  location,
                        depth_km,        region_name,     quality_indicator,
                        raw_payload,     processed_at,    pipeline_version,
                        ml_magnitude,    is_aftershock,   mainshock_source_id,
                        estimated_felt_radius_km,
                        estimated_intensity_mmi,
                        enriched_at,     analysis_version
                    )
                    VALUES (
                        %s, %s, %s,
                        %s, %s, ST_SetSRID(ST_MakePoint(%s, %s), 4326),
                        %s, %s, %s,
                        %s, %s, %s,
                        %s, %s, %s,
                        %s,
                        %s,
                        NOW(), %s
                    )
                    ON CONFLICT (source_id) DO UPDATE SET
                        ml_magnitude             = EXCLUDED.ml_magnitude,
                        is_aftershock            = EXCLUDED.is_aftershock,
                        mainshock_source_id      = EXCLUDED.mainshock_source_id,
                        estimated_felt_radius_km = EXCLUDED.estimated_felt_radius_km,
                        estimated_intensity_mmi  = EXCLUDED.estimated_intensity_mmi,
                        enriched_at              = NOW(),
                        analysis_version         = EXCLUDED.analysis_version
                    """,
                    (
                        # INSERT columns
                        event.source_id,
                        event.source_network,
                        event.event_time,
                        event.magnitude,
                        event.magnitude_type,
                        event.longitude,   # ST_MakePoint(lon, lat)
                        event.latitude,
                        event.depth_km,
                        event.region_name,
                        event.quality_indicator,
                        psycopg2.extras.Json(event.raw_payload_json),
                        datetime.fromtimestamp(
                            event.ingested_at_ms / 1000.0, tz=timezone.utc
                        ),
                        event.pipeline_version,
                        event.ml_magnitude,
                        event.is_aftershock,
                        event.mainshock_source_id,
                        event.estimated_felt_radius_km,
                        event.estimated_intensity_mmi,
                        event.analysis_version,
                    ),
                )
            conn.commit()
        except Exception:
            conn.rollback()
            raise
        finally:
            self._pool.putconn(conn)

    def get_conn(self) -> psycopg2.extensions.connection:
        """
        Check out a raw connection for callers that manage their own cursor
        (e.g. the aftershock detector).  Must be returned via put_conn().
        """
        return self._pool.getconn()

    def put_conn(self, conn: psycopg2.extensions.connection) -> None:
        """Return a connection checked out by get_conn() to the pool."""
        self._pool.putconn(conn)

    def close(self) -> None:
        """Close all connections and shut down the pool."""
        self._pool.closeall()
