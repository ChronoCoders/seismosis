use std::{env, time::Duration};
use crate::error::IngestError;

/// All configuration is sourced from environment variables with documented
/// defaults. See `.env.example` at the project root for descriptions.
#[derive(Debug, Clone)]
pub struct Config {
    /// Redpanda/Kafka bootstrap servers (comma-separated).
    pub kafka_brokers: String,

    /// Topic to publish raw ingest events to.
    pub kafka_topic_raw: String,

    /// Topic to route rejected / undeliverable events to.
    pub kafka_topic_dead_letter: String,

    /// Schema Registry base URL.
    pub schema_registry_url: String,

    /// Redis URL for the deduplication store.
    pub redis_url: String,

    /// TTL for dedup keys in Redis (seconds). Should equal topic retention.
    pub dedup_ttl_secs: u64,

    /// How often to poll external sources.
    pub poll_interval: Duration,

    /// Lookback window applied to each poll. A 10-minute overlap ensures
    /// late-arriving events from sources are not missed between polls.
    pub lookback_window: Duration,

    /// Minimum magnitude to ingest. Events below this threshold are dropped.
    pub min_magnitude: f64,

    /// TCP port for the Prometheus `/metrics` endpoint.
    pub metrics_port: u16,

    /// USGS FDSNWS base URL (no trailing slash).
    pub usgs_api_url: String,

    /// EMSC FDSNWS base URL (no trailing slash).
    pub emsc_api_url: String,

    /// AFAD earthquake API base URL (no trailing slash, must NOT include `/filter`).
    ///
    /// The ingestion service appends `/filter?…` to this value. Setting this
    /// to the full endpoint URL (e.g. `…/apiv2/event/filter`) will produce
    /// double-suffixed requests (`…/filter/filter?…`) that return 404.
    ///
    /// Correct:   `https://deprem.afad.gov.tr/apiv2/event`
    /// Incorrect: `https://deprem.afad.gov.tr/apiv2/event/filter`
    pub afad_api_url: String,

    /// Per-request HTTP timeout.
    pub http_timeout: Duration,

    /// Maximum retry attempts for HTTP source fetches.
    pub http_max_retries: u32,

    /// Maximum retry attempts for Kafka produce calls.
    /// Separate from `http_max_retries` — these are independent failure domains.
    pub kafka_max_retries: u32,

    /// Kafka producer `acks` setting.
    /// `"all"` (default) requires full ISR acknowledgment for `earthquakes.raw`.
    pub kafka_producer_acks: String,

    /// librdkafka `message.timeout.ms`: maximum time (ms) to await delivery
    /// acknowledgment before declaring a produce attempt failed.
    /// Set low in tests against a dead broker to avoid multi-second hangs.
    pub kafka_message_timeout_ms: u32,

    /// Injected into every produced Avro record for traceability.
    pub pipeline_version: String,
}

impl Config {
    pub fn from_env() -> Result<Self, IngestError> {
        Ok(Self {
            kafka_brokers: var_str("KAFKA_BROKERS", "redpanda:9092"),
            kafka_topic_raw: var_str("KAFKA_TOPIC_RAW", "earthquakes.raw"),
            kafka_topic_dead_letter: var_str("KAFKA_TOPIC_DEAD_LETTER", "earthquakes.dead-letter"),
            schema_registry_url: var_str("SCHEMA_REGISTRY_URL", "http://redpanda:8081"),
            redis_url: var_str("REDIS_URL", "redis://redis:6379"),
            dedup_ttl_secs: var_u64("DEDUP_TTL_SECS", 604_800)?,     // 7 days
            poll_interval: Duration::from_secs(
                var_u64("SOURCE_POLL_INTERVAL_SECS", 60)?,
            ),
            lookback_window: Duration::from_secs(
                var_u64("SOURCE_LOOKBACK_SECS", 600)?,                // 10-min overlap
            ),
            min_magnitude: var_f64("MIN_MAGNITUDE_INGEST", 2.0)?,
            metrics_port: var_u64("METRICS_PORT", 9091)? as u16,
            usgs_api_url: var_str(
                "USGS_API_BASE_URL",
                "https://earthquake.usgs.gov/fdsnws/event/1",
            ),
            emsc_api_url: var_str(
                "EMSC_FEED_URL",
                "https://www.seismicportal.eu/fdsnws/event/1",
            ),
            afad_api_url: var_str(
                "AFAD_API_BASE_URL",
                "https://deprem.afad.gov.tr/apiv2/event",
            ),
            http_timeout: Duration::from_secs(var_u64("HTTP_TIMEOUT_SECS", 30)?),
            http_max_retries: var_u64("HTTP_MAX_RETRIES", 3)? as u32,
            kafka_max_retries: var_u64("KAFKA_MAX_RETRIES", 3)? as u32,
            kafka_producer_acks: var_str("KAFKA_PRODUCER_ACKS", "all"),
            kafka_message_timeout_ms: var_u64("KAFKA_MESSAGE_TIMEOUT_MS", 30_000)? as u32,
            pipeline_version: var_str(
                "PIPELINE_VERSION",
                env!("CARGO_PKG_VERSION"),
            ),
        })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn var_str(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn var_u64(name: &'static str, default: u64) -> Result<u64, IngestError> {
    match env::var(name) {
        Ok(v) => v.parse::<u64>().map_err(|_| IngestError::Config {
            field: name,
            detail: format!("cannot parse '{}' as u64", v),
        }),
        Err(_) => Ok(default),
    }
}

fn var_f64(name: &'static str, default: f64) -> Result<f64, IngestError> {
    match env::var(name) {
        Ok(v) => {
            let f = v.parse::<f64>().map_err(|_| IngestError::Config {
                field: name,
                detail: format!("cannot parse '{}' as f64", v),
            })?;
            // Rust's f64::parse accepts "nan", "inf", "-inf". Reject them
            // explicitly: NaN/Inf in magnitude thresholds would silently
            // corrupt API query strings and filter comparisons.
            if !f.is_finite() {
                return Err(IngestError::Config {
                    field: name,
                    detail: format!("'{}' is not a finite number", v),
                });
            }
            Ok(f)
        }
        Err(_) => Ok(default),
    }
}
