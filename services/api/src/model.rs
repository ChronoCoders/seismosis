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
#[derive(Debug, Serialize, ToSchema)]
pub struct EventListResponse {
    pub events: Vec<EventResponse>,
    pub page: u32,
    pub page_size: u32,
    /// Total number of rows matching the applied filters (before pagination).
    pub total: i64,
}

/// Response body for `GET /v1/stats`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct StatsResponse {
    pub bands: Vec<BandStats>,
    /// Time this stats snapshot was computed (UTC).
    pub computed_at: DateTime<Utc>,
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
    /// Page number, 1-based. Default: 1.
    pub page: Option<u32>,
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
    pub const DEFAULT_PAGE: u32 = 1;
    pub const DEFAULT_PAGE_SIZE: u32 = 50;
    pub const MAX_PAGE_SIZE: u32 = 1000;
}
