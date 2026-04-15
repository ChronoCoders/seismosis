pub mod events;
pub mod health;
pub mod metrics_handler;
pub mod stats;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    error_handling::HandleErrorLayer,
    extract::{MatchedPath, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::get,
    BoxError, Router,
};
use prometheus::Registry;
use tower::ServiceBuilder;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

use crate::{cache::Cache, config::Config, metrics::Metrics, openapi::ApiDoc};

/// Shared application state cloned into every request handler.
///
/// All fields are cheaply cloneable (`Arc` or `ConnectionManager`/`PgPool`
/// which are `Clone` themselves).
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub cache: Cache,
    pub metrics: Arc<Metrics>,
    pub config: Arc<Config>,
    /// The Prometheus registry is stored here so the metrics handler can
    /// call `.gather()` without going through a global singleton.
    pub prom_registry: Arc<Registry>,
}

/// Build the main application `Router`.
///
/// Layers applied (outermost → innermost):
/// 1. **TimeoutLayer** — returns `408 Request Timeout` after
///    `config.request_timeout_secs` if the inner service hasn't responded.
/// 2. **Metrics middleware** — records request count and latency using the
///    matched (template) path so Prometheus label cardinality stays bounded.
///
/// # Note on `/metrics`
///
/// The Prometheus scrape endpoint is served on the same TCP port as the
/// application API (`HTTP_PORT`). In a future phase this should move to a
/// dedicated internal port so metrics are not reachable by API clients.
/// TODO(phase-2): bind `/metrics` on a separate internal port.
pub fn build_router(state: AppState) -> Router {
    let timeout_secs = state.config.request_timeout_secs;

    Router::new()
        .route("/health", get(health::health))
        .route("/v1/events", get(events::list_events))
        .route("/v1/events/:id", get(events::get_event))
        .route("/v1/stats", get(stats::get_stats))
        // NOTE: /metrics is on the public port in Phase 1 — see module doc.
        .route("/metrics", get(metrics_handler::metrics))
        .merge(SwaggerUi::new("/docs").url("/docs/openapi.json", ApiDoc::openapi()))
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, track_metrics))
        .layer(
            ServiceBuilder::new()
                // Convert timeout errors into 408 responses before they
                // propagate to the client as 500s.
                .layer(HandleErrorLayer::new(|_: BoxError| async {
                    StatusCode::REQUEST_TIMEOUT
                }))
                .timeout(Duration::from_secs(timeout_secs)),
        )
}

/// Axum middleware that records per-request Prometheus metrics.
///
/// The matched path template (e.g. `/v1/events/:id`) is extracted from
/// `MatchedPath` so label cardinality is bounded regardless of path
/// parameters. Falls back to `"unknown"` for unmatched routes (404s).
async fn track_metrics(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|mp| mp.as_str().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());

    let start = Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed().as_secs_f64();

    let status = response.status().as_u16().to_string();

    state
        .metrics
        .requests_total
        .with_label_values(&[&method, &path, &status])
        .inc();

    state
        .metrics
        .request_duration_seconds
        .with_label_values(&[&method, &path])
        .observe(elapsed);

    response
}
