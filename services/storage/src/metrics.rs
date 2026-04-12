use prometheus::{
    register_counter_vec_with_registry, register_histogram_vec_with_registry, CounterVec,
    HistogramVec, Registry,
};

use crate::error::StorageError;

pub struct Metrics {
    /// Total Kafka messages polled from `earthquakes.raw`.
    pub messages_consumed_total: CounterVec,

    /// Total events written to `seismology.seismic_events`.
    /// Labels: `outcome` = always `"upsert"` (insert/update not distinguished in Phase 1);
    ///         `magnitude_class` = `"minor"` | `"light"` | `"moderate"` | `"strong"` | `"major"`.
    pub events_upserted_total: CounterVec,

    /// Total messages routed to `earthquakes.dead-letter`.
    /// Label `reason` maps to `ProcessError::failure_reason()`.
    pub events_dead_letter_total: CounterVec,

    /// End-to-end per-message latency: decode + validate + upsert.
    pub processing_duration_seconds: HistogramVec,

    /// PostgreSQL `upsert_event` call latency.
    pub db_write_duration_seconds: HistogramVec,
}

impl Metrics {
    pub fn new(registry: &Registry) -> Result<Self, StorageError> {
        let messages_consumed_total = register_counter_vec_with_registry!(
            "seismosis_storage_messages_consumed_total",
            "Total Kafka messages consumed from earthquakes.raw",
            &["topic"],
            registry
        )
        .map_err(|e| StorageError::Metrics(e.to_string()))?;

        let events_upserted_total = register_counter_vec_with_registry!(
            "seismosis_storage_events_upserted_total",
            "Total events successfully upserted into seismology.seismic_events",
            &["outcome", "magnitude_class"],
            registry
        )
        .map_err(|e| StorageError::Metrics(e.to_string()))?;

        let events_dead_letter_total = register_counter_vec_with_registry!(
            "seismosis_storage_events_dead_letter_total",
            "Total events routed to the dead-letter topic, by failure reason",
            &["reason"],
            registry
        )
        .map_err(|e| StorageError::Metrics(e.to_string()))?;

        let processing_duration_seconds = register_histogram_vec_with_registry!(
            "seismosis_storage_processing_duration_seconds",
            "End-to-end per-message processing latency (decode + validate + upsert)",
            &["topic"],
            vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
            registry
        )
        .map_err(|e| StorageError::Metrics(e.to_string()))?;

        let db_write_duration_seconds = register_histogram_vec_with_registry!(
            "seismosis_storage_db_write_duration_seconds",
            "PostgreSQL upsert_event call latency",
            &["table"],
            vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0],
            registry
        )
        .map_err(|e| StorageError::Metrics(e.to_string()))?;

        Ok(Self {
            messages_consumed_total,
            events_upserted_total,
            events_dead_letter_total,
            processing_duration_seconds,
            db_write_duration_seconds,
        })
    }
}
