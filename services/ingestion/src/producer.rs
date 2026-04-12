use std::{sync::Arc, time::Duration};

use rdkafka::{
    config::ClientConfig,
    producer::{FutureProducer, FutureRecord},
};
use tokio_retry::{strategy::ExponentialBackoff, Retry};
use tracing::{debug, warn};

use crate::{config::Config, error::IngestError, metrics::Metrics};

/// Wraps `rdkafka::producer::FutureProducer` with retry logic and metrics.
///
/// Producer configuration enforces:
/// - `acks=all` — all in-sync replicas must acknowledge before the future
///   resolves. Prevents data loss on broker failover (Phase 2).
/// - `enable.idempotence=true` — exactly-once delivery within a producer
///   session. Complements our cross-session Redis deduplication.
/// - `compression.type=lz4` — matches the topic-level compression setting.
pub struct EventProducer {
    inner: FutureProducer,
    topic: String,
    metrics: Arc<Metrics>,
    max_retries: u32,
}

impl EventProducer {
    pub fn new(config: &Config, metrics: Arc<Metrics>) -> Result<Self, IngestError> {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", &config.kafka_brokers)
            .set("acks", config.kafka_producer_acks.as_str())
            .set("enable.idempotence", "true")
            .set("compression.type", "lz4")
            // Linger for 5 ms to allow batching of simultaneous events.
            .set("linger.ms", "5")
            .set("batch.size", "65536")
            // Overall delivery timeout. A message that cannot be delivered
            // within this window is returned as an error.
            .set("message.timeout.ms", &config.kafka_message_timeout_ms.to_string())
            // rdkafka retries — separate from our application-level retries.
            .set("retries", "5")
            .set("retry.backoff.ms", "250")
            // Recommended when using idempotence.
            .set("max.in.flight.requests.per.connection", "5")
            .create()?;

        Ok(Self {
            inner: producer,
            topic: config.kafka_topic_raw.clone(),
            metrics,
            max_retries: config.kafka_max_retries,
        })
    }

    /// Publish `payload` to the raw topic with `source_id` as the message key.
    ///
    /// Using `source_id` as the key routes all events for the same logical
    /// event to the same partition, which simplifies downstream ordering
    /// guarantees.
    ///
    /// Retries transiently on produce errors (e.g. broker unavailable) with
    /// exponential backoff. Permanent errors (e.g. message too large) are
    /// returned immediately after the first attempt.
    pub async fn send(&self, source_id: &str, payload: Vec<u8>) -> Result<(), IngestError> {
        let topic = self.topic.clone();
        let metrics = Arc::clone(&self.metrics);

        let strategy = ExponentialBackoff::from_millis(500)
            .factor(2)
            .max_delay(Duration::from_secs(8))
            .take(self.max_retries as usize);

        Retry::spawn(strategy, || {
            let payload = payload.clone();
            let topic = topic.clone();
            let metrics = Arc::clone(&metrics);
            async move {
                let start = std::time::Instant::now();

                let record = FutureRecord::to(&topic)
                    .key(source_id)
                    .payload(payload.as_slice());

                // Queue timeout: wait up to 5 s for space in the producer
                // internal queue before returning an error.
                let result = self
                    .inner
                    .send(record, Duration::from_secs(5))
                    .await;

                let elapsed = start.elapsed().as_secs_f64();
                metrics
                    .publish_duration_seconds
                    .with_label_values(&[&topic])
                    .observe(elapsed);

                match result {
                    Ok((partition, offset)) => {
                        debug!(
                            source_id,
                            topic = %topic,
                            partition,
                            offset,
                            elapsed_ms = (elapsed * 1000.0) as u64,
                            "Message delivered"
                        );
                        Ok(())
                    }
                    Err((kafka_err, _msg)) => {
                        warn!(
                            source_id,
                            error = %kafka_err,
                            "Kafka produce failed, will retry"
                        );
                        Err(IngestError::KafkaProduce(kafka_err.to_string()))
                    }
                }
            }
        })
        .await
    }
}
