use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::Utc;

use crate::{
    error::RequestError,
    model::{EventClassificationResponse, GrAnalysisResponse, GrMapQuery, GrMapResponse},
    routes::AppState,
};

/// Return the latest catalog-wide Gutenberg-Richter regression result.
///
/// Queries `seismology.gr_analysis` for the most recently computed row
/// where `grid_cell IS NULL` (i.e., a full-catalog, non-gridded result).
/// Returns **404** when the forecast service has not run yet.
#[utoipa::path(
    get,
    path = "/v1/analysis/gr",
    tag = "analysis",
    responses(
        (status = 200, description = "Latest catalog-wide GR regression", body = GrAnalysisResponse),
        (status = 404, description = "No GR analysis available yet"),
    )
)]
pub async fn get_gr_analysis(
    State(state): State<AppState>,
) -> Result<Json<GrAnalysisResponse>, RequestError> {
    let result = crate::db::get_gr_analysis(&state.pool).await?;
    match result {
        Some(r) => Ok(Json(r)),
        None => Err(RequestError::NotFound),
    }
}

/// Return a GeoJSON FeatureCollection of spatial Gutenberg-Richter cells.
///
/// Each feature polygon corresponds to a 0.5° grid cell with properties:
/// `b_value`, `b_std`, `a_value`, `mc`, `n_events`, `region_name`.
/// Returns an empty `features` array when no gridded analysis has been run.
#[utoipa::path(
    get,
    path = "/v1/analysis/gr/map",
    tag = "analysis",
    params(GrMapQuery),
    responses(
        (status = 200, description = "Spatial b-value GeoJSON FeatureCollection", body = GrMapResponse),
    )
)]
pub async fn get_gr_map(
    State(state): State<AppState>,
    Query(_q): Query<GrMapQuery>, // bbox fields validated below; spatial filter is a future optimisation
) -> Result<Json<GrMapResponse>, RequestError> {
    for (name, val) in [
        ("min_lat", _q.min_lat),
        ("max_lat", _q.max_lat),
        ("min_lon", _q.min_lon),
        ("max_lon", _q.max_lon),
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

    // Spatial filtering by bbox is reserved for a future optimisation once
    // the heatmap cell schema is stable. All cells are returned for now.
    let features = crate::db::get_gr_cells(&state.pool).await?;

    Ok(Json(GrMapResponse {
        feature_type: "FeatureCollection".to_owned(),
        features,
        computed_at: Utc::now(),
    }))
}

/// Return the seismicity classification for a single event.
///
/// Reads `event_class` and `class_confidence` from `seismology.seismic_events`.
/// Both fields are `null` when the classification service has not yet processed
/// the event. Returns **404** when no event with the given `source_id` exists.
#[utoipa::path(
    get,
    path = "/v1/analysis/classification/{source_id}",
    tag = "analysis",
    params(
        ("source_id" = String, Path, description = "Event source_id (e.g. us6000abcd)")
    ),
    responses(
        (status = 200, description = "Seismicity classification for the event", body = EventClassificationResponse),
        (status = 404, description = "No event found with this source_id"),
    )
)]
pub async fn get_event_classification(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
) -> Result<Json<EventClassificationResponse>, RequestError> {
    let result = crate::db::get_event_classification(&state.pool, &source_id).await?;
    match result {
        Some(r) => Ok(Json(r)),
        None => Err(RequestError::NotFound),
    }
}
