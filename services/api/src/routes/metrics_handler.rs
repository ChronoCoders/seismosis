use axum::extract::State;
use prometheus::{Encoder, TextEncoder};
use tracing::error;

use crate::routes::AppState;

/// Serve Prometheus metrics in text exposition format.
///
/// The `/metrics` route is served on the same HTTP port as the API
/// (`HTTP_PORT`, default 8000). `METRICS_PORT` (default 9093) is reserved
/// for a potential future dedicated metrics listener.
pub async fn metrics(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let encoder = TextEncoder::new();
    let mut buf = Vec::new();

    if let Err(e) = encoder.encode(&state.prom_registry.gather(), &mut buf) {
        error!(error = %e, "Failed to encode Prometheus metrics");
    }

    (
        [(
            axum::http::header::CONTENT_TYPE,
            encoder.format_type().to_owned(),
        )],
        buf,
    )
}
