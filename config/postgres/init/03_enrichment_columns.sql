-- ──────────────────────────────────────────────────────────────────────────
-- 03_enrichment_columns.sql
-- Analysis-service enrichment columns for seismology.seismic_events.
--
-- Added by the Phase 2 Python analysis service.  The Rust storage service
-- continues to own the raw columns; this migration adds the analysis-derived
-- columns without touching existing rows or constraints.
--
-- All additions use IF NOT EXISTS so this script is idempotent.
-- ──────────────────────────────────────────────────────────────────────────

ALTER TABLE seismology.seismic_events
    ADD COLUMN IF NOT EXISTS ml_magnitude             NUMERIC(4, 2),
    ADD COLUMN IF NOT EXISTS is_aftershock            BOOLEAN,
    ADD COLUMN IF NOT EXISTS mainshock_source_id      VARCHAR(255),
    ADD COLUMN IF NOT EXISTS estimated_felt_radius_km NUMERIC(8, 2),
    ADD COLUMN IF NOT EXISTS estimated_intensity_mmi  NUMERIC(4, 2),
    ADD COLUMN IF NOT EXISTS enriched_at              TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS analysis_version         VARCHAR(50);

COMMENT ON COLUMN seismology.seismic_events.ml_magnitude
    IS 'Refined local magnitude (ML scale) computed by the analysis service via empirical conversion.';
COMMENT ON COLUMN seismology.seismic_events.is_aftershock
    IS 'True if this event falls within the Gardner-Knopoff space-time window of a larger event.';
COMMENT ON COLUMN seismology.seismic_events.mainshock_source_id
    IS 'source_id of the candidate mainshock when is_aftershock = TRUE.';
COMMENT ON COLUMN seismology.seismic_events.estimated_felt_radius_km
    IS 'Empirical estimate of the felt radius in km (Bakun & Scotti 2006 simplified).';
COMMENT ON COLUMN seismology.seismic_events.estimated_intensity_mmi
    IS 'Estimated Modified Mercalli Intensity at the epicentre (Wald et al. 1999 simplified).';
COMMENT ON COLUMN seismology.seismic_events.enriched_at
    IS 'Wall-clock time when the analysis service last updated this row.';
COMMENT ON COLUMN seismology.seismic_events.analysis_version
    IS 'Semver of the analysis service that produced the enrichment fields.';

-- Index for aftershock chain queries (find mainshock → aftershock children).
CREATE INDEX IF NOT EXISTS seismic_events_mainshock_source_id_idx
    ON seismology.seismic_events (mainshock_source_id)
    WHERE mainshock_source_id IS NOT NULL;

-- Partial index for aftershock flag (small, high-selectivity).
CREATE INDEX IF NOT EXISTS seismic_events_is_aftershock_idx
    ON seismology.seismic_events (is_aftershock)
    WHERE is_aftershock = TRUE;
