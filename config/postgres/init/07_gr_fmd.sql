-- ============================================================
-- 07_gr_fmd.sql
-- Add frequency-magnitude distribution (FMD) JSONB column to
-- seismology.gr_analysis. Populated by the forecast service
-- alongside b_value and returned by the GR analysis API endpoint.
-- ============================================================

BEGIN;

ALTER TABLE seismology.gr_analysis
    ADD COLUMN IF NOT EXISTS fmd JSONB;

COMMENT ON COLUMN seismology.gr_analysis.fmd
    IS 'Frequency-magnitude distribution as [{magnitude, cumulative_count}, ...] array, starting at Mc.';

INSERT INTO monitoring.schema_migrations (version, description) VALUES
    (7, 'Add fmd JSONB column to seismology.gr_analysis');

COMMIT;
