# Session: 2026-04-11 — Phase 1 Completion

## Goal

Build the complete Phase 1 Seismosis platform from scratch: a production-grade seismic event ingestion, storage, and query pipeline. Deliverables were three Rust services (`ingestion`, `storage`, `api`), a PostgreSQL schema, a Kafka/Redpanda topic layout, and a full observability stack (Prometheus, Grafana, alerting).

This document is written for engineers onboarding to Phase 2. It assumes no shared context from the Phase 1 sessions.

---

## What Was Done

### Rust Cargo Workspace (`services/`)

- Created `services/Cargo.toml` as the workspace root with `members = ["ingestion", "storage", "api"]` and `resolver = "2"`.
- All shared dependency versions are declared once under `[workspace.dependencies]`. Individual crate `Cargo.toml` files inherit with `{ workspace = true }` and optionally add extra features.
- Key shared dependencies: `tokio 1` (full), `rdkafka 0.36` (cmake-build), `apache-avro 0.16`, `sqlx 0.7` (runtime-tokio-rustls + postgres + chrono + macros), `axum 0.7`, `prometheus 0.13`, `redis 0.25`, `reqwest 0.12` (rustls-tls), `utoipa 4`, `proptest 1`, `testcontainers 0.23`.

### Service: `services/ingestion/` — Seismic Source Ingestion

**Responsibility:** Poll USGS FDSNWS and EMSC FDSNWS APIs, normalise events to `RawEarthquakeEvent`, Avro-encode, publish to `earthquakes.raw`.

**Source polling:**
- Both sources polled concurrently on a configurable interval (default `SOURCE_POLL_INTERVAL_SECS=60`).
- Each poll applies a 10-minute lookback overlap (`SOURCE_LOOKBACK_SECS=600`) to absorb late-arriving events between poll cycles.
- HTTP client uses `reqwest` with `rustls-tls`; per-request timeout `HTTP_TIMEOUT_SECS=30`; up to `HTTP_MAX_RETRIES=3` retries.

**Normalisation:**
- Events are normalised to `RawEarthquakeEvent` (defined in `services/ingestion/src/schema.rs`), a flat struct with 13 fields. The `source_id` format is `{NETWORK_UPPERCASE}:{event_id}` (e.g. `USGS:us6000m0wb`).
- `quality_indicator` is a single character: `A`=reviewed, `B`=estimated, `C`=preliminary, `D`=not reviewed.
- `depth_km` and `region_name` are nullable (`Option<f64>` / `Option<String>`).
- `magnitude_type` is always stored uppercase.

**Avro encoding:**
- Schema subject: `earthquakes.raw-value`. Registered with the Confluent Schema Registry (`SCHEMA_REGISTRY_URL`) on first publish.
- Wire format: 5-byte Confluent header (magic byte `0x00` + 4-byte big-endian schema ID) prepended to every Avro binary datum.
- Schema defined inline in `services/ingestion/src/schema.rs` (`AVRO_SCHEMA` constant). Nullable fields use `["null", T]` union with `"default": null` for backward-compatible evolution.

**Kafka publish:**
- Topic: `earthquakes.raw`. Producer settings: `acks=all`, `enable.idempotence=true`, `compression.type=lz4`.
- `KAFKA_MESSAGE_TIMEOUT_MS=30000` controls the per-message delivery timeout. Setting this low (e.g. 5000) in tests against an unavailable broker prevents multi-second hangs.

**Deduplication (`services/ingestion/src/dedup.rs`):**
- Redis-backed, key prefix `seismosis:dedup:raw:`, TTL `DEDUP_TTL_SECS=604800` (7 days, matching topic retention).
- Check (`is_duplicate`) and mark (`mark_seen`) are intentionally separate. `mark_seen` is called only after a successful `earthquakes.raw` produce, so a failed produce never permanently suppresses an event.
- In-memory LRU fallback (capacity 100,000 entries, ~6 MB) activates when Redis is unavailable. The LRU is not durable across restarts and is not safe across multiple service replicas — Redis availability is a hard requirement if `services/ingestion` is scaled beyond one instance.
- On Redis `EXISTS` failure, the service assumes *not duplicate* to avoid silently dropping events; the downstream idempotent upsert handles any resulting duplicates.

**Dead-letter routing (`services/ingestion/src/dead_letter.rs`):**
- Envelope type: `IngestDlqEnvelope` with fields `failure_reason`, `failure_detail`, `source_name`, `source_id`, `raw_payload` (original source JSON).
- Published to `earthquakes.dead-letter` with `acks=1` (best-effort), `message.timeout.ms=10000`.
- DLQ delivery is **advisory and non-fatal**: since ingestion polls HTTP APIs (not Kafka), there are no offsets to withhold. Events that fail DLQ delivery are simply not marked seen in the dedup store and will be retried on the next poll cycle.
- Retry policy: up to 4 attempts (1 initial + 3 retries) with exponential backoff starting at 500 ms, capped at 8 s.

**Prometheus metrics** (exposed on `:9091/metrics`):

| Metric | Type | Labels |
|--------|------|--------|
| `seismosis_ingestion_events_received_total` | Counter | `source` |
| `seismosis_ingestion_events_published_total` | Counter | `source` |
| `seismosis_ingestion_events_deduplicated_total` | Counter | `source` |
| `seismosis_ingestion_events_rejected_total` | Counter | `source`, `reason` |
| `seismosis_ingestion_poll_duration_seconds` | Histogram | `source` |
| `seismosis_ingestion_publish_duration_seconds` | Histogram | `topic` |
| `seismosis_ingestion_schema_registry_requests_total` | Counter | `status` |
| `seismosis_ingestion_dedup_redis_fallback_total` | Counter | `source` |

`events_rejected_total` reason values: `"parse_error"`, `"magnitude_filter"`, `"non_earthquake"`, `"missing_field"`.

**Graceful shutdown:** poll loop drains the current batch before exit on SIGTERM/Ctrl-C.

---

### Service: `services/storage/` — Kafka-to-PostgreSQL Storage

**Responsibility:** Consume from `earthquakes.raw`, decode and validate events, upsert into `seismology.seismic_events`.

**Consumer configuration:**
- `enable.auto.commit=false` — manual offset commits only.
- Consumer group: `KAFKA_GROUP_ID=seismosis-storage-group`.
- Avro decode: reads the 5-byte Confluent wire header, fetches writer schema from Schema Registry by schema ID on first encounter (cached in-process).

**Validation (`services/storage/src/model.rs`):**
`RawFields::validate()` enforces all constraints before the DB write, preventing rejected rows at the PostgreSQL CHECK constraint level:
- `source_id`: non-empty
- `source_network`: max 50 chars (matches `VARCHAR(50)`)
- `magnitude_type`: max 10 chars (matches `VARCHAR(10)`)
- `latitude`: finite and in `[-90, 90]`
- `longitude`: finite and in `[-180, 180]`
- `depth_km`: finite and in `[-5, 800]` when present
- `magnitude`: finite and in `[-2.0, 10.0]`
- `quality_indicator`: one of `A`, `B`, `C`, `D`
- `raw_payload`: valid JSON (column type is `JSONB`)

**Upsert logic:**
```sql
ON CONFLICT (source_id) DO UPDATE SET ...
WHERE EXCLUDED.event_time > seismic_events.event_time
```
This is a **monotonic guard**: late-arriving duplicates from the 10-minute poll lookback are silently skipped without error. An event already in the database is only overwritten if the incoming record has a strictly newer `event_time`.

**Dead-letter routing (`services/storage/src/dead_letter.rs`):**
- Envelope type: `DeadLetterEnvelope` with fields `failure_reason`, `failure_detail`, `original_topic`, `kafka_partition`, `kafka_offset`, `raw_payload_hex` (hex-encoded to avoid non-UTF-8 bytes in JSON).
- Offset commit semantics: offset is committed **only after DLQ delivery succeeds**. On DLQ delivery failure the offset is not committed — the message will be reprocessed on the next consumer startup. This makes DLQ delivery a prerequisite for "at least once" processing.
- Same retry policy as ingestion: up to 4 attempts, exponential backoff 500 ms → 8 s cap.

**Prometheus metrics** (exposed on `:9090/metrics`):

| Metric | Type | Labels |
|--------|------|--------|
| `seismosis_storage_messages_consumed_total` | Counter | `topic` |
| `seismosis_storage_events_upserted_total` | Counter | `outcome`, `magnitude_class` |
| `seismosis_storage_events_dead_letter_total` | Counter | `reason` |
| `seismosis_storage_processing_duration_seconds` | Histogram | `topic` |
| `seismosis_storage_db_write_duration_seconds` | Histogram | `table` |

`magnitude_class` label values and breakpoints: `minor` (< 3.0), `light` (3.0–3.9), `moderate` (4.0–4.9), `strong` (5.0–6.9), `major` (≥ 7.0).

`outcome` is always `"upsert"` in Phase 1 — insert vs. update is not distinguished at the metric level.

---

### Service: `services/api/` — REST API

**Responsibility:** Expose `seismology.seismic_events` data over a versioned REST API with Redis caching, Swagger UI, and Prometheus metrics.

**Framework:** axum 0.7, tokio multi-thread runtime, Tower `TimeoutLayer` (returns 408 after `REQUEST_TIMEOUT_SECS=30`).

**PostgreSQL pool:** sqlx `PgPoolOptions` with `acquire_timeout(5s)` — requests that cannot obtain a pool connection within 5 s receive a 500, which is preferable to hanging until the client timeout.

**Endpoints:**

| Method | Path | Description |
|--------|------|-------------|
| GET | `/health` | Pings PostgreSQL (`SELECT 1`) and Redis (`PING`). Always 200; check `status` field for `"ok"` or `"degraded"`. |
| GET | `/v1/events` | Paginated event list with optional filters. |
| GET | `/v1/events/{id}` | Single event by `source_id`, with Redis cache. |
| GET | `/v1/stats` | Aggregate statistics by magnitude band and time window. |
| GET | `/metrics` | Prometheus metrics (same port as API in Phase 1). |
| GET | `/docs` | Swagger UI (utoipa). |
| GET | `/docs/openapi.json` | OpenAPI JSON spec. |

**`GET /v1/events` filter parameters:**
- `page` / `page_size` (max 500, default 20)
- `start_time` / `end_time` (RFC 3339)
- `min_magnitude` / `max_magnitude` (open-ended upper bound supported via `Option<f64>`)
- `min_lat`, `max_lat`, `min_lon`, `max_lon` — bounding box via `ST_MakeEnvelope`; all four must be provided together or none. Anti-meridian crossing is not supported in Phase 1.

All float parameters are validated for NaN and Infinity before reaching the database (`validate_events_query` in `services/api/src/routes/events.rs`). NaN values would silently corrupt SQL comparisons and are rejected with 400.

**`GET /v1/events/{id}` cache:**
- Cache key: `api:event:{source_id}`, TTL `EVENT_CACHE_TTL_SECS=300`.
- Cache is populated asynchronously after a DB hit via `tokio::spawn` — the response is not blocked by the cache write.

**`GET /v1/stats` cache:**
- Cache key: `api:stats`, TTL `STATS_CACHE_TTL_SECS=60`.
- Returns counts grouped by magnitude band (minor < 2.0, light 2.0–4.0, moderate 4.0–6.0, strong 6.0–8.0, major ≥ 8.0) across four time windows (1 h, 24 h, 7 d, 30 d).
- The major band has no upper bound. This is implemented as `Option<f64>` mapped to `$N::float8 IS NULL OR magnitude::float8 < $N::float8` in SQL — using `f64::MAX` as a sentinel would produce `'Infinity'` which fails PostgreSQL `NUMERIC` casts.

**Prometheus cardinality:** `MatchedPath` middleware extracts the route template (e.g. `/v1/events/:id`) rather than the raw path, keeping label cardinality bounded regardless of the number of unique event IDs queried.

**Prometheus metrics** (exposed on `:8000/metrics` in Phase 1):

| Metric | Type | Labels |
|--------|------|--------|
| `seismosis_api_requests_total` | IntCounter | `method`, `path`, `status` |
| `seismosis_api_request_duration_seconds` | Histogram | `method`, `path` |
| `seismosis_api_cache_hits_total` | IntCounter | `endpoint` |
| `seismosis_api_cache_misses_total` | IntCounter | `endpoint` |

`endpoint` label values: `"get_event"`, `"get_stats"`.

**TECH DEBT:** `/metrics` is served on the same port as the public API (`:8000`). Phase 2 should move it to a dedicated internal port so metrics are not accessible to public consumers.

---

### Database Schema (`config/postgres/init/02_schema.sql`)

Applied automatically on first `docker compose up` via the PostgreSQL init container. A prerequisite extensions script runs first as `01_extensions.sql` (uuid-ossp, PostGIS).

**Schemas created:** `seismology`, `monitoring`, `staging`.

**Tables:**

| Table | Purpose | Key design notes |
|-------|---------|-----------------|
| `seismology.seismic_events` | Primary event store | `source_id VARCHAR(255) UNIQUE` is the dedup key; `magnitude NUMERIC(4,2) CHECK BETWEEN -2.0 AND 10.0`; `quality_indicator CHAR(1) CHECK IN ('A','B','C','D')`; `location GEOMETRY(POINT, 4326)`; `depth_km NUMERIC(8,3) CHECK BETWEEN -5 AND 800` |
| `seismology.stations` | Seismic station registry | Not written in Phase 1 |
| `seismology.event_station_associations` | Event-to-station links | Table exists but is **not populated by any Phase 1 service** — see Open Items |
| `monitoring.pipeline_metrics` | Immutable audit log | Originally named `pipeline_events`; renamed during Phase 1 to avoid confusion with seismic event data. All references updated. `BIGSERIAL` PK (not UUID) for efficiency on high-frequency append. |
| `staging.raw_events` | DLQ replay buffer | Manual review workflow; `UNIQUE (kafka_topic, kafka_partition, kafka_offset)` prevents duplicate staging inserts |

**Index strategy:**
- `BRIN` indexes on all time-ordered append columns (`event_time`, `received_at`, `recorded_at`, `created_at`) — efficient for insert-heavy workloads where physical order correlates with time.
- `GIST` indexes on all `GEOMETRY` columns for spatial queries.
- Composite `(event_time DESC, magnitude DESC)` on `seismic_events` for the most common API filter pattern.

**Roles:** `seismosis_reader` (no-login) granted `SELECT` on all schemas for the Postgres exporter and future reporting consumers.

**Schema version tracking:** `monitoring.schema_migrations` table with a version integer and description.

---

### Kafka Topics (`redpanda-init` one-shot container)

| Topic | Partitions | Retention | Compression |
|-------|-----------|-----------|-------------|
| `earthquakes.raw` | 6 | 7 days | lz4 |
| `earthquakes.cleaned` | 6 | 14 days | lz4 |
| `earthquakes.enriched` | 6 | 14 days | lz4 |
| `earthquakes.alerts` | 3 | 30 days | zstd |
| `earthquakes.dead-letter` | 3 | 30 days | zstd |

Topics are created by the `redpanda-init` one-shot container. Application services (`seismosis-ingestion`, `seismosis-storage`) declare `condition: service_completed_successfully` on `redpanda-init` in `docker-compose.yml` to ensure topics exist before any produce or consume attempt.

`earthquakes.cleaned` and `earthquakes.enriched` are created in Phase 1 but not consumed until Phase 2. `earthquakes.alerts` is created in Phase 1 but no publisher exists yet.

---

### Infrastructure (`docker-compose.yml`)

**Services:**

| Container | Role |
|-----------|------|
| `redpanda` | Single-broker Redpanda (Kafka-compatible) |
| `redpanda-console` | Redpanda Console UI (`localhost:8080`) |
| `redpanda-init` | One-shot topic creator (exits 0 after topic creation) |
| `postgres` | PostgreSQL 16 + PostGIS 3.4 |
| `redis` | Redis 7.2 |
| `prometheus` | Prometheus TSDB |
| `grafana` | Grafana dashboard server (`localhost:3000`) |
| `kafka-exporter` | Consumer lag exporter (scrapped by Prometheus at `:9308`) |
| `postgres-exporter` | PostgreSQL stats exporter (`:9187`) |
| `redis-exporter` | Redis stats exporter (`:9121`) |
| `seismosis-ingestion` | Rust ingestion service |
| `seismosis-storage` | Rust storage service |
| `seismosis-api` | Rust API service (`localhost:8000`) |

**Dependency chain:**
- `seismosis-ingestion`: `redpanda` (healthy) + `redis` (healthy) + `redpanda-init` (completed)
- `seismosis-storage`: `postgres` (healthy) + `redpanda` (healthy) + `redpanda-init` (completed)
- `seismosis-api`: `postgres` (healthy) + `redis` (healthy)

All services have `healthcheck` blocks and `restart: unless-stopped`.

---

### Observability

**Prometheus (`config/prometheus/prometheus.yml`):**
- Scrape interval: 15 s global; 10 s for the three application services; 30 s for `kafka-exporter`.
- Application scrape targets: `seismosis-ingestion:9091`, `seismosis-storage:9090`, `seismosis-api:8000`.
- High-cardinality internal Redpanda metrics (`redpanda_scheduler_*`, `redpanda_reactor_*`) are dropped at the scrape level via `metric_relabel_configs`.
- No Alertmanager in Phase 1 — alerts are visible in the Prometheus UI and forwarded to Grafana unified alerting.

**Alert rules (`config/prometheus/rules/earthquake_alerts.yml`):**

| Alert | Expression basis | Severity |
|-------|-----------------|----------|
| `SignificantEarthquakeDetected` | `increase(seismosis_storage_events_upserted_total{magnitude_class="major"}[5m]) > 0` — fires on M≥7.0 (major class) | info |
| `SeismicIngestionStalled` | `rate(seismosis_ingestion_events_published_total[10m]) == 0` for 10 m | warning |
| `DeadLetterQueueSpike` | DLQ rate > 1% of ingested for 5 m | warning |
| `DeadLetterQueueCritical` | DLQ rate > 5% of ingested for 2 m | critical |
| `AlertTopicConsumerLagHigh` | `kafka_consumer_group_lag{topic="earthquakes.alerts"} > 50` for 2 m | critical |
| `ProcessedEventsConsumerLagHigh` | lag on `earthquakes.cleaned` > 500 for 5 m | warning |
| `RawEventsConsumerLagHigh` | lag on `earthquakes.raw` > 1000 for 5 m | warning |
| `RedpandaBrokerDown` | `up{job="redpanda"} == 0` for 1 m | critical |
| `PostgresConnectionsHigh` / `Critical` | connections / max_connections > 0.8 / 0.95 | warning / critical |
| `PostgresDown` | postgres-exporter unreachable | critical |
| `PostgresLongRunningTransaction` | max tx duration > 300 s | warning |
| `PostgresDatabaseSizeLarge` | db > 20 GiB | info |
| `RedisMemoryHigh` / `Critical` | used / maxmemory > 0.8 / 0.95 | warning / critical |
| `RedisDown` | redis-exporter unreachable | critical |
| `RedisCacheHitRateLow` | hit rate < 50% for 10 m | warning |
| `ServiceDown` | `up == 0` for 2 m | critical |

`SignificantEarthquakeDetected` uses the storage service metric (`seismosis_storage_events_upserted_total`) rather than the ingestion metric because only the storage metric carries the `magnitude_class` label. The alert fires on `magnitude_class="major"` which corresponds to M≥7.0 — the alert rule annotations mention "M≥5.0" but that text is incorrect; `major` begins at 7.0 by the storage service breakpoint definition. The annotation text should be corrected to "M≥7.0" in a follow-up pass.

**TECH DEBT:** The `SignificantEarthquakeDetected` alert annotation text says "M≥5.0" but the `major` magnitude class (and therefore this alert's firing threshold) is M≥7.0. The annotation in `config/prometheus/rules/earthquake_alerts.yml` needs correcting.

**Grafana dashboard (`config/grafana/dashboards/earthquake_overview.json`):**
Panels: event rate, magnitude distribution barchart (5 bands, panel id 12), consumer lag, service status, PostgreSQL connections/transactions, Redis memory/hit rate/ops. No `seismosis_analysis_*` metrics are referenced anywhere in the Phase 1 dashboard.

---

## Decisions Made

All decisions below are captured in full as ADRs in `docs/adr/`. A summary follows.

1. **Confluent Avro wire format** — 5-byte header (magic `0x00` + 4-byte big-endian schema ID) prepended to every Avro binary datum. Consumers fetch the writer schema from the Schema Registry by ID on first encounter and cache it in-process. This is the de-facto standard for Confluent-compatible Kafka ecosystems and enables schema evolution without coordination.

2. **Idempotent upsert with monotonic `event_time` guard** — The `ON CONFLICT (source_id) DO UPDATE ... WHERE EXCLUDED.event_time > seismic_events.event_time` pattern silently skips duplicate or late-arriving events without errors or DLQ routing. The 10-minute poll lookback overlap means duplicates are routine, not exceptional.

3. **DLQ offset commit semantics in storage** — Offset committed only after DLQ delivery succeeds. On DLQ delivery failure, offset is withheld — the message will be reprocessed on the next consumer startup. This guarantees at-least-once processing at the cost of potential reprocessing after a DLQ delivery failure.

4. **Ingestion DLQ is advisory, not blocking** — Since ingestion polls HTTP APIs (no Kafka offsets), DLQ delivery failure does not block processing. Replay occurs automatically on the next poll cycle because `mark_seen` is only called after a successful `earthquakes.raw` produce. The DLQ envelope exists for investigation, not for replay gating.

5. **Redis `ConnectionManager` in API and ingestion** — Auto-reconnecting handle; cache or dedup failures are logged as warnings and callers fall back gracefully. Cache correctness is not required — it is a performance layer only.

6. **`Option<f64>` for open-ended magnitude bands in stats** — Using `f64::MAX` as a sentinel would produce `'Infinity'` in the SQL parameter, which fails a `NUMERIC` cast. `band.max = None` maps to `$N::float8 IS NULL OR magnitude::float8 < $N::float8`, which is correct and PostgreSQL-safe.

7. **`MatchedPath` middleware for Prometheus label cardinality** — `MatchedPath` extracts the route template (e.g. `/v1/events/:id`) rather than the literal request path, keeping `seismosis_api_requests_total` label cardinality bounded regardless of request volume.

8. **Cargo workspace with shared dependency versions** — All three Rust services share a single `[workspace.dependencies]` table in `services/Cargo.toml`. Individual crates inherit with `{ workspace = true }` and may add features locally. Version skew across services is eliminated at the workspace level.

9. **`monitoring.pipeline_metrics` table name** — Originally created as `monitoring.pipeline_events`; renamed during Phase 1 to avoid confusion with seismic event data stored in `seismology.seismic_events`. All references updated (table name, indexes, schema migration record, `RAISE NOTICE`).

10. **Metric name: `publish_duration_seconds` not `produce_duration_seconds`** — The original name `produce_duration_seconds` was aligned with the Kafka `produce` API but leaked internal terminology into the external metric namespace. Renamed to `publish_duration_seconds` to match the service-level abstraction and the spec contract.

---

## Phase 2 Scope: Python Analysis Service

The following is recorded here to prevent scope creep in Phase 2 design discussions.

**What the Python analysis service IS:**
- Consumes from `earthquakes.enriched`.
- Performs magnitude refinement (ML-based correction of preliminary magnitude estimates).
- Performs aftershock clustering (spatial-temporal clustering to identify foreshock/mainshock/aftershock sequences).
- Publishes results back to a Kafka topic (exact topic TBD in Phase 2 design).
- Does NOT write directly to `seismology.seismic_events` — raw event storage is owned exclusively by `services/storage`.

**What is deferred to Phase 2 alongside the Python service:**
- Multi-broker Redpanda cluster
- Kubernetes / Helm charts
- External notifications (SMS, email, PagerDuty)
- Population of `seismology.event_station_associations` (table exists in Phase 1 schema but no Phase 1 service writes to it — see Open Items)

**What is deferred to Phase 3:**
- Waveform storage (seismograms)
- FDSN compliance
- Public API

---

## Open Items

### 1. `seismology.event_station_associations` should move to a Phase 2 migration script

**Flagged by:** spec-guardian  
**Owner:** Phase 2 lead  
**Target:** Before Phase 2 Python service design begins

The table `seismology.event_station_associations` was created in `config/postgres/init/02_schema.sql` during Phase 1 schema work because it has referential integrity dependencies on `seismology.seismic_events`. However, no Phase 1 service populates it — population is the responsibility of the Phase 2 Python analysis service, which receives phase arrival data.

Leaving the table in the Phase 1 init script is defensible (the FK constraint is valid regardless), but the creation should ideally move to a dedicated Phase 2 migration script (e.g. `config/postgres/init/03_phase2_associations.sql`) to make the schema evolution boundary explicit. The Phase 2 design session should make a final call.

**TECH DEBT:** Until this is resolved, the table in `02_schema.sql` is documented here as intentionally empty post-Phase 1.

### 3. `SignificantEarthquakeDetected` alert annotation threshold is wrong

**Flagged by:** session-documenter  
**Owner:** Phase 1 / Phase 2 ops team  
**Target:** Next infrastructure review pass

The alert rule in `config/prometheus/rules/earthquake_alerts.yml` has its annotation summary text as "significant seismic event (M≥5.0)" but the firing condition uses `magnitude_class="major"`, which the storage service only applies to events with M≥7.0. The expression is correct; the annotation is incorrect and will mislead operators reading the alert.

**Fix:** Change the annotation summary text to "M≥7.0" in `config/prometheus/rules/earthquake_alerts.yml`, group `seismic_events`.

---

### 2. Response time p99 SLA targets not formally verified

**Flagged by:** spec-guardian  
**Owner:** Phase 1 / Phase 2 ops team  
**Target:** Before Phase 2 goes to production

The API design assumes `GET /v1/events` and `GET /v1/events/{id}` will meet sub-second p99 latency under production-representative load. This assumption has not been verified with a load test. The histogram buckets in `seismosis_api_request_duration_seconds` go up to 5 s, which is appropriate for capturing degradation but does not constitute a verified SLA.

**Action required:** Run a load test (e.g. with `k6` or `wrk`) against a representative dataset before Phase 2 launch. Confirm p99 latency, identify any N+1 query patterns, and adjust PostgreSQL pool sizes and index coverage if needed. Document the confirmed SLA targets in `docs/api/`.

---

## Files Changed

| File | Description |
|------|-------------|
| `services/Cargo.toml` | Cargo workspace root; shared dependency versions for all three Rust services |
| `services/ingestion/Cargo.toml` | Ingestion crate manifest; inherits workspace deps |
| `services/ingestion/src/main.rs` | Service entry point; poll loop, graceful shutdown |
| `services/ingestion/src/config.rs` | Environment variable configuration with typed parsing |
| `services/ingestion/src/schema.rs` | `RawEarthquakeEvent` struct and `AVRO_SCHEMA` constant; Schema Registry subject `earthquakes.raw-value` |
| `services/ingestion/src/avro.rs` | Avro encode/decode with Confluent 5-byte wire header |
| `services/ingestion/src/dedup.rs` | Redis-backed deduplicator with in-memory LRU fallback |
| `services/ingestion/src/producer.rs` | Kafka producer wrapper; idempotent publish to `earthquakes.raw` |
| `services/ingestion/src/dead_letter.rs` | `IngestDlqEnvelope`; advisory DLQ publisher |
| `services/ingestion/src/metrics.rs` | All 8 Prometheus metrics for the ingestion service |
| `services/ingestion/src/registry.rs` | Schema Registry HTTP client; schema registration and ID lookup |
| `services/ingestion/src/error.rs` | `IngestError` enum (thiserror) |
| `services/ingestion/src/sources/usgs.rs` | USGS FDSNWS polling and normalisation |
| `services/ingestion/src/sources/emsc.rs` | EMSC FDSNWS polling and normalisation |
| `services/ingestion/src/sources/mod.rs` | Source trait and shared polling logic |
| `services/ingestion/src/integration_tests.rs` | Integration tests using `testcontainers` |
| `services/storage/Cargo.toml` | Storage crate manifest |
| `services/storage/src/main.rs` | Service entry point; Kafka consume loop |
| `services/storage/src/config.rs` | Environment variable configuration |
| `services/storage/src/model.rs` | `EarthquakeEvent`, `RawFields`, `RawFields::validate()` |
| `services/storage/src/avro.rs` | Avro decode with Confluent wire header |
| `services/storage/src/db.rs` | `upsert_event` with monotonic `event_time` guard |
| `services/storage/src/dead_letter.rs` | `DeadLetterEnvelope`; DLQ publisher with offset-commit semantics |
| `services/storage/src/metrics.rs` | All 5 Prometheus metrics for the storage service |
| `services/storage/src/registry.rs` | Schema Registry client (shared pattern with ingestion) |
| `services/storage/src/error.rs` | `StorageError`, `ProcessError` enums |
| `services/api/Cargo.toml` | API crate manifest |
| `services/api/src/main.rs` | Service entry point; axum server setup, pool acquire_timeout |
| `services/api/src/config.rs` | Environment variable configuration |
| `services/api/src/routes/events.rs` | `list_events`, `get_event` handlers; `validate_events_query` with property tests |
| `services/api/src/routes/stats.rs` | `get_stats` handler with Redis cache |
| `services/api/src/routes/health.rs` | `health` handler; pings PostgreSQL and Redis |
| `services/api/src/routes/metrics_handler.rs` | Prometheus text exposition handler |
| `services/api/src/routes/mod.rs` | Router construction; `MatchedPath` middleware; `AppState` |
| `services/api/src/db.rs` | `list_events`, `get_event_by_source_id`, `get_stats` SQL queries |
| `services/api/src/cache.rs` | `Cache` wrapper around `redis::aio::ConnectionManager`; key constants |
| `services/api/src/metrics.rs` | All 4 Prometheus metrics for the API service |
| `services/api/src/model.rs` | Request/response types; `EventsQuery`, `EventResponse`, `StatsResponse` |
| `services/api/src/error.rs` | `ApiError`, `RequestError` enums; axum `IntoResponse` impl |
| `services/api/src/openapi.rs` | utoipa OpenAPI spec assembly |
| `config/postgres/init/01_extensions.sql` | Enables uuid-ossp and PostGIS extensions |
| `config/postgres/init/02_schema.sql` | Full Phase 1 database schema |
| `config/prometheus/prometheus.yml` | Prometheus scrape configuration; 8 targets |
| `config/prometheus/rules/earthquake_alerts.yml` | All alert rules across 5 rule groups |
| `config/grafana/dashboards/earthquake_overview.json` | Grafana dashboard provisioning |
| `docker-compose.yml` | Full infrastructure definition; 13 services |
| `.env.example` | Template for all environment variables |
