use prometheus::{
    register_histogram_vec_with_registry, register_int_counter_vec_with_registry, HistogramVec,
    IntCounterVec, Registry,
};

use crate::error::ApiError;

/// Prometheus metrics for the API service.
///
/// All metrics are registered against the provided `Registry` (not the global
/// default) so tests can create isolated registries without interference.
pub struct Metrics {
    /// Total HTTP requests, partitioned by method, normalised path, and status.
    pub requests_total: IntCounterVec,
    /// Request latency in seconds, partitioned by method and normalised path.
    pub request_duration_seconds: HistogramVec,
    /// Redis cache hits, partitioned by endpoint name.
    pub cache_hits_total: IntCounterVec,
    /// Redis cache misses, partitioned by endpoint name.
    pub cache_misses_total: IntCounterVec,
}

impl Metrics {
    pub fn new(registry: &Registry) -> Result<Self, ApiError> {
        let requests_total = register_int_counter_vec_with_registry!(
            "seismosis_api_requests_total",
            "Total HTTP requests processed by the API service",
            &["method", "path", "status"],
            registry
        )
        .map_err(|e| ApiError::Metrics(e.to_string()))?;

        let request_duration_seconds = register_histogram_vec_with_registry!(
            "seismosis_api_request_duration_seconds",
            "HTTP request latency in seconds",
            &["method", "path"],
            // Fine-grained buckets up to 5 s to capture DB query latency.
            vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
            registry
        )
        .map_err(|e| ApiError::Metrics(e.to_string()))?;

        let cache_hits_total = register_int_counter_vec_with_registry!(
            "seismosis_api_cache_hits_total",
            "Number of Redis cache hits",
            &["endpoint"],
            registry
        )
        .map_err(|e| ApiError::Metrics(e.to_string()))?;

        let cache_misses_total = register_int_counter_vec_with_registry!(
            "seismosis_api_cache_misses_total",
            "Number of Redis cache misses",
            &["endpoint"],
            registry
        )
        .map_err(|e| ApiError::Metrics(e.to_string()))?;

        Ok(Metrics {
            requests_total,
            request_duration_seconds,
            cache_hits_total,
            cache_misses_total,
        })
    }
}
