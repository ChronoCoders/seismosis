/// Integration tests for the produce + dedup protocol.
///
/// These tests start real Docker containers (Kafka and Redis) via testcontainers.
/// Docker must be running on the host. They are compiled only in test mode.
///
/// Run with:
///   cargo test --test-threads=1    # containers share the Docker daemon; serialise to avoid port conflicts
#[cfg(test)]
mod tests {
    use std::time::Duration;

    use apache_avro::Schema;
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::{kafka::Kafka, redis::Redis};

    use crate::{
        avro::AvroEncoder,
        config::Config,
        dedup::Deduplicator,
        metrics::Metrics,
        producer::EventProducer,
        schema::{RawEarthquakeEvent, AVRO_SCHEMA},
    };

    // ─── Helpers ─────────────────────────────────────────────────────────────

    fn test_event(source_id: &str) -> RawEarthquakeEvent {
        RawEarthquakeEvent {
            source_id: source_id.to_owned(),
            source_network: "USGS".to_owned(),
            event_time_ms: 1_700_000_000_000,
            latitude: 37.5,
            longitude: -122.1,
            depth_km: Some(10.0),
            magnitude: 4.5,
            magnitude_type: "ML".to_owned(),
            region_name: Some("Test Region".to_owned()),
            quality_indicator: "A".to_owned(),
            raw_payload: r#"{"test":true}"#.to_owned(),
            ingested_at_ms: 1_700_000_001_000,
            pipeline_version: "integration-test".to_owned(),
        }
    }

    fn test_config(kafka_brokers: &str, topic: &str) -> Config {
        Config {
            kafka_brokers: kafka_brokers.to_owned(),
            kafka_topic_raw: topic.to_owned(),
            // Use acks=1 in tests: we have a single broker and don't need
            // full ISR acknowledgment to verify produce correctness.
            kafka_producer_acks: "1".to_owned(),
            kafka_max_retries: 1,
            kafka_message_timeout_ms: 30_000,
            schema_registry_url: "http://localhost:8081".to_owned(),
            redis_url: "redis://localhost:6379".to_owned(),
            dedup_ttl_secs: 60,
            poll_interval: Duration::from_secs(60),
            lookback_window: Duration::from_secs(60),
            min_magnitude: 2.0,
            metrics_port: 0,
            usgs_api_url: "https://example.com".to_owned(),
            emsc_api_url: "https://example.com".to_owned(),
            http_timeout: Duration::from_secs(10),
            http_max_retries: 1,
            pipeline_version: "integration-test".to_owned(),
        }
    }

    /// Config variant for dead-broker tests. Sets a short delivery timeout and
    /// zero application-level retries so the test fails fast instead of hanging
    /// for 30+ seconds waiting for librdkafka to exhaust its delivery window.
    fn dead_broker_config() -> Config {
        Config {
            kafka_brokers: "127.0.0.1:19099".to_owned(),
            kafka_topic_raw: "non-existent-topic".to_owned(),
            kafka_producer_acks: "1".to_owned(),
            kafka_max_retries: 0,
            kafka_message_timeout_ms: 1_000, // fail fast — broker is unreachable
            schema_registry_url: "http://localhost:8081".to_owned(),
            redis_url: "redis://localhost:6379".to_owned(),
            dedup_ttl_secs: 60,
            poll_interval: Duration::from_secs(60),
            lookback_window: Duration::from_secs(60),
            min_magnitude: 2.0,
            metrics_port: 0,
            usgs_api_url: "https://example.com".to_owned(),
            emsc_api_url: "https://example.com".to_owned(),
            http_timeout: Duration::from_secs(10),
            http_max_retries: 1,
            pipeline_version: "integration-test".to_owned(),
        }
    }

    async fn create_topic(brokers: &str, topic: &str) {
        use rdkafka::{
            admin::{AdminClient, AdminOptions, NewTopic, TopicReplication},
            client::DefaultClientContext,
            config::ClientConfig,
        };

        let admin: AdminClient<DefaultClientContext> = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .create()
            .expect("Failed to create Kafka admin client");

        let new_topic = NewTopic::new(topic, 1, TopicReplication::Fixed(1));
        let opts = AdminOptions::new()
            .operation_timeout(Some(Duration::from_secs(15)));

        let results = admin
            .create_topics(&[new_topic], &opts)
            .await
            .expect("Admin create_topics call failed");

        for result in results {
            result.expect("Topic creation returned an error");
        }
    }

    // ─── Tests ───────────────────────────────────────────────────────────────

    /// Verify the Redis-backed dedup protocol with a real Redis instance.
    ///
    /// Exercises the full check-then-mark sequence documented on `Deduplicator`.
    #[tokio::test]
    async fn dedup_redis_protocol() {
        let redis = Redis.start().await.expect("Failed to start Redis container");
        let port = redis
            .get_host_port_ipv4(6379)
            .await
            .expect("Failed to get Redis port");
        let redis_url = format!("redis://127.0.0.1:{}", port);

        let dedup = Deduplicator::new(&redis_url, 60).await;
        assert!(dedup.is_redis_healthy(), "Redis should be reachable after container start");

        // A fresh key is not a duplicate.
        assert!(!dedup.is_duplicate("USGS:int_dedup_001").await);

        // After mark_seen, the same key is a duplicate.
        dedup.mark_seen("USGS:int_dedup_001").await;
        assert!(dedup.is_duplicate("USGS:int_dedup_001").await);

        // A different key is still fresh.
        assert!(!dedup.is_duplicate("USGS:int_dedup_002").await);
    }

    /// Verify the full produce → mark protocol against real Kafka and Redis.
    ///
    /// This is the critical path: check → encode → produce → mark_seen.
    /// After a successful produce, the event must be flagged as a duplicate.
    #[tokio::test]
    async fn produce_and_dedup_full_protocol() {
        // ── Containers ────────────────────────────────────────────────────
        let kafka = Kafka::default()
            .start()
            .await
            .expect("Failed to start Kafka container");
        let kafka_port = kafka
            .get_host_port_ipv4(9093)
            .await
            .expect("Failed to get Kafka bootstrap port");
        let brokers = format!("127.0.0.1:{}", kafka_port);

        let redis = Redis
            .start()
            .await
            .expect("Failed to start Redis container");
        let redis_port = redis
            .get_host_port_ipv4(6379)
            .await
            .expect("Failed to get Redis port");
        let redis_url = format!("redis://127.0.0.1:{}", redis_port);

        // ── Topic setup ───────────────────────────────────────────────────
        let topic = "int-earthquakes-raw";
        create_topic(&brokers, topic).await;

        // ── Service objects ───────────────────────────────────────────────
        let dedup = Deduplicator::new(&redis_url, 60).await;
        let config = test_config(&brokers, topic);
        let metrics = Metrics::new().expect("Failed to create metrics");
        let producer = EventProducer::new(&config, metrics).expect("Failed to create producer");
        let avro_schema = Schema::parse_str(AVRO_SCHEMA).expect("Avro schema parse failed");
        let encoder = AvroEncoder::new(avro_schema, 1);

        let event = test_event("USGS:int_produce_001");

        // ── Step 1: check (read-only) ─────────────────────────────────────
        assert!(
            !dedup.is_duplicate(&event.source_id).await,
            "Event must not be a duplicate before first produce"
        );

        // ── Step 2: encode ────────────────────────────────────────────────
        let payload = encoder.encode(&event).expect("Avro encoding failed");

        // ── Step 3: produce ───────────────────────────────────────────────
        producer
            .send(&event.source_id, payload)
            .await
            .expect("Kafka produce failed");

        // ── Step 4: mark seen (only after successful produce) ─────────────
        dedup.mark_seen(&event.source_id).await;

        // ── Verify: event is now a duplicate ──────────────────────────────
        assert!(
            dedup.is_duplicate(&event.source_id).await,
            "Event must be a duplicate after produce + mark_seen"
        );
    }

    /// Verify that a failed produce does NOT commit the dedup key.
    ///
    /// This is the critical correctness invariant: if produce fails, the event
    /// must be retryable on the next poll cycle — the dedup key must be absent.
    #[tokio::test]
    async fn produce_failure_does_not_commit_dedup_key() {
        // Use a non-existent broker so every produce attempt fails.
        let redis = Redis
            .start()
            .await
            .expect("Failed to start Redis container");
        let redis_port = redis
            .get_host_port_ipv4(6379)
            .await
            .expect("Failed to get Redis port");
        let redis_url = format!("redis://127.0.0.1:{}", redis_port);

        let dedup = Deduplicator::new(&redis_url, 60).await;

        // Use the fast-fail config: 0 retries + 1 s delivery timeout so the
        // test completes in ~1 s instead of the default 30 s window.
        let config = dead_broker_config();
        let metrics = Metrics::new().expect("Failed to create metrics");
        let producer = EventProducer::new(&config, metrics).expect("Failed to create producer");
        let avro_schema = Schema::parse_str(AVRO_SCHEMA).expect("Avro schema parse failed");
        let encoder = AvroEncoder::new(avro_schema, 1);

        let event = test_event("USGS:int_fail_001");

        // Not a duplicate initially.
        assert!(!dedup.is_duplicate(&event.source_id).await);

        // Produce to dead broker — must fail.
        let payload = encoder.encode(&event).expect("Avro encoding failed");
        let result = producer.send(&event.source_id, payload).await;
        assert!(result.is_err(), "Produce to dead broker must fail");

        // mark_seen is NOT called on failure (as in the poll loop).
        // The dedup key must still be absent — event can be retried.
        assert!(
            !dedup.is_duplicate(&event.source_id).await,
            "Dedup key must not be written after a failed produce"
        );
    }
}
