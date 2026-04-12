-- ============================================================
-- 02_schema.sql — Core Database Schema
-- ============================================================
-- Run order: second (02_), after 01_extensions.sql
-- All tables use TIMESTAMPTZ (UTC-aware) for temporal columns.
-- All geometry columns use SRID 4326 (WGS-84 lon/lat).
-- Primary keys: UUID (uuid_generate_v4()).

-- ─── Schemas ─────────────────────────────────────────────────
CREATE SCHEMA IF NOT EXISTS seismology;   -- Core seismic data
CREATE SCHEMA IF NOT EXISTS monitoring;   -- Pipeline audit trail
CREATE SCHEMA IF NOT EXISTS staging;      -- DLQ replay buffer

COMMENT ON SCHEMA seismology  IS 'Core seismic event and station data';
COMMENT ON SCHEMA monitoring  IS 'Pipeline health and audit metrics';
COMMENT ON SCHEMA staging     IS 'Staging area for DLQ replay and data correction';

-- ─── seismology.stations ─────────────────────────────────────
-- Seismic monitoring stations. Populated at startup and updated
-- via station-heartbeats topic. Used for event association and
-- distance-based filtering.
CREATE TABLE seismology.stations (
    id                  UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    network_code        VARCHAR(10) NOT NULL,
    station_code        VARCHAR(10) NOT NULL,
    channel_code        VARCHAR(10) NOT NULL DEFAULT '???',
    location_code       VARCHAR(10) NOT NULL DEFAULT '00',

    -- WGS-84 point geometry (lon, lat)
    location            GEOMETRY(POINT, 4326) NOT NULL,
    elevation_m         NUMERIC(8, 2),

    -- Operational status
    installed_at        TIMESTAMPTZ,
    decommissioned_at   TIMESTAMPTZ,
    last_heartbeat_at   TIMESTAMPTZ,
    is_active           BOOLEAN     NOT NULL DEFAULT TRUE,

    -- SEED/FDSN metadata (flexible JSONB for Phase 2 compliance)
    metadata            JSONB       NOT NULL DEFAULT '{}',

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT stations_network_station_channel_location_unique
        UNIQUE (network_code, station_code, channel_code, location_code)
);

COMMENT ON TABLE  seismology.stations IS 'Seismic monitoring station registry';
COMMENT ON COLUMN seismology.stations.network_code  IS 'FDSN network code (e.g., IU, HV, CI)';
COMMENT ON COLUMN seismology.stations.station_code  IS 'Station identifier within the network';
COMMENT ON COLUMN seismology.stations.channel_code  IS 'Channel code (e.g., BHZ, HHN, EHE)';
COMMENT ON COLUMN seismology.stations.location_code IS 'Location code within station (e.g., 00, 10)';
COMMENT ON COLUMN seismology.stations.location       IS 'Station position (WGS-84 lon/lat)';
COMMENT ON COLUMN seismology.stations.elevation_m    IS 'Station elevation above sea level in meters';

CREATE INDEX idx_stations_location_gist
    ON seismology.stations USING GIST (location);

CREATE INDEX idx_stations_network_code
    ON seismology.stations (network_code);

CREATE INDEX idx_stations_is_active
    ON seismology.stations (is_active)
    WHERE is_active = TRUE;

CREATE INDEX idx_stations_last_heartbeat
    ON seismology.stations (last_heartbeat_at DESC NULLS LAST)
    WHERE is_active = TRUE;

-- BRIN index on created_at — efficient for time-ordered append workloads
CREATE INDEX idx_stations_created_at_brin
    ON seismology.stations USING BRIN (created_at)
    WITH (pages_per_range = 32);

-- ─── seismology.seismic_events ───────────────────────────────
-- Processed seismic events written by the analysis service.
-- Partitioning by event_time month is deferred to Phase 2
-- (requires TimescaleDB or declarative partitioning migration).
CREATE TABLE seismology.seismic_events (
    id                  UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),

    -- Temporal
    event_time          TIMESTAMPTZ NOT NULL,

    -- Magnitude
    magnitude           NUMERIC(4, 2) NOT NULL
                            CONSTRAINT magnitude_range CHECK (magnitude BETWEEN -2.0 AND 10.0),
    magnitude_type      VARCHAR(10) NOT NULL DEFAULT 'ML',

    -- Spatial (WGS-84 point; depth stored separately as Z is lossy in 2D)
    location            GEOMETRY(POINT, 4326) NOT NULL,
    depth_km            NUMERIC(8, 3)
                            CONSTRAINT depth_range CHECK (depth_km IS NULL OR depth_km BETWEEN -5 AND 800),

    -- Source provenance
    source_network      VARCHAR(50),
    source_id           VARCHAR(255) NOT NULL,   -- Deduplication key from upstream source
    region_name         VARCHAR(512),

    -- Quality
    -- A = reviewed, B = estimated, C = preliminary, D = not reviewed
    quality_indicator   CHAR(1)     NOT NULL DEFAULT 'D'
                            CONSTRAINT quality_valid CHECK (quality_indicator IN ('A', 'B', 'C', 'D')),
    horizontal_error_km NUMERIC(8, 3),
    depth_error_km      NUMERIC(8, 3),
    magnitude_error     NUMERIC(4, 3),
    phase_count         SMALLINT,

    -- Raw payload preserved for reprocessing
    raw_payload         JSONB       NOT NULL DEFAULT '{}',

    -- Pipeline metadata
    pipeline_version    VARCHAR(50),
    processed_at        TIMESTAMPTZ,

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT seismic_events_source_id_unique UNIQUE (source_id)
);

COMMENT ON TABLE  seismology.seismic_events IS 'Processed seismic events from the analysis pipeline';
COMMENT ON COLUMN seismology.seismic_events.source_id       IS 'Canonical deduplication key from the upstream source. Format: {network}:{event_id}';
COMMENT ON COLUMN seismology.seismic_events.location        IS 'Epicenter as WGS-84 point (lon, lat). Depth stored in depth_km.';
COMMENT ON COLUMN seismology.seismic_events.quality_indicator IS 'Event quality: A=reviewed, B=estimated, C=preliminary, D=not reviewed';
COMMENT ON COLUMN seismology.seismic_events.raw_payload     IS 'Original ingested payload for audit and reprocessing';

-- Spatial index on epicenter (most frequent query pattern)
CREATE INDEX idx_seismic_events_location_gist
    ON seismology.seismic_events USING GIST (location);

-- BRIN index on event_time — very efficient for time-ordered append workloads
CREATE INDEX idx_seismic_events_event_time_brin
    ON seismology.seismic_events USING BRIN (event_time)
    WITH (pages_per_range = 32);

-- Composite index for time-range + magnitude queries (most common API filter)
CREATE INDEX idx_seismic_events_time_magnitude
    ON seismology.seismic_events (event_time DESC, magnitude DESC);

-- Index for magnitude-only queries (alert thresholds, statistics)
CREATE INDEX idx_seismic_events_magnitude
    ON seismology.seismic_events (magnitude DESC);

-- Index for quality-filtered queries
CREATE INDEX idx_seismic_events_quality
    ON seismology.seismic_events (quality_indicator)
    WHERE quality_indicator IN ('A', 'B');

-- Index for source network filtering
CREATE INDEX idx_seismic_events_source_network
    ON seismology.seismic_events (source_network, event_time DESC);

-- ─── seismology.event_station_associations ───────────────────
-- Links processed events to the stations that detected them.
-- Populated by the analysis service from phase arrival data.
CREATE TABLE seismology.event_station_associations (
    id                  UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    event_id            UUID        NOT NULL REFERENCES seismology.seismic_events(id) ON DELETE CASCADE,
    station_id          UUID        NOT NULL REFERENCES seismology.stations(id) ON DELETE RESTRICT,

    -- Arrival data
    p_arrival_time      TIMESTAMPTZ,
    s_arrival_time      TIMESTAMPTZ,
    distance_km         NUMERIC(10, 3),
    azimuth_deg         NUMERIC(6, 2),
    time_residual_s     NUMERIC(8, 4),

    -- Peak ground motion at this station
    peak_ground_velocity_ms   NUMERIC(15, 12),
    peak_ground_acceleration  NUMERIC(15, 12),

    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT event_station_unique UNIQUE (event_id, station_id)
);

CREATE INDEX idx_esa_event_id ON seismology.event_station_associations (event_id);
CREATE INDEX idx_esa_station_id ON seismology.event_station_associations (station_id);

-- ─── monitoring.pipeline_metrics ──────────────────────────────
-- Immutable audit log of significant pipeline state changes.
-- Written by all services; never updated or deleted.
CREATE TABLE monitoring.pipeline_metrics (
    id                  BIGSERIAL   PRIMARY KEY,
    recorded_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    service_name        VARCHAR(100) NOT NULL,
    event_type          VARCHAR(100) NOT NULL,
    severity            VARCHAR(20)  NOT NULL DEFAULT 'INFO'
                            CONSTRAINT severity_valid CHECK (severity IN ('DEBUG', 'INFO', 'WARN', 'ERROR', 'CRITICAL')),
    message             TEXT,
    payload             JSONB       NOT NULL DEFAULT '{}',
    source_id           VARCHAR(255),   -- seismic_events.source_id if event-related
    kafka_topic         VARCHAR(255),
    kafka_partition     SMALLINT,
    kafka_offset        BIGINT
);

COMMENT ON TABLE monitoring.pipeline_metrics IS 'Immutable audit log of pipeline state transitions and errors';

CREATE INDEX idx_pipeline_metrics_recorded_at_brin
    ON monitoring.pipeline_metrics USING BRIN (recorded_at);

CREATE INDEX idx_pipeline_metrics_service_severity
    ON monitoring.pipeline_metrics (service_name, severity, recorded_at DESC);

CREATE INDEX idx_pipeline_metrics_source_id
    ON monitoring.pipeline_metrics (source_id)
    WHERE source_id IS NOT NULL;

-- ─── staging.raw_events ──────────────────────────────────────
-- Temporary holding table for DLQ replay.
-- Events here failed initial processing and are awaiting manual
-- review and correction before re-ingestion.
CREATE TABLE staging.raw_events (
    id                  UUID        PRIMARY KEY DEFAULT uuid_generate_v4(),
    received_at         TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    failure_reason      VARCHAR(100) NOT NULL,
    failure_detail      TEXT,
    kafka_topic         VARCHAR(255) NOT NULL,
    kafka_partition     SMALLINT    NOT NULL,
    kafka_offset        BIGINT      NOT NULL,
    kafka_timestamp     TIMESTAMPTZ,
    raw_payload         JSONB       NOT NULL,
    replay_attempts     SMALLINT    NOT NULL DEFAULT 0,
    last_replay_at      TIMESTAMPTZ,
    resolved_at         TIMESTAMPTZ,
    resolved_by         VARCHAR(100),
    resolution_notes    TEXT,

    CONSTRAINT staging_raw_events_kafka_unique
        UNIQUE (kafka_topic, kafka_partition, kafka_offset)
);

COMMENT ON TABLE staging.raw_events IS 'DLQ event buffer. Events here require manual review before replay.';

CREATE INDEX idx_staging_raw_events_failure_reason
    ON staging.raw_events (failure_reason, received_at DESC);

CREATE INDEX idx_staging_raw_events_unresolved
    ON staging.raw_events (received_at DESC)
    WHERE resolved_at IS NULL;

-- BRIN index on received_at — efficient for time-ordered append workloads
CREATE INDEX idx_staging_raw_events_received_at_brin
    ON staging.raw_events USING BRIN (received_at);

-- ─── updated_at auto-maintenance ─────────────────────────────
CREATE OR REPLACE FUNCTION seismology.set_updated_at()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

CREATE TRIGGER trg_stations_updated_at
    BEFORE UPDATE ON seismology.stations
    FOR EACH ROW EXECUTE FUNCTION seismology.set_updated_at();

CREATE TRIGGER trg_seismic_events_updated_at
    BEFORE UPDATE ON seismology.seismic_events
    FOR EACH ROW EXECUTE FUNCTION seismology.set_updated_at();

-- ─── Read-only role for reporting/exporter ───────────────────
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'seismosis_reader') THEN
        CREATE ROLE seismosis_reader NOLOGIN;
    END IF;
END;
$$;

GRANT USAGE ON SCHEMA seismology, monitoring, staging TO seismosis_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA seismology TO seismosis_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA monitoring TO seismosis_reader;
GRANT SELECT ON ALL TABLES IN SCHEMA staging TO seismosis_reader;

-- Allow future tables to inherit the reader grant
ALTER DEFAULT PRIVILEGES IN SCHEMA seismology
    GRANT SELECT ON TABLES TO seismosis_reader;
ALTER DEFAULT PRIVILEGES IN SCHEMA monitoring
    GRANT SELECT ON TABLES TO seismosis_reader;

-- ─── Schema version marker ───────────────────────────────────
CREATE TABLE IF NOT EXISTS monitoring.schema_migrations (
    version         INTEGER     PRIMARY KEY,
    applied_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    description     TEXT        NOT NULL
);

INSERT INTO monitoring.schema_migrations (version, description) VALUES
    (1, 'Initial schema: extensions, stations, seismic_events, event_station_associations, pipeline_metrics, staging.raw_events');

-- Final verification
DO $$
BEGIN
    RAISE NOTICE 'Schema initialization complete.';
    RAISE NOTICE 'Tables created: seismology.stations, seismology.seismic_events, seismology.event_station_associations, monitoring.pipeline_metrics, staging.raw_events';
END;
$$;
