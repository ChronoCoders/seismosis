# Session: 2026-04-12 — Phase 2 Python Analysis Service

## Goal

Implement the Phase 2 Python analysis service (`services/analysis/`) in full, integrate it into the Seismosis infrastructure (docker-compose, Prometheus, PostgreSQL), and clean up residual dead code in the Phase 1 Rust services that was discovered during review.

This document is written for engineers who were not present. It assumes familiarity with the Phase 1 session notes at `docs/sessions/phase1-completion.md` but no other shared context.

---

## What Was Done

### Commit `8b81867` — Python analysis service (24 new files)

#### Package definition (`services/analysis/pyproject.toml`)

Runtime dependencies: `confluent-kafka`, `fastavro`, `psycopg2-binary`, `redis[hiredis]`, `prometheus-client`, `structlog`. Dev/test extras: `testcontainers` (for integration tests using real containers). The package is not installed as an editable install; `PYTHONPATH=/app/src` is set in the Dockerfile instead.

#### Container (`services/analysis/Dockerfile`)

Multi-stage build on `python:3.11-slim`. A non-root user (`analysis`, uid 1001) is created with `useradd`; the final stage runs as `USER analysis`. This was not present in the initial commit and was added during the deployment-validator review pass (commit `3defab1` — see below).

`PYTHONPATH=/app/src` is set so all imports resolve to `services/analysis/src/`.

#### Configuration (`services/analysis/src/analysis/config.py`)

A frozen `dataclass` `Config` with a `from_env()` classmethod. All values come from environment variables; there are no hardcoded defaults for secrets or infrastructure addresses. Fields include Kafka bootstrap server, Schema Registry URL, Kafka consumer group ID, PostgreSQL DSN, Redis URL, DB pool sizes, event cache TTL, Prometheus metrics port, and analysis version string.

#### Data models (`services/analysis/src/analysis/models.py`)

Four frozen dataclasses: `RawEvent`, `EnrichedEvent`, `AlertEvent`, `MainshockInfo`. All fields are typed. A `_to_ms()` helper handles both `int` and `datetime` inputs for timestamp-millis Avro fields — necessary because `fastavro` returns `datetime` objects when deserialising `timestamp-millis` logical types but the Avro encoder expects integer milliseconds.

#### Avro schemas (`services/analysis/src/analysis/schema.py`)

Two Avro schema dicts are defined inline: `ENRICHED_SCHEMA` and `ALERT_SCHEMA`. Schema Registry subjects:
- `earthquakes.enriched-value` — produced to `earthquakes.enriched`
- `earthquakes.alerts-value` — produced to `earthquakes.alerts`

Both schemas use Confluent wire format (5-byte magic-byte + schema ID header), consistent with the Phase 1 Avro encoding pattern used by the Rust ingestion and storage services.

#### Magnitude refinement (`services/analysis/src/analysis/magnitude.py`)

`refine_to_ml()` converts preliminary magnitude estimates (any type) to local magnitude (ML) using the Grünthal 2009 Mw→ML regression coefficients, with a depth correction applied for events shallower than 70 km. Input magnitude type is normalised to uppercase before coefficient lookup. Unknown magnitude types fall back to the identity transform (input value returned unchanged).

#### Aftershock detection (`services/analysis/src/analysis/aftershock.py`)

Three public functions:

- `gardner_knopoff_window(magnitude)` — returns the Gardner & Knopoff (1974) spatial radius (km) and temporal window (days) for a given mainshock magnitude.
- `find_mainshock(event, db)` — queries PostGIS using `ST_DWithin` to find any event within the spatial-temporal window that has a strictly larger magnitude. Returns the closest such event (by distance) as a `MainshockInfo`, or `None` if the current event is itself a mainshock candidate.
- `classify_aftershock(event, db)` — calls `find_mainshock` and returns `(is_aftershock: bool, mainshock: Optional[MainshockInfo])`.

The `find_mainshock` query operates against `seismology.seismic_events` using the `location` GEOMETRY column (SRID 4326). The spatial radius from `gardner_knopoff_window` is converted from km to degrees using a fixed 111.32 km/degree approximation; this approximation is acceptable for the magnitude ranges involved but will introduce small errors at high latitudes.

#### Risk assessment (`services/analysis/src/analysis/risk.py`)

Three functions:

- `estimate_felt_radius_km(magnitude, depth_km)` — empirical formula based on magnitude and depth.
- `estimate_epicentral_mmi(magnitude, depth_km)` — estimates Modified Mercalli Intensity at the epicentre.
- `alert_level(mmi, felt_radius_km)` — returns a string alert level (`"green"`, `"yellow"`, `"orange"`, `"red"`) based on MMI and felt radius thresholds.

#### Avro codec (`services/analysis/src/analysis/avro_codec.py`)

`SchemaRegistry` — HTTP client for the Confluent Schema Registry: registers schemas, fetches schemas by ID, caches fetched schemas in-process to avoid repeated HTTP calls.

`AvroDecoder` — decodes Confluent wire-format messages: strips the 5-byte header, fetches the writer schema by ID from the registry, deserialises with `fastavro`.

`AvroEncoder` — encodes dicts to Avro, prepends the Confluent 5-byte header with the registered schema ID.

#### Database (`services/analysis/src/analysis/db.py`)

`Database` wraps a `psycopg2.pool.ThreadedConnectionPool`. The `upsert_enriched_event()` method issues an `INSERT ... ON CONFLICT (source_id) DO UPDATE SET` that writes only the enrichment columns added by migration `03_enrichment_columns.sql` (see schema section below). It does not touch the base event columns owned by the Rust storage service.

The `INSERT ON CONFLICT` design is deliberate: the Rust storage service and the Python analysis service both consume `earthquakes.raw` independently. The storage service writes the base row; the analysis service writes the enrichment columns on top. Either service may arrive first — the upsert handles both orderings without coordination.

#### Cache (`services/analysis/src/analysis/cache.py`)

`Cache` wraps `redis-py`. Key pattern: `analysis:event:{source_id}`. TTL is controlled by `EVENT_CACHE_TTL_SECS` from `Config`. The cache is used to avoid re-processing events that have already been enriched (e.g. on consumer restart with `auto.offset.reset=earliest`).

#### Metrics (`services/analysis/src/analysis/metrics.py`)

Nine Prometheus metrics, all namespaced `seismosis_analysis_`:

| Metric | Type | Labels |
|--------|------|--------|
| `seismosis_analysis_messages_consumed_total` | Counter | `topic` |
| `seismosis_analysis_events_enriched_total` | Counter | `magnitude_class` |
| `seismosis_analysis_events_aftershock_total` | Counter | — |
| `seismosis_analysis_alerts_produced_total` | Counter | `level` |
| `seismosis_analysis_events_dead_letter_total` | Counter | `reason` |
| `seismosis_analysis_processing_duration_seconds` | Histogram | — |
| `seismosis_analysis_db_write_duration_seconds` | Histogram | — |
| `seismosis_analysis_magnitude_refinement_delta` | Histogram | — |
| `seismosis_analysis_cache_hits_total` | Counter | — |

All metrics are registered against a caller-supplied `CollectorRegistry`, not the global registry, to allow clean unit testing without cross-test metric state contamination.

The Prometheus HTTP server runs in a daemon thread started from `main.py`. The analysis consume loop itself is synchronous (see Architecture Decisions).

#### Kafka consumer (`services/analysis/src/analysis/consumer.py`)

`KafkaConsumer` wraps `confluent_kafka.Consumer`. Manual offset commit (`enable.auto.commit=false`). Offsets are committed only after a message has been fully processed and either enriched or dead-lettered. Topic subscribed: `earthquakes.raw`.

#### Kafka producer (`services/analysis/src/analysis/producer.py`)

`KafkaProducer` maintains two underlying `confluent_kafka.Producer` instances:

- **Main producer** — `acks=all`, `enable.idempotence=true`, `compression.type=lz4`. Publishes to `earthquakes.enriched` and `earthquakes.alerts`.
- **DLQ producer** — `acks=1`, best-effort. Publishes to `earthquakes.dead-letter`.

The DLQ producer uses lower guarantees intentionally: DLQ delivery failure should not block offset commit for the main processing path.

#### Main pipeline (`services/analysis/src/analysis/main.py`)

`process_message()` implements an 8-step pipeline per message:

1. Decode Avro (Confluent wire format) from `earthquakes.raw`
2. Check cache — skip if already enriched (returns early, commits offset)
3. Refine magnitude via `refine_to_ml()`
4. Classify as aftershock or mainshock via `classify_aftershock()`
5. Estimate felt radius and epicentral MMI via `risk.py`
6. Determine alert level
7. Upsert enrichment columns to PostgreSQL
8. Publish `EnrichedEvent` to `earthquakes.enriched`; publish `AlertEvent` to `earthquakes.alerts` if level is not `"green"`

Graceful shutdown is handled via `threading.Event` set by SIGTERM/SIGINT signal handlers. The consume loop checks the event between each message and exits cleanly.

### Tests

| File | Count | Coverage |
|------|-------|----------|
| `services/analysis/tests/test_magnitude.py` | 17 unit tests | `magnitude.py` — all coefficient paths, depth correction, unknown type fallback |
| `services/analysis/tests/test_aftershock.py` | 12 unit tests | `aftershock.py` — window calculations, mainshock lookup with mocked DB |
| `services/analysis/tests/test_models.py` | 16 unit tests | Dataclass construction, `_to_ms()` with int and datetime inputs |
| `services/analysis/tests/test_integration.py` | Full pipeline | Real Redpanda, PostGIS, and Redis containers via `testcontainers` |

`services/analysis/tests/conftest.py` provides session-scoped `testcontainers` fixtures for all three infrastructure components, shared across all integration test cases to minimise container startup overhead.

---

### Database migration (`config/postgres/init/03_enrichment_columns.sql`)

Adds enrichment columns to `seismology.seismic_events` via `ALTER TABLE`:

| Column | Type | Notes |
|--------|------|-------|
| `ml_magnitude` | `NUMERIC(4,2)` | Refined local magnitude; nullable (null until enriched) |
| `is_aftershock` | `BOOLEAN` | NULL until classified |
| `mainshock_source_id` | `VARCHAR(255)` | FK-like reference; nullable |
| `estimated_felt_radius_km` | `NUMERIC(8,2)` | Nullable |
| `estimated_intensity_mmi` | `NUMERIC(4,2)` | Modified Mercalli Intensity at epicentre; nullable |
| `enriched_at` | `TIMESTAMPTZ` | Set by analysis service on write; nullable |
| `analysis_version` | `VARCHAR(50)` | From `Config.analysis_version`; nullable |

Two partial indexes are added:
- On `(ml_magnitude DESC)` `WHERE ml_magnitude IS NOT NULL` — for magnitude-based queries on enriched events only.
- On `(is_aftershock)` `WHERE is_aftershock = true` — for aftershock cluster queries.

The migration is backward compatible: all new columns are nullable with no `NOT NULL` constraint, so the Rust storage service can continue writing rows without supplying enrichment values.

---

### Infrastructure updates

**`docker-compose.yml`** — Added `seismosis-analysis` service:
- Builds from `services/analysis/Dockerfile`
- Depends on `postgres` (healthy), `redpanda` (healthy), `redis` (healthy), `redpanda-init` (completed)
- Environment block includes all `Config` fields plus `DB_POOL_MIN`, `DB_POOL_MAX`, `EVENT_CACHE_TTL_SECS`
- Metrics exposed on container port 9092 (internal only — not bound to a host port)
- Healthcheck retries set to 5 (raised from the default 3 during the deployment-validator pass)
- `restart: unless-stopped`

**`config/prometheus/prometheus.yml`** — Added scrape job for `seismosis-analysis` at `seismosis-analysis:9092`, scrape interval 10 s (matching the other application services).

**`.env.example`** — Added Phase 2 analysis variables under a new `## Analysis Service` section: `SCHEMA_REGISTRY_URL`, `ANALYSIS_KAFKA_BOOTSTRAP_SERVERS`, `ANALYSIS_GROUP_ID`, `DB_POOL_MIN`, `DB_POOL_MAX`, `EVENT_CACHE_TTL_SECS`, `ANALYSIS_VERSION`.

---

### Commit `3defab1` — Deployment-validator fixes

Issues identified by the `deployment-validator` agent and resolved:

1. `services/analysis/Dockerfile` — non-root user added (`useradd analysis`, uid 1001, `USER analysis` in final stage). The initial commit ran as root, which violates container security requirements.
2. `docker-compose.yml` — `DB_POOL_MIN` and `DB_POOL_MAX` were referenced in `config.py` but missing from the `seismosis-analysis` environment block. Added.
3. `docker-compose.yml` — `EVENT_CACHE_TTL_SECS` was also missing from the environment block. Added.
4. `docker-compose.yml` — healthcheck `retries` raised from 3 to 5 for `seismosis-analysis`, consistent with the slower startup characteristics of Python services relative to the Rust services.
5. `.env.example` — `DB_POOL_MIN` and `DB_POOL_MAX` were missing from the Analysis Service section. Added.

---

### Phase 1 dead code removal (earlier in session)

Two sets of dead code were removed from Phase 1 Rust services. No functional behaviour was changed.

**`services/api/src/error.rs`:**
- `ApiError::Database` variant — defined but never constructed; database errors were handled via `anyhow` at the call site.
- `ApiError::Redis` variant — same; Redis errors were handled inline.
- `RequestError::Cache` variant — never constructed.
- `IntoResponse` arm for `RequestError::Cache` — unreachable, removed alongside the variant.

**`services/storage/src/config.rs`:**
- `pipeline_version` field removed from the `Config` struct. The field was read from the `PIPELINE_VERSION` environment variable and stored in `Config`, but was never accessed after construction. In the DB write path, `pipeline_version` comes from the decoded Avro event payload (`RawEvent.pipeline_version`), not from the service config. Retaining an unused config field created a misleading impression that the service was injecting its own version tag into stored events.

---

## Architecture Decisions Made

**1. Synchronous threading model, not asyncio**

The analysis service uses synchronous Python with `threading` rather than `asyncio`. `confluent-kafka`'s Python bindings are not async-native — the `poll()` call is blocking, and wrapping it in `asyncio` would require `run_in_executor`, adding complexity without benefit. The Prometheus metrics server runs in a daemon thread alongside the synchronous consume loop. This is consistent with the `confluent-kafka` documentation's recommended usage pattern.

**2. INSERT ON CONFLICT for enrichment writes**

The Rust storage service and the Python analysis service both consume `earthquakes.raw` independently. The storage service inserts the base row; the analysis service writes only the enrichment columns on top. Either may arrive first. Using `INSERT ON CONFLICT (source_id) DO UPDATE SET` on only the enrichment columns means both services can operate without coordination and without relying on ordering guarantees between consumer groups.

**3. Analysis service metrics on port 9092**

Container-internal port assignments:
- `seismosis-storage`: 9090
- `seismosis-ingestion`: 9091
- `seismosis-analysis`: 9092

Port 9092 is not bound to the host — it is only reachable within the Docker network by Prometheus. Prometheus scrapes it via the container name `seismosis-analysis:9092`.

**4. `fastavro` datetime handling via `_to_ms()`**

`fastavro` deserialises `timestamp-millis` Avro logical type fields as `datetime` objects when reading. The Avro encoder expects integer milliseconds. Rather than special-casing this in every encoding call, a `_to_ms()` helper in `models.py` accepts both `int` and `datetime` inputs, converting `datetime` via `int(dt.timestamp() * 1000)`. This prevents silent type errors when round-tripping events through decode → re-encode.

---

## Tech Debt

**TD-001: `deploy.resources.limits.memory` missing from all services in `docker-compose.yml`**

No service in `docker-compose.yml` — including Redpanda, PostgreSQL, Redis, Prometheus, Grafana, all application services, and all exporters — has a `deploy.resources.limits.memory` block. Without memory limits, a runaway service (e.g. Redpanda log retention not trimming, or a memory leak in the analysis service) can exhaust host memory and cause OOM kills of unrelated containers.

This must be addressed before Vultr deployment. All services must be given limits consistently — adding limits to some services but not others would produce unpredictable resource contention. Limits cannot be set without first profiling each service under representative load to determine appropriate values.

**Owner:** Infrastructure / deployment lead  
**Target:** Before Vultr deployment (Phase 2 production launch)  
**Blocking:** Yes — this is a hard blocker for production deployment.

---

## Open Items

### 1. Memory limits required before Vultr deployment (TD-001)

See Tech Debt section above. Profile each service under representative load, determine appropriate `deploy.resources.limits.memory` values, and apply them consistently to all services in `docker-compose.yml`.

### 2. `seismology.event_station_associations` table ownership unresolved

Carried forward from Phase 1 session notes. The table was created in `02_schema.sql` and remains empty. No Phase 1 or Phase 2 service writes to it. This should be moved to a dedicated Phase 2 (or Phase 3) migration script. See `docs/sessions/phase1-completion.md` — Open Item 1 for full context.

**Owner:** Phase 2 lead  
**Target:** Before Phase 3 schema design begins

### 3. `SignificantEarthquakeDetected` alert annotation still incorrect

Carried forward from Phase 1. The alert fires on `magnitude_class="major"` (M≥7.0) but the annotation summary text reads "M≥5.0". The expression is correct; only the annotation needs changing.

**File:** `config/prometheus/rules/earthquake_alerts.yml`  
**Owner:** Ops team  
**Target:** Next infrastructure review pass

### 4. Phase 2 remaining work: WebSocket service and frontend

The following Phase 2 deliverables are not yet started:
- `services/websocket/` — Rust WebSocket push server (`tokio-tungstenite`) consuming `earthquakes.enriched` and pushing events to connected clients in real time.
- `frontend/` — Turkish-language SPA with a real-time earthquake map connected via WebSocket.

Both are in scope for Phase 2. Phase 3 work (waveform storage, FDSN compliance, Kubernetes) must not begin until these are complete and validated.

### 5. Grafana dashboard not updated for Phase 2 analysis metrics

`config/grafana/dashboards/earthquake_overview.json` contains no panels for `seismosis_analysis_*` metrics. The nine analysis metrics (events enriched, aftershocks detected, alerts produced, processing duration, DB write duration, magnitude refinement delta, etc.) are being scraped by Prometheus but are invisible in Grafana.

**Owner:** Phase 2 lead  
**Target:** Before Phase 2 production launch

### 6. Gardner-Knopoff high-latitude approximation not validated

`aftershock.py` converts the spatial window from km to degrees using a fixed 111.32 km/degree approximation. This approximation degrades at high latitudes (e.g. for events in Alaska, Iceland, or northern Russia). The error is bounded and acceptable for the magnitudes involved in the current dataset, but it has not been formally validated against a geodetic distance calculation.

**TECH DEBT:** If the platform is extended to cover high-latitude regions at Phase 3, replace the fixed-degree conversion in `aftershock.py` with a geodetic distance query (PostGIS `ST_DWithin` with `geography` type instead of `geometry`) or a cosine-corrected degree conversion.

---

## Files Changed

### New files — `services/analysis/`

| File | Description |
|------|-------------|
| `services/analysis/pyproject.toml` | Package metadata; runtime and dev dependencies |
| `services/analysis/Dockerfile` | Multi-stage Python 3.11-slim build; non-root user (uid 1001) |
| `services/analysis/src/analysis/config.py` | Frozen `Config` dataclass with `from_env()` |
| `services/analysis/src/analysis/models.py` | `RawEvent`, `EnrichedEvent`, `AlertEvent`, `MainshockInfo` dataclasses; `_to_ms()` helper |
| `services/analysis/src/analysis/schema.py` | `ENRICHED_SCHEMA` and `ALERT_SCHEMA` Avro dicts; Schema Registry subjects |
| `services/analysis/src/analysis/magnitude.py` | `refine_to_ml()` — Grünthal 2009 coefficients + depth correction |
| `services/analysis/src/analysis/aftershock.py` | `gardner_knopoff_window()`, `find_mainshock()`, `classify_aftershock()` |
| `services/analysis/src/analysis/risk.py` | `estimate_felt_radius_km()`, `estimate_epicentral_mmi()`, `alert_level()` |
| `services/analysis/src/analysis/avro_codec.py` | `SchemaRegistry`, `AvroDecoder`, `AvroEncoder` — Confluent wire format |
| `services/analysis/src/analysis/db.py` | `Database` with `ThreadedConnectionPool`; `upsert_enriched_event()` |
| `services/analysis/src/analysis/cache.py` | `Cache` wrapping `redis-py`; key pattern `analysis:event:{source_id}` |
| `services/analysis/src/analysis/metrics.py` | 9 Prometheus metrics; caller-supplied `CollectorRegistry` |
| `services/analysis/src/analysis/consumer.py` | `KafkaConsumer` wrapping `confluent_kafka`; manual offset commit |
| `services/analysis/src/analysis/producer.py` | `KafkaProducer` with idempotent main producer and best-effort DLQ producer |
| `services/analysis/src/analysis/main.py` | `process_message()` 8-step pipeline; graceful shutdown via `threading.Event` |
| `services/analysis/tests/conftest.py` | Session-scoped `testcontainers` fixtures: Redpanda, PostGIS, Redis |
| `services/analysis/tests/test_magnitude.py` | 17 unit tests for `magnitude.py` |
| `services/analysis/tests/test_aftershock.py` | 12 unit tests for `aftershock.py` |
| `services/analysis/tests/test_models.py` | 16 unit tests for `models.py` |
| `services/analysis/tests/test_integration.py` | Full pipeline integration tests against real containers |

### New files — infrastructure

| File | Description |
|------|-------------|
| `config/postgres/init/03_enrichment_columns.sql` | `ALTER TABLE seismology.seismic_events` — adds 7 enrichment columns and 2 partial indexes |

### Modified files

| File | Change |
|------|--------|
| `docker-compose.yml` | Added `seismosis-analysis` service; added missing env vars (`DB_POOL_MIN`, `DB_POOL_MAX`, `EVENT_CACHE_TTL_SECS`); raised healthcheck retries to 5 |
| `config/prometheus/prometheus.yml` | Added `seismosis-analysis` scrape job at `seismosis-analysis:9092` |
| `.env.example` | Added Phase 2 Analysis Service section with all analysis environment variables |
| `services/api/src/error.rs` | Removed `ApiError::Database`, `ApiError::Redis`, `RequestError::Cache` — never constructed; removed corresponding `IntoResponse` arm |
| `services/storage/src/config.rs` | Removed `pipeline_version` field — populated from env but never read by the service |
