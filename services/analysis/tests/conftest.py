"""
pytest fixtures for the analysis service.

Integration tests use testcontainers to spin up real Redpanda (Kafka +
Schema Registry), PostgreSQL with PostGIS, and Redis instances.

Fixtures are session-scoped for speed: containers are started once per test
run and shared across all integration tests.
"""
from __future__ import annotations

import time
from typing import Generator

import psycopg2
import pytest

# ─── Inline SQL — minimal schema required by integration tests ─────────────────

_INIT_SQL = """
CREATE EXTENSION IF NOT EXISTS postgis;
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

CREATE SCHEMA IF NOT EXISTS seismology;

CREATE TABLE IF NOT EXISTS seismology.seismic_events (
    id                       UUID         PRIMARY KEY DEFAULT uuid_generate_v4(),
    event_time               TIMESTAMPTZ  NOT NULL,
    magnitude                NUMERIC(4,2) NOT NULL,
    magnitude_type           VARCHAR(10)  NOT NULL DEFAULT 'ML',
    location                 GEOMETRY(POINT, 4326) NOT NULL,
    depth_km                 NUMERIC(8,3),
    source_network           VARCHAR(50),
    source_id                VARCHAR(255) NOT NULL UNIQUE,
    region_name              VARCHAR(512),
    quality_indicator        CHAR(1)      NOT NULL DEFAULT 'D',
    raw_payload              JSONB        NOT NULL DEFAULT '{}',
    pipeline_version         VARCHAR(50),
    processed_at             TIMESTAMPTZ,
    ml_magnitude             NUMERIC(4,2),
    is_aftershock            BOOLEAN,
    mainshock_source_id      VARCHAR(255),
    estimated_felt_radius_km NUMERIC(8,2),
    estimated_intensity_mmi  NUMERIC(4,2),
    enriched_at              TIMESTAMPTZ,
    analysis_version         VARCHAR(50),
    created_at               TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
    updated_at               TIMESTAMPTZ  NOT NULL DEFAULT NOW()
);
"""


def _wait_for_postgres(dsn: str, retries: int = 30, delay: float = 1.0) -> None:
    for _ in range(retries):
        try:
            conn = psycopg2.connect(dsn)
            conn.close()
            return
        except psycopg2.OperationalError:
            time.sleep(delay)
    raise RuntimeError(f"PostgreSQL not ready after {retries * delay:.0f}s")


# ─── PostgreSQL ────────────────────────────────────────────────────────────────

@pytest.fixture(scope="session")
def postgres_dsn() -> Generator[str, None, None]:
    """
    Session-scoped PostGIS container with the analysis schema applied.

    Skipped if testcontainers.postgres is unavailable (no Docker).
    """
    testcontainers_pg = pytest.importorskip("testcontainers.postgres")
    from testcontainers.postgres import PostgresContainer

    with PostgresContainer(
        image="postgis/postgis:16-3.4",
        username="seismosis",
        password="seismosis",
        dbname="seismosis",
    ) as pg:
        dsn = pg.get_connection_url().replace("postgresql+psycopg2://", "postgresql://")
        _wait_for_postgres(dsn)
        conn = psycopg2.connect(dsn)
        try:
            with conn.cursor() as cur:
                cur.execute(_INIT_SQL)
            conn.commit()
        finally:
            conn.close()
        yield dsn


# ─── Redis ─────────────────────────────────────────────────────────────────────

@pytest.fixture(scope="session")
def redis_url() -> Generator[str, None, None]:
    """
    Session-scoped Redis container.  Returns a connection URL.

    Skipped if testcontainers.redis is unavailable.
    """
    pytest.importorskip("testcontainers.redis")
    from testcontainers.redis import RedisContainer

    with RedisContainer(image="redis:7.2-alpine") as rc:
        host = rc.get_container_host_ip()
        port = rc.get_exposed_port(6379)
        yield f"redis://{host}:{port}"


# ─── Redpanda (Kafka + Schema Registry) ────────────────────────────────────────

@pytest.fixture(scope="session")
def redpanda_brokers_and_sr() -> Generator[tuple[str, str], None, None]:
    """
    Session-scoped Redpanda container.

    Returns (bootstrap_brokers, schema_registry_url).
    Skipped if testcontainers.redpanda is unavailable.
    """
    pytest.importorskip("testcontainers.redpanda")
    from testcontainers.redpanda import RedpandaContainer

    with RedpandaContainer() as rp:
        brokers = rp.get_bootstrap_server()
        sr_url = rp.get_schema_registry_url()
        # Allow time for Redpanda to finish internal initialisation.
        time.sleep(3)
        yield brokers, sr_url
