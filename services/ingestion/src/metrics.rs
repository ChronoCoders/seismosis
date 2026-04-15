use prometheus::{CounterVec, HistogramOpts, HistogramVec, Opts, Registry};
use std::sync::Arc;

/// All Prometheus metrics for the ingestion service.
///
/// Naming convention: `seismosis_ingestion_{metric}_{unit}`.
/// Labels follow the OpenMetrics cardinality guideline — no unbounded labels.
pub struct Metrics {
    /// Total events received from source APIs (before dedup/validation).
    pub events_received_total: CounterVec,

    /// Events successfully published to the Kafka topic.
    pub events_published_total: CounterVec,

    /// Events skipped because the source_id was already in the dedup store.
    pub events_deduplicated_total: CounterVec,

    /// Events rejected before publish (parse error, magnitude filter, etc.).
    /// `reason` label: "parse_error" | "magnitude_filter" | "non_earthquake" | "missing_field"
    pub events_rejected_total: CounterVec,

    /// Histogram of full source poll round-trip duration (per source).
    pub poll_duration_seconds: HistogramVec,

    /// Histogram of Kafka publish call duration (end-to-end, including ack wait).
    pub publish_duration_seconds: HistogramVec,

    /// Schema Registry registration request outcomes.
    /// `status` label: "ok" | "error"
    pub schema_registry_requests_total: CounterVec,

    /// Whether the dedup Redis backend is healthy (1) or falling back to LRU (0).
    /// Reported as a counter-based proxy — incremented each poll cycle.
    pub dedup_redis_fallback_total: CounterVec,

    /// Backing registry — used by the metrics HTTP handler.
    pub registry: Registry,
}

impl Metrics {
    pub fn new() -> Result<Arc<Self>, prometheus::Error> {
        let registry = Registry::new_custom(None, None)?;

        macro_rules! register {
            ($metric:expr) => {{
                let m = $metric;
                registry.register(Box::new(m.clone()))?;
                m
            }};
        }

        let events_received_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_ingestion_events_received_total",
                "Total events received from source APIs",
            ),
            &["source"],
        )?);

        let events_published_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_ingestion_events_published_total",
                "Events successfully published to the raw Kafka topic",
            ),
            &["source"],
        )?);

        let events_deduplicated_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_ingestion_events_deduplicated_total",
                "Events skipped due to source_id already present in dedup store",
            ),
            &["source"],
        )?);

        let events_rejected_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_ingestion_events_rejected_total",
                "Events rejected before publish (parse errors, filters, etc.)",
            ),
            &["source", "reason"],
        )?);

        let poll_duration_seconds = register!(HistogramVec::new(
            HistogramOpts::new(
                "seismosis_ingestion_poll_duration_seconds",
                "End-to-end duration of a single source poll cycle",
            )
            .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 20.0, 30.0, 60.0]),
            &["source"],
        )?);

        let publish_duration_seconds = register!(HistogramVec::new(
            HistogramOpts::new(
                "seismosis_ingestion_publish_duration_seconds",
                "Kafka publish call duration including broker acknowledgment",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0
            ]),
            &["topic"],
        )?);

        let schema_registry_requests_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_ingestion_schema_registry_requests_total",
                "Schema Registry HTTP requests",
            ),
            &["status"],
        )?);

        let dedup_redis_fallback_total = register!(CounterVec::new(
            Opts::new(
                "seismosis_ingestion_dedup_redis_fallback_total",
                "Number of dedup checks that fell back to the in-memory LRU (Redis unavailable)",
            ),
            &["source"],
        )?);

        Ok(Arc::new(Self {
            events_received_total,
            events_published_total,
            events_deduplicated_total,
            events_rejected_total,
            poll_duration_seconds,
            publish_duration_seconds,
            schema_registry_requests_total,
            dedup_redis_fallback_total,
            registry,
        }))
    }
}
