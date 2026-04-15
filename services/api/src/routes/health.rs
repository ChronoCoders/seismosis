use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use utoipa::ToSchema;

use super::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: &'static str,
    pub dependencies: DependencyStatus,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DependencyStatus {
    pub postgres: &'static str,
    pub redis: &'static str,
}

/// Return service health including dependency reachability.
///
/// Always returns `200 OK`. Check `status` for `"ok"` vs `"degraded"`.
/// Dependency fields are `"ok"` or `"unreachable"`.
///
/// Intentionally does not return 503 — load balancers should route to this
/// service regardless; downstream consumers can inspect the body for context.
#[utoipa::path(
    get,
    path = "/health",
    tag = "health",
    responses(
        (status = 200, description = "Service health (check body for degraded status)", body = HealthResponse)
    )
)]
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    let postgres_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();

    let redis_ok = state.cache.ping().await;

    let postgres = if postgres_ok { "ok" } else { "unreachable" };
    let redis = if redis_ok { "ok" } else { "unreachable" };
    let status = if postgres_ok && redis_ok {
        "ok"
    } else {
        "degraded"
    };

    (
        StatusCode::OK,
        Json(HealthResponse {
            status,
            dependencies: DependencyStatus { postgres, redis },
        }),
    )
}
