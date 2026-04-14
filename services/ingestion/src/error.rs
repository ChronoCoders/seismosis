use thiserror::Error;

/// Fatal errors that abort the service startup or poll loop.
#[derive(Debug, Error)]
pub enum IngestError {
    #[error("HTTP request failed for source '{src}': {inner}")]
    HttpFetch {
        src: &'static str,
        #[source]
        inner: reqwest::Error,
    },

    #[error("empty response body from source '{src}'")]
    EmptyResponse { src: &'static str },

    #[error("JSON deserialisation error from source '{src}' (event '{event_id}'): {inner}")]
    JsonParse {
        src: &'static str,
        event_id: String,
        #[source]
        inner: serde_json::Error,
    },

    #[error("Avro encoding error: {0}")]
    AvroCoding(#[from] apache_avro::Error),

    #[error("Schema Registry error ({url}): {detail}")]
    SchemaRegistry { url: String, detail: String },

    #[error("Kafka produce error: {0}")]
    KafkaProduce(String),

    #[error("Kafka client creation error: {0}")]
    KafkaClient(#[from] rdkafka::error::KafkaError),

    #[error("Deduplication store error: {0}")]
    DedupStore(String),

    #[error("Configuration error — field '{field}': {detail}")]
    Config { field: &'static str, detail: String },

    #[error("HTTP server error: {0}")]
    HttpServer(String),
}

/// Non-fatal parsing errors — logged and the event is skipped.
/// These do not abort the poll loop.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("missing required field '{field}' in {src} event '{event_id}'")]
    MissingField {
        field: &'static str,
        src: &'static str,
        event_id: String,
    },

    #[error("invalid value for field '{field}' in {src} event '{event_id}': {detail}")]
    InvalidField {
        field: &'static str,
        src: &'static str,
        event_id: String,
        detail: String,
    },

    #[error("event filtered out ({reason}) — {src} event '{event_id}'")]
    Filtered {
        reason: &'static str,
        src: &'static str,
        event_id: String,
    },
}
