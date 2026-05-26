use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::Utc;

use crate::{
    error::RequestError,
    model::{AftershockForecastResponse, RegionalForecastQuery, RegionalForecastResponse},
    routes::AppState,
};

/// Return the latest ETAS aftershock forecast for a given mainshock.
///
/// Queries `seismology.etas_forecasts` for the most recently computed row
/// matching `mainshock_source_id`. Returns **404** when no forecast exists
/// (either the mainshock is unknown or the forecast service has not run yet).
#[utoipa::path(
    get,
    path = "/v1/forecasts/aftershock/{source_id}",
    tag = "forecasts",
    params(
        ("source_id" = String, Path, description = "Mainshock source_id (e.g. us6000abcd)")
    ),
    responses(
        (status = 200, description = "Latest ETAS aftershock forecast", body = AftershockForecastResponse),
        (status = 404, description = "No forecast found for this mainshock"),
    )
)]
pub async fn get_aftershock_forecast(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
) -> Result<Json<AftershockForecastResponse>, RequestError> {
    let forecast = crate::db::get_aftershock_forecast(&state.pool, &source_id).await?;
    match forecast {
        Some(f) => Ok(Json(f)),
        None => Err(RequestError::NotFound),
    }
}

/// Return a regional seismicity forecast as a GeoJSON FeatureCollection.
///
/// Aggregates the latest spatial heatmaps from `seismology.etas_forecasts`
/// for the requested forecast horizon. Returns an empty `features` array
/// when no ETAS runs have completed yet. Bounding-box filtering is applied
/// client-side once the GeoJSON is assembled; a future optimisation can push
/// the spatial filter into the query once the heatmap cell schema is stable.
#[utoipa::path(
    get,
    path = "/v1/forecasts/regional",
    tag = "forecasts",
    params(RegionalForecastQuery),
    responses(
        (status = 200, description = "Regional seismicity forecast GeoJSON", body = RegionalForecastResponse),
    )
)]
pub async fn get_regional_forecast(
    State(state): State<AppState>,
    Query(q): Query<RegionalForecastQuery>,
) -> Result<Json<RegionalForecastResponse>, RequestError> {
    for (name, val) in [
        ("min_lat", q.min_lat),
        ("max_lat", q.max_lat),
        ("min_lon", q.min_lon),
        ("max_lon", q.max_lon),
    ] {
        if let Some(v) = val {
            if !v.is_finite() {
                return Err(RequestError::BadParam {
                    param: name,
                    detail: format!("{} is not a finite number", v),
                });
            }
        }
    }

    let horizon = q.horizon_days.unwrap_or(30).clamp(1, 365);
    let heatmaps = crate::db::get_regional_heatmaps(&state.pool, horizon).await?;

    // Flatten any nested FeatureCollection arrays into a flat features list.
    let mut features: Vec<serde_json::Value> = Vec::new();
    for h in heatmaps {
        if let Some(inner) = h.get("features").and_then(|v| v.as_array()) {
            features.extend_from_slice(inner);
        }
    }

    Ok(Json(RegionalForecastResponse {
        feature_type: "FeatureCollection".to_owned(),
        features,
        computed_at: Utc::now(),
    }))
}
