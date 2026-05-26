"""
PostgreSQL writer for historical seismic events.

Uses asyncpg for async bulk upserts into seismology.historical_events.

Checkpoint resume
-----------------
On startup, reads the last ingested event_time from
seismology.ingest_checkpoints for a given job_name.
If the row exists, ingestion resumes from that timestamp; otherwise it starts
from the configured start date (2016-01-01).

After each batch upsert the checkpoint row is updated with the latest
event_time so the job is restartable without re-processing already-ingested
events.

Table contract
--------------
  seismology.historical_events  — target event table
  seismology.ingest_checkpoints — checkpoint tracking table
    columns: job_name TEXT PK, last_event_time TIMESTAMPTZ, events_ingested BIGINT
"""
from __future__ import annotations

from datetime import datetime, timezone
from typing import Protocol

import asyncpg
import structlog

log: structlog.stdlib.BoundLogger = structlog.get_logger(__name__)


class HistoricalEventRow(Protocol):
    """Structural protocol satisfied by ComCatEvent, AfadEvent, and any future source."""

    source_id: str
    source_network: str
    event_time: datetime
    latitude: float
    longitude: float
    depth_km: float
    magnitude: float
    magnitude_type: str
    region_name: str


_UPSERT_SQL = """
INSERT INTO seismology.historical_events (
    source_id,
    source_network,
    event_time,
    latitude,
    longitude,
    depth_km,
    magnitude,
    magnitude_type,
    region_name,
    location
)
VALUES (
    $1, $2, $3, $4, $5, $6, $7, $8, $9,
    ST_SetSRID(ST_MakePoint($10, $11), 4326)::geography
)
ON CONFLICT (source_id) DO UPDATE SET
    magnitude    = EXCLUDED.magnitude,
    region_name  = EXCLUDED.region_name,
    ingested_at  = now()
"""

_CHECKPOINT_SELECT = """
SELECT last_event_time
FROM seismology.ingest_checkpoints
WHERE job_name = $1
"""

_CHECKPOINT_UPSERT = """
INSERT INTO seismology.ingest_checkpoints (job_name, last_event_time, events_ingested)
VALUES ($1, $2, $3)
ON CONFLICT (job_name) DO UPDATE SET
    last_event_time  = EXCLUDED.last_event_time,
    events_ingested  = EXCLUDED.events_ingested,
    updated_at       = NOW()
"""

_BATCH_SIZE = 1000


class Database:
    """Async PostgreSQL connection pool for the historical ingestion service."""

    def __init__(self, pool: asyncpg.Pool) -> None:  # type: ignore[type-arg]
        self._pool = pool

    @classmethod
    async def connect(cls, dsn: str) -> "Database":
        """Create and return a Database backed by a new asyncpg pool."""
        pool: asyncpg.Pool = await asyncpg.create_pool(dsn=dsn, min_size=2, max_size=10)  # type: ignore[type-arg]
        return cls(pool)

    async def close(self) -> None:
        """Close all connections in the pool."""
        await self._pool.close()

    async def read_checkpoint(self, job_name: str) -> datetime | None:
        """
        Return the last successfully ingested event_time for *job_name*,
        or None if no checkpoint exists yet.
        """
        async with self._pool.acquire() as conn:
            row: asyncpg.Record | None = await conn.fetchrow(_CHECKPOINT_SELECT, job_name)  # type: ignore[type-arg]
        if row is None:
            return None
        ts: datetime = row["last_event_time"]
        # asyncpg returns timezone-aware datetimes for TIMESTAMPTZ columns
        if ts.tzinfo is None:
            ts = ts.replace(tzinfo=timezone.utc)
        return ts

    async def upsert_batch(
        self,
        events: list[HistoricalEventRow],
        total_ingested: int,
        job_name: str,
    ) -> int:
        """
        Upsert *events* into seismology.historical_events in batches of 1 000
        rows and update the *job_name* checkpoint with the latest event_time.

        Returns the number of rows submitted (not the number of rows actually
        inserted — ON CONFLICT rows count towards this total too).
        """
        if not events:
            return 0

        # Build parameter tuples: 11 columns per row
        # ($1 source_id, $2 source_network, $3 event_time,
        #  $4 lat, $5 lon, $6 depth_km, $7 mag, $8 mag_type,
        #  $9 region_name, $10 lon, $11 lat)
        rows: list[tuple[
            str, str, datetime,
            float, float, float,
            float, str, str,
            float, float,
        ]] = [
            (
                e.source_id,
                e.source_network,
                e.event_time,
                e.latitude,
                e.longitude,
                e.depth_km,
                e.magnitude,
                e.magnitude_type,
                e.region_name,
                e.longitude,   # ST_MakePoint($10=lon, $11=lat)
                e.latitude,
            )
            for e in events
        ]

        latest_event_time = max(e.event_time for e in events)
        new_total = total_ingested + len(events)

        async with self._pool.acquire() as conn:
            async with conn.transaction():
                # executemany in asyncpg batches the rows efficiently
                for chunk_start in range(0, len(rows), _BATCH_SIZE):
                    chunk = rows[chunk_start : chunk_start + _BATCH_SIZE]
                    await conn.executemany(_UPSERT_SQL, chunk)

                await conn.execute(_CHECKPOINT_UPSERT, job_name, latest_event_time, new_total)

        log.debug(
            "batch_upserted",
            job_name=job_name,
            count=len(events),
            latest_event_time=latest_event_time.isoformat(),
            total_ingested=new_total,
        )
        return len(events)
