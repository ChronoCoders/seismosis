use thiserror::Error;

/// Fatal errors that abort service startup.
#[derive(Debug, Error)]
pub enum WsError {
    #[error("Kafka error: {0}")]
    Kafka(#[from] rdkafka::error::KafkaError),

    #[error("Avro error: {0}")]
    Avro(#[from] apache_avro::Error),

    #[error("Schema Registry error ({url}): {detail}")]
    SchemaRegistry { url: String, detail: String },

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid Confluent wire format: {0}")]
    InvalidWireFormat(String),

    #[error("HTTP server error: {0}")]
    HttpServer(String),
}
