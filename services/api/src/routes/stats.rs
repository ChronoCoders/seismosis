use axum::{extract::State, Json};
use chrono::Utc;
use tracing::debug;

use crate::{cache, error::RequestError, model::StatsResponse, routes::AppState};

/// Return aggregate seismic statistics grouped by magnitude band and time window.
///
/// Statistics are cached in Redis at key `api:stats` with a 60-second TTL.
/// The cache is populated on the first miss and served directly on subsequent
/// requests within the TTL window.
///
/// **Magnitude bands**
///
/// | Band     | Range     |
/// |----------|-----------|
/// | minor    | < 2.0     |
/// | light    | 2.0 – 4.0 |
/// | moderate | 4.0 – 6.0 |
/// | strong   | 6.0 – 8.0 |
/// | major    | ≥ 8.0     |
///
/// **Time windows**: last 1 h, 24 h, 7 days, 30 days relative to query time.
#[utoipa::path(
    get,
    path = "/v1/stats",
    tag = "stats",
    responses(
        (status = 200, description = "Aggregate statistics", body = StatsResponse),
    )
)]
pub async fn get_stats(State(state): State<AppState>) -> Result<Json<StatsResponse>, RequestError> {
    // Cache hit.
    if let Some(cached) = state.cache.get::<StatsResponse>(cache::STATS_KEY).await {
        state
            .metrics
            .cache_hits_total
            .with_label_values(&["get_stats"])
            .inc();
        return Ok(Json(cached));
    }

    state
        .metrics
        .cache_misses_total
        .with_label_values(&["get_stats"])
        .inc();

    debug!("Stats cache miss — querying DB");

    let bands = crate::db::get_stats(&state.pool).await?;

    let response = StatsResponse {
        bands,
        computed_at: Utc::now(),
    };

    // Cache the result for the configured TTL. Fire-and-forget; the response
    // is already formed and we don't block on the cache write. The JoinHandle
    // is intentionally dropped — Cache::set already logs on failure, and a
    // cache-write panic should not affect the response already sent to the client.
    let cache = state.cache.clone();
    let ttl = state.config.stats_cache_ttl_secs;
    let to_cache = response.clone();
    drop(tokio::spawn(async move {
        cache.set(cache::STATS_KEY, &to_cache, ttl).await;
    })); // intentional drop: cache failure is non-fatal, logged by Cache::set

    Ok(Json(response))
}
