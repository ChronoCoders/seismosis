use utoipa::OpenApi;

use crate::model::{
    AftershockForecastResponse, BandStats, EventClassificationResponse, EventListResponse,
    EventResponse, EventsQuery, GrAnalysisResponse, GrMapQuery, GrMapResponse,
    RegionalForecastQuery, RegionalForecastResponse, StatsResponse,
};
use crate::routes::health::{DependencyStatus, HealthResponse};
use crate::routes::{analysis, events, forecasts, health, stats};

/// OpenAPI 3.1 specification for the Seismosis REST API.
///
/// The derive macro generates the spec from the `#[utoipa::path]` annotations
/// on each handler and the `ToSchema` derives on the model types.
/// Served as JSON at `GET /docs/openapi.json` and via Swagger UI at `GET /docs`.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Seismosis API",
        version = "0.2.0",
        description = "Earthquake event query, risk statistics, and Phase 3 forecast API",
        contact(
            name = "Seismosis",
            url = "https://github.com/seismosis"
        ),
        license(name = "MIT")
    ),
    paths(
        health::health,
        events::list_events,
        events::get_event,
        stats::get_stats,
        forecasts::get_aftershock_forecast,
        forecasts::get_regional_forecast,
        analysis::get_gr_analysis,
        analysis::get_gr_map,
        analysis::get_event_classification,
    ),
    components(
        schemas(
            EventResponse,
            EventListResponse,
            EventsQuery,
            StatsResponse,
            BandStats,
            HealthResponse,
            DependencyStatus,
            AftershockForecastResponse,
            RegionalForecastResponse,
            RegionalForecastQuery,
            GrAnalysisResponse,
            GrMapResponse,
            GrMapQuery,
            EventClassificationResponse,
        )
    ),
    tags(
        (name = "events",    description = "Earthquake event queries"),
        (name = "stats",     description = "Aggregate seismic statistics"),
        (name = "health",    description = "Service health check"),
        (name = "forecasts", description = "ETAS aftershock and regional seismicity forecasts"),
        (name = "analysis",  description = "Gutenberg-Richter analysis and event classification"),
    )
)]
pub struct ApiDoc;
