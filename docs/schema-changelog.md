# Schema Changelog

All database schema and Kafka schema changes are recorded here in reverse-chronological order.

---

## [2026-04-11] — Phase 1 initial schema

**Type:** PostgreSQL  
**Subject/Table:** seismology.seismic_events, seismology.stations, seismology.event_station_associations, monitoring.pipeline_metrics, monitoring.schema_migrations, staging.raw_events  
**Change:** Initial creation of all Phase 1 tables via `config/postgres/init/02_schema.sql`. Schemas created: `seismology`, `monitoring`, `staging`. Read-only role `seismosis_reader` granted SELECT on all three schemas. `updated_at` auto-maintenance triggers installed on `seismology.stations` and `seismology.seismic_events`. Schema version 1 inserted into `monitoring.schema_migrations`.  
**Backward Compatible:** N/A — initial creation  
**Migration Required:** No — applied automatically by Docker init container on first `docker compose up`

### Table inventory

| Table | Notes |
|-------|-------|
| `seismology.seismic_events` | Primary event store. `source_id VARCHAR(255) UNIQUE` is the canonical dedup key. Indexes: GIST on `location`, BRIN on `event_time`, composite B-tree `(event_time DESC, magnitude DESC)`, partial on `quality_indicator IN ('A','B')`, B-tree on `source_network`. |
| `seismology.stations` | Station registry. Not populated by any Phase 1 service. Indexes: GIST on `location`, B-tree on `network_code`, partial on `is_active = TRUE`, BRIN on `created_at`. |
| `seismology.event_station_associations` | Phase 1 placeholder; intentionally empty post-Phase 1. Populated by Phase 2 analysis service. FK references `seismic_events(id)` (CASCADE) and `stations(id)` (RESTRICT). |
| `monitoring.pipeline_metrics` | Immutable audit log. `BIGSERIAL` PK (not UUID) for high-frequency append efficiency. BRIN on `recorded_at`. Originally created as `pipeline_events`; renamed during Phase 1 to avoid confusion with seismic event tables. |
| `monitoring.schema_migrations` | Schema version tracker. Version 1 inserted at init. |
| `staging.raw_events` | DLQ replay buffer. `UNIQUE (kafka_topic, kafka_partition, kafka_offset)` prevents duplicate staging inserts. BRIN on `received_at`. |

---

## [2026-04-11] — Phase 1 Avro schema for RawEarthquakeEvent

**Type:** Kafka Schema (Avro, Confluent wire format)  
**Subject/Table:** earthquakes.raw-value  
**Change:** Initial registration of `com.seismosis.RawEarthquakeEvent` schema (13 fields: source_id, source_network, event_time_ms, latitude, longitude, depth_km, magnitude, magnitude_type, region_name, quality_indicator, raw_payload, ingested_at_ms, pipeline_version)  
**Backward Compatible:** N/A — initial registration  
**Migration Required:** No
