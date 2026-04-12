use std::env;

/// All configuration is sourced from environment variables with documented
/// defaults. See `.env.example` at the project root for descriptions.
#[derive(Debug, Clone)]
pub struct Config {
    /// Redpanda/Kafka bootstrap servers (comma-separated).
    pub kafka_brokers: String,

    /// Topic carrying enriched earthquake events.
    pub kafka_topic_enriched: String,

    /// Topic carrying M ≥ threshold alert events.
    pub kafka_topic_alerts: String,

    /// Consumer group ID. Use a unique group so this service gets its own
    /// offset pointer and does not interfere with storage/analysis consumers.
    pub kafka_group_id: String,

    /// Schema Registry base URL.
    pub schema_registry_url: String,

    /// TCP port on which the WebSocket server listens for client connections.
    pub ws_port: u16,

    /// TCP port for the Prometheus `/metrics` and `/health` endpoints.
    pub metrics_port: u16,

    /// Capacity of the per-client outbound message channel. If the buffer is
    /// full when a message arrives, the message is dropped for that client and
    /// `seismosis_websocket_messages_dropped_total` is incremented.
    pub client_channel_capacity: usize,

    /// Default minimum `ml_magnitude` applied when a connecting client does
    /// not specify `min_magnitude` in its query parameters.
    pub default_min_magnitude: f64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            kafka_brokers: var_str("KAFKA_BROKERS", "redpanda:9092"),
            kafka_topic_enriched: var_str("KAFKA_TOPIC_ENRICHED", "earthquakes.enriched"),
            kafka_topic_alerts: var_str("KAFKA_TOPIC_ALERTS", "earthquakes.alerts"),
            kafka_group_id: var_str("KAFKA_GROUP_ID", "seismosis-websocket"),
            schema_registry_url: var_str("SCHEMA_REGISTRY_URL", "http://redpanda:8081"),
            ws_port: var_u16("WS_PORT", 9093)?,
            metrics_port: var_u16("METRICS_PORT", 9094)?,
            client_channel_capacity: var_usize("CLIENT_CHANNEL_CAPACITY", 128)?,
            default_min_magnitude: var_f64("DEFAULT_MIN_MAGNITUDE", 0.0)?,
        })
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn var_str(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn var_u16(name: &'static str, default: u16) -> anyhow::Result<u16> {
    match env::var(name) {
        Ok(v) => v
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("config {}: '{}' is not a valid port number", name, v)),
        Err(_) => Ok(default),
    }
}

fn var_usize(name: &'static str, default: usize) -> anyhow::Result<usize> {
    match env::var(name) {
        Ok(v) => v
            .parse::<usize>()
            .map_err(|_| anyhow::anyhow!("config {}: '{}' cannot be parsed as usize", name, v)),
        Err(_) => Ok(default),
    }
}

fn var_f64(name: &'static str, default: f64) -> anyhow::Result<f64> {
    match env::var(name) {
        Ok(v) => {
            let f = v
                .parse::<f64>()
                .map_err(|_| anyhow::anyhow!("config {}: '{}' cannot be parsed as f64", name, v))?;
            if !f.is_finite() {
                return Err(anyhow::anyhow!(
                    "config {}: '{}' is not a finite number",
                    name, v
                ));
            }
            Ok(f)
        }
        Err(_) => Ok(default),
    }
}
