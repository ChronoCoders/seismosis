# Seismosis — Phase 1 and Phase 2 Completion Summary

**Written:** 2026-05-25  
**Audience:** Engineers onboarding to Phase 3; future Claude sessions.  
**Source documents:** `docs/sessions/phase1-completion.md`, `docs/sessions/2026-04-12-phase2-analysis-service.md`, `docs/sessions/2026-04-13-phase2-frontend-websocket-fixes.md`, `docs/adr/ADR-0001-websocket-kafka-offset-management.md`, `docs/adr/ADR-0002-frontend-maplibre-over-mapbox.md`.

---

## Phase 1 — Complete

### Infrastructure

- Single-broker Redpanda cluster with Schema Registry and Redpanda Console (`localhost:8080`).
- Five Kafka topics provisioned by a `redpanda-init` one-shot container: `earthquakes.raw` (6 partitions, 7 d, lz4), `earthquakes.cleaned` (6, 14 d, lz4), `earthquakes.enriched` (6, 14 d, lz4), `earthquakes.alerts` (3, 30 d, zstd), `earthquakes.dead-letter` (3, 30 d, zstd).
- PostgreSQL 16 + PostGIS 3.4 on host port 5433 (5433 chosen to avoid WSL2 relay conflict with the default 5432).
- Redis 7.2 for hot-path caching and deduplication.
- Prometheus + Grafana observability stack; 17 alert rules across 5 rule groups covering ingestion stall, DLQ spikes, consumer lag, PostgreSQL connections and transaction age, Redis memory and hit rate, and broker availability.
- All infrastructure defined in `docker-compose.yml` (13 containers total, each with healthcheck and `restart: unless-stopped`).

### Rust Cargo Workspace (`services/`)

- Workspace root at `services/Cargo.toml` with `resolver = "2"`. Shared dependency versions declared once under `[workspace.dependencies]`; individual crate manifests inherit with `{ workspace = true }`. Eliminates version skew across services.
- Three member crates: `ingestion`, `storage`, `api`.

### Service: `services/ingestion/` — Seismic Source Ingestion (Rust)

- Polls USGS FDSNWS and EMSC FDSNWS APIs concurrently on a configurable interval (default 60 s) with a 10-minute lookback overlap to absorb late-arriving events.
- Normalises events to `RawEarthquakeEvent` (13 fields). `source_id` format: `{NETWORK_UPPERCASE}:{event_id}`. `quality_indicator` is single char A/B/C/D.
- Avro-encodes with Confluent 5-byte wire format (magic `0x00` + 4-byte big-endian schema ID); schema subject `earthquakes.raw-value` registered with the Schema Registry.
- Publishes to `earthquakes.raw` with `acks=all` and `enable.idempotence=true`.
- Redis-backed deduplication (key prefix `seismosis:dedup:raw:`, TTL 7 days). In-memory LRU fallback (100,000 entries) activates when Redis is unavailable. `mark_seen` is called only after a successful produce to prevent permanently suppressing events on a failed publish.
- Dead-letter routing to `earthquakes.dead-letter` is advisory and non-fatal: a failed DLQ delivery does not suppress the event — it will be retried on the next poll cycle because `mark_seen` was never called.
- Prometheus metrics on `:9091/metrics`; 8 metrics including per-source event counters, poll duration histogram, and a Redis fallback counter.
- Graceful shutdown on SIGTERM/Ctrl-C: poll loop drains the current batch before exit.

### Service: `services/storage/` — Kafka-to-PostgreSQL Storage (Rust)

- Consumes `earthquakes.raw` with `enable.auto.commit=false` (manual offset commits).
- Decodes Avro using the Confluent wire header; fetches writer schema from the Schema Registry by ID on first encounter and caches it in-process.
- Validates all fields before the DB write (`RawFields::validate()` in `model.rs`): bounds on lat/lon/depth/magnitude, non-empty `source_id`, max-length strings, valid `quality_indicator`, valid JSON payload.
- Upserts into `seismology.seismic_events` with a monotonic guard: `ON CONFLICT (source_id) DO UPDATE ... WHERE EXCLUDED.event_time > seismic_events.event_time`. Duplicate or late-arriving events are silently skipped.
- Dead-letter routing: offset committed only after DLQ delivery succeeds. On DLQ failure the offset is withheld and the message is reprocessed on next startup, preserving at-least-once semantics.
- Prometheus metrics on `:9090/metrics`; 5 metrics including `events_upserted_total` with a `magnitude_class` label (minor/light/moderate/strong/major).

### Service: `services/api/` — REST API (Rust, axum 0.7)

- Endpoints: `GET /health`, `GET /v1/events` (paginated, filterable), `GET /v1/events/{id}` (Redis-cached, TTL 300 s), `GET /v1/stats` (Redis-cached, TTL 60 s), `GET /metrics`, `GET /docs` (Swagger UI), `GET /docs/openapi.json`.
- Bounding box filter uses PostGIS `ST_MakeEnvelope` (all four coordinates required or none). Anti-meridian crossing not supported.
- Float parameters validated for NaN and Infinity before reaching SQL — NaN values would silently corrupt comparisons.
- Cache writes are non-blocking (`tokio::spawn`); response is not delayed by the Redis write.
- `MatchedPath` middleware extracts route templates for Prometheus labels, keeping cardinality bounded.
- sqlx pool with 5 s `acquire_timeout` — requests that cannot acquire a connection return 500 rather than hanging.
- **TECH DEBT:** `/metrics` served on the same port as the public API (`:8000`). Should move to a dedicated internal port in Phase 3.

### Database Schema (`config/postgres/init/`)

- `01_extensions.sql` enables `uuid-ossp` and PostGIS.
- `02_schema.sql` defines schemas `seismology`, `monitoring`, `staging` and five tables:
  - `seismology.seismic_events` — primary event store; `source_id VARCHAR(255) UNIQUE` is the dedup key; `location GEOMETRY(POINT, 4326)` for spatial queries; `magnitude NUMERIC(4,2)` with CHECK constraint.
  - `seismology.stations`, `seismology.event_station_associations` — exist but are not written by any Phase 1 service (**TECH DEBT:** `event_station_associations` table ownership is unresolved; no service populates it).
  - `monitoring.pipeline_metrics` — append-only audit log; `BIGSERIAL` PK for high-frequency inserts.
  - `staging.raw_events` — DLQ replay buffer.
- Index strategy: `BRIN` on time-ordered columns, `GIST` on geometry columns, composite `(event_time DESC, magnitude DESC)` for common API filter pattern.
- `seismosis_reader` role granted `SELECT` on all schemas for the Postgres exporter.

### Key Architecture Decisions — Phase 1

1. **Confluent Avro wire format** for all Kafka messages — 5-byte header enables schema evolution without consumer coordination.
2. **Idempotent upsert with monotonic `event_time` guard** — duplicate events from the 10-minute poll overlap are silently discarded, not DLQ'd.
3. **DLQ offset commit semantics** in the storage service — offset advances only after DLQ delivery succeeds; guarantees at-least-once processing.
4. **Ingestion DLQ is advisory** — HTTP-polled sources have no Kafka offsets; DLQ failure is non-blocking and events are retried on the next poll cycle.
5. **Cargo workspace shared dependency versions** — version skew eliminated at the workspace level.

---

## Phase 2 — Complete

### Service: `services/analysis/` — Python Analysis Service

- Consumes `earthquakes.raw`. Publishes enriched events to `earthquakes.enriched` and alerts (level not green) to `earthquakes.alerts`.
- Language: Python 3.11. Libraries: `confluent-kafka` (not `kafka-python`), `fastavro`, `psycopg2`, `redis`, `prometheus-client`, `structlog`.
- Magnitude refinement (`magnitude.py`): `refine_to_ml()` applies Grünthal 2009 Mw→ML regression coefficients with depth correction for events shallower than 70 km. Unknown magnitude types fall back to the identity transform.
- Aftershock detection (`aftershock.py`): `gardner_knopoff_window()` returns the Gardner & Knopoff (1974) spatial and temporal window for a given mainshock magnitude. `find_mainshock()` queries PostGIS via `ST_DWithin` to find the closest event with a strictly larger magnitude within the window. **TECH DEBT:** Spatial radius converted from km to degrees using a fixed 111.32 km/degree approximation — degrades at high latitudes. For Phase 3 high-latitude coverage, replace with PostGIS `geography` type queries.
- Risk assessment (`risk.py`): empirical felt radius (km), Modified Mercalli Intensity (MMI) at epicentre, and alert level (`green`/`yellow`/`orange`/`red`).
- Avro codec (`avro_codec.py`): `SchemaRegistry` HTTP client, `AvroDecoder`, `AvroEncoder` — all using Confluent wire format consistent with Phase 1.
- Database (`db.py`): `upsert_enriched_event()` writes only the 7 enrichment columns added by migration `03_enrichment_columns.sql`. Does not touch base event columns owned by the Rust storage service.
- **Architecture:** Both the Rust storage service and the Python analysis service consume `earthquakes.raw` independently. Either may arrive first; `INSERT ON CONFLICT (source_id) DO UPDATE SET` on only the enrichment columns handles both orderings without coordination.
- **Threading model:** Synchronous Python with `threading`, not `asyncio`. `confluent-kafka`'s `poll()` is blocking; wrapping it in `asyncio` would require `run_in_executor` with no benefit.
- Cache (`cache.py`): key pattern `analysis:event:{source_id}`. Used to skip re-processing on consumer restart with `auto.offset.reset=earliest`.
- Prometheus metrics on container port 9092 (internal only); 9 metrics namespaced `seismosis_analysis_`, including `magnitude_refinement_delta` histogram and `events_aftershock_total`.
- Container: multi-stage Python 3.11-slim; non-root user `analysis` uid 1001; `PYTHONPATH=/app/src`.
- Tests: 45 unit tests across `test_magnitude.py` (17), `test_aftershock.py` (12), `test_models.py` (16); full pipeline integration test against real `testcontainers` containers (Redpanda, PostGIS, Redis).

### Database Migration (`config/postgres/init/03_enrichment_columns.sql`)

- `ALTER TABLE seismology.seismic_events` adds 7 nullable enrichment columns: `ml_magnitude NUMERIC(4,2)`, `is_aftershock BOOLEAN`, `mainshock_source_id VARCHAR(255)`, `estimated_felt_radius_km NUMERIC(8,2)`, `estimated_intensity_mmi NUMERIC(4,2)`, `enriched_at TIMESTAMPTZ`, `analysis_version VARCHAR(50)`.
- Two partial indexes added: `(ml_magnitude DESC) WHERE ml_magnitude IS NOT NULL` and `(is_aftershock) WHERE is_aftershock = true`.
- Backward compatible: all new columns are nullable; Rust storage service continues writing base rows unchanged.

### Service: `services/websocket/` — Rust WebSocket Push Server

- Consumes `earthquakes.enriched` and `earthquakes.alerts`; fans out events to connected WebSocket clients in real time.
- Library: `tokio-tungstenite` + `rdkafka`.
- **Kafka offset management (ADR-0001):** `enable.auto.commit=true`, `enable.auto.offset.store=false`, `auto.commit.interval.ms=5000`. After each message is fully processed, `store_offset_from_message(&msg)` is called. Offsets flush every 5 s via the librdkafka background thread — no Kafka RPC on the hot path. At-most 5 s of events replayed to clients on restart; frontend deduplicates by `source_id`.
- Per-client filters are applied from query parameters (parsed and percent-decoded); `Arc::clone` is deferred until a client passes the filter to avoid unnecessary clones on high-magnitude-only subscribers.
- Shutdown sequence: broadcast shutdown signal → await accept loop (`JoinSet` drains all in-flight client tasks) → `hub.close_all()` (safety net) → consume and metrics handles.
- Mutex poison is surfaced as a typed `WsError::SchemaRegistry` rather than a panic.
- Prometheus metrics on container port 9093 (host-exposed for browser WebSocket, Prometheus scrapes same port).

### Service: `frontend/` — Turkish-Language SPA (Next.js 14)

- Real-time earthquake map; connects to the WebSocket service; REST fallback to `services/api/`.
- Language: TypeScript. Framework: Next.js 14 with `output: 'standalone'` for minimal Docker image.
- **Map library (ADR-0002):** `maplibre-gl@4.x` via `react-map-gl@7.1.7`. Chosen over Mapbox GL JS (requires access token, proprietary license, per-request billing) and Leaflet (weaker real-time GeoJSON support). Tiles from CartoCDN dark-matter style — no API token required.
- Events rendered as a GeoJSON FeatureCollection with two `circle` layers (live-ring + base), not individual `<Marker>` components — scales to thousands of events with a single WebGL draw call.
- Map default view: longitude 35.0, latitude 39.0, zoom 4.5 (centred on Turkey).
- CORS: `next.config.ts` rewrites `/api/v1/*` to the Rust API service, eliminating CORS configuration on the Rust side. SSR detail pages call the API directly via the internal Docker network (`API_URL` env var).
- `maplibre-gl` accesses `window` at module load time and cannot SSR; all map components use `dynamic(() => import(...), { ssr: false })`.
- WebSocket auto-reconnect at 5 s fixed delay. **TECH DEBT:** No exponential backoff. Acceptable for Phase 2 single-instance deployment. Must add backoff with jitter before Phase 3 rolling restarts.
- Alert banners auto-dismissed after 60 s; timer restarted on each new alert.
- Magnitude colour coding matches alert levels from `services/analysis/src/analysis/risk.py`: green (< M3), yellow (M3–5), orange (M5–7), red (M≥7).
- `NEXT_PUBLIC_WS_URL` is embedded in the bundle at build time; defaults to `ws://localhost:9093`. **Must be overridden** as a Docker build ARG for Vultr production deployment.
- Custom SVG bar chart in `RegionalStats.tsx`; recharts dependency rejected on bundle size grounds (~350 KB compressed).
- Container: multi-stage `node:20-alpine`; non-root `nextjs` user uid 1001; exposed on host port 3001 (Grafana occupies 3000).

### Infrastructure Updates — Phase 2

- `docker-compose.yml`: added `seismosis-analysis` (port 9092), `seismosis-websocket` (port 9093), `seismosis-frontend` (port 3001).
- `config/prometheus/prometheus.yml`: added scrape jobs for `seismosis-analysis:9092` and `seismosis-websocket:9093`, both at 10 s scrape interval.
- Phase 1 dead code removed: `ApiError::Database`, `ApiError::Redis`, `RequestError::Cache` variants (never constructed) from `services/api/src/error.rs`; unused `pipeline_version` field from `services/storage/src/config.rs`.

### Key Architecture Decisions — Phase 2

1. **ADR-0001:** WebSocket Kafka offset management — manual store with 5 s auto-commit timer. See `docs/adr/ADR-0001-websocket-kafka-offset-management.md`.
2. **ADR-0002:** Frontend map library — maplibre-gl over Mapbox GL JS. See `docs/adr/ADR-0002-frontend-maplibre-over-mapbox.md`.
3. **INSERT ON CONFLICT for enrichment** — storage service and analysis service consume `earthquakes.raw` independently; upsert on enrichment columns only; no inter-service coordination required.
4. **Synchronous threading model in Python** — `confluent-kafka` `poll()` is blocking; `asyncio` would add `run_in_executor` complexity with no benefit.
5. **No recharts dependency** — custom SVG bar chart for the one-dimensional magnitude distribution; rejects a ~350 KB bundle addition.

---

## Open Items Carried into Phase 3

| Item | File | Owner | Blocking |
|------|------|-------|----------|
| Memory limits (`deploy.resources.limits.memory`) missing from all containers in `docker-compose.yml` — hard blocker for Vultr production deployment | `docker-compose.yml` | Infrastructure lead | Yes |
| End-to-end full-stack integration test not yet run (analysis + WebSocket + frontend together) | — | Phase 2 lead | Yes (before Phase 2 declared complete) |
| `NEXT_PUBLIC_WS_URL` must be overridden for Vultr deployment (currently defaults to `ws://localhost:9093`) | `frontend/Dockerfile`, `docker-compose.yml` | Infrastructure lead | Yes |
| Grafana dashboard not updated for `seismosis_analysis_*` (9 metrics) or `seismosis_websocket_*` (8 metrics) | `config/grafana/dashboards/earthquake_overview.json` | Phase 2 lead | Before production launch |
| `SignificantEarthquakeDetected` alert annotation text reads "M≥5.0" but fires at M≥7.0 (`magnitude_class="major"`) | `config/prometheus/rules/earthquake_alerts.yml` | Ops team | No (annotation only; expression is correct) |
| `seismology.event_station_associations` table created in `02_schema.sql` but never populated by any service | `config/postgres/init/02_schema.sql` | Phase 2/3 lead | No |
| Gardner-Knopoff spatial window uses fixed 111.32 km/degree approximation — degrades at high latitudes | `services/analysis/src/analysis/aftershock.py` | Phase 3 analysis lead | No (Phase 2 dataset bounded) |
| WebSocket auto-reconnect fixed 5 s delay — no exponential backoff | `frontend/src/hooks/useWebSocket.ts` | Frontend lead | No (acceptable for single-instance Phase 2) |
| API `/metrics` served on public port 8000 — should move to an internal-only port | `services/api/src/main.rs` | Phase 3 | No |
