-- ──────────────────────────────────────────────────────────────────────────
-- 05_shakemap_column.sql
-- ShakeMap enrichment column for seismology.seismic_events.
--
-- Added for Phase 3 ShakeMap enrichment (ADR-0003).  The Python analysis
-- service fetches the USGS ShakeMap intensity GeoJSON for M≥3.5 USGS events
-- and stores the raw FeatureCollection here.
--
-- Uses IF NOT EXISTS so this script is idempotent on re-run.
-- ──────────────────────────────────────────────────────────────────────────

ALTER TABLE seismology.seismic_events
    ADD COLUMN IF NOT EXISTS shakemap JSONB;

COMMENT ON COLUMN seismology.seismic_events.shakemap
    IS 'Raw USGS ShakeMap intensity GeoJSON FeatureCollection for M≥3.5 USGS events (https://earthquake.usgs.gov/.../shakemap/intensity.geojson). NULL until fetched by the analysis service.';

-- GIN index for JSONB containment / key-path queries on the ShakeMap payload.
CREATE INDEX IF NOT EXISTS idx_seismic_events_shakemap_gin
    ON seismology.seismic_events USING GIN (shakemap)
    WHERE shakemap IS NOT NULL;
