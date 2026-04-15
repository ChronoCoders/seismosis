use std::time::Duration;

use rdkafka::{
    config::ClientConfig,
    producer::{FutureProducer, FutureRecord},
};
use serde::Serialize;
use tracing::{error, warn};

use crate::error::{ProcessError, StorageError};

/// Maximum total delivery attempts (1 initial + 3 retries).
const MAX_ATTEMPTS: u32 = 4;

/// JSON envelope written to `earthquakes.dead-letter` for every failed message.
///
/// The raw payload is hex-encoded so it can be stored verbatim as a JSON
/// string without embedded NUL bytes or non-UTF-8 sequences. The Kafka
/// coordinates allow the original message to be located for manual replay.
#[derive(Debug, Serialize)]
pub struct DeadLetterEnvelope<'a> {
    pub failure_reason: &'a str,
    pub failure_detail: String,
    pub original_topic: &'a str,
    pub kafka_partition: i32,
    pub kafka_offset: i64,
    /// Original Kafka message payload, hex-encoded.
    pub raw_payload_hex: String,
}

/// Publishes failed messages to `earthquakes.dead-letter`.
///
/// Uses `acks=1` (best-effort). Delivery is retried with exponential backoff
/// (500 ms → 1 s → 2 s → 4 s, capped at 8 s) up to `MAX_ATTEMPTS` total
/// attempts. Returns `Ok(())` on success, `Err(())` after all attempts fail.
///
/// Callers must NOT commit the original Kafka offset on `Err(())` — the
/// undelivered message should be reprocessed on the next consumer startup.
pub struct DlqProducer {
    inner: FutureProducer,
    topic: String,
}

impl DlqProducer {
    pub fn new(brokers: &str, topic: String) -> Result<Self, StorageError> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("acks", "1")
            // Short timeout — DLQ messages are fire-and-forget; don't block processing.
            .set("message.timeout.ms", "10000")
            // DLQ volume is low; disable linger to deliver promptly.
            .set("linger.ms", "0")
            .create()
            .map_err(|e| {
                StorageError::KafkaSetup(format!("DLQ producer creation failed: {}", e))
            })?;

        Ok(Self {
            inner: producer,
            topic,
        })
    }

    /// Route a failed message to the dead-letter topic.
    ///
    /// `source_id` is used as the Kafka message key for partitioning when
    /// available. Returns `Ok(())` when the envelope is durably delivered,
    /// `Err(())` when all retry attempts are exhausted (already logged at
    /// ERROR). The caller must not commit the original offset on `Err(())`.
    pub async fn send(
        &self,
        source_id: Option<&str>,
        original_payload: &[u8],
        err: &ProcessError,
        original_topic: &str,
        partition: i32,
        offset: i64,
    ) -> Result<(), ()> {
        let envelope = DeadLetterEnvelope {
            failure_reason: err.failure_reason(),
            failure_detail: err.to_string(),
            original_topic,
            kafka_partition: partition,
            kafka_offset: offset,
            raw_payload_hex: to_hex(original_payload),
        };

        let json = match serde_json::to_vec(&envelope) {
            Ok(v) => v,
            Err(e) => {
                error!(
                    error = %e,
                    "Failed to serialize DLQ envelope — offset will not be committed"
                );
                return Err(());
            }
        };

        let key = source_id.unwrap_or("unknown");
        let mut delay = Duration::from_millis(500);

        for attempt in 1..=MAX_ATTEMPTS {
            let record = FutureRecord::to(&self.topic)
                .key(key)
                .payload(json.as_slice());

            match self.inner.send(record, Duration::from_secs(5)).await {
                Ok((dlq_partition, dlq_offset)) => {
                    warn!(
                        failure_reason = err.failure_reason(),
                        original_topic,
                        original_partition = partition,
                        original_offset = offset,
                        dlq_partition,
                        dlq_offset,
                        "Message routed to dead-letter topic"
                    );
                    return Ok(());
                }
                // Not the final attempt — log and sleep before retrying.
                Err((kafka_err, _)) if attempt < MAX_ATTEMPTS => {
                    warn!(
                        error = %kafka_err,
                        attempt,
                        max_attempts = MAX_ATTEMPTS,
                        delay_ms = delay.as_millis(),
                        "DLQ delivery attempt failed, retrying"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(8));
                }
                // Final attempt failed — give up, do not commit offset.
                Err((kafka_err, _)) => {
                    error!(
                        error = %kafka_err,
                        original_topic,
                        original_partition = partition,
                        original_offset = offset,
                        attempts = MAX_ATTEMPTS,
                        "DLQ delivery failed after all attempts — offset will not be committed"
                    );
                    return Err(());
                }
            }
        }

        unreachable!("loop always returns before exhausting MAX_ATTEMPTS")
    }
}

fn to_hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}
