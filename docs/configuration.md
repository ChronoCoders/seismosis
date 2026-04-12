# Configuration Reference

All Seismosis services are configured exclusively through environment variables. Defaults are chosen for local `docker compose` development. Production deployments must override at minimum `POSTGRES_PASSWORD`, `GRAFANA_ADMIN_PASSWORD`, `DATABASE_URL`, and any secret values.

See `.env.example` at the repository root for the full list of variables. The sections below document every non-obvious value, including operational impact of changes.

---

## Infrastructure (docker-compose.yml / .env)

These variables are consumed by the Docker Compose infrastructure services, not by the application services directly.

| Variable | Default | Notes |
|----------|---------|-------|
| `POSTGRES_DB` | `seismosis` | Database name created on first boot |
| `POSTGRES_USER` | `seismosis` | Application user; also used by `postgres-exporter` DATA_SOURCE_NAME |
| `POSTGRES_PASSWORD` | *(no default — must set)* | Must be set before first `docker compose up`. Minimum 20 characters in production. Used in `DATABASE_URL` interpolation. |
| `REDIS_PASSWORD` | *(empty — auth disabled)* | If set, must match `requirepass` in `config/redis/redis.conf`. Empty disables Redis AUTH. Dev only with empty value. |
| `GRAFANA_ADMIN_USER` | `admin` | Initial Grafana admin username |
| `GRAFANA_ADMIN_PASSWORD` | *(no default — must set)* | Must be set before first `docker compose up`. Grafana rejects passwords too similar to the username. |
| `LOG_LEVEL` | `info` | Applies to all services: `trace`, `debug`, `info`, `warn`, `error`. `debug` produces high log volume and should not be used in production. |
| `PROMETHEUS_REMOTE_WRITE_URL` | *(empty — disabled)* | Set to a Prometheus-compatible remote write endpoint to enable long-term metric storage. Not configured in Phase 1. |
| `ENABLE_ALERT_NOTIFICATIONS` | `false` | Feature flag. Alert notifications (SMS, email, PagerDuty) are a Phase 2 feature; this flag is a placeholder. |
| `ENABLE_REDIS_CACHE` | `true` | Feature flag. Setting to `false` disables cache reads/writes in the API service; all requests fall through to PostgreSQL. Useful for debugging cache-related issues. |
| `ENABLE_SCHEMA_VALIDATION` | `true` | Feature flag. Setting to `false` would disable Avro schema validation in the ingestion service. Do not disable in production. |

---

## services/ingestion

| Variable | Default | Notes |
|----------|---------|-------|
| `KAFKA_BROKERS` | `redpanda:9092` | Comma-separated broker list |
| `KAFKA_TOPIC_RAW` | `earthquakes.raw` | Destination topic for normalised events |
| `KAFKA_TOPIC_DEAD_LETTER` | `earthquakes.dead-letter` | Destination for rejected events |
| `SCHEMA_REGISTRY_URL` | `http://redpanda:8081` | Confluent-compatible Schema Registry |
| `REDIS_URL` | `redis://redis:6379` | Dedup store; service degrades to in-memory LRU on failure |
| `DEDUP_TTL_SECS` | `604800` | 7 days — matches `earthquakes.raw` topic retention |
| `SOURCE_POLL_INTERVAL_SECS` | `60` | How often to poll USGS and EMSC APIs |
| `SOURCE_LOOKBACK_SECS` | `600` | 10-minute lookback per poll to catch late-arriving source events |
| `MIN_MAGNITUDE_INGEST` | `2.0` | Events below this magnitude are dropped before publish |
| `METRICS_PORT` | `9091` | Prometheus scrape port |
| `USGS_API_BASE_URL` | `https://earthquake.usgs.gov/fdsnws/event/1` | USGS FDSNWS endpoint |
| `EMSC_FEED_URL` | `https://www.seismicportal.eu/fdsnws/event/1` | EMSC FDSNWS endpoint |
| `HTTP_TIMEOUT_SECS` | `30` | Per-request HTTP timeout to source APIs |
| `HTTP_MAX_RETRIES` | `3` | Retry attempts for HTTP source fetches |
| `KAFKA_MAX_RETRIES` | `3` | Retry attempts for Kafka produce calls |
| `KAFKA_PRODUCER_ACKS` | `all` | Requires full ISR acknowledgment for `earthquakes.raw` |
| `KAFKA_MESSAGE_TIMEOUT_MS` | `30000` | Max ms to await broker delivery ack before declaring failure |
| `PIPELINE_VERSION` | `CARGO_PKG_VERSION` | Injected into every Avro record for traceability |

## services/storage

| Variable | Default | Notes |
|----------|---------|-------|
| `KAFKA_BROKERS` | `redpanda:9092` | |
| `KAFKA_TOPIC_RAW` | `earthquakes.raw` | Topic consumed by this service |
| `KAFKA_TOPIC_DEAD_LETTER` | `earthquakes.dead-letter` | Failed messages routed here |
| `KAFKA_GROUP_ID` | `seismosis-storage-group` | Consumer group; change with care — offset reset required |
| `SCHEMA_REGISTRY_URL` | `http://redpanda:8081` | |
| `DATABASE_URL` | `postgres://seismosis:changeme@postgres:5432/seismosis` | Full connection string |
| `DB_MAX_CONNECTIONS` | `10` | sqlx PgPool max size |
| `DB_CONNECT_TIMEOUT_SECS` | `30` | Timeout for acquiring a DB connection |
| `METRICS_PORT` | `9090` | Prometheus scrape port |
| `PIPELINE_VERSION` | `CARGO_PKG_VERSION` | |

## services/api

| Variable | Default | Notes |
|----------|---------|-------|
| `DATABASE_URL` | `postgres://seismosis:seismosis@postgres:5432/seismosis` | |
| `DB_MAX_CONNECTIONS` | `20` | Higher default than storage because API handles concurrent HTTP requests |
| `REDIS_URL` | `redis://redis:6379` | Cache; failures logged as warnings, requests fall through to PostgreSQL |
| `HTTP_PORT` | `8000` | Axum HTTP server bind port; also serves `/metrics` in Phase 1 |
| `REQUEST_TIMEOUT_SECS` | `30` | Tower `TimeoutLayer` — returns 408 after this duration |
| `EVENT_CACHE_TTL_SECS` | `300` | Redis TTL for per-event cache entries (`api:event:{source_id}`) |
| `STATS_CACHE_TTL_SECS` | `60` | Redis TTL for the stats cache entry (`api:stats`) |

---

## config/redis/redis.conf

Non-obvious settings with operational impact. Full file at `config/redis/redis.conf`.

| Setting | Value | Notes |
|---------|-------|-------|
| `maxmemory` | `512mb` | Sized for ~200K recent event records at ~2 KB each (~400 MB operational). Raise before production if event volume grows. |
| `maxmemory-policy` | `allkeys-lru` | On memory pressure, evicts the least-recently-used key regardless of whether it has a TTL. Correct policy for a pure cache use case. Changing to `noeviction` would cause write failures instead of evictions. |
| `appendonly yes` + `appendfsync everysec` | — | AOF enabled with per-second fsync. Protects against up to ~1 s of data loss on crash. Combined with RDB (`aof-use-rdb-preamble yes`) for fast restarts. |
| `requirepass ""` | *(empty)* | Auth disabled by default for development. Production: set `REDIS_PASSWORD` in `.env` and configure `requirepass` in this file (or inject via Docker entrypoint). |
| `notify-keyspace-events "Ex"` | — | Publishes keyspace notifications for expired keys via Pub/Sub. Enables cache invalidation callbacks without polling. Only expired-key events are published (not all keyspace events) to avoid overhead. |
| `activedefrag yes` | — | Active memory defragmentation using jemalloc. Reduces fragmentation ratio over time at a small background CPU cost. Requires the official Redis Docker image (jemalloc allocator). |
| `hz 20` | — | Background task frequency (times/second). Default is 10; raised to 20 for more responsive active key expiry under high TTL churn. Increase has modest CPU cost. |
| `slowlog-log-slower-than 10000` | 10 ms | Commands slower than 10 ms are recorded in the slow log ring buffer (max 256 entries). Inspect with `SLOWLOG GET` to identify pathological access patterns. |

---

## config/prometheus/prometheus.yml

| Setting | Value | Notes |
|---------|-------|-------|
| `scrape_interval` (global) | `15s` | Default scrape frequency for all jobs not overridden. |
| `evaluation_interval` | `15s` | How often alert rules are evaluated against collected data. |
| `scrape_interval` (application services) | `10s` | Ingestion, storage, and API services scraped at 10 s for finer-grained pipeline metrics. |
| `scrape_interval` (kafka-exporter) | `30s` | Consumer lag metrics do not need sub-15 s resolution; 30 s reduces overhead. |
| `metric_relabel_configs` (redpanda job) | drops `redpanda_scheduler_.*` and `redpanda_reactor_.*` | These Redpanda-internal metrics are high-cardinality and not useful for platform observability. Dropping them at scrape time prevents TSDB bloat. |
| Alertmanager | `alertmanagers: []` | No Alertmanager in Phase 1. Alerts are visible in the Prometheus UI and surfaced via Grafana unified alerting. External notifications (PagerDuty, email) are Phase 2. |
