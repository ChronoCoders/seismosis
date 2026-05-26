use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// A single earthquake event as returned by the API.
///
/// Coordinates are extracted from the PostGIS `GEOMETRY(POINT, 4326)` column
/// via `ST_X` (longitude) and `ST_Y` (latitude) in the query layer, then
/// returned here as plain floats.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventResponse {
    /// Internal row UUID (stable primary key).
    pub id: Uuid,
    /// External source identifier (e.g. `us6000abcd`). Unique per pipeline.
    pub source_id: String,
    /// Seismic network code (e.g. `us`, `emsc`).
    pub source_network: String,
    /// Time of the seismic event (UTC).
    pub event_time: DateTime<Utc>,
    /// WGS-84 latitude in degrees, [-90, 90].
    pub latitude: f64,
    /// WGS-84 longitude in degrees, [-180, 180].
    pub longitude: f64,
    /// Focal depth in km. `null` when not reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth_km: Option<f64>,
    /// Moment magnitude (or local / surface-wave magnitude where unavailable).
    pub magnitude: f64,
    /// Magnitude scale code (e.g. `Mw`, `Ml`, `mb`).
    pub magnitude_type: String,
    /// Human-readable region name. `null` when not geocoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_name: Option<String>,
    /// Data quality indicator: A (best) through D (worst).
    pub quality_indicator: String,
    /// Time this event was first ingested by the pipeline (UTC).
    pub processed_at: DateTime<Utc>,
    /// Pipeline version that produced the record.
    pub pipeline_version: String,
}

/// Paginated list response for `GET /v1/events`.
///
/// Uses keyset (cursor) pagination rather than OFFSET so performance is stable
/// regardless of result depth. Pass `next_cursor` as the `cursor` query
/// parameter on the next request to fetch the following page. `has_more` is
/// `false` on the last page.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventListResponse {
    pub events: Vec<EventResponse>,
    pub page_size: u32,
    /// Whether more results exist beyond this page.
    pub has_more: bool,
    /// Opaque cursor for the next page; absent when `has_more` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Response body for `GET /v1/stats`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StatsResponse {
    pub bands: Vec<BandStats>,
    /// Time this stats snapshot was computed (UTC).
    pub computed_at: DateTime<Utc>,

    // ── Phase 3 fields ────────────────────────────────────────────────────────
    /// Latest global Gutenberg-Richter b-value (Turkey region, no grid cell).
    /// Absent when the forecast service has not run yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub b_value_region: Option<f64>,
    /// Magnitude of completeness (Mc) for the Turkey region GR fit.
    /// Absent when the forecast service has not run yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mc_region: Option<f64>,
    /// Number of distinct mainshocks with an ETAS forecast computed in the
    /// last 7 days.
    pub active_sequences: i64,
    /// Model version string from the most recently computed ETAS forecast.
    /// Absent when no ETAS runs have been completed yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forecast_model_version: Option<String>,
}

/// Phase 3 supplementary fields fetched alongside band stats.
///
/// Returned from a single async DB call in `db::get_phase3_stats` and merged
/// into [`StatsResponse`] by the stats handler.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct Phase3StatsFields {
    pub b_value_region: Option<f64>,
    pub mc_region: Option<f64>,
    pub active_sequences: i64,
    pub forecast_model_version: Option<String>,
}

/// Per-band statistics as stored in/returned from cache.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BandStats {
    pub band: String,
    pub min_magnitude: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_magnitude: Option<f64>,
    pub count_1h: i64,
    pub count_24h: i64,
    pub count_7d: i64,
    pub count_30d: i64,
    pub max_mag_1h: Option<f64>,
    pub max_mag_24h: Option<f64>,
    pub max_mag_7d: Option<f64>,
    pub max_mag_30d: Option<f64>,
}

/// Query parameters for `GET /v1/events`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct EventsQuery {
    /// Opaque pagination cursor returned by a previous response. Omit for the
    /// first page. Cursors encode the last `event_time` and `source_id` seen.
    pub cursor: Option<String>,
    /// Results per page, 1–1000. Default: 50.
    pub page_size: Option<u32>,
    /// Return events at or after this time (RFC 3339).
    pub start_time: Option<DateTime<Utc>>,
    /// Return events at or before this time (RFC 3339).
    pub end_time: Option<DateTime<Utc>>,
    /// Minimum magnitude (inclusive).
    pub min_magnitude: Option<f64>,
    /// Maximum magnitude (inclusive).
    pub max_magnitude: Option<f64>,
    /// Bounding box south edge (latitude, degrees).
    pub min_lat: Option<f64>,
    /// Bounding box north edge (latitude, degrees).
    pub max_lat: Option<f64>,
    /// Bounding box west edge (longitude, degrees).
    pub min_lon: Option<f64>,
    /// Bounding box east edge (longitude, degrees).
    pub max_lon: Option<f64>,
}

impl EventsQuery {
    pub const DEFAULT_PAGE_SIZE: u32 = 50;
    pub const MAX_PAGE_SIZE: u32 = 1000;
}

/// ETAS aftershock forecast returned by `GET /v1/forecasts/aftershock/:source_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AftershockForecastResponse {
    pub mainshock_source_id: String,
    pub computed_at: DateTime<Utc>,
    pub horizon_days: i32,
    pub min_magnitude: f64,
    /// Expected number of aftershocks M ≥ `min_magnitude` over `horizon_days`.
    pub expected_count: f64,
    /// Probability of at least one aftershock M ≥ `min_magnitude`.
    pub p_at_least_one: f64,
    /// Per-magnitude exceedance probabilities `{magnitude: probability}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub p_exceedance: Option<serde_json::Value>,
    /// Expected aftershock rate per day `[day_1, …, day_N]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_rates: Option<serde_json::Value>,
    /// Name of the seismic zone whose ETAS parameters were used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params_zone: Option<String>,
    /// ETAS parameter snapshot `{mu, K, alpha, c, p, mc}` used for this run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params_snapshot: Option<serde_json::Value>,
    pub model_version: String,
}

/// Placeholder regional seismicity forecast for `GET /v1/forecasts/regional`.
///
/// Returns the latest spatial heatmaps from `seismology.etas_forecasts`, filtered
/// to the requested bounding box where available. An empty `features` list is
/// returned when no ETAS runs have been completed yet.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RegionalForecastResponse {
    #[serde(rename = "type")]
    pub feature_type: String,
    pub features: Vec<serde_json::Value>,
    pub computed_at: DateTime<Utc>,
}

/// Query parameters for `GET /v1/forecasts/regional`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct RegionalForecastQuery {
    pub min_lat: Option<f64>,
    pub max_lat: Option<f64>,
    pub min_lon: Option<f64>,
    pub max_lon: Option<f64>,
    /// Forecast horizon in days (default: 30).
    pub horizon_days: Option<i32>,
}

/// Gutenberg-Richter regression result returned by `GET /v1/analysis/gr`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GrAnalysisResponse {
    pub id: i64,
    pub computed_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_name: Option<String>,
    /// Gutenberg-Richter b-value.
    pub b_value: f64,
    /// Standard error of the b-value estimate.
    pub b_std: f64,
    /// Gutenberg-Richter a-value (log₁₀ seismicity rate at M = 0).
    pub a_value: f64,
    /// Estimated magnitude of completeness.
    pub mc: f64,
    pub n_events: i32,
    pub catalog_start: DateTime<Utc>,
    pub catalog_end: DateTime<Utc>,
    pub model_version: String,
}

/// GeoJSON FeatureCollection of GR spatial cells for `GET /v1/analysis/gr/map`.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GrMapResponse {
    #[serde(rename = "type")]
    pub feature_type: String,
    pub features: Vec<serde_json::Value>,
    pub computed_at: DateTime<Utc>,
}

/// Query parameters for `GET /v1/analysis/gr/map`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct GrMapQuery {
    pub min_lat: Option<f64>,
    pub max_lat: Option<f64>,
    pub min_lon: Option<f64>,
    pub max_lon: Option<f64>,
}

/// Event seismicity classification for `GET /v1/analysis/classification/:source_id`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EventClassificationResponse {
    pub source_id: String,
    /// Source classification: `tectonic`, `induced`, `volcanic`, or `unknown`.
    /// `null` when the classification service has not yet processed this event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_class: Option<String>,
    /// Model confidence in `[0, 1]`. `null` when `event_class` is null.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_confidence: Option<f64>,
}
