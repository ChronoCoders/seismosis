use std::{env, time::Duration};

use crate::error::StorageError;

/// All configuration sourced from environment variables with documented defaults.
/// See `.env.example` at the project root for descriptions.
#[derive(Debug, Clone)]
pub struct Config {
    /// Redpanda/Kafka bootstrap servers (comma-separated).
    pub kafka_brokers: String,

    /// Topic to consume raw ingest events from.
    pub kafka_topic_raw: String,

    /// Topic to publish dead-letter events to.
    pub kafka_topic_dead_letter: String,

    /// Kafka consumer group ID.
    pub kafka_group_id: String,

    /// Schema Registry base URL.
    pub schema_registry_url: String,

    /// PostgreSQL connection URL.
    pub database_url: String,

    /// Maximum number of connections in the PgPool.
    pub db_max_connections: u32,

    /// Timeout for acquiring a DB connection from the pool.
    pub db_connect_timeout: Duration,

    /// TCP port for the Prometheus `/metrics` endpoint.
    pub metrics_port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self, StorageError> {
        Ok(Self {
            kafka_brokers: var_str("KAFKA_BROKERS", "redpanda:9092"),
            kafka_topic_raw: var_str("KAFKA_TOPIC_RAW", "earthquakes.raw"),
            kafka_topic_dead_letter: var_str("KAFKA_TOPIC_DEAD_LETTER", "earthquakes.dead-letter"),
            kafka_group_id: var_str("KAFKA_GROUP_ID", "seismosis-storage-group"),
            schema_registry_url: var_str("SCHEMA_REGISTRY_URL", "http://redpanda:8081"),
            database_url: var_str(
                "DATABASE_URL",
                "postgres://seismosis:changeme@postgres:5432/seismosis",
            ),
            db_max_connections: u32::try_from(var_u64("DB_MAX_CONNECTIONS", 10)?).map_err(
                |_| StorageError::Config {
                    field: "DB_MAX_CONNECTIONS",
                    detail: "value exceeds u32::MAX (4294967295)".to_owned(),
                },
            )?,
            db_connect_timeout: Duration::from_secs(var_u64("DB_CONNECT_TIMEOUT_SECS", 30)?),
            metrics_port: u16::try_from(var_u64("METRICS_PORT", 9090)?).map_err(|_| {
                StorageError::Config {
                    field: "METRICS_PORT",
                    detail: "value exceeds u16::MAX (65535)".to_owned(),
                }
            })?,
        })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn var_str(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn var_u64(name: &'static str, default: u64) -> Result<u64, StorageError> {
    match env::var(name) {
        Ok(v) => v.parse::<u64>().map_err(|_| StorageError::Config {
            field: name,
            detail: format!("cannot parse '{}' as u64", v),
        }),
        Err(_) => Ok(default),
    }
}
