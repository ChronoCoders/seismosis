"""asyncpg-based database layer for the forecast service."""

from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Optional

import asyncpg  # type: ignore[import-untyped]


@dataclass(frozen=True)
class CatalogEvent:
    source_id: str
    event_time: datetime
    latitude: float
    longitude: float
    depth_km: float
    magnitude: float
    magnitude_type: str
    region_name: str


def _ensure_utc(dt: datetime) -> datetime:
    """Return *dt* as a timezone-aware UTC datetime."""
    if dt.tzinfo is None:
        return dt.replace(tzinfo=timezone.utc)
    return dt


def _row_to_catalog_event(row: Any) -> CatalogEvent:  # type: ignore[type-arg]
    return CatalogEvent(
        source_id=str(row["source_id"]),
        event_time=_ensure_utc(row["event_time"]),
        latitude=float(row["latitude"]),
        longitude=float(row["longitude"]),
        depth_km=float(row["depth_km"]),
        magnitude=float(row["magnitude"]),
        magnitude_type=str(row["magnitude_type"]),
        region_name=str(row["region_name"]) if row["region_name"] is not None else "",
    )


class Database:
    """Thin wrapper around an asyncpg connection pool."""

    def __init__(self, pool: asyncpg.Pool) -> None:  # type: ignore[type-arg]
        self._pool: asyncpg.Pool = pool  # type: ignore[type-arg]

    # ------------------------------------------------------------------
    # Lifecycle
    # ------------------------------------------------------------------

    @classmethod
    async def connect(cls, dsn: str) -> "Database":
        pool: asyncpg.Pool = await asyncpg.create_pool(dsn=dsn, min_size=2, max_size=10)  # type: ignore[type-arg]
        return cls(pool)

    async def close(self) -> None:
        await self._pool.close()

    # ------------------------------------------------------------------
    # Read helpers
    # ------------------------------------------------------------------

    async def get_catalog(
        self,
        min_lat: float,
        max_lat: float,
        min_lon: float,
        max_lon: float,
        min_mag: float,
        start_time: datetime,
        end_time: datetime,
        limit: int = 50_000,
    ) -> list[CatalogEvent]:
        query = """
            SELECT source_id, event_time, latitude, longitude,
                   depth_km, magnitude, magnitude_type, region_name
            FROM seismology.historical_events
            WHERE latitude  BETWEEN $1 AND $2
              AND longitude BETWEEN $3 AND $4
              AND magnitude >= $5
              AND event_time BETWEEN $6 AND $7
            ORDER BY event_time ASC
            LIMIT $8
        """
        async with self._pool.acquire() as conn:
            rows: list[Any] = await conn.fetch(  # type: ignore[type-arg]
                query,
                min_lat, max_lat,
                min_lon, max_lon,
                min_mag,
                start_time, end_time,
                limit,
            )
        return [_row_to_catalog_event(r) for r in rows]

    async def get_recent_mainshocks(
        self,
        min_mag: float,
        lookback_days: int,
    ) -> list[CatalogEvent]:
        query = """
            SELECT source_id, event_time,
                   ST_Y(location::geometry) AS latitude,
                   ST_X(location::geometry) AS longitude,
                   depth_km, magnitude, magnitude_type, region_name
            FROM seismology.seismic_events
            WHERE magnitude >= $1
              AND event_time >= NOW() - ($2 || ' days')::interval
            ORDER BY event_time DESC
        """
        async with self._pool.acquire() as conn:
            rows: list[Any] = await conn.fetch(query, min_mag, str(lookback_days))  # type: ignore[type-arg]
        return [_row_to_catalog_event(r) for r in rows]

    async def get_nearby_catalog(
        self,
        center_lon: float,
        center_lat: float,
        radius_m: float,
        min_mag: float,
        start_time: datetime,
        end_time: datetime,
    ) -> list[CatalogEvent]:
        query = """
            SELECT source_id, event_time, latitude, longitude,
                   depth_km, magnitude, magnitude_type, region_name
            FROM seismology.historical_events
            WHERE ST_DWithin(
                      location,
                      ST_SetSRID(ST_MakePoint($1, $2), 4326)::geography,
                      $3
                  )
              AND magnitude >= $4
              AND event_time BETWEEN $5 AND $6
            ORDER BY event_time ASC
        """
        async with self._pool.acquire() as conn:
            rows: list[Any] = await conn.fetch(  # type: ignore[type-arg]
                query,
                center_lon, center_lat,
                radius_m,
                min_mag,
                start_time, end_time,
            )
        return [_row_to_catalog_event(r) for r in rows]

    async def count_historical_events(self) -> int:
        query = "SELECT COUNT(*) FROM seismology.historical_events"
        async with self._pool.acquire() as conn:
            row: Any = await conn.fetchrow(query)  # type: ignore[type-arg]
        return int(row[0]) if row else 0

    async def get_b_value_for_location(
        self, lat: float, lon: float
    ) -> float | None:
        """Return the most recent b_value from gr_analysis for the 0.5° grid cell containing (lat, lon).

        Returns None if no matching row exists.
        """
        query = """
            SELECT b_value
            FROM seismology.gr_analysis
            WHERE grid_cell IS NOT NULL
              AND ST_Contains(
                      grid_cell::geometry,
                      ST_SetSRID(ST_MakePoint($2, $1), 4326)
                  )
            ORDER BY computed_at DESC
            LIMIT 1
        """
        async with self._pool.acquire() as conn:
            row: Any = await conn.fetchrow(query, lat, lon)  # type: ignore[type-arg]
        if row is None:
            return None
        return float(row["b_value"])

    async def get_nearby_recent_events(
        self,
        lat: float,
        lon: float,
        radius_km: float,
        hours: int,
        limit: int = 100,
    ) -> list[CatalogEvent]:
        """Return events within radius_km and the last `hours` hours from seismology.seismic_events."""
        query = """
            SELECT source_id, event_time,
                   ST_Y(location::geometry) AS latitude,
                   ST_X(location::geometry) AS longitude,
                   depth_km, magnitude, magnitude_type, region_name
            FROM seismology.seismic_events
            WHERE ST_DWithin(
                      location,
                      ST_SetSRID(ST_MakePoint($2, $1), 4326)::geography,
                      $3
                  )
              AND event_time >= NOW() - ($4 || ' hours')::interval
            ORDER BY event_time DESC
            LIMIT $5
        """
        async with self._pool.acquire() as conn:
            rows: list[Any] = await conn.fetch(  # type: ignore[type-arg]
                query,
                lat, lon,
                radius_km * 1000.0,
                str(hours),
                limit,
            )
        return [_row_to_catalog_event(r) for r in rows]

    async def get_unclassified_events(self, limit: int = 500) -> list[CatalogEvent]:
        query = """
            SELECT source_id, event_time,
                   ST_Y(location::geometry) AS latitude,
                   ST_X(location::geometry) AS longitude,
                   depth_km, magnitude, magnitude_type, region_name
            FROM seismology.seismic_events
            WHERE event_class IS NULL
            ORDER BY event_time DESC
            LIMIT $1
        """
        async with self._pool.acquire() as conn:
            rows: list[Any] = await conn.fetch(query, limit)  # type: ignore[type-arg]
        return [_row_to_catalog_event(r) for r in rows]

    # ------------------------------------------------------------------
    # Write helpers
    # ------------------------------------------------------------------

    async def upsert_etas_forecast(
        self,
        mainshock_source_id: str,
        horizon_days: int,
        min_magnitude: float,
        expected_count: float,
        p_at_least_one: float,
        p_exceedance_json: str,
        daily_rates_json: str,
        params_snapshot_json: str,
        model_version: str,
        spatial_heatmap_json: str | None = None,
    ) -> None:
        if spatial_heatmap_json is not None:
            query = """
                INSERT INTO seismology.etas_forecasts (
                    mainshock_source_id,
                    horizon_days,
                    min_magnitude,
                    expected_count,
                    p_at_least_one,
                    p_exceedance,
                    daily_rates,
                    params_snapshot,
                    model_version,
                    spatial_heatmap,
                    computed_at
                ) VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8::jsonb, $9, $10::jsonb, NOW())
            """
            async with self._pool.acquire() as conn:
                await conn.execute(
                    query,
                    mainshock_source_id,
                    horizon_days,
                    min_magnitude,
                    expected_count,
                    p_at_least_one,
                    p_exceedance_json,
                    daily_rates_json,
                    params_snapshot_json,
                    model_version,
                    spatial_heatmap_json,
                )
        else:
            query = """
                INSERT INTO seismology.etas_forecasts (
                    mainshock_source_id,
                    horizon_days,
                    min_magnitude,
                    expected_count,
                    p_at_least_one,
                    p_exceedance,
                    daily_rates,
                    params_snapshot,
                    model_version,
                    spatial_heatmap,
                    computed_at
                ) VALUES ($1, $2, $3, $4, $5, $6::jsonb, $7::jsonb, $8::jsonb, $9, NULL, NOW())
            """
            async with self._pool.acquire() as conn:
                await conn.execute(
                    query,
                    mainshock_source_id,
                    horizon_days,
                    min_magnitude,
                    expected_count,
                    p_at_least_one,
                    p_exceedance_json,
                    daily_rates_json,
                    params_snapshot_json,
                    model_version,
                )

    async def upsert_gr_analysis(
        self,
        region_name: str,
        grid_cell_wkt: Optional[str],
        b_value: float,
        b_std: float,
        a_value: float,
        mc: float,
        n_events: int,
        catalog_start: datetime,
        catalog_end: datetime,
        model_version: str,
    ) -> None:
        if grid_cell_wkt is not None:
            query = """
                INSERT INTO seismology.gr_analysis (
                    region_name,
                    grid_cell,
                    b_value,
                    b_std,
                    a_value,
                    mc,
                    n_events,
                    catalog_start,
                    catalog_end,
                    model_version,
                    computed_at
                ) VALUES (
                    $1,
                    ST_GeomFromText($2, 4326)::geography,
                    $3, $4, $5, $6, $7, $8, $9, $10, NOW()
                )
            """
            async with self._pool.acquire() as conn:
                await conn.execute(
                    query,
                    region_name,
                    grid_cell_wkt,
                    b_value, b_std, a_value, mc,
                    n_events,
                    catalog_start, catalog_end,
                    model_version,
                )
        else:
            query = """
                INSERT INTO seismology.gr_analysis (
                    region_name,
                    grid_cell,
                    b_value,
                    b_std,
                    a_value,
                    mc,
                    n_events,
                    catalog_start,
                    catalog_end,
                    model_version,
                    computed_at
                ) VALUES (
                    $1, NULL,
                    $2, $3, $4, $5, $6, $7, $8, $9, NOW()
                )
            """
            async with self._pool.acquire() as conn:
                await conn.execute(
                    query,
                    region_name,
                    b_value, b_std, a_value, mc,
                    n_events,
                    catalog_start, catalog_end,
                    model_version,
                )

    async def update_event_classifications(
        self,
        classifications: list[tuple[str, str, float, str]],
    ) -> None:
        """Write event_class, class_confidence, and class_probabilities to seismic_events.

        Each tuple is (source_id, event_class, class_confidence, class_probabilities_json).
        """
        if not classifications:
            return
        query = """
            UPDATE seismology.seismic_events
               SET event_class          = $2,
                   class_confidence     = $3,
                   class_probabilities  = $4::jsonb
             WHERE source_id = $1
        """
        async with self._pool.acquire() as conn:
            await conn.executemany(query, classifications)

    async def upsert_model_registry(
        self,
        model_type: str,
        version: str,
        trained_at: datetime,
        n_train: int,
        metrics_json: str,
        artifact_path: str,
        is_active: bool,
    ) -> None:
        query = """
            INSERT INTO seismology.model_registry (
                model_type,
                version,
                trained_at,
                n_train,
                metrics,
                artifact_path,
                is_active
            ) VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7)
            ON CONFLICT (model_type, version) DO UPDATE
                SET metrics      = EXCLUDED.metrics,
                    is_active    = EXCLUDED.is_active,
                    trained_at   = EXCLUDED.trained_at
        """
        async with self._pool.acquire() as conn:
            await conn.execute(
                query,
                model_type,
                version,
                trained_at,
                n_train,
                metrics_json,
                artifact_path,
                is_active,
            )

    async def deactivate_model_type(self, model_type: str) -> None:
        query = """
            UPDATE seismology.model_registry
               SET is_active = FALSE
             WHERE model_type = $1
        """
        async with self._pool.acquire() as conn:
            await conn.execute(query, model_type)
