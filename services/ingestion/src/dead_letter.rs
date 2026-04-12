use std::time::Duration;

use rdkafka::{
    config::ClientConfig,
    producer::{FutureProducer, FutureRecord},
};
use serde::Serialize;
use tracing::{error, warn};

use crate::error::IngestError;

/// Maximum total delivery attempts (1 initial + 3 retries).
const MAX_ATTEMPTS: u32 = 4;

/// JSON envelope written to `earthquakes.dead-letter` for each rejected event.
///
/// Unlike the storage-side DLQ envelope, ingestion events originate from HTTP
/// source APIs rather than Kafka, so there are no partition/offset coordinates.
/// The `raw_payload` field contains the original JSON from the upstream API,
/// which is sufficient for manual investigation and replay.
#[derive(Debug, Serialize)]
pub struct IngestDlqEnvelope<'a> {
    pub failure_reason: &'a str,
    pub failure_detail: String,
    pub source_name: &'a str,
    pub source_id: &'a str,
    /// Original JSON payload from the upstream source API (USGS / EMSC).
    pub raw_payload: &'a str,
}

/// Publishes rejected ingest events to `earthquakes.dead-letter`.
///
/// Uses `acks=1` (best-effort). Delivery is retried with exponential backoff
/// (500 ms → 1 s → 2 s → 4 s, capped at 8 s) up to `MAX_ATTEMPTS` total.
///
/// DLQ delivery failure is **non-fatal** in the ingestion service: rejected
/// events are never marked as seen in the dedup store, so they will be
/// retried on the next poll cycle from the upstream source. The DLQ envelope
/// is advisory — it assists investigation but does not gate event replay.
pub struct DlqProducer {
    inner: FutureProducer,
    topic: String,
}

impl DlqProducer {
    pub fn new(brokers: &str, topic: String) -> Result<Self, IngestError> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("acks", "1")
            // Short timeout — DLQ messages are best-effort; don't stall the poll loop.
            .set("message.timeout.ms", "10000")
            // DLQ volume is low; disable linger to deliver promptly.
            .set("linger.ms", "0")
            .create()?;

        Ok(Self { inner: producer, topic })
    }

    /// Route a rejected event to the dead-letter topic.
    ///
    /// Returns `Ok(())` on successful delivery, `Err(())` after all retry
    /// attempts are exhausted (already logged at ERROR). Callers continue
    /// processing regardless of the return value.
    pub async fn send(&self, envelope: &IngestDlqEnvelope<'_>) -> Result<(), ()> {
        let json = match serde_json::to_vec(envelope) {
            Ok(v) => v,
            Err(e) => {
                error!(
                    error = %e,
                    source_id = envelope.source_id,
                    failure_reason = envelope.failure_reason,
                    "Failed to serialise ingest DLQ envelope — event will not be dead-lettered"
                );
                return Err(());
            }
        };

        let key = envelope.source_id;
        let mut delay = Duration::from_millis(500);

        for attempt in 1..=MAX_ATTEMPTS {
            let record = FutureRecord::to(&self.topic)
                .key(key)
                .payload(json.as_slice());

            match self.inner.send(record, Duration::from_secs(5)).await {
                Ok((dlq_partition, dlq_offset)) => {
                    warn!(
                        failure_reason = envelope.failure_reason,
                        source_id = envelope.source_id,
                        source_name = envelope.source_name,
                        dlq_partition,
                        dlq_offset,
                        "Ingest event routed to dead-letter topic"
                    );
                    return Ok(());
                }
                Err((kafka_err, _)) if attempt < MAX_ATTEMPTS => {
                    warn!(
                        error = %kafka_err,
                        attempt,
                        max_attempts = MAX_ATTEMPTS,
                        delay_ms = delay.as_millis(),
                        "Ingest DLQ delivery attempt failed, retrying"
                    );
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(8));
                }
                Err((kafka_err, _)) => {
                    error!(
                        error = %kafka_err,
                        source_id = envelope.source_id,
                        failure_reason = envelope.failure_reason,
                        attempts = MAX_ATTEMPTS,
                        "Ingest DLQ delivery failed after all attempts"
                    );
                    return Err(());
                }
            }
        }

        unreachable!("loop always returns before exhausting MAX_ATTEMPTS")
    }
}
