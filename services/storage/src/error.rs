use thiserror::Error;

/// Fatal service-level error — triggers shutdown.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("config error: field={field}: {detail}")]
    Config { field: &'static str, detail: String },

    #[error("database connection failed: {0}")]
    Database(#[from] sqlx::Error),

    #[error("kafka setup failed: {0}")]
    KafkaSetup(String),

    #[error("schema registry setup failed: {url}: {detail}")]
    SchemaRegistry { url: String, detail: String },

    #[error("metrics error: {0}")]
    Metrics(String),
}

/// Per-message processing failure — routes the message to the dead-letter topic.
/// The original Kafka offset is committed only after DLQ delivery succeeds;
/// see `consume_loop` in `main.rs` for the commit decision logic.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("invalid confluent wire header: {0}")]
    InvalidWireHeader(String),

    /// `http_status` is `Some` for HTTP-level errors and `None` for network
    /// failures. Used by the retry predicate in `registry.rs` to distinguish
    /// permanent 4xx errors (don't retry) from transient 5xx / network errors.
    #[error("schema registry error: {url}: {detail}")]
    SchemaRegistry {
        url: String,
        /// HTTP status code, or `None` for network-level failures.
        http_status: Option<u16>,
        detail: String,
    },

    #[error("avro decode failed: {0}")]
    AvroDecode(String),

    #[error("raw_payload is not valid JSON: {0}")]
    InvalidPayloadJson(String),

    #[error("validation failed: field={field}: {detail}")]
    Validation { field: &'static str, detail: String },

    #[error("database upsert failed: {0}")]
    DbUpsert(String),
}

impl ProcessError {
    /// Short machine-readable tag written into the DLQ envelope and Prometheus label.
    pub fn failure_reason(&self) -> &'static str {
        match self {
            ProcessError::InvalidWireHeader(_) => "invalid_wire_header",
            ProcessError::SchemaRegistry { .. } => "schema_registry_error",
            ProcessError::AvroDecode(_) => "avro_decode_error",
            ProcessError::InvalidPayloadJson(_) => "invalid_payload_json",
            ProcessError::Validation { .. } => "validation_error",
            ProcessError::DbUpsert(_) => "db_upsert_error",
        }
    }
}
