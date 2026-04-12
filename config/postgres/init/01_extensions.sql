-- ============================================================
-- 01_extensions.sql — PostgreSQL Extension Initialization
-- ============================================================
-- Run order: first (01_)
-- Image: postgis/postgis:16-3.4
--
-- PostGIS is pre-installed in this image but must be activated
-- per-database. All extensions are created in the public schema
-- unless noted.

-- Spatial data support (required for all geometry columns)
CREATE EXTENSION IF NOT EXISTS postgis;

-- Topology support (used for regional polygon analysis in Phase 2)
-- Created here so the schema exists; tables added in Phase 2.
CREATE EXTENSION IF NOT EXISTS postgis_topology;

-- Raster support (for shakemap/intensity grid data in Phase 3)
-- CREATE EXTENSION IF NOT EXISTS postgis_raster; -- Phase 3

-- UUID generation (used as primary keys throughout)
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Fast trigram-based text search (station names, region names)
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- btree_gist: allows GiST indexes on non-geometric types.
-- Required for exclusion constraints combining time + geometry.
CREATE EXTENSION IF NOT EXISTS btree_gist;

-- pg_stat_statements: query performance tracking.
-- Required by postgres-exporter for query statistics.
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Verify extensions installed
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'postgis') THEN
        RAISE EXCEPTION 'PostGIS extension failed to install';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'uuid-ossp') THEN
        RAISE EXCEPTION 'uuid-ossp extension failed to install';
    END IF;
    RAISE NOTICE 'All required extensions installed successfully.';
    RAISE NOTICE 'PostGIS version: %', PostGIS_Lib_Version();
END;
$$;
