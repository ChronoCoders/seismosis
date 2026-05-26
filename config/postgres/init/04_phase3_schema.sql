-- ============================================================
-- 04_phase3_schema.sql — Phase 3 Schema: ETAS Forecasting,
--   Historical Catalog, and Event Classification
-- ============================================================
-- Run order: fourth (04_), after 03_enrichment_columns.sql
-- Phase 3: Waveform storage, FDSN compliance, public API,
--   ETAS aftershock forecasting, Gutenberg-Richter analysis,
--   ML event classification, and historical catalog ingestion.
--
-- All additions use IF NOT EXISTS so this script is idempotent.
-- All geometry columns use SRID 4326 (WGS-84 lon/lat).
-- All timestamps are TIMESTAMPTZ (UTC-aware).
-- ============================================================

BEGIN;

-- ─── seismology.historical_events ────────────────────────────
-- Large historical catalog imported from USGS ComCat / EMSC
-- archives for Gutenberg-Richter calibration and ETAS training.
-- Distinct from seismic_events (which is the live pipeline table)
-- so bulk-load operations do not interfere with real-time writes.
CREATE TABLE IF NOT EXISTS seismology.historical_events (
    id                  BIGSERIAL       PRIMARY KEY,

    -- Source provenance
    source_id           VARCHAR(255)    NOT NULL,
    source_network      VARCHAR(20)     NOT NULL,

    -- Temporal
    event_time          TIMESTAMPTZ     NOT NULL,

    -- Spatial
    latitude            NUMERIC(9, 6)   NOT NULL,
    longitude           NUMERIC(9, 6)   NOT NULL,
    depth_km            NUMERIC(8, 2),
    location            GEOGRAPHY(POINT, 4326) NOT NULL,

    -- Magnitude
    magnitude           NUMERIC(4, 2)   NOT NULL,
    magnitude_type      VARCHAR(10)     NOT NULL,

    -- Description
    region_name         TEXT,

    -- Gutenberg-Richter / b-value (populated later by forecast service)
    b_value_local       NUMERIC(6, 4),

    -- ML classification (populated later by classification service)
    event_class         VARCHAR(20)
                            CONSTRAINT historical_events_event_class_valid
                            CHECK (event_class IN ('tectonic', 'induced', 'volcanic', 'unknown')),
    class_confidence    NUMERIC(4, 3)
                            CONSTRAINT historical_events_class_confidence_range
                            CHECK (class_confidence IS NULL OR class_confidence BETWEEN 0 AND 1),

    ingested_at         TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    CONSTRAINT historical_events_source_id_unique UNIQUE (source_id)
);

COMMENT ON TABLE  seismology.historical_events IS 'Historical seismic catalog imported from USGS ComCat / EMSC for ETAS training and Gutenberg-Richter calibration';
COMMENT ON COLUMN seismology.historical_events.source_id        IS 'Canonical deduplication key. Format: {network}:{event_id} (e.g., usgs:us7000xyz)';
COMMENT ON COLUMN seismology.historical_events.location         IS 'Epicenter as WGS-84 geography point (lon, lat). Depth stored separately in depth_km.';
COMMENT ON COLUMN seismology.historical_events.b_value_local    IS 'Local Gutenberg-Richter b-value estimated for the surrounding region; populated by the forecast service after initial ingest.';
COMMENT ON COLUMN seismology.historical_events.event_class      IS 'Source classification: tectonic | induced | volcanic | unknown';
COMMENT ON COLUMN seismology.historical_events.class_confidence IS 'Model confidence for event_class in [0, 1].';

-- BRIN on event_time — efficient for time-ordered append of large catalog batches
CREATE INDEX IF NOT EXISTS idx_historical_events_event_time_brin
    ON seismology.historical_events USING BRIN (event_time)
    WITH (pages_per_range = 128);

-- Magnitude sort (threshold queries, statistics)
CREATE INDEX IF NOT EXISTS idx_historical_events_magnitude
    ON seismology.historical_events (magnitude DESC);

-- Spatial index (radius and bounding-box queries)
CREATE INDEX IF NOT EXISTS idx_historical_events_location_gist
    ON seismology.historical_events USING GIST (location);

-- Partial index for classified events (small, high-selectivity)
CREATE INDEX IF NOT EXISTS idx_historical_events_event_class
    ON seismology.historical_events (event_class)
    WHERE event_class IS NOT NULL;

-- ─── seismology.ingest_checkpoints ───────────────────────────
-- Persistent cursor for incremental historical catalog ingestion.
-- Each ingestion job records the latest event_time it has processed
-- so subsequent runs can resume without re-scanning the full archive.
CREATE TABLE IF NOT EXISTS seismology.ingest_checkpoints (
    id                  SERIAL          PRIMARY KEY,
    job_name            VARCHAR(100)    NOT NULL,
    last_event_time     TIMESTAMPTZ     NOT NULL,
    events_ingested     BIGINT          NOT NULL DEFAULT 0,
    updated_at          TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    CONSTRAINT ingest_checkpoints_job_name_unique UNIQUE (job_name)
);

COMMENT ON TABLE  seismology.ingest_checkpoints IS 'Incremental ingestion cursors; one row per catalog import job';
COMMENT ON COLUMN seismology.ingest_checkpoints.job_name        IS 'Logical job identifier, e.g. usgs_comcat_global or emsc_europe';
COMMENT ON COLUMN seismology.ingest_checkpoints.last_event_time IS 'Exclusive lower bound for the next incremental fetch window';
COMMENT ON COLUMN seismology.ingest_checkpoints.events_ingested IS 'Cumulative count of events written across all runs of this job';

-- ─── seismology.etas_forecasts ────────────────────────────────
-- ETAS (Epidemic-Type Aftershock Sequence) model output.
-- One row per (mainshock, forecast horizon, minimum magnitude) triple.
-- Spatial heatmap and daily rates are stored as JSONB to avoid a
-- separate normalised table; they are opaque blobs to PostgreSQL.
CREATE TABLE IF NOT EXISTS seismology.etas_forecasts (
    id                  BIGSERIAL       PRIMARY KEY,

    -- Mainshock reference (soft FK — historical_events may not exist yet)
    mainshock_source_id VARCHAR(255)    NOT NULL,

    computed_at         TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    -- Forecast window
    horizon_days        INTEGER         NOT NULL
                            CONSTRAINT etas_forecasts_horizon_positive CHECK (horizon_days > 0),
    min_magnitude       NUMERIC(3, 1)   NOT NULL,

    -- Summary statistics
    expected_count      NUMERIC(10, 4)  NOT NULL,
    p_at_least_one      NUMERIC(6, 5)   NOT NULL
                            CONSTRAINT etas_forecasts_p_at_least_one_range
                            CHECK (p_at_least_one BETWEEN 0 AND 1),

    -- Detailed probabilistic output
    p_exceedance        JSONB,   -- {magnitude: probability} map
    daily_rates         JSONB,   -- [rate_day1, ..., rate_dayN] array
    spatial_heatmap     JSONB,   -- GeoJSON FeatureCollection

    -- Model provenance
    params_zone         VARCHAR(100),
    params_snapshot     JSONB,   -- {mu, K, alpha, c, p, mc}
    model_version       VARCHAR(50)     NOT NULL
);

COMMENT ON TABLE  seismology.etas_forecasts IS 'ETAS aftershock sequence forecasts keyed by mainshock and forecast horizon';
COMMENT ON COLUMN seismology.etas_forecasts.mainshock_source_id IS 'source_id of the triggering mainshock; matches seismology.seismic_events.source_id';
COMMENT ON COLUMN seismology.etas_forecasts.horizon_days        IS 'Length of the forecast window in days from computed_at';
COMMENT ON COLUMN seismology.etas_forecasts.min_magnitude       IS 'Lower magnitude threshold for the forecast';
COMMENT ON COLUMN seismology.etas_forecasts.expected_count      IS 'Expected number of aftershocks M >= min_magnitude over horizon_days';
COMMENT ON COLUMN seismology.etas_forecasts.p_at_least_one      IS 'Probability of at least one aftershock M >= min_magnitude over horizon_days';
COMMENT ON COLUMN seismology.etas_forecasts.p_exceedance        IS 'JSONB map of {magnitude: exceedance_probability} pairs';
COMMENT ON COLUMN seismology.etas_forecasts.daily_rates         IS 'JSONB array of expected aftershock rates per day [day_1, ..., day_N]';
COMMENT ON COLUMN seismology.etas_forecasts.spatial_heatmap     IS 'GeoJSON FeatureCollection of probability density cells';
COMMENT ON COLUMN seismology.etas_forecasts.params_snapshot     IS 'ETAS parameter snapshot {mu, K, alpha, c, p, mc} used for this run';

-- Most common access: latest forecasts for a given mainshock
CREATE INDEX IF NOT EXISTS idx_etas_forecasts_mainshock_computed
    ON seismology.etas_forecasts (mainshock_source_id, computed_at DESC);

-- ─── seismology.gr_analysis ───────────────────────────────────
-- Gutenberg-Richter (G-R) regression results.
-- Supports both global (region_name only) and gridded
-- (grid_cell POLYGON) analyses; grid_cell is NULL for catalog-wide runs.
CREATE TABLE IF NOT EXISTS seismology.gr_analysis (
    id                  BIGSERIAL       PRIMARY KEY,
    computed_at         TIMESTAMPTZ     NOT NULL DEFAULT NOW(),

    -- Spatial scope
    region_name         VARCHAR(100),
    grid_cell           GEOGRAPHY(POLYGON, 4326),   -- NULL for full-catalog result

    -- G-R parameters
    b_value             NUMERIC(6, 4)   NOT NULL,
    b_std               NUMERIC(6, 4)   NOT NULL,
    a_value             NUMERIC(8, 4)   NOT NULL,
    mc                  NUMERIC(4, 2)   NOT NULL,   -- Magnitude of completeness

    -- Catalog window used for regression
    n_events            INTEGER         NOT NULL
                            CONSTRAINT gr_analysis_n_events_positive CHECK (n_events > 0),
    catalog_start       TIMESTAMPTZ     NOT NULL,
    catalog_end         TIMESTAMPTZ     NOT NULL,

    model_version       VARCHAR(50)     NOT NULL,

    CONSTRAINT gr_analysis_catalog_window_valid
        CHECK (catalog_end > catalog_start)
);

COMMENT ON TABLE  seismology.gr_analysis IS 'Gutenberg-Richter regression results, per region or grid cell';
COMMENT ON COLUMN seismology.gr_analysis.grid_cell     IS 'WGS-84 polygon bounding the analysis area; NULL for a full-catalog (global) result';
COMMENT ON COLUMN seismology.gr_analysis.b_value       IS 'Gutenberg-Richter b-value (slope of log-linear frequency-magnitude relation)';
COMMENT ON COLUMN seismology.gr_analysis.b_std         IS 'Standard error of the b-value estimate';
COMMENT ON COLUMN seismology.gr_analysis.a_value       IS 'Gutenberg-Richter a-value (log10 of seismicity rate at M=0)';
COMMENT ON COLUMN seismology.gr_analysis.mc            IS 'Estimated magnitude of completeness for the catalog window';

-- Spatial index for grid-cell overlap queries
CREATE INDEX IF NOT EXISTS idx_gr_analysis_grid_cell_gist
    ON seismology.gr_analysis USING GIST (grid_cell);

-- Time-ordered access (dashboards, latest run per region)
CREATE INDEX IF NOT EXISTS idx_gr_analysis_computed_at
    ON seismology.gr_analysis (computed_at DESC);

-- ─── seismology.model_registry ───────────────────────────────
-- Central registry for trained ML model versions.
-- Only one version per model_type may be active at a time;
-- that constraint is enforced at the application layer (partial unique
-- index approach would require a filtered unique expression not
-- portable to all Postgres versions, so we rely on app + monitoring).
CREATE TABLE IF NOT EXISTS seismology.model_registry (
    id                  SERIAL          PRIMARY KEY,
    model_type          VARCHAR(50)     NOT NULL,
    version             VARCHAR(50)     NOT NULL,
    trained_at          TIMESTAMPTZ     NOT NULL,
    n_train             INTEGER         NOT NULL
                            CONSTRAINT model_registry_n_train_positive CHECK (n_train > 0),
    metrics             JSONB           NOT NULL DEFAULT '{}',   -- {precision, recall, f1, auc_roc}
    artifact_path       TEXT            NOT NULL,
    is_active           BOOLEAN         NOT NULL DEFAULT FALSE,

    CONSTRAINT model_registry_type_version_unique UNIQUE (model_type, version)
);

COMMENT ON TABLE  seismology.model_registry IS 'Registry of trained ML model versions and their evaluation metrics';
COMMENT ON COLUMN seismology.model_registry.model_type    IS 'Logical model identifier, e.g. event_classifier or b_value_estimator';
COMMENT ON COLUMN seismology.model_registry.version       IS 'Semver string for this training run, e.g. 1.2.0';
COMMENT ON COLUMN seismology.model_registry.n_train       IS 'Number of training examples used for this model version';
COMMENT ON COLUMN seismology.model_registry.metrics       IS 'Evaluation metrics JSONB: {precision, recall, f1, auc_roc}';
COMMENT ON COLUMN seismology.model_registry.artifact_path IS 'Path or URI to the serialised model artifact (local path or object-store URI)';
COMMENT ON COLUMN seismology.model_registry.is_active     IS 'TRUE for the version currently serving predictions; at most one active version per model_type';

-- ─── seismology.seismic_events enrichment (Phase 3 columns) ──
-- Add event classification columns to the live events table so the
-- Phase 3 classification service can annotate events in real time.
-- Uses IF NOT EXISTS to be safe when the Phase 2 enrichment script
-- has already been applied.
ALTER TABLE seismology.seismic_events
    ADD COLUMN IF NOT EXISTS event_class        VARCHAR(20),
    ADD COLUMN IF NOT EXISTS class_confidence   NUMERIC(4, 3);

-- Apply check constraints only if column was just added (idempotent via DO block)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'seismic_events_event_class_valid'
          AND conrelid = 'seismology.seismic_events'::regclass
    ) THEN
        ALTER TABLE seismology.seismic_events
            ADD CONSTRAINT seismic_events_event_class_valid
            CHECK (event_class IN ('tectonic', 'induced', 'volcanic', 'unknown'));
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'seismic_events_class_confidence_range'
          AND conrelid = 'seismology.seismic_events'::regclass
    ) THEN
        ALTER TABLE seismology.seismic_events
            ADD CONSTRAINT seismic_events_class_confidence_range
            CHECK (class_confidence IS NULL OR class_confidence BETWEEN 0 AND 1);
    END IF;
END;
$$;

COMMENT ON COLUMN seismology.seismic_events.event_class
    IS 'ML-derived source classification: tectonic | induced | volcanic | unknown';
COMMENT ON COLUMN seismology.seismic_events.class_confidence
    IS 'Model confidence for event_class in [0, 1]; NULL until classification service processes the event.';

-- Partial index for classification queries on the live events table
CREATE INDEX IF NOT EXISTS idx_seismic_events_event_class
    ON seismology.seismic_events (event_class)
    WHERE event_class IS NOT NULL;

-- ─── Grants for new Phase 3 tables ───────────────────────────
GRANT SELECT ON seismology.historical_events    TO seismosis_reader;
GRANT SELECT ON seismology.ingest_checkpoints   TO seismosis_reader;
GRANT SELECT ON seismology.etas_forecasts       TO seismosis_reader;
GRANT SELECT ON seismology.gr_analysis          TO seismosis_reader;
GRANT SELECT ON seismology.model_registry       TO seismosis_reader;

-- ─── Schema version marker ────────────────────────────────────
INSERT INTO monitoring.schema_migrations (version, description) VALUES
    (4, 'Phase 3: historical_events, ingest_checkpoints, etas_forecasts, gr_analysis, model_registry; event_class + class_confidence on seismic_events');

-- Final verification
DO $$
BEGIN
    RAISE NOTICE 'Phase 3 schema migration complete.';
    RAISE NOTICE 'Tables created: seismology.historical_events, seismology.ingest_checkpoints, seismology.etas_forecasts, seismology.gr_analysis, seismology.model_registry';
    RAISE NOTICE 'Columns added to seismology.seismic_events: event_class, class_confidence';
END;
$$;

COMMIT;
