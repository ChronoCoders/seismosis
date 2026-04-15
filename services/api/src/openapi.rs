use utoipa::OpenApi;

use crate::model::{BandStats, EventListResponse, EventResponse, EventsQuery, StatsResponse};
use crate::routes::health::{DependencyStatus, HealthResponse};
use crate::routes::{events, health, stats};

/// OpenAPI 3.1 specification for the Seismosis REST API.
///
/// The derive macro generates the spec from the `#[utoipa::path]` annotations
/// on each handler and the `ToSchema` derives on the model types.
/// Served as JSON at `GET /docs/openapi.json` and via Swagger UI at `GET /docs`.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Seismosis API",
        version = "0.1.0",
        description = "Earthquake event query and risk statistics API — Phase 1",
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
        )
    ),
    tags(
        (name = "events", description = "Earthquake event queries"),
        (name = "stats",  description = "Aggregate seismic statistics"),
        (name = "health", description = "Service health check"),
    )
)]
pub struct ApiDoc;
