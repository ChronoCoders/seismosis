-- ============================================================
-- 06_class_probabilities.sql
-- Add per-class probability JSONB column to seismic_events.
-- Populated by the forecast service classifier alongside
-- event_class and class_confidence.
-- ============================================================

BEGIN;

ALTER TABLE seismology.seismic_events
    ADD COLUMN IF NOT EXISTS class_probabilities JSONB;

COMMENT ON COLUMN seismology.seismic_events.class_probabilities
    IS 'Per-class probability map {tectonic, induced, volcanic} from the ML classifier.';

INSERT INTO monitoring.schema_migrations (version, description) VALUES
    (6, 'Add class_probabilities JSONB column to seismology.seismic_events');

COMMIT;
